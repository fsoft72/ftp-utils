//! Upgrades `Match` diff entries to a hash-verified `Match` or
//! `HashMismatch` by comparing MD5 hashes. For each side, prefers an
//! already-known hash (e.g. recycled from a `--local-csv`/`--remote-csv`
//! source) over live access; if a side has neither, that entry is left
//! at its size-only `Match` result rather than erroring.

use std::collections::HashMap;
use std::path::Path;

use crate::diff::{DiffEntry, DiffStatus};
use crate::remote::FtpConnection;

/// For every entry currently marked `Match`, resolves and compares each
/// side's MD5 hash, updating `status` to `Match` or `HashMismatch` (only
/// when both sides' hashes could be resolved) and filling in whichever of
/// `local_md5`/`remote_md5` were resolved. Entries with any other status
/// are untouched.
///
/// Each side's hash is resolved in this order: `local_known_md5`/
/// `remote_known_md5` first (an already-known hash, e.g. recycled from a
/// CSV source); then, if that side has a live source (`conn`+
/// `remote_root` for remote, `local_root` for local), fetched live
/// exactly as before. If neither is available for a side, that side's
/// hash stays unresolved and the entry's status is left unchanged.
///
/// Calls `progress` (if given) with a human-readable message for every
/// entry a hash resolution is attempted for, for `--verbose` output.
pub fn apply_hash_comparison<C: FtpConnection>(
    mut conn: Option<&mut C>,
    remote_root: Option<&str>,
    local_root: Option<&Path>,
    local_known_md5: &HashMap<String, String>,
    remote_known_md5: &HashMap<String, String>,
    entries: &mut [DiffEntry],
    mut progress: Option<&mut (dyn FnMut(&str) + '_)>,
) -> std::io::Result<()> {
    for entry in entries.iter_mut() {
        if entry.status != DiffStatus::Match {
            continue;
        }

        if let Some(cb) = progress.as_deref_mut() {
            cb(&format!("hash: {}", entry.relative_path));
        }

        let remote_md5 = if let Some(md5) = remote_known_md5.get(&entry.relative_path) {
            Some(md5.clone())
        } else if let (Some(conn), Some(remote_root)) = (conn.as_deref_mut(), remote_root) {
            let remote_path = format!("{remote_root}/{}", entry.relative_path);
            match conn.try_hash(&remote_path) {
                Some(hash) => Some(hash),
                None => {
                    let bytes = conn
                        .retr_to_buffer(&remote_path)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
                    Some(format!("{:x}", md5::compute(&bytes)))
                }
            }
        } else {
            None
        };

        let local_md5 = if let Some(md5) = local_known_md5.get(&entry.relative_path) {
            Some(md5.clone())
        } else if let Some(local_root) = local_root {
            let local_path = local_root.join(&entry.relative_path);
            let bytes = std::fs::read(&local_path)?;
            Some(format!("{:x}", md5::compute(&bytes)))
        } else {
            None
        };

        if let (Some(local_md5), Some(remote_md5)) = (&local_md5, &remote_md5) {
            entry.status = if local_md5 == remote_md5 { DiffStatus::Match } else { DiffStatus::HashMismatch };
        }
        entry.local_md5 = local_md5;
        entry.remote_md5 = remote_md5;
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

        fn store_from_buffer(&mut self, _path: &str, _data: &[u8]) -> Result<(), FtpConnectionError> {
            Ok(())
        }

        fn delete(&mut self, _path: &str) -> Result<(), FtpConnectionError> {
            Ok(())
        }

        fn create_dir(&mut self, _path: &str) -> Result<(), FtpConnectionError> {
            Ok(())
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

        apply_hash_comparison(
            Some(&mut conn),
            Some("/remote"),
            Some(dir.path()),
            &HashMap::new(),
            &HashMap::new(),
            &mut entries,
            None,
        )
        .unwrap();

        assert_eq!(entries[0].status, DiffStatus::Match);
        assert_eq!(entries[0].remote_md5, Some(expected_hash));
    }

    #[test]
    fn falls_back_to_download_when_hash_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();

        let mut entries = vec![entry("f.txt")];
        let mut conn = MockConnection { hash: None, remote_bytes: b"different".to_vec() };

        apply_hash_comparison(
            Some(&mut conn),
            Some("/remote"),
            Some(dir.path()),
            &HashMap::new(),
            &HashMap::new(),
            &mut entries,
            None,
        )
        .unwrap();

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

        apply_hash_comparison(
            Some(&mut conn),
            Some("/remote"),
            Some(dir.path()),
            &HashMap::new(),
            &HashMap::new(),
            &mut entries,
            None,
        )
        .unwrap();

        assert_eq!(entries[0].status, DiffStatus::LocalOnly);
        assert_eq!(entries[0].remote_md5, None);
    }

    #[test]
    fn reports_progress_only_for_hashed_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();

        let mut entries = vec![
            entry("f.txt"),
            DiffEntry {
                relative_path: "only-local.txt".to_string(),
                status: DiffStatus::LocalOnly,
                local_size: Some(5),
                remote_size: None,
                local_md5: None,
                remote_md5: None,
            },
        ];
        let mut conn = MockConnection { hash: None, remote_bytes: b"hello".to_vec() };

        let mut messages = Vec::new();
        let mut progress = |msg: &str| messages.push(msg.to_string());

        apply_hash_comparison(
            Some(&mut conn),
            Some("/remote"),
            Some(dir.path()),
            &HashMap::new(),
            &HashMap::new(),
            &mut entries,
            Some(&mut progress),
        )
        .unwrap();

        assert_eq!(messages, vec!["hash: f.txt".to_string()]);
    }

    #[test]
    fn known_md5_short_circuits_live_access() {
        // No live connection or local file backing this entry at all -
        // both known-md5 maps must be consulted first.
        let mut entries = vec![entry("f.txt")];
        let mut local_known = HashMap::new();
        local_known.insert("f.txt".to_string(), "same-hash".to_string());
        let mut remote_known = HashMap::new();
        remote_known.insert("f.txt".to_string(), "same-hash".to_string());

        apply_hash_comparison::<MockConnection>(None, None, None, &local_known, &remote_known, &mut entries, None)
            .unwrap();

        assert_eq!(entries[0].status, DiffStatus::Match);
        assert_eq!(entries[0].local_md5, Some("same-hash".to_string()));
        assert_eq!(entries[0].remote_md5, Some("same-hash".to_string()));
    }

    #[test]
    fn leaves_entry_at_match_when_hash_unresolvable_on_either_side() {
        let mut entries = vec![entry("f.txt")];

        apply_hash_comparison::<MockConnection>(
            None,
            None,
            None,
            &HashMap::new(),
            &HashMap::new(),
            &mut entries,
            None,
        )
        .unwrap();

        assert_eq!(entries[0].status, DiffStatus::Match);
        assert_eq!(entries[0].local_md5, None);
        assert_eq!(entries[0].remote_md5, None);
    }
}
