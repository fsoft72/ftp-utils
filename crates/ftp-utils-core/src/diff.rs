//! Diff types produced when comparing a local directory tree against a
//! remote FTP/FTPS directory tree. `compare::compare_entries` produces
//! them and `hash::apply_hash_comparison` refines them with hashes.

/// Outcome of comparing one relative path present locally and/or remotely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffStatus {
    /// File exists locally but not on the remote server.
    LocalOnly,
    /// File exists on the remote server but not locally.
    RemoteOnly,
    /// File exists on both sides but sizes differ.
    SizeMismatch,
    /// Sizes match but content hashes differ (only when hashing is enabled).
    HashMismatch,
    /// File matches on both sides.
    Match,
    /// Scanned by `--build`, not compared against the other side (that
    /// side wasn't scanned at all).
    Scan,
}

impl DiffStatus {
    /// Every status, in declaration order.
    pub const ALL: [DiffStatus; 6] = [
        DiffStatus::LocalOnly,
        DiffStatus::RemoteOnly,
        DiffStatus::SizeMismatch,
        DiffStatus::HashMismatch,
        DiffStatus::Match,
        DiffStatus::Scan,
    ];

    /// The name written to the CSV `status` column and accepted by
    /// `FromStr`. Part of the CSV report format: don't rename casually.
    pub fn as_str(self) -> &'static str {
        match self {
            DiffStatus::LocalOnly => "LocalOnly",
            DiffStatus::RemoteOnly => "RemoteOnly",
            DiffStatus::SizeMismatch => "SizeMismatch",
            DiffStatus::HashMismatch => "HashMismatch",
            DiffStatus::Match => "Match",
            DiffStatus::Scan => "Scan",
        }
    }
}

impl std::fmt::Display for DiffStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for DiffStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        DiffStatus::ALL
            .into_iter()
            .find(|status| status.as_str() == value)
            .ok_or_else(|| format!("unknown status '{value}'"))
    }
}

/// A single comparison result for one relative path.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
    pub relative_path: String,
    pub status: DiffStatus,
    pub local_size: Option<u64>,
    pub remote_size: Option<u64>,
    pub local_md5: Option<String>,
    pub remote_md5: Option<String>,
}

impl DiffEntry {
    /// Creates an entry with the given sizes and no hashes yet (those are
    /// filled in later by `hash::apply_hash_comparison` or `--build`).
    pub fn new(
        relative_path: impl Into<String>,
        status: DiffStatus,
        local_size: Option<u64>,
        remote_size: Option<u64>,
    ) -> Self {
        Self { relative_path: relative_path.into(), status, local_size, remote_size, local_md5: None, remote_md5: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_names_round_trip_and_match_debug_output() {
        for status in DiffStatus::ALL {
            assert_eq!(status.to_string().parse::<DiffStatus>(), Ok(status));
            assert_eq!(status.to_string(), format!("{status:?}"));
        }
    }

    #[test]
    fn unknown_status_is_an_error() {
        assert!("Bogus".parse::<DiffStatus>().is_err());
        assert!("localonly".parse::<DiffStatus>().is_err());
    }

    #[test]
    fn new_leaves_hashes_unset() {
        let entry = DiffEntry::new("a.txt", DiffStatus::SizeMismatch, Some(1), Some(2));

        assert_eq!(entry.relative_path, "a.txt");
        assert_eq!(entry.status, DiffStatus::SizeMismatch);
        assert_eq!((entry.local_size, entry.remote_size), (Some(1), Some(2)));
        assert_eq!((entry.local_md5, entry.remote_md5), (None, None));
    }

    #[test]
    fn diff_entries_with_same_fields_are_equal() {
        let a = DiffEntry {
            relative_path: "a.txt".to_string(),
            status: DiffStatus::Match,
            local_size: Some(10),
            remote_size: Some(10),
            local_md5: None,
            remote_md5: None,
        };
        let b = a.clone();
        assert_eq!(a, b);
    }
}
