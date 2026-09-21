//! Upgrades `Match` diff entries to a hash-verified `Match` or
//! `HashMismatch` by comparing MD5 hashes. Tries the connection's
//! server-side hash first; falls back to downloading the remote file and
//! hashing it locally if the server doesn't support that.

use std::path::Path;

use crate::diff::{DiffEntry, DiffStatus};
use crate::remote::FtpConnection;

/// For every entry currently marked `Match`, computes and compares MD5
/// hashes, updating `status` to `Match` or `HashMismatch` and filling in
/// `local_md5`/`remote_md5`. Entries with any other status are untouched.
pub fn apply_hash_comparison<C: FtpConnection>(
    conn: &mut C,
    remote_root: &str,
    local_root: &Path,
    entries: &mut [DiffEntry],
) -> std::io::Result<()> {
    for entry in entries.iter_mut() {
        if entry.status != DiffStatus::Match {
            continue;
        }

        let remote_path = format!("{remote_root}/{}", entry.relative_path);
        let local_path = local_root.join(&entry.relative_path);

        let remote_md5 = match conn.try_hash(&remote_path) {
            Some(hash) => hash,
            None => {
                let bytes = conn
                    .retr_to_buffer(&remote_path)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
                format!("{:x}", md5::compute(&bytes))
            }
        };

        let local_bytes = std::fs::read(&local_path)?;
        let local_md5 = format!("{:x}", md5::compute(&local_bytes));

        entry.status = if local_md5 == remote_md5 {
            DiffStatus::Match
        } else {
            DiffStatus::HashMismatch
        };
        entry.local_md5 = Some(local_md5);
        entry.remote_md5 = Some(remote_md5);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{FtpConnectionError, RawRemoteEntry};

    struct MockConnection {
        hash: Option<String>,
        remote_bytes: Vec<u8>,
    }

    impl FtpConnection for MockConnection {
        fn list_dir(&mut self, _path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
            Ok(Vec::new())
        }

        fn try_hash(&mut self, _path: &str) -> Option<String> {
            self.hash.clone()
        }

        fn retr_to_buffer(&mut self, _path: &str) -> Result<Vec<u8>, FtpConnectionError> {
            Ok(self.remote_bytes.clone())
        }
    }

    fn entry(relative_path: &str) -> DiffEntry {
        DiffEntry {
            relative_path: relative_path.to_string(),
            status: DiffStatus::Match,
            local_size: Some(5),
            remote_size: Some(5),
            local_md5: None,
            remote_md5: None,
        }
    }

    #[test]
    fn uses_remote_hash_when_available() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();
        let expected_hash = format!("{:x}", md5::compute(b"hello"));

        let mut entries = vec![entry("f.txt")];
        let mut conn = MockConnection { hash: Some(expected_hash.clone()), remote_bytes: Vec::new() };

        apply_hash_comparison(&mut conn, "/remote", dir.path(), &mut entries).unwrap();

        assert_eq!(entries[0].status, DiffStatus::Match);
        assert_eq!(entries[0].remote_md5, Some(expected_hash));
    }

    #[test]
    fn falls_back_to_download_when_hash_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();

        let mut entries = vec![entry("f.txt")];
        let mut conn = MockConnection { hash: None, remote_bytes: b"different".to_vec() };

        apply_hash_comparison(&mut conn, "/remote", dir.path(), &mut entries).unwrap();

        assert_eq!(entries[0].status, DiffStatus::HashMismatch);
    }

    #[test]
    fn leaves_non_match_entries_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let mut entries = vec![DiffEntry {
            relative_path: "only-local.txt".to_string(),
            status: DiffStatus::LocalOnly,
            local_size: Some(5),
            remote_size: None,
            local_md5: None,
            remote_md5: None,
        }];
        let mut conn = MockConnection { hash: None, remote_bytes: Vec::new() };

        apply_hash_comparison(&mut conn, "/remote", dir.path(), &mut entries).unwrap();

        assert_eq!(entries[0].status, DiffStatus::LocalOnly);
        assert_eq!(entries[0].remote_md5, None);
    }
}
