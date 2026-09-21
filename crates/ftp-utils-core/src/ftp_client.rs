//! Real `FtpConnection` implementation backed by the `suppaftp` crate,
//! supporting both plain FTP and explicit FTPS (AUTH TLS).

use suppaftp::list::ListParser;
use suppaftp::native_tls::TlsConnector;
use suppaftp::{FtpStream, NativeTlsConnector, NativeTlsFtpStream};

use crate::remote::{FtpConnection, FtpConnectionError, RawRemoteEntry};

/// A live FTP or FTPS connection. Kept as an enum (rather than a trait
/// object) because `suppaftp`'s plain and TLS streams are distinct
/// concrete types selected once at connect time.
pub enum SuppaFtpConnection {
    Plain(FtpStream),
    Tls(NativeTlsFtpStream),
}

impl SuppaFtpConnection {
    /// Connects and authenticates. Uses explicit FTPS (AUTH TLS upgrade on
    /// the plain control channel) when `ftps` is true.
    pub fn connect(
        host: &str,
        port: u16,
        user: &str,
        password: &str,
        ftps: bool,
    ) -> Result<Self, FtpConnectionError> {
        let address = format!("{host}:{port}");

        if ftps {
            let stream = NativeTlsFtpStream::connect(&address)
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            let connector = TlsConnector::new().map_err(|e| FtpConnectionError(e.to_string()))?;
            let mut stream = stream
                .into_secure(NativeTlsConnector::from(connector), host)
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            stream
                .login(user, password)
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            Ok(SuppaFtpConnection::Tls(stream))
        } else {
            let mut stream =
                FtpStream::connect(&address).map_err(|e| FtpConnectionError(e.to_string()))?;
            stream
                .login(user, password)
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

impl FtpConnection for SuppaFtpConnection {
    fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
        let lines = match self {
            SuppaFtpConnection::Plain(stream) => stream.list(Some(path)),
            SuppaFtpConnection::Tls(stream) => stream.list(Some(path)),
        }
        .map_err(|e| FtpConnectionError(e.to_string()))?;

        let mut entries = Vec::new();
        for line in lines {
            let Ok(file) = ListParser::parse_posix(&line) else {
                continue;
            };
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
}
