//! Streaming remote file downloads shared by the tools: binary mode, via a
//! temporary part file renamed on success, skipping files that already
//! exist locally with the same size.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crate::paths::validate_relative_path;
use crate::remote::{self, FtpConnection, FtpConnectionError, RemoteEntry};

crate::message_error! {
    /// A download failed or was refused; the message names the file.
    pub DownloadError
}

impl From<FtpConnectionError> for DownloadError {
    fn from(e: FtpConnectionError) -> Self {
        DownloadError(e.to_string())
    }
}

impl From<std::io::Error> for DownloadError {
    fn from(e: std::io::Error) -> Self {
        DownloadError(e.to_string())
    }
}

/// Downloads remote files into one directory, recreating subdirectories.
/// A file already there with the same size is skipped. Each file is
/// written to a temporary `.<name>.ftp-part` file and renamed on success,
/// so a failed transfer never leaves a truncated file under its final name.
pub struct Downloader<'a> {
    dest: &'a Path,
    remote_root: &'a str,
    verbose: bool,
    /// Files actually transferred.
    pub downloaded: usize,
    /// Files skipped because a same-size copy was already present.
    pub skipped: usize,
}

impl<'a> Downloader<'a> {
    /// Creates a downloader writing under `dest` for files found under
    /// `remote_root` on the server. With `verbose`, prints each transfer
    /// to stderr.
    pub fn new(dest: &'a Path, remote_root: &'a str, verbose: bool) -> Self {
        Self { dest, remote_root, verbose, downloaded: 0, skipped: 0 }
    }

    /// Fetches one listed file, or skips it if it is already up to date.
    /// Refuses relative paths that are absolute or contain `..`.
    pub fn fetch<C: FtpConnection>(&mut self, conn: &mut C, entry: &RemoteEntry) -> Result<(), DownloadError> {
        validate_relative_path(&entry.relative_path)
            .map_err(|e| DownloadError(format!("refusing to download {}: {e}", entry.relative_path)))?;

        let target = self.dest.join(&entry.relative_path);
        if fs::metadata(&target).is_ok_and(|m| m.is_file() && m.len() == entry.size) {
            self.skipped += 1;
            return Ok(());
        }

        if self.verbose {
            eprintln!("Downloading {}", entry.relative_path);
        }
        download_one(conn, &remote::join_remote(self.remote_root, &entry.relative_path), &target)
            .map_err(|e| DownloadError(format!("failed to download {}: {e}", entry.relative_path)))?;
        self.downloaded += 1;
        Ok(())
    }
}

/// Streams `remote_path` to `target` through a temporary part file.
fn download_one<C: FtpConnection>(conn: &mut C, remote_path: &str, target: &Path) -> Result<(), DownloadError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let part: PathBuf = target.with_file_name(format!(".{name}.ftp-part"));

    let result = File::create(&part)
        .map_err(DownloadError::from)
        .and_then(|mut file| conn.retr_to_writer(remote_path, &mut file).map(|_| ()).map_err(DownloadError::from));

    match result {
        Ok(()) => Ok(fs::rename(&part, target)?),
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockFtpConnection;

    fn entry(path: &str, size: u64) -> RemoteEntry {
        RemoteEntry { relative_path: path.to_string(), size, modified: None }
    }

    fn server_with(path: &str, content: &[u8]) -> MockFtpConnection {
        let mut conn = MockFtpConnection::default();
        conn.files.insert(path.to_string(), content.to_vec());
        conn
    }

    #[test]
    fn downloads_into_subdirectories_and_leaves_no_part_file() {
        let dest = tempfile::tempdir().unwrap();
        let mut conn = server_with("/remote/sub/b.txt", b"hello");
        let mut downloader = Downloader::new(dest.path(), "/remote", false);

        downloader.fetch(&mut conn, &entry("sub/b.txt", 5)).unwrap();

        assert_eq!(std::fs::read(dest.path().join("sub/b.txt")).unwrap(), b"hello");
        assert!(!dest.path().join("sub/.b.txt.ftp-part").exists());
        assert_eq!((downloader.downloaded, downloader.skipped), (1, 0));
    }

    #[test]
    fn skips_same_size_and_replaces_different_size() {
        let dest = tempfile::tempdir().unwrap();
        std::fs::write(dest.path().join("a.txt"), b"HELLO").unwrap(); // same size: kept
        let mut conn = server_with("/remote/a.txt", b"hello");
        let mut downloader = Downloader::new(dest.path(), "/remote", false);

        downloader.fetch(&mut conn, &entry("a.txt", 5)).unwrap();
        assert_eq!(std::fs::read(dest.path().join("a.txt")).unwrap(), b"HELLO");
        assert_eq!((downloader.downloaded, downloader.skipped), (0, 1));

        std::fs::write(dest.path().join("a.txt"), b"old").unwrap(); // different size: replaced
        downloader.fetch(&mut conn, &entry("a.txt", 5)).unwrap();
        assert_eq!(std::fs::read(dest.path().join("a.txt")).unwrap(), b"hello");
    }

    #[test]
    fn refuses_paths_escaping_the_destination() {
        let dest = tempfile::tempdir().unwrap();
        let mut downloader = Downloader::new(dest.path(), "/remote", false);

        let err = downloader.fetch(&mut MockFtpConnection::default(), &entry("../evil.txt", 1)).unwrap_err();

        assert!(err.to_string().contains("evil.txt"), "{err}");
    }

    #[test]
    fn failed_transfer_names_the_file_and_leaves_no_partial_file() {
        let dest = tempfile::tempdir().unwrap();
        let mut downloader = Downloader::new(dest.path(), "/remote", false);

        let err = downloader.fetch(&mut MockFtpConnection::default(), &entry("a.txt", 5)).unwrap_err();

        assert!(err.to_string().contains("a.txt"), "{err}");
        assert!(!dest.path().join("a.txt").exists());
        assert!(!dest.path().join(".a.txt.ftp-part").exists());
    }
}
