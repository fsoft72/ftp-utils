//! CSV report output for diff results.

use std::path::Path;

use ftp_utils_core::DiffEntry;

/// Writes `entries` to `path` as CSV with columns:
/// path,status,local_size,remote_size,local_md5,remote_md5
pub fn write_csv(path: &Path, entries: &[DiffEntry]) -> std::io::Result<()> {
    let mut writer = csv::Writer::from_path(path).map_err(csv_error_to_io)?;
    writer
        .write_record(["path", "status", "local_size", "remote_size", "local_md5", "remote_md5"])
        .map_err(csv_error_to_io)?;

    for entry in entries {
        writer
            .write_record([
                entry.relative_path.clone(),
                format!("{:?}", entry.status),
                entry.local_size.map(|v| v.to_string()).unwrap_or_default(),
                entry.remote_size.map(|v| v.to_string()).unwrap_or_default(),
                entry.local_md5.clone().unwrap_or_default(),
                entry.remote_md5.clone().unwrap_or_default(),
            ])
            .map_err(csv_error_to_io)?;
    }

    writer.flush()?;
    Ok(())
}

fn csv_error_to_io(e: csv::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ftp_utils_core::DiffStatus;

    #[test]
    fn writes_header_and_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.csv");

        let entries = vec![DiffEntry {
            relative_path: "a.txt".to_string(),
            status: DiffStatus::SizeMismatch,
            local_size: Some(10),
            remote_size: Some(20),
            local_md5: None,
            remote_md5: None,
        }];

        write_csv(&path, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let mut lines = content.lines();
        assert_eq!(lines.next().unwrap(), "path,status,local_size,remote_size,local_md5,remote_md5");
        assert_eq!(lines.next().unwrap(), "a.txt,SizeMismatch,10,20,,");
    }
}
