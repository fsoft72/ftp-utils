//! Combines local and remote entry lists into size-based diff results.

use std::collections::HashMap;

use crate::diff::{DiffEntry, DiffStatus};
use crate::local::LocalEntry;
use crate::remote::RemoteEntry;

/// Compares `local` and `remote` entries by relative path and size,
/// producing one `DiffEntry` per distinct relative path seen on either
/// side. Does not consider hashes; see `hash::apply_hash_comparison` for
/// the optional hash-based upgrade pass.
pub fn compare_entries(local: &[LocalEntry], remote: &[RemoteEntry]) -> Vec<DiffEntry> {
    let mut local_by_path: HashMap<&str, &LocalEntry> =
        local.iter().map(|e| (e.relative_path.as_str(), e)).collect();
    let mut remote_by_path: HashMap<&str, &RemoteEntry> =
        remote.iter().map(|e| (e.relative_path.as_str(), e)).collect();

    let mut all_paths: Vec<&str> = local_by_path
        .keys()
        .chain(remote_by_path.keys())
        .copied()
        .collect();
    all_paths.sort_unstable();
    all_paths.dedup();

    all_paths
        .into_iter()
        .map(|path| {
            let local_entry = local_by_path.remove(path);
            let remote_entry = remote_by_path.remove(path);

            match (local_entry, remote_entry) {
                (Some(l), None) => DiffEntry {
                    relative_path: path.to_string(),
                    status: DiffStatus::LocalOnly,
                    local_size: Some(l.size),
                    remote_size: None,
                    local_md5: None,
                    remote_md5: None,
                },
                (None, Some(r)) => DiffEntry {
                    relative_path: path.to_string(),
                    status: DiffStatus::RemoteOnly,
                    local_size: None,
                    remote_size: Some(r.size),
                    local_md5: None,
                    remote_md5: None,
                },
                (Some(l), Some(r)) if l.size == r.size => DiffEntry {
                    relative_path: path.to_string(),
                    status: DiffStatus::Match,
                    local_size: Some(l.size),
                    remote_size: Some(r.size),
                    local_md5: None,
                    remote_md5: None,
                },
                (Some(l), Some(r)) => DiffEntry {
                    relative_path: path.to_string(),
                    status: DiffStatus::SizeMismatch,
                    local_size: Some(l.size),
                    remote_size: Some(r.size),
                    local_md5: None,
                    remote_md5: None,
                },
                (None, None) => unreachable!("path came from one of the two maps"),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_local_only() {
        let local = vec![LocalEntry { relative_path: "a.txt".into(), size: 5 }];
        let result = compare_entries(&local, &[]);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, DiffStatus::LocalOnly);
        assert_eq!(result[0].local_size, Some(5));
        assert_eq!(result[0].remote_size, None);
    }

    #[test]
    fn detects_remote_only() {
        let remote = vec![RemoteEntry { relative_path: "a.txt".into(), size: 5 }];
        let result = compare_entries(&[], &remote);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, DiffStatus::RemoteOnly);
        assert_eq!(result[0].remote_size, Some(5));
    }

    #[test]
    fn detects_match_and_size_mismatch() {
        let local = vec![
            LocalEntry { relative_path: "match.txt".into(), size: 10 },
            LocalEntry { relative_path: "diff.txt".into(), size: 10 },
        ];
        let remote = vec![
            RemoteEntry { relative_path: "match.txt".into(), size: 10 },
            RemoteEntry { relative_path: "diff.txt".into(), size: 20 },
        ];

        let result = compare_entries(&local, &remote);

        let matched = result.iter().find(|e| e.relative_path == "match.txt").unwrap();
        assert_eq!(matched.status, DiffStatus::Match);

        let mismatched = result.iter().find(|e| e.relative_path == "diff.txt").unwrap();
        assert_eq!(mismatched.status, DiffStatus::SizeMismatch);
    }
}
