//! Real `FtpConnection` implementation backed by the `suppaftp` crate,
//! supporting both plain FTP and explicit FTPS (AUTH TLS).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use suppaftp::list::ListParser;
use suppaftp::native_tls::TlsConnector;
use suppaftp::Status;
use suppaftp::types::FileType;
use suppaftp::{FtpStream, ImplFtpStream, NativeTlsConnector, NativeTlsFtpStream, TlsStream};

use crate::connection::RemoteParams;
use crate::remote::{FtpConnection, FtpConnectionError, RawRemoteEntry};

/// Size of the buffer used when streaming downloads.
const TRANSFER_CHUNK_SIZE: usize = 64 * 1024;

/// Converts a suppaftp error into this crate's error type.
fn _ftp_error(e: suppaftp::FtpError) -> FtpConnectionError {
    FtpConnectionError(e.to_string())
}

/// Opens a control connection to `host:port`, trying each resolved
/// address with `timeout`, and applies `timeout` as the read/write timeout
/// of the control channel and of every passive data channel it opens.
fn _connect_with_timeout<T: TlsStream>(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<ImplFtpStream<T>, FtpConnectionError> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|e| FtpConnectionError(format!("cannot resolve {host}:{port}: {e}")))?;

    let mut last_error = FtpConnectionError(format!("no addresses found for {host}:{port}"));
    for address in addresses {
        // Build the TCP stream ourselves so its timeouts are in place
        // before suppaftp reads the server greeting.
        let tcp = match TcpStream::connect_timeout(&address, timeout) {
            Ok(tcp) => tcp,
            Err(e) => {
                last_error = FtpConnectionError(format!("cannot connect to {address}: {e}"));
                continue;
            }
        };
        tcp.set_read_timeout(Some(timeout)).map_err(|e| FtpConnectionError(e.to_string()))?;
        tcp.set_write_timeout(Some(timeout)).map_err(|e| FtpConnectionError(e.to_string()))?;

        let stream = match ImplFtpStream::<T>::connect_with_stream(tcp) {
            Ok(stream) => stream,
            Err(e) => {
                last_error = _ftp_error(e);
                continue;
            }
        };

        return Ok(stream.passive_stream_builder(move |data_address| {
            let data = TcpStream::connect_timeout(&data_address, timeout).map_err(suppaftp::FtpError::ConnectionError)?;
            data.set_read_timeout(Some(timeout)).map_err(suppaftp::FtpError::ConnectionError)?;
            data.set_write_timeout(Some(timeout)).map_err(suppaftp::FtpError::ConnectionError)?;
            Ok(data)
        }));
    }

    Err(last_error)
}

/// Logs in and switches to binary transfer mode; shared by the plain and
/// TLS connect paths, which differ only in the stream type.
fn _login_binary<T: TlsStream>(
    stream: &mut ImplFtpStream<T>,
    user: &str,
    password: &str,
) -> Result<(), FtpConnectionError> {
    stream.login(user, password).map_err(_ftp_error)?;
    stream.transfer_type(FileType::Binary).map_err(_ftp_error)
}

/// Server-side hash commands probed by `try_hash`, in order of preference.
const HASH_COMMANDS: [&str; 2] = ["XMD5", "MD5"];

/// Length of an MD5 digest in hex characters.
const MD5_HEX_LEN: usize = 32;

/// `suppaftp`'s plain and TLS streams are distinct concrete types selected
/// once at connect time, so they are kept in an enum (rather than a trait
/// object).
enum Stream {
    Plain(FtpStream),
    Tls(NativeTlsFtpStream),
}

