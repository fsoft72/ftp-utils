//! Upgrades `Match` diff entries to a hash-verified `Match` or
//! `HashMismatch` by comparing MD5 hashes. For each side, prefers an
//! already-known hash (e.g. recycled from a `--local-csv`/`--remote-csv`
//! source) over live access; if a side has neither, that entry is left
//! at its size-only `Match` result rather than erroring.

use std::collections::HashMap;
use std::path::Path;

use crate::diff::{DiffEntry, DiffStatus};
use crate::remote::{join_remote, FtpConnection, FtpConnectionError};

/// Computes the MD5 of the local file at `path` as lowercase hex, reading
/// it in chunks so memory use doesn't depend on the file size.
pub fn local_md5(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut context = md5::Context::new();
    std::io::copy(&mut file, &mut context)?;
    Ok(format!("{:x}", context.compute()))
}

/// Resolves the MD5 of the remote file at `path` as lowercase hex: the
/// server-computed hash when `try_hash` provides one, otherwise a
/// streamed download hashed on the fly (never held fully in memory).
pub fn remote_md5<C: FtpConnection>(conn: &mut C, path: &str) -> Result<String, FtpConnectionError> {
    if let Some(hash) = conn.try_hash(path) {
        return Ok(hash);
    }

    let mut context = md5::Context::new();
    conn.retr_to_writer(path, &mut context)?;
    Ok(format!("{:x}", context.compute()))
}

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
            let remote_path = join_remote(remote_root, &entry.relative_path);
            let hash = remote_md5(conn, &remote_path)
                .map_err(|e| std::io::Error::other(format!("{}: {e}", entry.relative_path)))?;
            Some(hash)
        } else {
            None
        };

        let local_md5 = if let Some(md5) = local_known_md5.get(&entry.relative_path) {
            Some(md5.clone())
        } else if let Some(local_root) = local_root {
            let hash = local_md5(&local_root.join(&entry.relative_path))
                .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", entry.relative_path)))?;
            Some(hash)
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
    
    use crate::testing::MockFtpConnection;

    /// A server that returns `hash` from `try_hash` (if any) and serves
    /// `content` for every downloaded path.
    fn server(hash: Option<&str>, content: &[u8]) -> MockFtpConnection {
        MockFtpConnection {
            hash: hash.map(str::to_string),
            default_content: Some(content.to_vec()),
            ..MockFtpConnection::default()
        }
    }

    fn entry(relative_path: &str) -> DiffEntry {
        DiffEntry::new(relative_path, DiffStatus::Match, Some(5), Some(5))
    }

    #[test]
    fn local_md5_matches_in_memory_md5_for_large_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();

        assert_eq!(local_md5(&path).unwrap(), format!("{:x}", md5::compute(&data)));
    }

    #[test]
    fn local_md5_reports_missing_file() {
        assert!(local_md5(Path::new("/nonexistent/file")).is_err());
    }

    #[test]
    fn remote_md5_hashes_streamed_download_when_server_hash_missing() {
        let mut conn = server(None, b"hello");

        assert_eq!(remote_md5(&mut conn, "/r/f.txt").unwrap(), format!("{:x}", md5::compute(b"hello")));
    }

    #[test]
    fn remote_md5_prefers_server_hash() {
        let mut conn = server(Some("server-hash"), b"x");

        assert_eq!(remote_md5(&mut conn, "/r/f.txt").unwrap(), "server-hash");
    }

    #[test]
    fn hash_error_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut entries = vec![entry("missing.txt")];

        let err = apply_hash_comparison::<MockFtpConnection>(
            None,
            None,
            Some(dir.path()),
            &HashMap::new(),
            &HashMap::new(),
            &mut entries,
            None,
        )
        .unwrap_err();

        assert!(err.to_string().contains("missing.txt"), "{err}");
    }

    #[test]
    fn uses_remote_hash_when_available() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();
        let expected_hash = format!("{:x}", md5::compute(b"hello"));

        let mut entries = vec![entry("f.txt")];
        let mut conn = server(Some(&expected_hash), b"");

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
        let mut conn = server(None, b"different");

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
        let mut entries = vec![DiffEntry::new("only-local.txt", DiffStatus::LocalOnly, Some(5), None)];
        let mut conn = server(None, b"");

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
            DiffEntry::new("only-local.txt", DiffStatus::LocalOnly, Some(5), None),
        ];
        let mut conn = server(None, b"hello");

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

        apply_hash_comparison::<MockFtpConnection>(None, None, None, &local_known, &remote_known, &mut entries, None)
            .unwrap();

        assert_eq!(entries[0].status, DiffStatus::Match);
        assert_eq!(entries[0].local_md5, Some("same-hash".to_string()));
        assert_eq!(entries[0].remote_md5, Some("same-hash".to_string()));
    }

    #[test]
    fn leaves_entry_at_match_when_hash_unresolvable_on_either_side() {
        let mut entries = vec![entry("f.txt")];

        apply_hash_comparison::<MockFtpConnection>(
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
