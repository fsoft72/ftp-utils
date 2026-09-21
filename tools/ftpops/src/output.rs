//! Colored text output for ftpops operation results.

use colored::Colorize;

use crate::ops::{OpOutcome, OpResult};

/// Formats one result as a colored line: `+` copied, `-` deleted,
/// `~` skipped, `!` failed.
pub fn format_result(result: &OpResult) -> String {
    match &result.outcome {
        OpOutcome::Copied => format!("{} {}", "+".green(), result.relative_path),
        OpOutcome::Deleted => format!("{} {}", "-".red(), result.relative_path),
        OpOutcome::Skipped => format!("{} {} (skipped, already exists)", "~".yellow(), result.relative_path),
        OpOutcome::Failed(e) => format!("{} {} ({e})", "!".red().bold(), result.relative_path),
    }
}

/// Formats a single dry-run preview line: "Would <verb>: <path>".
pub fn format_dry_run_line(verb: &str, relative_path: &str) -> String {
    format!("Would {verb}: {relative_path}")
}

/// Counts of results by outcome.
pub struct Summary {
    pub succeeded: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Tallies `results` by outcome (`Copied`/`Deleted` both count as
/// succeeded).
pub fn summarize(results: &[OpResult]) -> Summary {
    let mut summary = Summary { succeeded: 0, skipped: 0, failed: 0 };
    for result in results {
        match result.outcome {
            OpOutcome::Copied | OpOutcome::Deleted => summary.succeeded += 1,
            OpOutcome::Skipped => summary.skipped += 1,
            OpOutcome::Failed(_) => summary.failed += 1,
        }
    }
    summary
}

/// Formats a one-line summary of the tallies.
pub fn format_summary(summary: &Summary) -> String {
    format!(
        "Summary: {} succeeded, {} skipped, {} failed",
        summary.succeeded, summary.skipped, summary.failed
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(relative_path: &str, outcome: OpOutcome) -> OpResult {
        OpResult { relative_path: relative_path.to_string(), outcome }
    }

    #[test]
    fn formats_each_outcome_with_the_relative_path() {
        colored::control::set_override(false);

        assert!(format_result(&result("a.txt", OpOutcome::Copied)).contains("a.txt"));
        assert!(format_result(&result("b.txt", OpOutcome::Deleted)).contains("b.txt"));
        assert!(format_result(&result("c.txt", OpOutcome::Skipped)).contains("c.txt"));
        assert!(format_result(&result("d.txt", OpOutcome::Failed("boom".to_string()))).contains("d.txt"));
        assert!(format_result(&result("d.txt", OpOutcome::Failed("boom".to_string()))).contains("boom"));
    }

    #[test]
    fn formats_dry_run_line() {
        let line = format_dry_run_line("copy", "a.txt");

        assert_eq!(line, "Would copy: a.txt");
    }

    #[test]
    fn summarizes_counts_by_outcome() {
        let results = vec![
            result("a", OpOutcome::Copied),
            result("b", OpOutcome::Deleted),
            result("c", OpOutcome::Skipped),
            result("d", OpOutcome::Failed("x".to_string())),
        ];

        let summary = summarize(&results);

        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.failed, 1);
    }

    #[test]
    fn formats_summary_line() {
        let summary = Summary { succeeded: 3, skipped: 1, failed: 2 };
        let line = format_summary(&summary);

        assert!(line.contains("3 succeeded"));
        assert!(line.contains("1 skipped"));
        assert!(line.contains("2 failed"));
    }
}