/// What is known about the server's support for hash commands.
#[derive(Clone, Copy)]
enum HashSupport {
    /// No hash command has been tried yet on this connection.
    Unprobed,
    /// This command worked earlier; use it directly.
    Command(&'static str),
    /// Every command failed once; don't try again (fall back to download).
    Unsupported,
}

/// A live FTP or FTPS connection.
pub struct SuppaFtpConnection {
    stream: Stream,
    hash_support: HashSupport,
}

/// Runs `$body` with `$stream` bound to the underlying suppaftp stream,
/// whichever variant it is, so each operation is written once.
macro_rules! with_stream {
    ($conn:expr, $stream:ident => $body:expr) => {
        match &mut $conn.stream {
            Stream::Plain($stream) => $body,
            Stream::Tls($stream) => $body,
        }
    };
}

impl SuppaFtpConnection {
    fn new(stream: Stream) -> Self {
        Self { stream, hash_support: HashSupport::Unprobed }
    }

    /// Connects and authenticates. Uses explicit FTPS (AUTH TLS upgrade on
    /// the plain control channel) when `ftps` is true. Switches to binary
    /// (`TYPE I`) transfer mode, since the FTP default of ASCII mode
    /// rewrites line endings in transit and would corrupt size/hash
    /// comparisons for any file containing `\n`.
    ///
    /// `timeout` bounds the TCP connect and every later read/write on the
    /// control and data channels, so a dead or stalled server fails the
    /// operation instead of hanging it.
    ///
    /// When `ftps` is true, `insecure_tls` controls certificate
    /// validation: false (default) validates the server's certificate and
    /// hostname normally; true accepts any certificate, including expired,
    /// self-signed, or hostname-mismatched ones. Only set `insecure_tls`
    /// for servers with certificates you can't otherwise validate (e.g.
    /// internal self-signed setups) - it removes protection against
    /// man-in-the-middle attacks.
    pub fn connect(
        host: &str,
        port: u16,
        user: &str,
        password: &str,
        ftps: bool,
        insecure_tls: bool,
        timeout: Duration,
    ) -> Result<Self, FtpConnectionError> {
        if !ftps {
            let mut stream: FtpStream = _connect_with_timeout(host, port, timeout)?;
            _login_binary(&mut stream, user, password)?;
            return Ok(Self::new(Stream::Plain(stream)));
        }

        let stream: NativeTlsFtpStream = _connect_with_timeout(host, port, timeout)?;
        let mut connector_builder = TlsConnector::builder();
        if insecure_tls {
            connector_builder
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true);
        }
        let connector = connector_builder.build().map_err(|e| FtpConnectionError(e.to_string()))?;
        let mut stream = stream
            .into_secure(NativeTlsConnector::from(connector), host)
            .map_err(_ftp_error)?;
        _login_binary(&mut stream, user, password)?;
        Ok(Self::new(Stream::Tls(stream)))
    }

    /// Connects using resolved `RemoteParams` and `password`; see
    /// `connect` for the individual settings.
    pub fn connect_params(params: &RemoteParams, password: &str) -> Result<Self, FtpConnectionError> {
        Self::connect(
            &params.host,
            params.port,
            &params.user,
            password,
            params.ftps,
            params.insecure_tls,
            Duration::from_secs(params.timeout_secs),
        )
    }

    /// Sends QUIT and closes the connection. Errors are ignored: by the
    /// time this is called, the comparison has already finished.
    pub fn close(mut self) {
        let _ = with_stream!(self, stream => stream.quit());
    }

    /// Sends one hash command (`XMD5 <path>` or `MD5 <path>`) and extracts
    /// the digest from the reply, or `None` if the server rejects it or
    /// replies with something that doesn't contain an MD5.
    fn _send_hash_command(&mut self, command: &str, path: &str) -> Option<String> {
        let expected = [Status::Unknown, Status::CommandOk, Status::File, Status::RequestedFileActionOk];
        let response = with_stream!(self, stream => stream.custom_command(format!("{command} {path}"), &expected)).ok()?;
        parse_hash_response(&response.as_string().ok()?)
    }
}

