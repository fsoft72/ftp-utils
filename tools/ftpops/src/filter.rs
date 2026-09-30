//! The `--filter` CLI value and its mapping to ftpdiff diff statuses.

use clap::ValueEnum;
use ftp_utils_core::csv_source::{ReportStatus, StatusRow};
use ftp_utils_core::DiffStatus;

/// Which diff status to operate on. clap's default kebab-case casing
/// gives `--filter remote-only` / `--filter local-only`.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    RemoteOnly,
    LocalOnly,
}

impl Filter {
    /// The diff status this filter selects in the report's `status` column.
    pub fn status(self) -> ReportStatus {
        match self {
            Filter::RemoteOnly => ReportStatus::Diff(DiffStatus::RemoteOnly),
            Filter::LocalOnly => ReportStatus::Diff(DiffStatus::LocalOnly),
        }
    }
}

/// Returns the rows whose status is `status`.
pub fn filter_by_status(rows: &[StatusRow], status: ReportStatus) -> Vec<&StatusRow> {
    rows.iter().filter(|r| r.status == status).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_map_to_diff_statuses() {
        assert_eq!(Filter::RemoteOnly.status(), ReportStatus::Diff(DiffStatus::RemoteOnly));
        assert_eq!(Filter::LocalOnly.status(), ReportStatus::Diff(DiffStatus::LocalOnly));
    }

    #[test]
    fn filter_by_status_selects_matching_rows_only() {
        let row = |path: &str, status| StatusRow { relative_path: path.to_string(), status };
        let rows = vec![
            row("a.txt", ReportStatus::Diff(DiffStatus::RemoteOnly)),
            row("b.txt", ReportStatus::Diff(DiffStatus::Match)),
            row("c.txt", ReportStatus::Scan),
            row("d.txt", ReportStatus::Diff(DiffStatus::RemoteOnly)),
        ];

        let filtered = filter_by_status(&rows, ReportStatus::Diff(DiffStatus::RemoteOnly));

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].relative_path, "a.txt");
        assert_eq!(filtered[1].relative_path, "d.txt");
    }
}
