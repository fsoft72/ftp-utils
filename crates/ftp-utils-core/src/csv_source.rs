//! Reading and writing `ftpdiff --csv` reports. Reading serves both as a
//! substitute for a live directory listing on one side of a comparison
//! (`--local-csv` / `--remote-csv`) and as ftpops' list of operations.

use std::collections::HashMap;
use std::path::Path;

use crate::diff::{DiffEntry, DiffStatus};
use crate::local::LocalEntry;
use crate::remote::RemoteEntry;

crate::message_error! {
    /// Error reading a CSV report used as a comparison source.
    pub CsvSourceError
}

/// CSV column names, in the order they are written.
const COLUMNS: [&str; 6] = ["path", "status", "local_size", "remote_size", "local_md5", "remote_md5"];

fn _column_index(headers: &csv::StringRecord, name: &str) -> Result<usize, CsvSourceError> {
    headers.iter().position(|h| h == name).ok_or_else(|| CsvSourceError(format!("CSV missing '{name}' column")))
}

/// Opens `path` as CSV and returns the reader with its header row.
fn _open(path: &Path) -> Result<(csv::Reader<std::fs::File>, csv::StringRecord), CsvSourceError> {
    let mut reader =
        csv::Reader::from_path(path).map_err(|e| CsvSourceError(format!("cannot read CSV {}: {e}", path.display())))?;
    let headers = reader.headers().map_err(|e| CsvSourceError(e.to_string()))?.clone();
    Ok((reader, headers))
}

/// Reads the `path` column of `record` and validates it as a safe
/// relative path.
fn _row_path(record: &csv::StringRecord, path_idx: usize) -> Result<String, CsvSourceError> {
    let relative_path =
        record.get(path_idx).ok_or_else(|| CsvSourceError("row missing 'path' value".to_string()))?.to_string();
    crate::paths::validate_relative_path(&relative_path)
        .map_err(|e| CsvSourceError(format!("invalid CSV row: {e}")))?;
    Ok(relative_path)
}

/// The `status` column of a report: the outcome of a comparison, or
/// `Scan` for a row written by `ftpdiff --build`, which scanned one side
/// without comparing it to anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportStatus {
    Diff(DiffStatus),
    Scan,
}

impl ReportStatus {
    /// The name written to the CSV `status` column.
    pub fn as_str(self) -> &'static str {
        match self {
            ReportStatus::Diff(status) => status.as_str(),
            ReportStatus::Scan => "Scan",
        }
    }
}

impl std::fmt::Display for ReportStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ReportStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "Scan" {
            return Ok(ReportStatus::Scan);
        }
        value.parse::<DiffStatus>().map(ReportStatus::Diff)
    }
}

/// One row of a CSV report, as written by `write_report`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportEntry {
    pub relative_path: String,
    pub status: ReportStatus,
    pub local_size: Option<u64>,
    pub remote_size: Option<u64>,
    pub local_md5: Option<String>,
    pub remote_md5: Option<String>,
}

impl ReportEntry {
    /// A `Scan` row for `--build`: exactly one side's size (and, later,
    /// hash) is known.
    pub fn scan(relative_path: impl Into<String>, local_size: Option<u64>, remote_size: Option<u64>) -> Self {
        Self {
            relative_path: relative_path.into(),
            status: ReportStatus::Scan,
            local_size,
            remote_size,
            local_md5: None,
            remote_md5: None,
        }
    }
}

impl From<&DiffEntry> for ReportEntry {
    fn from(entry: &DiffEntry) -> Self {
        Self {
            relative_path: entry.relative_path.clone(),
            status: ReportStatus::Diff(entry.status),
            local_size: entry.local_size,
            remote_size: entry.remote_size,
            local_md5: entry.local_md5.clone(),
            remote_md5: entry.remote_md5.clone(),
        }
    }
}

/// The path and status of one report row: all ftpops needs to decide what
/// to operate on.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusRow {
    pub relative_path: String,
    pub status: ReportStatus,
}

/// Reads every row's `path` and `status` from a report (other columns are
/// ignored). Fails on an unsafe path or an unknown status.
pub fn read_status_rows(path: &Path) -> Result<Vec<StatusRow>, CsvSourceError> {
    let (mut reader, headers) = _open(path)?;
    let path_idx = _column_index(&headers, "path")?;
    let status_idx = _column_index(&headers, "status")?;

    let mut rows = Vec::new();
    for result in reader.records() {
        let record = result.map_err(|e| CsvSourceError(format!("invalid CSV row: {e}")))?;
        let relative_path = _row_path(&record, path_idx)?;
        let status_text =
            record.get(status_idx).ok_or_else(|| CsvSourceError("row missing 'status' value".to_string()))?;
        let status = status_text
            .parse::<ReportStatus>()
            .map_err(|e| CsvSourceError(format!("row for '{relative_path}': {e}")))?;
        rows.push(StatusRow { relative_path, status });
    }

    Ok(rows)
}

