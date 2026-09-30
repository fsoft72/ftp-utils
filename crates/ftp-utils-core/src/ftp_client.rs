//! Real `FtpConnection` implementation backed by the `suppaftp` crate,
//! supporting both plain FTP and explicit FTPS (AUTH TLS).

use std::io::{Read, Write};

use suppaftp::list::ListParser;
use suppaftp::native_tls::TlsConnector;
use suppaftp::types::FileType;
use suppaftp::{FtpStream, NativeTlsConnector, NativeTlsFtpStream};

use crate::remote::{FtpConnection, FtpConnectionError, RawRemoteEntry};

/// Size of the buffer used when streaming downloads.
const TRANSFER_CHUNK_SIZE: usize = 64 * 1024;

/// A live FTP or FTPS connection. Kept as an enum (rather than a trait
/// object) because `suppaftp`'s plain and TLS streams are distinct
/// concrete types selected once at connect time.
pub enum SuppaFtpConnection {
    Plain(FtpStream),
    Tls(NativeTlsFtpStream),
}

impl SuppaFtpConnection {
    /// Connects and authenticates. Uses explicit FTPS (AUTH TLS upgrade on
    /// the plain control channel) when `ftps` is true. Switches to binary
    /// (`TYPE I`) transfer mode, since the FTP default of ASCII mode
    /// rewrites line endings in transit and would corrupt size/hash
    /// comparisons for any file containing `\n`.
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
    ) -> Result<Self, FtpConnectionError> {
        let address = format!("{host}:{port}");

        if ftps {
            let stream = NativeTlsFtpStream::connect(&address)
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            let mut connector_builder = TlsConnector::builder();
            if insecure_tls {
                connector_builder
                    .danger_accept_invalid_certs(true)
                    .danger_accept_invalid_hostnames(true);
            }
            let connector = connector_builder
                .build()
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            let mut stream = stream
                .into_secure(NativeTlsConnector::from(connector), host)
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            stream
                .login(user, password)
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            stream
                .transfer_type(FileType::Binary)
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            Ok(SuppaFtpConnection::Tls(stream))
        } else {
            let mut stream =
                FtpStream::connect(&address).map_err(|e| FtpConnectionError(e.to_string()))?;
            stream
                .login(user, password)
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            stream
                .transfer_type(FileType::Binary)
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            Ok(SuppaFtpConnection::Plain(stream))
        }
    }

    /// Sends QUIT and closes the connection. Errors are ignored: by the
    /// time this is called, the comparison has already finished.
    pub fn close(self) {
        match self {
            SuppaFtpConnection::Plain(mut stream) => {
                let _ = stream.quit();
            }
            SuppaFtpConnection::Tls(mut stream) => {
                let _ = stream.quit();
            }
        }
    }
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
        let lines = match self {
            SuppaFtpConnection::Plain(stream) => stream.list(Some(path)),
            SuppaFtpConnection::Tls(stream) => stream.list(Some(path)),
        }
        .map_err(|e| FtpConnectionError(e.to_string()))?;

        parse_listing(&lines)
    }

    fn try_hash(&mut self, _path: &str) -> Option<String> {
        // suppaftp (v10, as researched via its current docs) does not
        // expose a public API for sending non-standard hash commands
        // (XMD5/MD5/HASH). Always fall back to download + local MD5; see
        // hash::apply_hash_comparison.
        None
    }

    fn retr_to_buffer(&mut self, path: &str) -> Result<Vec<u8>, FtpConnectionError> {
        let cursor = match self {
            SuppaFtpConnection::Plain(stream) => stream.retr_as_buffer(path),
            SuppaFtpConnection::Tls(stream) => stream.retr_as_buffer(path),
        }
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
        let result = match self {
            SuppaFtpConnection::Plain(stream) => stream.retr(path, copy),
            SuppaFtpConnection::Tls(stream) => stream.retr(path, copy),
        };
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
        match self {
            SuppaFtpConnection::Plain(stream) => stream.put_file(path, &mut input),
            SuppaFtpConnection::Tls(stream) => stream.put_file(path, &mut input),
        }
        .map_err(|e| FtpConnectionError(e.to_string()))?;
        Ok(())
    }

    fn delete(&mut self, path: &str) -> Result<(), FtpConnectionError> {
        match self {
            SuppaFtpConnection::Plain(stream) => stream.rm(path),
            SuppaFtpConnection::Tls(stream) => stream.rm(path),
        }
        .map_err(|e| FtpConnectionError(e.to_string()))
    }

    fn create_dir(&mut self, path: &str) -> Result<(), FtpConnectionError> {
        match self {
            SuppaFtpConnection::Plain(stream) => stream.mkdir(path),
            SuppaFtpConnection::Tls(stream) => stream.mkdir(path),
        }
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
