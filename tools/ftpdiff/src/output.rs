//! Human-readable colored text output for diff results.

use colored::Colorize;
use ftp_utils_core::{DiffEntry, DiffStatus};

/// Formats one diff entry as a single colored line.
pub fn format_entry(entry: &DiffEntry) -> String {
    match entry.status {
        DiffStatus::LocalOnly => format!("{} {}", "+".green(), entry.relative_path),
        DiffStatus::RemoteOnly => format!("{} {}", "-".red(), entry.relative_path),
        DiffStatus::SizeMismatch => format!(
            "{} {} (local={:?}, remote={:?})",
            "~".yellow(),
            entry.relative_path,
            entry.local_size,
            entry.remote_size
        ),
        DiffStatus::HashMismatch => format!("{} {} (hash differs)", "~".yellow(), entry.relative_path),
        DiffStatus::Match => format!("{} {}", "=".dimmed(), entry.relative_path),
        DiffStatus::Scan => format!("{} {}", "*".cyan(), entry.relative_path),
    }
}

/// Counts of entries by status.
pub struct Summary {
    pub local_only: usize,
    pub remote_only: usize,
    pub size_mismatch: usize,
    pub hash_mismatch: usize,
    pub matched: usize,
}

/// Tallies `entries` by status.
pub fn summarize(entries: &[DiffEntry]) -> Summary {
    let mut summary = Summary { local_only: 0, remote_only: 0, size_mismatch: 0, hash_mismatch: 0, matched: 0 };
    for entry in entries {
        match entry.status {
            DiffStatus::LocalOnly => summary.local_only += 1,
            DiffStatus::RemoteOnly => summary.remote_only += 1,
            DiffStatus::SizeMismatch => summary.size_mismatch += 1,
            DiffStatus::HashMismatch => summary.hash_mismatch += 1,
            DiffStatus::Match => summary.matched += 1,
            // Never produced by compare_entries; --build mode has its own
            // "Scanned N entries." line instead of this summary.
            DiffStatus::Scan => {}
        }
    }
    summary
}

/// Formats a one-line summary of the tallies.
pub fn format_summary(summary: &Summary) -> String {
    format!(
        "Summary: {} matched, {} local-only, {} remote-only, {} size mismatch, {} hash mismatch",
        summary.matched, summary.local_only, summary.remote_only, summary.size_mismatch, summary.hash_mismatch
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(relative_path: &str, status: DiffStatus) -> DiffEntry {
        DiffEntry {
            relative_path: relative_path.to_string(),
            status,
            local_size: None,
            remote_size: None,
            local_md5: None,
            remote_md5: None,
        }
    }

    #[test]
    fn formats_each_status_with_the_relative_path() {
        colored::control::set_override(false);

        assert!(format_entry(&entry("a.txt", DiffStatus::LocalOnly)).contains("a.txt"));
        assert!(format_entry(&entry("b.txt", DiffStatus::RemoteOnly)).contains("b.txt"));
        assert!(format_entry(&entry("c.txt", DiffStatus::SizeMismatch)).contains("c.txt"));
        assert!(format_entry(&entry("d.txt", DiffStatus::HashMismatch)).contains("d.txt"));
        assert!(format_entry(&entry("e.txt", DiffStatus::Match)).contains("e.txt"));
        assert!(format_entry(&entry("f.txt", DiffStatus::Scan)).contains("f.txt"));
    }

    #[test]
    fn summarizes_counts_by_status() {
        let entries = vec![
            entry("a", DiffStatus::LocalOnly),
            entry("b", DiffStatus::LocalOnly),
            entry("c", DiffStatus::RemoteOnly),
            entry("d", DiffStatus::Match),
        ];

        let summary = summarize(&entries);

        assert_eq!(summary.local_only, 2);
        assert_eq!(summary.remote_only, 1);
        assert_eq!(summary.matched, 1);
        assert_eq!(summary.size_mismatch, 0);
        assert_eq!(summary.hash_mismatch, 0);
    }

    #[test]
    fn formats_summary_line() {
        let summary = Summary { local_only: 1, remote_only: 2, size_mismatch: 3, hash_mismatch: 4, matched: 5 };
        let line = format_summary(&summary);

        assert!(line.contains("5 matched"));
        assert!(line.contains("1 local-only"));
        assert!(line.contains("2 remote-only"));
        assert!(line.contains("3 size mismatch"));
        assert!(line.contains("4 hash mismatch"));
    }
}
