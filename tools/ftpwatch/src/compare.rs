//! Pure comparison of two remote snapshots: what is new, deleted or
//! modified between a previous scan and the current one.

use std::collections::{HashMap, HashSet};

use ftp_utils_core::remote::RemoteEntry;

/// Seconds per day; mtimes are compared at day precision because `LIST`
/// shows only the date (no time of day) for files older than about six
/// months, so exact comparison would flag them spuriously.
const SECONDS_PER_DAY: i64 = 86_400;

/// What happened to a file between two scans. The declaration order is
/// the order of the lines in the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeKind {
    New,
    Deleted,
    Modified,
}

/// Size and modification time of a file at one scan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FileState {
    pub size: u64,
    /// Unix seconds as printed by the server (its local time, treated as UTC), `None` if the listing had no usable date.
    pub modified: Option<i64>,
}

/// One changed file. `old` is `None` for `New`, `new` is `None` for `Deleted`.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub path: String,
    pub kind: ChangeKind,
    pub old: Option<FileState>,
    pub new: Option<FileState>,
}

/// The outcome of comparing two scans.
#[derive(Debug, Default, PartialEq)]
pub struct Comparison {
    /// Sorted by kind (new, deleted, modified), then by path.
    pub changes: Vec<Change>,
    /// Files present in both scans and not modified.
    pub unchanged: usize,
}

/// Extracts the size and mtime of a snapshot entry.
fn _state(entry: &RemoteEntry) -> FileState {
    FileState { size: entry.size, modified: entry.modified }
}

/// Whether a file present in both scans counts as modified: the size
/// differs, or both mtimes are known and fall on different days.
fn _is_modified(old: &FileState, new: &FileState) -> bool {
    if old.size != new.size {
        return true;
    }
    match (old.modified, new.modified) {
        (Some(a), Some(b)) => a.div_euclid(SECONDS_PER_DAY) != b.div_euclid(SECONDS_PER_DAY),
        _ => false,
    }
}

/// Compares the `previous` scan with the `current` one, keyed by relative
/// path.
pub fn compare_snapshots(previous: &[RemoteEntry], current: &[RemoteEntry]) -> Comparison {
    let old_by_path: HashMap<&str, FileState> =
        previous.iter().map(|e| (e.relative_path.as_str(), _state(e))).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut comparison = Comparison::default();

    for entry in current {
        let path = entry.relative_path.as_str();
        seen.insert(path);
        let new = _state(entry);
        match old_by_path.get(path) {
            None => comparison.changes.push(Change {
                path: path.to_string(),
                kind: ChangeKind::New,
                old: None,
                new: Some(new),
            }),
            Some(old) if _is_modified(old, &new) => comparison.changes.push(Change {
                path: path.to_string(),
                kind: ChangeKind::Modified,
                old: Some(*old),
                new: Some(new),
            }),
            Some(_) => comparison.unchanged += 1,
        }
    }

    for entry in previous {
        if seen.contains(entry.relative_path.as_str()) {
            continue;
        }
        comparison.changes.push(Change {
            path: entry.relative_path.clone(),
            kind: ChangeKind::Deleted,
            old: Some(_state(entry)),
            new: None,
        });
    }

    comparison.changes.sort_by(|a, b| (a.kind, &a.path).cmp(&(b.kind, &b.path)));
    comparison
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    const T0: i64 = 1_790_000_000;

    fn entry(path: &str, size: u64, modified: Option<i64>) -> RemoteEntry {
        RemoteEntry { relative_path: path.to_string(), size, modified }
    }

    fn kinds(comparison: &Comparison) -> Vec<(ChangeKind, &str)> {
        comparison.changes.iter().map(|c| (c.kind, c.path.as_str())).collect()
    }

    #[test]
    fn identical_snapshots_have_no_changes() {
        let snapshot = vec![entry("a.php", 10, Some(T0)), entry("b.php", 20, Some(T0))];

        let comparison = compare_snapshots(&snapshot, &snapshot);

        assert!(comparison.changes.is_empty());
        assert_eq!(comparison.unchanged, 2);
    }

    #[test]
    fn reports_new_deleted_and_modified_sorted_by_kind_then_path() {
        let previous = vec![entry("gone.php", 1, Some(T0)), entry("edit.php", 5, Some(T0)), entry("same.php", 7, None)];
        let current = vec![
            entry("z-new.php", 3, Some(T0)),
            entry("a-new.php", 4, Some(T0)),
            entry("edit.php", 6, Some(T0)),
            entry("same.php", 7, None),
        ];

        let comparison = compare_snapshots(&previous, &current);

        assert_eq!(
            kinds(&comparison),
            vec![
                (ChangeKind::New, "a-new.php"),
                (ChangeKind::New, "z-new.php"),
                (ChangeKind::Deleted, "gone.php"),
                (ChangeKind::Modified, "edit.php"),
            ]
        );
        assert_eq!(comparison.unchanged, 1);
        let modified = &comparison.changes[3];
        assert_eq!(modified.old, Some(FileState { size: 5, modified: Some(T0) }));
        assert_eq!(modified.new, Some(FileState { size: 6, modified: Some(T0) }));
    }

    #[test]
    fn a_different_mtime_alone_is_a_modification() {
        let previous = vec![entry("a.php", 10, Some(T0))];
        let current = vec![entry("a.php", 10, Some(T0 + 3 * DAY))];

        assert_eq!(kinds(&compare_snapshots(&previous, &current)), vec![(ChangeKind::Modified, "a.php")]);
    }

    #[test]
    fn mtimes_within_the_same_day_are_equal() {
        // LIST only gives the day for old files, so time of day is ignored.
        let day_start = (T0 / DAY) * DAY;
        let previous = vec![entry("a.php", 10, Some(day_start))];
        let current = vec![entry("a.php", 10, Some(day_start + DAY - 1))];

        assert!(compare_snapshots(&previous, &current).changes.is_empty());
    }

    #[test]
    fn unknown_mtime_on_either_side_compares_size_only() {
        let known = vec![entry("a.php", 10, Some(T0))];
        let unknown = vec![entry("a.php", 10, None)];

        assert!(compare_snapshots(&known, &unknown).changes.is_empty());
        assert!(compare_snapshots(&unknown, &known).changes.is_empty());

        let resized = vec![entry("a.php", 11, None)];
        assert_eq!(kinds(&compare_snapshots(&known, &resized)), vec![(ChangeKind::Modified, "a.php")]);
    }

    #[test]
    fn empty_previous_makes_everything_new() {
        let comparison = compare_snapshots(&[], &[entry("a.php", 1, None)]);

        assert_eq!(kinds(&comparison), vec![(ChangeKind::New, "a.php")]);
        assert_eq!(comparison.unchanged, 0);
    }
}
