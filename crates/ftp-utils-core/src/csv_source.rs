//! Reads a previously-written `ftpdiff --csv` report as a substitute for
//! a live filesystem/FTP directory listing, for one side of a comparison
//! (`--local-csv` / `--remote-csv`).

use std::collections::HashMap;
use std::path::Path;

use crate::local::LocalEntry;
use crate::remote::RemoteEntry;

#[derive(Debug)]
pub struct CsvSourceError(pub String);

impl std::fmt::Display for CsvSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CsvSourceError {}

struct RawRow {
    relative_path: String,
    size: Option<u64>,
    md5: Option<String>,
}

fn read_side(path: &Path, size_column: &str, md5_column: &str) -> Result<Vec<RawRow>, CsvSourceError> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|e| CsvSourceError(format!("cannot read CSV {}: {e}", path.display())))?;

    let headers = reader.headers().map_err(|e| CsvSourceError(e.to_string()))?.clone();
    let path_idx = headers
        .iter()
        .position(|h| h == "path")
        .ok_or_else(|| CsvSourceError("CSV missing 'path' column".to_string()))?;
    let size_idx = headers
        .iter()
        .position(|h| h == size_column)
        .ok_or_else(|| CsvSourceError(format!("CSV missing '{size_column}' column")))?;
    let md5_idx = headers
        .iter()
        .position(|h| h == md5_column)
        .ok_or_else(|| CsvSourceError(format!("CSV missing '{md5_column}' column")))?;

    let mut rows = Vec::new();
    for result in reader.records() {
        let record = result.map_err(|e| CsvSourceError(format!("invalid CSV row: {e}")))?;

        let relative_path = record
            .get(path_idx)
            .ok_or_else(|| CsvSourceError("row missing 'path' value".to_string()))?
            .to_string();

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
            entries.push(RemoteEntry { relative_path: row.relative_path, size });
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

        assert_eq!(entries, vec![RemoteEntry { relative_path: "a.txt".to_string(), size: 10 }]);
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
    fn errors_when_file_missing() {
        let result = read_local_entries(std::path::Path::new("/nonexistent/report.csv"));

        assert!(result.is_err());
    }
}
