//! The `--filter` CLI value and its mapping to ftpdiff diff statuses.

use clap::ValueEnum;
use ftp_utils_core::csv_source::StatusRow;
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
    pub fn status(self) -> DiffStatus {
        match self {
            Filter::RemoteOnly => DiffStatus::RemoteOnly,
            Filter::LocalOnly => DiffStatus::LocalOnly,
        }
    }
}

/// Returns the rows whose status is `status`.
pub fn filter_by_status(rows: &[StatusRow], status: DiffStatus) -> Vec<&StatusRow> {
    rows.iter().filter(|r| r.status == status).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_map_to_diff_statuses() {
        assert_eq!(Filter::RemoteOnly.status(), DiffStatus::RemoteOnly);
        assert_eq!(Filter::LocalOnly.status(), DiffStatus::LocalOnly);
    }

    #[test]
    fn filter_by_status_selects_matching_rows_only() {
        let row = |path: &str, status| StatusRow { relative_path: path.to_string(), status };
        let rows = vec![
            row("a.txt", DiffStatus::RemoteOnly),
            row("b.txt", DiffStatus::Match),
            row("c.txt", DiffStatus::RemoteOnly),
        ];

        let filtered = filter_by_status(&rows, DiffStatus::RemoteOnly);

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].relative_path, "a.txt");
        assert_eq!(filtered[1].relative_path, "c.txt");
    }
}
