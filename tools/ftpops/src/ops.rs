//! Copy and delete operations, executed against CSV rows already
//! filtered by status.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use ftp_utils_core::csv_source::StatusRow;
use ftp_utils_core::remote::join_remote;
use ftp_utils_core::FtpConnection;

/// What happened to one file.
#[derive(Debug, Clone, PartialEq)]
pub enum OpOutcome {
    Copied,
    Deleted,
    Skipped,
    Failed(String),
}

/// The outcome for one CSV row.
#[derive(Debug, Clone, PartialEq)]
pub struct OpResult {
    pub relative_path: String,
    pub outcome: OpOutcome,
}

/// Downloads each row's remote file into the local directory, creating
/// missing local parent directories. Skips (without overwriting) a row
/// whose local destination already exists when `skip_existing` is true.
pub fn copy_to_local<C: FtpConnection>(
    conn: &mut C,
    remote_dir: &str,
    local_dir: &Path,
    rows: &[&StatusRow],
    skip_existing: bool,
) -> Vec<OpResult> {
    rows.iter()
        .map(|row| {
            let local_path = local_dir.join(&row.relative_path);
            if skip_existing && local_path.exists() {
                return OpResult { relative_path: row.relative_path.clone(), outcome: OpOutcome::Skipped };
            }

            let remote_path = join_remote(remote_dir, &row.relative_path);
            let outcome = (|| -> Result<(), String> {
                if let Some(parent) = local_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                _download_atomically(conn, &remote_path, &local_path)
            })();

            OpResult {
                relative_path: row.relative_path.clone(),
                outcome: match outcome {
                    Ok(()) => OpOutcome::Copied,
                    Err(e) => OpOutcome::Failed(e),
                },
            }
        })
        .collect()
}

