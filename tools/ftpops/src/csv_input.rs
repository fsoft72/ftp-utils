//! Reads an `ftpdiff --csv` report and filters rows by status.

use std::path::Path;

/// One row from an ftpdiff CSV report, reduced to what ftpops needs.
#[derive(Debug, Clone, PartialEq)]
pub struct CsvRow {
    pub relative_path: String,
    pub status: String,
}

#[derive(Debug)]
pub struct CsvError(pub String);

impl std::fmt::Display for CsvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CsvError {}

/// Reads every row of the CSV at `path`, keyed by its `path` and `status`
/// columns (as written by `ftpdiff --csv`; extra columns are ignored).
pub fn read_rows(path: &Path) -> Result<Vec<CsvRow>, CsvError> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|e| CsvError(format!("cannot read CSV {}: {e}", path.display())))?;

    let headers = reader.headers().map_err(|e| CsvError(e.to_string()))?.clone();
    let path_idx = headers
        .iter()
        .position(|h| h == "path")
        .ok_or_else(|| CsvError("CSV missing 'path' column".to_string()))?;
    let status_idx = headers
        .iter()
        .position(|h| h == "status")
        .ok_or_else(|| CsvError("CSV missing 'status' column".to_string()))?;

    let mut rows = Vec::new();
    for result in reader.records() {
        let record = result.map_err(|e| CsvError(format!("invalid CSV row: {e}")))?;
        let relative_path = record
            .get(path_idx)
            .ok_or_else(|| CsvError("row missing 'path' value".to_string()))?
            .to_string();
        let status = record
            .get(status_idx)
            .ok_or_else(|| CsvError("row missing 'status' value".to_string()))?
            .to_string();
        rows.push(CsvRow { relative_path, status });
    }

    Ok(rows)
}

/// Returns the rows whose `status` exactly matches `status`.
pub fn filter_by_status<'a>(rows: &'a [CsvRow], status: &str) -> Vec<&'a CsvRow> {
    rows.iter().filter(|r| r.status == status).collect()
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

    #[test]
    fn reads_path_and_status_columns() {
        let (_dir, path) = write_csv(
            "path,status,local_size,remote_size,local_md5,remote_md5\n\
             a.txt,RemoteOnly,,10,,\n\
             sub/b.txt,LocalOnly,5,,,\n",
        );

        let rows = read_rows(&path).unwrap();

        assert_eq!(
            rows,
            vec![
                CsvRow { relative_path: "a.txt".to_string(), status: "RemoteOnly".to_string() },
                CsvRow { relative_path: "sub/b.txt".to_string(), status: "LocalOnly".to_string() },
            ]
        );
    }

    #[test]
    fn errors_when_file_missing() {
        let result = read_rows(std::path::Path::new("/nonexistent/report.csv"));

        assert!(result.is_err());
    }

    #[test]
    fn filter_by_status_selects_matching_rows_only() {
        let rows = vec![
            CsvRow { relative_path: "a.txt".to_string(), status: "RemoteOnly".to_string() },
            CsvRow { relative_path: "b.txt".to_string(), status: "Match".to_string() },
            CsvRow { relative_path: "c.txt".to_string(), status: "RemoteOnly".to_string() },
        ];

        let filtered = filter_by_status(&rows, "RemoteOnly");

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].relative_path, "a.txt");
        assert_eq!(filtered[1].relative_path, "c.txt");
    }
}