/// Writes `entries` to `path` as a CSV report with the columns
/// `path,status,local_size,remote_size,local_md5,remote_md5`.
pub fn write_report(path: &Path, entries: &[ReportEntry]) -> std::io::Result<()> {
    let to_io = |e: csv::Error| std::io::Error::other(e);

    let mut writer = csv::Writer::from_path(path).map_err(to_io)?;
    writer.write_record(COLUMNS).map_err(to_io)?;

    for entry in entries {
        writer
            .write_record([
                entry.relative_path.clone(),
                entry.status.to_string(),
                entry.local_size.map(|v| v.to_string()).unwrap_or_default(),
                entry.remote_size.map(|v| v.to_string()).unwrap_or_default(),
                entry.local_md5.clone().unwrap_or_default(),
                entry.remote_md5.clone().unwrap_or_default(),
            ])
            .map_err(to_io)?;
    }

    writer.flush()
}

struct RawRow {
    relative_path: String,
    size: Option<u64>,
    md5: Option<String>,
}

fn read_side(path: &Path, size_column: &str, md5_column: &str) -> Result<Vec<RawRow>, CsvSourceError> {
    let (mut reader, headers) = _open(path)?;
    let path_idx = _column_index(&headers, "path")?;
    let size_idx = _column_index(&headers, size_column)?;
    let md5_idx = _column_index(&headers, md5_column)?;

    let mut rows = Vec::new();
    for result in reader.records() {
        let record = result.map_err(|e| CsvSourceError(format!("invalid CSV row: {e}")))?;
        let relative_path = _row_path(&record, path_idx)?;

        let size_str = record.get(size_idx).unwrap_or("");
        let size = if size_str.is_empty() {
            None
        } else {
            Some(size_str.parse::<u64>().map_err(|e| {
                CsvSourceError(format!("row for '{relative_path}' has invalid {size_column} '{size_str}': {e}"))
            })?)
        };

        let md5_str = record.get(md5_idx).unwrap_or("");
        let md5 = if md5_str.is_empty() { None } else { Some(md5_str.to_string()) };

        rows.push(RawRow { relative_path, size, md5 });
    }

    Ok(rows)
}

/// Reads `local_size`/`local_md5` for every row that has a `local_size`
/// value, from a CSV report written by `ftpdiff --csv`. Rows without a
/// `local_size` are skipped (they had no local file in the original
/// report).
pub fn read_local_entries(path: &Path) -> Result<(Vec<LocalEntry>, HashMap<String, String>), CsvSourceError> {
    let rows = read_side(path, "local_size", "local_md5")?;

    let mut entries = Vec::new();
    let mut known_md5 = HashMap::new();
    for row in rows {
        if let Some(size) = row.size {
            if let Some(md5) = row.md5 {
                known_md5.insert(row.relative_path.clone(), md5);
            }
            entries.push(LocalEntry { relative_path: row.relative_path, size });
        }
    }

    Ok((entries, known_md5))
}

