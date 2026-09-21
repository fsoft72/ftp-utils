//! Shared FTP/FTPS client, directory comparison, and diff logic used by
//! all tools in the ftp-utils suite.
//!
//! See `docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md`
//! for the design this crate implements.

pub mod compare;
pub mod diff;
pub mod exclude;
pub mod ftp_client;
pub mod hash;
pub mod local;
pub mod remote;

pub use diff::{DiffEntry, DiffStatus};
pub use remote::{FtpConnection, FtpConnectionError, RawRemoteEntry};

use std::path::PathBuf;

/// Inputs to a single `compare` run.
pub struct CompareOptions {
    pub local_dir: PathBuf,
    pub remote_dir: String,
    pub excludes: Vec<String>,
    pub hash: bool,
}

/// Error from a top-level `compare` call.
#[derive(Debug)]
pub enum CompareError {
    Io(std::io::Error),
    Ftp(FtpConnectionError),
}

impl std::fmt::Display for CompareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompareError::Io(e) => write!(f, "{e}"),
            CompareError::Ftp(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CompareError {}

impl From<std::io::Error> for CompareError {
    fn from(e: std::io::Error) -> Self {
        CompareError::Io(e)
    }
}

impl From<FtpConnectionError> for CompareError {
    fn from(e: FtpConnectionError) -> Self {
        CompareError::Ftp(e)
    }
}

/// Walks the local and remote directory trees, diffs them by size, and
/// (if `opts.hash` is set) upgrades same-size matches with an MD5 check.
/// Calls `progress` (if given) with a human-readable message for every
/// file examined (local walk, remote walk, hashing), for `--verbose`
/// output.
pub fn compare<C: FtpConnection>(
    conn: &mut C,
    opts: &CompareOptions,
    mut progress: Option<&mut (dyn FnMut(&str) + '_)>,
) -> Result<Vec<DiffEntry>, CompareError> {
    let local_entries = local::walk_local_dir(&opts.local_dir, &opts.excludes, progress.as_deref_mut())?;
    let remote_entries = remote::walk_remote(conn, &opts.remote_dir, &opts.excludes, progress.as_deref_mut())?;
    let mut entries = compare::compare_entries(&local_entries, &remote_entries);

    if opts.hash {
        hash::apply_hash_comparison(conn, &opts.remote_dir, &opts.local_dir, &mut entries, progress.as_deref_mut())?;
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::RawRemoteEntry;
    use std::collections::HashMap;

    struct MockConnection {
        listings: HashMap<String, Vec<RawRemoteEntry>>,
    }

    impl FtpConnection for MockConnection {
        fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
            self.listings
                .get(path)
                .cloned()
                .ok_or_else(|| FtpConnectionError(format!("no listing for {path}")))
        }

        fn try_hash(&mut self, _path: &str) -> Option<String> {
            None
        }

        fn retr_to_buffer(&mut self, _path: &str) -> Result<Vec<u8>, FtpConnectionError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn compares_local_and_remote_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("only_local.txt"), b"local").unwrap();
        std::fs::write(dir.path().join("shared.txt"), b"12345").unwrap();

        let mut listings = HashMap::new();
        listings.insert(
            "/remote".to_string(),
            vec![
                RawRemoteEntry { name: "shared.txt".into(), is_dir: false, size: 5 },
                RawRemoteEntry { name: "only_remote.txt".into(), is_dir: false, size: 3 },
            ],
        );
        let mut conn = MockConnection { listings };

        let opts = CompareOptions {
            local_dir: dir.path().to_path_buf(),
            remote_dir: "/remote".to_string(),
            excludes: Vec::new(),
            hash: false,
        };

        let entries = compare(&mut conn, &opts, None).unwrap();

        assert!(entries
            .iter()
            .any(|e| e.relative_path == "only_local.txt" && e.status == DiffStatus::LocalOnly));
        assert!(entries
            .iter()
            .any(|e| e.relative_path == "only_remote.txt" && e.status == DiffStatus::RemoteOnly));
        assert!(entries
            .iter()
            .any(|e| e.relative_path == "shared.txt" && e.status == DiffStatus::Match));
    }

    #[test]
    fn reports_progress_across_local_and_remote_walks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("local.txt"), b"x").unwrap();

        let mut listings = HashMap::new();
        listings.insert(
            "/remote".to_string(),
            vec![RawRemoteEntry { name: "remote.txt".into(), is_dir: false, size: 1 }],
        );
        let mut conn = MockConnection { listings };

        let opts = CompareOptions {
            local_dir: dir.path().to_path_buf(),
            remote_dir: "/remote".to_string(),
            excludes: Vec::new(),
            hash: false,
        };

        let mut messages = Vec::new();
        let mut progress = |msg: &str| messages.push(msg.to_string());

        compare(&mut conn, &opts, Some(&mut progress)).unwrap();

        assert_eq!(messages, vec!["local: local.txt".to_string(), "remote: remote.txt".to_string()]);
    }
}