/// Extracts an MD5 digest from a hash command reply such as
/// `251 d41d8cd98f00b204e9800998ecf8427e` or `250 <md5> <path>`: the first
/// whitespace-separated token that is exactly 32 hex digits, lowercased.
/// Anything else (an error text, a different hash algorithm) is `None`.
fn parse_hash_response(reply: &str) -> Option<String> {
    reply
        .split_whitespace()
        .find(|token| token.len() == MD5_HEX_LEN && token.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(|token| token.to_ascii_lowercase())
}

/// Parses raw `LIST` output lines into entries. Blank lines and the
/// `total N` header some servers emit are skipped, as are `.`/`..`. Any
/// other line that neither the POSIX nor the DOS parser understands is an
/// error: dropping it silently would make the file vanish from the remote
/// tree and produce false `LocalOnly` results.
fn parse_listing(lines: &[String]) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
    let mut entries = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("total ") {
            continue;
        }
        let file = ListParser::parse_posix(trimmed)
            .or_else(|_| ListParser::parse_dos(trimmed))
            .map_err(|e| FtpConnectionError(format!("cannot parse directory listing line '{line}': {e}")))?;
        let name = file.name();
        if name == "." || name == ".." {
            continue;
        }
        entries.push(RawRemoteEntry {
            name: name.to_string(),
            is_dir: file.is_directory(),
            size: file.size() as u64,
        });
    }
    Ok(entries)
}

impl FtpConnection for SuppaFtpConnection {
    fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
        let lines = with_stream!(self, stream => stream.list(Some(path)))
        .map_err(|e| FtpConnectionError(e.to_string()))?;

