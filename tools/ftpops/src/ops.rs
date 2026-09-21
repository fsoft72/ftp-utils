//! Copy and delete operations, executed against CSV rows already
//! filtered by status.

use std::path::Path;

use ftp_utils_core::FtpConnection;

use crate::csv_input::CsvRow;

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
    rows: &[&CsvRow],
    skip_existing: bool,
) -> Vec<OpResult> {
    rows.iter()
        .map(|row| {
            let local_path = local_dir.join(&row.relative_path);
            if skip_existing && local_path.exists() {
                return OpResult { relative_path: row.relative_path.clone(), outcome: OpOutcome::Skipped };
            }

            let remote_path = format!("{remote_dir}/{}", row.relative_path);
            let outcome = (|| -> Result<(), String> {
                let data = conn.retr_to_buffer(&remote_path).map_err(|e| e.to_string())?;
                if let Some(parent) = local_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&local_path, data).map_err(|e| e.to_string())?;
                Ok(())
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

/// Uploads each row's local file to the remote directory, creating
/// missing remote parent directories. Skips (without overwriting) a row
/// whose remote destination already exists when `skip_existing` is true.
pub fn copy_to_remote<C: FtpConnection>(
    conn: &mut C,
    remote_dir: &str,
    local_dir: &Path,
    rows: &[&CsvRow],
    skip_existing: bool,
) -> Vec<OpResult> {
    rows.iter()
        .map(|row| {
            let remote_path = format!("{remote_dir}/{}", row.relative_path);

            if skip_existing {
                let (remote_parent, file_name) = remote_path.rsplit_once('/').unwrap_or(("", &remote_path));
                let remote_parent = if remote_parent.is_empty() { "/" } else { remote_parent };
                let exists = conn
                    .list_dir(remote_parent)
                    .map(|entries| entries.iter().any(|e| !e.is_dir && e.name == file_name))
                    .unwrap_or(false);
                if exists {
                    return OpResult { relative_path: row.relative_path.clone(), outcome: OpOutcome::Skipped };
                }
            }

            let local_path = local_dir.join(&row.relative_path);
            let outcome = (|| -> Result<(), String> {
                let data = std::fs::read(&local_path).map_err(|e| e.to_string())?;
                conn.ensure_remote_dir(&remote_path).map_err(|e| e.to_string())?;
                conn.store_from_buffer(&remote_path, &data).map_err(|e| e.to_string())?;
                Ok(())
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

/// Deletes each row's remote file.
pub fn delete_remote<C: FtpConnection>(conn: &mut C, remote_dir: &str, rows: &[&CsvRow]) -> Vec<OpResult> {
    rows.iter()
        .map(|row| {
            let remote_path = format!("{remote_dir}/{}", row.relative_path);
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
pub fn delete_local(local_dir: &Path, rows: &[&CsvRow]) -> Vec<OpResult> {
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
    use ftp_utils_core::remote::{FtpConnectionError, RawRemoteEntry};
    use std::collections::HashMap;

    #[derive(Default)]
    struct MockConnection {
        listings: HashMap<String, Vec<RawRemoteEntry>>,
        remote_files: HashMap<String, Vec<u8>>,
        stored: Vec<(String, Vec<u8>)>,
        deleted: Vec<String>,
        created_dirs: Vec<String>,
    }

    impl FtpConnection for MockConnection {
        fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
            Ok(self.listings.get(path).cloned().unwrap_or_default())
        }

        fn try_hash(&mut self, _path: &str) -> Option<String> {
            None
        }

        fn retr_to_buffer(&mut self, path: &str) -> Result<Vec<u8>, FtpConnectionError> {
            self.remote_files
                .get(path)
                .cloned()
                .ok_or_else(|| FtpConnectionError(format!("no such remote file: {path}")))
        }

        fn store_from_buffer(&mut self, path: &str, data: &[u8]) -> Result<(), FtpConnectionError> {
            self.stored.push((path.to_string(), data.to_vec()));
            Ok(())
        }

        fn delete(&mut self, path: &str) -> Result<(), FtpConnectionError> {
            self.deleted.push(path.to_string());
            Ok(())
        }

        fn create_dir(&mut self, path: &str) -> Result<(), FtpConnectionError> {
            self.created_dirs.push(path.to_string());

            let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
            let parent = if parent.is_empty() { "/" } else { parent };

            self.listings
                .entry(parent.to_string())
                .or_default()
                .push(RawRemoteEntry { name: name.to_string(), is_dir: true, size: 0 });
            self.listings.entry(path.to_string()).or_default();

            Ok(())
        }
    }

    fn row(relative_path: &str) -> CsvRow {
        CsvRow { relative_path: relative_path.to_string(), status: "RemoteOnly".to_string() }
    }

    #[test]
    fn copy_to_local_downloads_and_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = MockConnection::default();
        conn.remote_files.insert("/remote/sub/file.txt".to_string(), b"hello".to_vec());

        let rows = vec![row("sub/file.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = copy_to_local(&mut conn, "/remote", dir.path(), &refs, false);

        assert_eq!(results, vec![OpResult { relative_path: "sub/file.txt".to_string(), outcome: OpOutcome::Copied }]);
        let written = std::fs::read(dir.path().join("sub/file.txt")).unwrap();
        assert_eq!(written, b"hello");
    }

    #[test]
    fn copy_to_local_skips_when_destination_exists_and_skip_existing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), b"already here").unwrap();
        let mut conn = MockConnection::default();
        conn.remote_files.insert("/remote/file.txt".to_string(), b"new content".to_vec());

        let rows = vec![row("file.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = copy_to_local(&mut conn, "/remote", dir.path(), &refs, true);

        assert_eq!(results, vec![OpResult { relative_path: "file.txt".to_string(), outcome: OpOutcome::Skipped }]);
        let content = std::fs::read(dir.path().join("file.txt")).unwrap();
        assert_eq!(content, b"already here");
    }

    #[test]
    fn copy_to_local_reports_failure_for_missing_remote_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = MockConnection::default();

        let rows = vec![row("missing.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = copy_to_local(&mut conn, "/remote", dir.path(), &refs, false);

        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, OpOutcome::Failed(_)));
    }

    #[test]
    fn copy_to_remote_uploads_and_ensures_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/file.txt"), b"hello").unwrap();
        let mut conn = MockConnection::default();
        conn.listings.insert("/".to_string(), vec![]);

        let rows = vec![row("sub/file.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = copy_to_remote(&mut conn, "/remote", dir.path(), &refs, false);

        assert_eq!(results, vec![OpResult { relative_path: "sub/file.txt".to_string(), outcome: OpOutcome::Copied }]);
        assert_eq!(conn.stored, vec![("/remote/sub/file.txt".to_string(), b"hello".to_vec())]);
        assert_eq!(conn.created_dirs, vec!["/remote".to_string(), "/remote/sub".to_string()]);
    }

    #[test]
    fn copy_to_remote_skips_when_destination_exists_and_skip_existing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), b"local content").unwrap();
        let mut conn = MockConnection::default();
        conn.listings.insert("/remote".to_string(), vec![RawRemoteEntry { name: "file.txt".into(), is_dir: false, size: 5 }]);

        let rows = vec![row("file.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = copy_to_remote(&mut conn, "/remote", dir.path(), &refs, true);

        assert_eq!(results, vec![OpResult { relative_path: "file.txt".to_string(), outcome: OpOutcome::Skipped }]);
        assert!(conn.stored.is_empty());
    }

    #[test]
    fn delete_remote_calls_delete_with_full_path() {
        let mut conn = MockConnection::default();

        let rows = vec![row("a.txt"), row("sub/b.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

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

        let rows = vec![row("a.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = delete_local(dir.path(), &refs);

        assert_eq!(results, vec![OpResult { relative_path: "a.txt".to_string(), outcome: OpOutcome::Deleted }]);
        assert!(!dir.path().join("a.txt").exists());
    }

    #[test]
    fn delete_local_reports_failure_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();

        let rows = vec![row("missing.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = delete_local(dir.path(), &refs);

        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, OpOutcome::Failed(_)));
    }
}