/// Reads `remote_size`/`remote_md5` for every row that has a
/// `remote_size` value, from a CSV report written by `ftpdiff --csv`.
/// Rows without a `remote_size` are skipped (they had no remote file in
/// the original report).
pub fn read_remote_entries(path: &Path) -> Result<(Vec<RemoteEntry>, HashMap<String, String>), CsvSourceError> {
    let rows = read_side(path, "remote_size", "remote_md5")?;

    let mut entries = Vec::new();
    let mut known_md5 = HashMap::new();
    for row in rows {
        if let Some(size) = row.size {
            if let Some(md5) = row.md5 {
                known_md5.insert(row.relative_path.clone(), md5);
            }
            entries.push(RemoteEntry { relative_path: row.relative_path, size, modified: None });
        }
    }

    Ok((entries, known_md5))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_csv(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.csv");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    const HEADER: &str = "path,status,local_size,remote_size,local_md5,remote_md5\n";

    #[test]
    fn reads_local_entries_and_skips_rows_without_local_size() {
        let (_dir, path) = write_csv(&format!(
            "{HEADER}\
             a.txt,Match,10,10,abc,abc\n\
             b.txt,RemoteOnly,,20,,def\n"
        ));

        let (entries, known_md5) = read_local_entries(&path).unwrap();

        assert_eq!(entries, vec![LocalEntry { relative_path: "a.txt".to_string(), size: 10 }]);
        assert_eq!(known_md5.get("a.txt"), Some(&"abc".to_string()));
        assert_eq!(known_md5.get("b.txt"), None);
    }

    #[test]
    fn reads_remote_entries_and_skips_rows_without_remote_size() {
        let (_dir, path) = write_csv(&format!(
            "{HEADER}\
             a.txt,Match,10,10,abc,abc\n\
             c.txt,LocalOnly,5,,ghi,\n"
        ));

        let (entries, known_md5) = read_remote_entries(&path).unwrap();

        assert_eq!(entries, vec![RemoteEntry { relative_path: "a.txt".to_string(), size: 10, modified: None }]);
        assert_eq!(known_md5.get("a.txt"), Some(&"abc".to_string()));
        assert_eq!(known_md5.get("c.txt"), None);
    }

    #[test]
    fn row_without_md5_is_not_added_to_known_map() {
        let (_dir, path) = write_csv(&format!("{HEADER}a.txt,SizeMismatch,10,20,,\n"));

        let (entries, known_md5) = read_local_entries(&path).unwrap();

        assert_eq!(entries, vec![LocalEntry { relative_path: "a.txt".to_string(), size: 10 }]);
        assert!(known_md5.is_empty());
    }

    #[test]
    fn errors_on_malformed_size_value() {
        let (_dir, path) = write_csv(&format!("{HEADER}a.txt,Match,not-a-number,10,,\n"));

        let result = read_local_entries(&path);

        assert!(result.is_err());
    }

    #[test]
    fn errors_when_required_column_missing() {
        let (_dir, path) = write_csv("path,status\na.txt,Match\n");

        let result = read_local_entries(&path);

        assert!(result.is_err());
    }

    #[test]
    fn errors_on_path_traversal_row() {
        let (_dir, path) = write_csv(&format!("{HEADER}../../etc/x,Match,1,1,,\n"));

        assert!(read_local_entries(&path).is_err());
        assert!(read_remote_entries(&path).is_err());
    }

    #[test]
    fn writes_header_and_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.csv");
        let entries = vec![ReportEntry::from(&DiffEntry::new("a.txt", DiffStatus::SizeMismatch, Some(10), Some(20)))];

        write_report(&path, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let mut lines = content.lines();
        assert_eq!(lines.next().unwrap(), "path,status,local_size,remote_size,local_md5,remote_md5");
        assert_eq!(lines.next().unwrap(), "a.txt,SizeMismatch,10,20,,");
    }

    #[test]
    fn report_status_names_round_trip() {
        for status in DiffStatus::ALL.map(ReportStatus::Diff).into_iter().chain([ReportStatus::Scan]) {
            assert_eq!(status.to_string().parse::<ReportStatus>(), Ok(status));
        }
        assert!("Bogus".parse::<ReportStatus>().is_err());
    }

    #[test]
    fn written_report_can_be_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.csv");
        let mut entry = DiffEntry::new("sub/a.txt", DiffStatus::Match, Some(3), Some(3));
        entry.local_md5 = Some("abc".to_string());
        entry.remote_md5 = Some("abc".to_string());
        let entries = vec![
            ReportEntry::from(&entry),
            ReportEntry::from(&DiffEntry::new("b.txt", DiffStatus::RemoteOnly, None, Some(9))),
            ReportEntry::scan("c.txt", Some(1), None),
        ];

        write_report(&path, &entries).unwrap();

        let rows = read_status_rows(&path).unwrap();
        assert_eq!(
            rows,
            vec![
                StatusRow { relative_path: "sub/a.txt".to_string(), status: ReportStatus::Diff(DiffStatus::Match) },
                StatusRow { relative_path: "b.txt".to_string(), status: ReportStatus::Diff(DiffStatus::RemoteOnly) },
                StatusRow { relative_path: "c.txt".to_string(), status: ReportStatus::Scan },
            ]
        );
        let (local, known) = read_local_entries(&path).unwrap();
        assert_eq!(local.len(), 2); // sub/a.txt and the scanned c.txt both have a local size
        assert_eq!(known.get("sub/a.txt"), Some(&"abc".to_string()));
    }

    #[test]
    fn status_rows_reject_unknown_status_and_unsafe_paths() {
        let (_dir, bad_status) = write_csv("path,status\na.txt,Weird\n");
        let err = read_status_rows(&bad_status).unwrap_err();
        assert!(err.to_string().contains("Weird") && err.to_string().contains("a.txt"), "{err}");

        let (_dir2, bad_path) = write_csv("path,status\n../x,LocalOnly\n");
        assert!(read_status_rows(&bad_path).is_err());

        let (_dir3, no_status_col) = write_csv("path,size\na.txt,1\n");
        assert!(read_status_rows(&no_status_col).is_err());
    }

    #[test]
    fn errors_when_file_missing() {
        let result = read_local_entries(std::path::Path::new("/nonexistent/report.csv"));

        assert!(result.is_err());
    }
}