/// Downloads `remote_path` to a temporary file next to `local_path` and
/// renames it into place only once the transfer succeeded, so a failed or
/// interrupted download never leaves a truncated file at (or replaces the
/// previous content of) `local_path`.
fn _download_atomically<C: FtpConnection>(conn: &mut C, remote_path: &str, local_path: &Path) -> Result<(), String> {
    let file_name = local_path.file_name().ok_or_else(|| format!("invalid local path {}", local_path.display()))?;
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(".ftpops-part");
    let temp_path = local_path.with_file_name(temp_name);

    let result = (|| -> Result<(), String> {
        let mut file = std::fs::File::create(&temp_path).map_err(|e| e.to_string())?;
        conn.retr_to_writer(remote_path, &mut file).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temp_path, local_path).map_err(|e| e.to_string())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

/// Uploads each row's local file to the remote directory, creating
/// missing remote parent directories. Skips (without overwriting) a row
/// whose remote destination already exists when `skip_existing` is true.
/// Directory creation and existence listings are cached per run, so each
/// remote directory is listed at most once.
pub fn copy_to_remote<C: FtpConnection>(
    conn: &mut C,
    remote_dir: &str,
    local_dir: &Path,
    rows: &[&StatusRow],
    skip_existing: bool,
) -> Vec<OpResult> {
    let mut known_dirs: HashSet<String> = HashSet::new();
    let mut listed_files: HashMap<String, HashSet<String>> = HashMap::new();
    let mut results = Vec::with_capacity(rows.len());

    for row in rows {
        let remote_path = join_remote(remote_dir, &row.relative_path);
        let local_path = local_dir.join(&row.relative_path);

        let outcome = (|| -> Result<OpOutcome, String> {
            // Parents first: a listing failure below is then a real error,
            // not just a directory that doesn't exist yet.
            conn.ensure_remote_dir_cached(&remote_path, &mut known_dirs).map_err(|e| e.to_string())?;

            if skip_existing && _remote_file_exists(conn, &remote_path, &mut listed_files)? {
                return Ok(OpOutcome::Skipped);
            }

            let mut file = std::fs::File::open(&local_path).map_err(|e| e.to_string())?;
            conn.store_from_reader(&remote_path, &mut file).map_err(|e| e.to_string())?;
            Ok(OpOutcome::Copied)
        })();

        results.push(OpResult {
            relative_path: row.relative_path.clone(),
            outcome: outcome.unwrap_or_else(OpOutcome::Failed),
        });
    }

    results
}

/// Checks whether `remote_path` is an existing file, listing its parent
/// directory only the first time it is seen (results live in `listed`).
/// A failed listing is an error, never "does not exist": that would
/// overwrite the files `--skip-existing` is meant to protect.
fn _remote_file_exists<C: FtpConnection>(
    conn: &mut C,
    remote_path: &str,
    listed: &mut HashMap<String, HashSet<String>>,
) -> Result<bool, String> {
    let (parent, file_name) = remote_path.rsplit_once('/').unwrap_or(("", remote_path));
    let parent = if parent.is_empty() { "/" } else { parent };

    if !listed.contains_key(parent) {
        let entries = conn.list_dir(parent).map_err(|e| format!("cannot check existing file: {e}"))?;
        let files = entries.into_iter().filter(|e| !e.is_dir).map(|e| e.name).collect();
        listed.insert(parent.to_string(), files);
    }

    Ok(listed[parent].contains(file_name))
}

/// Deletes each row's remote file.
pub fn delete_remote<C: FtpConnection>(conn: &mut C, remote_dir: &str, rows: &[&StatusRow]) -> Vec<OpResult> {
    rows.iter()
        .map(|row| {
            let remote_path = join_remote(remote_dir, &row.relative_path);
            let outcome = conn.delete(&remote_path).map_err(|e| e.to_string());
            OpResult {
                relative_path: row.relative_path.clone(),
                outcome: match outcome {
                    Ok(()) => OpOutcome::Deleted,
                    Err(e) => OpOutcome::Failed(e),
                },
            }
        })
        .collect()
}

/// Deletes each row's local file.
pub fn delete_local(local_dir: &Path, rows: &[&StatusRow]) -> Vec<OpResult> {
    rows.iter()
        .map(|row| {
            let local_path = local_dir.join(&row.relative_path);
            let outcome = std::fs::remove_file(&local_path).map_err(|e| e.to_string());
            OpResult {
                relative_path: row.relative_path.clone(),
                outcome: match outcome {
                    Ok(()) => OpOutcome::Deleted,
                    Err(e) => OpOutcome::Failed(e),
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use ftp_utils_core::csv_source::ReportStatus;
    use ftp_utils_core::remote::RawRemoteEntry;
    use ftp_utils_core::testing::MockFtpConnection;

    fn row(relative_path: &str) -> StatusRow {
        StatusRow {
            relative_path: relative_path.to_string(),
            status: ReportStatus::Diff(ftp_utils_core::DiffStatus::RemoteOnly),
        }
    }

    #[test]
    fn copy_to_local_downloads_and_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = MockFtpConnection::default();
        conn.files.insert("/remote/sub/file.txt".to_string(), b"hello".to_vec());

        let rows = [row("sub/file.txt")];
        let refs: Vec<&StatusRow> = rows.iter().collect();

        let results = copy_to_local(&mut conn, "/remote", dir.path(), &refs, false);

        assert_eq!(results, vec![OpResult { relative_path: "sub/file.txt".to_string(), outcome: OpOutcome::Copied }]);
        let written = std::fs::read(dir.path().join("sub/file.txt")).unwrap();
        assert_eq!(written, b"hello");
    }

    #[test]
    fn copy_to_local_skips_when_destination_exists_and_skip_existing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), b"already here").unwrap();
        let mut conn = MockFtpConnection::default();
        conn.files.insert("/remote/file.txt".to_string(), b"new content".to_vec());

        let rows = [row("file.txt")];
        let refs: Vec<&StatusRow> = rows.iter().collect();

        let results = copy_to_local(&mut conn, "/remote", dir.path(), &refs, true);

        assert_eq!(results, vec![OpResult { relative_path: "file.txt".to_string(), outcome: OpOutcome::Skipped }]);
        let content = std::fs::read(dir.path().join("file.txt")).unwrap();
        assert_eq!(content, b"already here");
    }

    #[test]
    fn copy_to_local_failure_keeps_existing_file_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), b"previous content").unwrap();
        let mut conn = MockFtpConnection::default(); // remote file is missing: download fails

        let rows = [row("file.txt")];
        let refs: Vec<&StatusRow> = rows.iter().collect();

        let results = copy_to_local(&mut conn, "/remote", dir.path(), &refs, false);

        assert!(matches!(results[0].outcome, OpOutcome::Failed(_)));
        assert_eq!(std::fs::read(dir.path().join("file.txt")).unwrap(), b"previous content");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "unexpected files: {leftovers:?}");
    }

    #[test]
    fn copy_to_local_overwrites_existing_file_on_success() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), b"old").unwrap();
        let mut conn = MockFtpConnection::default();
        conn.files.insert("/remote/file.txt".to_string(), b"new".to_vec());

        let rows = [row("file.txt")];
        let refs: Vec<&StatusRow> = rows.iter().collect();

        copy_to_local(&mut conn, "/remote", dir.path(), &refs, false);

        assert_eq!(std::fs::read(dir.path().join("file.txt")).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn copy_to_local_reports_failure_for_missing_remote_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = MockFtpConnection::default();

        let rows = [row("missing.txt")];
        let refs: Vec<&StatusRow> = rows.iter().collect();

        let results = copy_to_local(&mut conn, "/remote", dir.path(), &refs, false);

        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, OpOutcome::Failed(_)));
    }

    #[test]
    fn copy_to_remote_uploads_and_ensures_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/file.txt"), b"hello").unwrap();
        let mut conn = MockFtpConnection::default();
        conn.listings.insert("/".to_string(), vec![]);

        let rows = [row("sub/file.txt")];
        let refs: Vec<&StatusRow> = rows.iter().collect();

        let results = copy_to_remote(&mut conn, "/remote", dir.path(), &refs, false);

        assert_eq!(results, vec![OpResult { relative_path: "sub/file.txt".to_string(), outcome: OpOutcome::Copied }]);
        assert_eq!(conn.stored, vec![("/remote/sub/file.txt".to_string(), b"hello".to_vec())]);
        assert_eq!(conn.created_dirs, vec!["/remote".to_string(), "/remote/sub".to_string()]);
    }

    #[test]
    fn copy_to_remote_skips_when_destination_exists_and_skip_existing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), b"local content").unwrap();
        let mut conn = MockFtpConnection::default();
        conn.listings
            .insert("/remote".to_string(), vec![RawRemoteEntry { name: "file.txt".into(), is_dir: false, size: 5 }]);

        let rows = [row("file.txt")];
        let refs: Vec<&StatusRow> = rows.iter().collect();

        let results = copy_to_remote(&mut conn, "/remote", dir.path(), &refs, true);

        assert_eq!(results, vec![OpResult { relative_path: "file.txt".to_string(), outcome: OpOutcome::Skipped }]);
        assert!(conn.stored.is_empty());
    }

    #[test]
    fn copy_to_remote_does_not_overwrite_when_existence_check_fails() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), b"local content").unwrap();
        let mut conn = MockFtpConnection { fail_listings: true, ..MockFtpConnection::default() };

        let rows = [row("file.txt")];
        let refs: Vec<&StatusRow> = rows.iter().collect();

        let results = copy_to_remote(&mut conn, "/remote", dir.path(), &refs, true);

        assert!(matches!(results[0].outcome, OpOutcome::Failed(_)));
        assert!(conn.stored.is_empty());
    }

    #[test]
    fn copy_to_remote_lists_each_directory_once_with_skip_existing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        for name in ["sub/a.txt", "sub/b.txt", "sub/c.txt"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let mut conn = MockFtpConnection::default();
        conn.listings.insert("/".to_string(), vec![]);

        let rows = [row("sub/a.txt"), row("sub/b.txt"), row("sub/c.txt")];
        let refs: Vec<&StatusRow> = rows.iter().collect();

        let results = copy_to_remote(&mut conn, "/remote", dir.path(), &refs, true);

        assert!(results.iter().all(|r| r.outcome == OpOutcome::Copied));
        // "/" and "/remote" are listed once each while creating parents;
        // "/remote/sub" is listed once, by the existence check.
        let sub_listings = conn.list_calls.iter().filter(|p| *p == "/remote/sub").count();
        assert_eq!(sub_listings, 1, "calls: {:?}", conn.list_calls);
        assert_eq!(conn.list_calls.iter().filter(|p| *p == "/").count(), 1);
    }

    #[test]
    fn delete_remote_calls_delete_with_full_path() {
        let mut conn = MockFtpConnection::default();

        let rows = [row("a.txt"), row("sub/b.txt")];
        let refs: Vec<&StatusRow> = rows.iter().collect();

        let results = delete_remote(&mut conn, "/remote", &refs);

        assert_eq!(
            results,
            vec![
                OpResult { relative_path: "a.txt".to_string(), outcome: OpOutcome::Deleted },
                OpResult { relative_path: "sub/b.txt".to_string(), outcome: OpOutcome::Deleted },
            ]
        );
        assert_eq!(conn.deleted, vec!["/remote/a.txt".to_string(), "/remote/sub/b.txt".to_string()]);
    }

    #[test]
    fn delete_local_removes_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();

        let rows = [row("a.txt")];
        let refs: Vec<&StatusRow> = rows.iter().collect();

        let results = delete_local(dir.path(), &refs);

        assert_eq!(results, vec![OpResult { relative_path: "a.txt".to_string(), outcome: OpOutcome::Deleted }]);
        assert!(!dir.path().join("a.txt").exists());
    }

    #[test]
    fn delete_local_reports_failure_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();

        let rows = [row("missing.txt")];
        let refs: Vec<&StatusRow> = rows.iter().collect();

        let results = delete_local(dir.path(), &refs);

        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, OpOutcome::Failed(_)));
    }
}
