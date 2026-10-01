//! Plain-text change log written by `ftpwatch check`.

use crate::compare::{Change, ChangeKind, Comparison, FileState};
use crate::timefmt::format_datetime;

/// Upper-case label of a change kind.
fn _kind_label(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::New => "NEW",
        ChangeKind::Deleted => "DELETED",
        ChangeKind::Modified => "MODIFIED",
    }
}

/// `size=N` plus ` mtime=...` when the modification time is known.
fn _describe(state: &FileState) -> String {
    match state.modified {
        Some(m) => format!("size={}  mtime={}", state.size, format_datetime(m)),
        None => format!("size={}", state.size),
    }
}

/// The parts of a modification that differ: size and/or mtime.
fn _describe_modification(old: &FileState, new: &FileState) -> String {
    let mut parts = Vec::new();
    if old.size != new.size {
        parts.push(format!("size {} -> {}", old.size, new.size));
    }
    if let (Some(a), Some(b)) = (old.modified, new.modified) {
        if a != b {
            parts.push(format!("mtime {} -> {}", format_datetime(a), format_datetime(b)));
        }
    }
    parts.join(", ")
}

/// One log line for a change: label, path and details.
fn _change_line(change: &Change) -> String {
    let detail = match (change.kind, &change.old, &change.new) {
        (ChangeKind::New, _, Some(new)) => _describe(new),
        (ChangeKind::Deleted, Some(old), _) => _describe(old),
        (ChangeKind::Modified, Some(old), Some(new)) => _describe_modification(old, new),
        _ => String::new(),
    };
    format!("{:<9} {}  {}", _kind_label(change.kind), change.path, detail).trim_end().to_string()
}

/// Builds the text of a log file: a header, a summary and one line per
/// change (already sorted by `compare_snapshots`). The header time is UTC; file
/// mtimes are printed as the server listed them (its local time).
pub fn format_log(site: &str, generated: i64, previous_snapshot: &str, comparison: &Comparison) -> String {
    let mut out = format!(
        "ftpwatch check - {site} - {} UTC\nPrevious snapshot: {previous_snapshot}\n",
        format_datetime(generated)
    );

    if comparison.changes.is_empty() {
        out.push_str("Summary: no changes\n");
        return out;
    }

    let count = |kind: ChangeKind| comparison.changes.iter().filter(|c| c.kind == kind).count();
    out.push_str(&format!(
        "Summary: {} new, {} deleted, {} modified, {} unchanged\n\n",
        count(ChangeKind::New),
        count(ChangeKind::Deleted),
        count(ChangeKind::Modified),
        comparison.unchanged
    ));
    for change in &comparison.changes {
        out.push_str(&_change_line(change));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::{Change, ChangeKind, Comparison, FileState};

    const GENERATED: i64 = 1_000_000_000; // 2001-09-09 01:46:40 UTC

    fn state(size: u64, modified: Option<i64>) -> Option<FileState> {
        Some(FileState { size, modified })
    }

    #[test]
    fn no_changes_log_has_only_header_and_summary() {
        let comparison = Comparison { changes: vec![], unchanged: 12 };

        let log = format_log("test.com", GENERATED, "2001-09-08_014640.csv", &comparison);

        assert_eq!(
            log,
            "ftpwatch check - test.com - 2001-09-09 01:46:40 UTC\n\
             Previous snapshot: 2001-09-08_014640.csv\n\
             Summary: no changes\n"
        );
    }

    #[test]
    fn lists_each_change_with_details() {
        let comparison = Comparison {
            changes: vec![
                Change {
                    path: "new.php".into(),
                    kind: ChangeKind::New,
                    old: None,
                    new: state(1204, Some(1_578_182_400)),
                },
                Change { path: "old.css".into(), kind: ChangeKind::Deleted, old: state(88_210, None), new: None },
                Change {
                    path: "wp-config.php".into(),
                    kind: ChangeKind::Modified,
                    old: state(3021, Some(1_578_182_400)),
                    new: state(3050, Some(1_578_268_800)),
                },
            ],
            unchanged: 1840,
        };

        let log = format_log("test.com", GENERATED, "prev.csv", &comparison);

        assert!(log.contains("Summary: 1 new, 1 deleted, 1 modified, 1840 unchanged\n"), "{log}");
        assert!(log.contains("NEW       new.php  size=1204  mtime=2020-01-05 00:00:00\n"), "{log}");
        assert!(log.contains("DELETED   old.css  size=88210\n"), "{log}");
        assert!(
            log.contains(
                "MODIFIED  wp-config.php  size 3021 -> 3050, mtime 2020-01-05 00:00:00 -> 2020-01-06 00:00:00\n"
            ),
            "{log}"
        );
    }

    #[test]
    fn modified_with_same_size_shows_only_the_mtime() {
        let comparison = Comparison {
            changes: vec![Change {
                path: "a.php".into(),
                kind: ChangeKind::Modified,
                old: state(10, Some(1_578_182_400)),
                new: state(10, Some(1_578_268_800)),
            }],
            unchanged: 0,
        };

        let log = format_log("s", GENERATED, "p.csv", &comparison);

        assert!(log.contains("MODIFIED  a.php  mtime 2020-01-05 00:00:00 -> 2020-01-06 00:00:00\n"), "{log}");
    }
}