        parse_listing(&lines)
    }

    /// Asks the server for the file's MD5 with `XMD5`, then `MD5`. The
    /// first command that works is remembered for the rest of the
    /// connection; if none works, later calls return `None` immediately
    /// (the caller then downloads and hashes locally).
    fn try_hash(&mut self, path: &str) -> Option<String> {
        let candidates: Vec<&'static str> = match self.hash_support {
            HashSupport::Unsupported => return None,
            HashSupport::Command(command) => vec![command],
            HashSupport::Unprobed => HASH_COMMANDS.to_vec(),
        };

        for command in candidates {
            if let Some(hash) = self._send_hash_command(command, path) {
                self.hash_support = HashSupport::Command(command);
                return Some(hash);
            }
        }

        if matches!(self.hash_support, HashSupport::Unprobed) {
            self.hash_support = HashSupport::Unsupported;
        }
        None
    }

    fn retr_to_buffer(&mut self, path: &str) -> Result<Vec<u8>, FtpConnectionError> {
        let cursor = with_stream!(self, stream => stream.retr_as_buffer(path))
        .map_err(|e| FtpConnectionError(e.to_string()))?;
        Ok(cursor.into_inner())
    }

    fn retr_to_writer(&mut self, path: &str, out: &mut dyn Write) -> Result<u64, FtpConnectionError> {
        // Distinguish a failure writing to `out` (e.g. disk full) from a
        // network/protocol failure, since suppaftp reports both alike.
        let mut write_error: Option<std::io::Error> = None;
        let copy = |reader: &mut dyn Read| -> Result<u64, suppaftp::FtpError> {
            let mut buf = [0u8; TRANSFER_CHUNK_SIZE];
            let mut total = 0u64;
            loop {
                let n = reader.read(&mut buf).map_err(suppaftp::FtpError::ConnectionError)?;
                if n == 0 {
                    return Ok(total);
                }
                if let Err(e) = out.write_all(&buf[..n]) {
                    write_error = Some(e);
                    return Err(suppaftp::FtpError::BadResponse);
                }
                total += n as u64;
            }
        };
        let result = with_stream!(self, stream => stream.retr(path, copy));
        match (result, write_error) {
            (_, Some(e)) => Err(FtpConnectionError(format!("cannot write downloaded data: {e}"))),
            (Ok(total), None) => Ok(total),
            (Err(e), None) => Err(FtpConnectionError(e.to_string())),
        }
    }

    fn store_from_buffer(&mut self, path: &str, data: &[u8]) -> Result<(), FtpConnectionError> {
        self.store_from_reader(path, &mut std::io::Cursor::new(data))
    }

    fn store_from_reader(&mut self, path: &str, mut input: &mut dyn Read) -> Result<(), FtpConnectionError> {
        with_stream!(self, stream => stream.put_file(path, &mut input))
        .map_err(|e| FtpConnectionError(e.to_string()))?;
        Ok(())
    }

    fn delete(&mut self, path: &str) -> Result<(), FtpConnectionError> {
        with_stream!(self, stream => stream.rm(path))
        .map_err(|e| FtpConnectionError(e.to_string()))
    }

    fn create_dir(&mut self, path: &str) -> Result<(), FtpConnectionError> {
        with_stream!(self, stream => stream.mkdir(path))
        .map_err(|e| FtpConnectionError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(input: &[&str]) -> Vec<String> {
        input.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn connect_times_out_when_server_never_sends_a_greeting() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // Accept and hold the socket open without ever writing to it.
        let holder = std::thread::spawn(move || listener.accept().map(|(socket, _)| {
            std::thread::sleep(Duration::from_millis(1500));
            drop(socket);
        }));

        let started = std::time::Instant::now();
        let result = SuppaFtpConnection::connect("127.0.0.1", port, "u", "p", false, false, Duration::from_millis(300));

        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(1), "took {:?}", started.elapsed());
        holder.join().unwrap().unwrap();
    }

    #[test]
    fn connect_fails_fast_when_nothing_listens() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();

        let result = SuppaFtpConnection::connect("127.0.0.1", port, "u", "p", false, false, Duration::from_millis(300));

        assert!(result.is_err());
    }

    #[test]
    fn parses_hash_from_typical_replies() {
        let md5 = "d41d8cd98f00b204e9800998ecf8427e";

        assert_eq!(parse_hash_response(&format!("251 {md5}")), Some(md5.to_string()));
        assert_eq!(parse_hash_response(&format!("250 {} /a/b.txt", md5.to_uppercase())), Some(md5.to_string()));
        assert_eq!(parse_hash_response(&format!("213 {md5}\r\n")), Some(md5.to_string()));
    }

    #[test]
    fn rejects_replies_without_an_md5() {
        assert_eq!(parse_hash_response("500 Unknown command"), None);
        assert_eq!(parse_hash_response("250 da39a3ee5e6b4b0d3255bfef95601890afd80709"), None);
        assert_eq!(parse_hash_response("251 zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"), None);
        assert_eq!(parse_hash_response(""), None);
    }

    #[test]
    fn parses_posix_files_and_directories() {
        let entries = parse_listing(&lines(&[
            "-rw-r--r--   1 user group      1234 Jan 10 12:00 a.txt",
            "drwxr-xr-x   2 user group      4096 Jan 10 12:00 sub",
        ]))
        .unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "a.txt");
        assert!(!entries[0].is_dir);
        assert_eq!(entries[0].size, 1234);
        assert_eq!(entries[1].name, "sub");
        assert!(entries[1].is_dir);
    }

    #[test]
    fn skips_blank_total_dot_and_dotdot_lines() {
        let entries = parse_listing(&lines(&[
            "total 8",
            "",
            "drwxr-xr-x   2 user group      4096 Jan 10 12:00 .",
            "drwxr-xr-x   2 user group      4096 Jan 10 12:00 ..",
            "-rw-r--r--   1 user group         1 Jan 10 12:00 keep.txt",
        ]))
        .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "keep.txt");
    }

    #[test]
    fn errors_on_unparseable_line_instead_of_dropping_it() {
        let result = parse_listing(&lines(&[
            "-rw-r--r--   1 user group         1 Jan 10 12:00 ok.txt",
            "this is not a listing line",
        ]));

        let err = result.unwrap_err();
        assert!(err.0.contains("this is not a listing line"));
    }
}
