//! The on-disk layout of one monitored site and its snapshot/log files:
//!
//! ```text
//! <site>/config.json
//! <site>/files/                    copy downloaded by `init`
//! <site>/snapshots/<stamp>.csv     one per run
//! <site>/logs/<stamp>.log          one per `check` run
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use ftp_utils_core::csv_source::{self, ReportEntry};
use ftp_utils_core::remote::RemoteEntry;

ftp_utils_core::message_error! {
    /// A snapshot, log or directory could not be read or written.
    pub StoreError
}

/// A monitored site's directory.
pub struct SiteDir {
    root: PathBuf,
}

impl SiteDir {
    /// Wraps the site directory `root` (it need not exist yet).
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The site directory itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path of the site's `config.json`.
    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.json")
    }

    /// Directory holding the files downloaded by `init`.
    pub fn files_dir(&self) -> PathBuf {
        self.root.join("files")
    }

    /// Directory holding snapshot CSV files.
    fn snapshots_dir(&self) -> PathBuf {
        self.root.join("snapshots")
    }

    /// Path of the snapshot named `<stamp>.csv`.
    pub fn snapshot_path(&self, stamp: &str) -> PathBuf {
        self.snapshots_dir().join(format!("{stamp}.csv"))
    }

    /// Path of the log named `<stamp>.log`.
    pub fn log_path(&self, stamp: &str) -> PathBuf {
        self.root.join("logs").join(format!("{stamp}.log"))
    }

    /// The newest snapshot (greatest file name; stamps sort by time), or
    /// `None` if there are none. Only `*.csv` files count, so a
    /// half-written `.csv.part` is never picked.
    pub fn latest_snapshot(&self) -> Result<Option<PathBuf>, StoreError> {
        let dir = self.snapshots_dir();
        if !dir.is_dir() {
            return Ok(None);
        }
        let mut newest: Option<PathBuf> = None;
        let listing = fs::read_dir(&dir).map_err(|e| StoreError(format!("cannot read {}: {e}", dir.display())))?;
        for item in listing {
            let path = item.map_err(|e| StoreError(format!("cannot read {}: {e}", dir.display())))?.path();
            if path.extension().is_none_or(|ext| ext != "csv") {
                continue;
            }
            if newest.as_ref().is_none_or(|current| path.file_name() > current.file_name()) {
                newest = Some(path);
            }
        }
        Ok(newest)
    }
}

/// Creates parent directories for a path.
fn _create_parent(path: &Path) -> Result<(), StoreError> {
    let Some(parent) = path.parent() else { return Ok(()) };
    fs::create_dir_all(parent).map_err(|e| StoreError(format!("cannot create {}: {e}", parent.display())))
}

/// Writes `entries` (sorted by path) as a snapshot CSV at `path`, creating
/// parent directories. The file appears under its final name only once
/// complete.
pub fn write_snapshot(path: &Path, entries: &[RemoteEntry]) -> Result<(), StoreError> {
    _create_parent(path)?;
    let mut sorted: Vec<&RemoteEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let rows: Vec<ReportEntry> = sorted
        .into_iter()
        .map(|e| ReportEntry::scan(e.relative_path.clone(), None, Some(e.size)).with_remote_mtime(e.modified))
        .collect();

    let part = path.with_extension("csv.part");
    csv_source::write_report(&part, &rows)
        .map_err(|e| StoreError(format!("cannot write snapshot {}: {e}", path.display())))?;
    fs::rename(&part, path).map_err(|e| StoreError(format!("cannot write snapshot {}: {e}", path.display())))
}

/// Reads a snapshot written by `write_snapshot`.
pub fn read_snapshot(path: &Path) -> Result<Vec<RemoteEntry>, StoreError> {
    csv_source::read_remote_entries(path)
        .map(|(entries, _md5)| entries)
        .map_err(|e| StoreError(format!("cannot read snapshot {}: {e}", path.display())))
}

/// Writes `text` to `path`, creating parent directories.
pub fn write_log(path: &Path, text: &str) -> Result<(), StoreError> {
    _create_parent(path)?;
    fs::write(path, text).map_err(|e| StoreError(format!("cannot write log {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, size: u64, modified: Option<i64>) -> RemoteEntry {
        RemoteEntry { relative_path: path.to_string(), size, modified }
    }

    #[test]
    fn layout_paths_live_under_the_site_directory() {
        let site = SiteDir::new(PathBuf::from("/srv/test.com"));

        assert_eq!(site.config_path(), PathBuf::from("/srv/test.com/config.json"));
        assert_eq!(site.files_dir(), PathBuf::from("/srv/test.com/files"));
        assert_eq!(
            site.snapshot_path("2026-10-01_030000"),
            PathBuf::from("/srv/test.com/snapshots/2026-10-01_030000.csv")
        );
        assert_eq!(site.log_path("2026-10-01_030000"), PathBuf::from("/srv/test.com/logs/2026-10-01_030000.log"));
    }

    #[test]
    fn snapshot_round_trips_including_awkward_names() {
        let dir = tempfile::tempdir().unwrap();
        let site = SiteDir::new(dir.path().to_path_buf());
        let path = site.snapshot_path("2026-10-01_030000");
        let entries = vec![
            entry("wp-content/my file, \"v2\".php", 10, Some(1_578_182_400)),
            entry("caf\u{e9}/\u{e8}.txt", 5, None),
        ];

        write_snapshot(&path, &entries).unwrap();
        let mut read = read_snapshot(&path).unwrap();
        read.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        let mut expected = entries.clone();
        expected.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

        assert_eq!(read, expected);
        assert!(!path.with_extension("csv.part").exists());
    }

    #[test]
    fn latest_snapshot_is_the_newest_by_name_and_ignores_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let site = SiteDir::new(dir.path().to_path_buf());
        assert_eq!(site.latest_snapshot().unwrap(), None); // no snapshots directory yet

        write_snapshot(&site.snapshot_path("2026-10-01_030000"), &[]).unwrap();
        write_snapshot(&site.snapshot_path("2026-10-02_030000"), &[]).unwrap();
        write_snapshot(&site.snapshot_path("2026-10-01_235959"), &[]).unwrap();
        std::fs::write(site.snapshot_path("2026-10-09_000000").with_extension("csv.part"), "x").unwrap();
        std::fs::write(dir.path().join("snapshots/notes.txt"), "x").unwrap();

        assert_eq!(site.latest_snapshot().unwrap(), Some(site.snapshot_path("2026-10-02_030000")));
    }

    #[test]
    fn write_log_creates_the_logs_directory() {
        let dir = tempfile::tempdir().unwrap();
        let site = SiteDir::new(dir.path().to_path_buf());

        write_log(&site.log_path("2026-10-01_030000"), "hello\n").unwrap();

        assert_eq!(std::fs::read_to_string(site.log_path("2026-10-01_030000")).unwrap(), "hello\n");
    }

    #[test]
    fn reading_a_missing_snapshot_is_an_error() {
        assert!(read_snapshot(Path::new("/nonexistent/snap.csv")).is_err());
    }
}
