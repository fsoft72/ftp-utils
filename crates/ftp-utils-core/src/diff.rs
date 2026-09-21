//! Diff types produced when comparing a local directory tree against a
//! remote FTP/FTPS directory tree. Comparison logic will be implemented
//! according to the implementation plan derived from the design spec.

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

#[cfg(test)]
mod tests {
    use super::*;

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
