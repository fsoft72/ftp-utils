//! A configurable in-memory `FtpConnection` for tests, shared by this
//! crate's unit tests and (through the `test-utils` feature) the tools'.

use std::collections::HashMap;

use crate::remote::{FtpConnection, FtpConnectionError, RawRemoteEntry};

/// In-memory fake server state plus a record of every mutating call.
#[derive(Default)]
pub struct MockFtpConnection {
    /// Directory listings by full remote path.
    pub listings: HashMap<String, Vec<RawRemoteEntry>>,
    /// File contents by full remote path, served by `retr_*`.
    pub files: HashMap<String, Vec<u8>>,
    /// Served by `retr_*` for paths missing from `files`.
    pub default_content: Option<Vec<u8>>,
    /// Returned by `try_hash` (the server-side hash), if any.
    pub hash: Option<String>,
    /// When true, listing a path missing from `listings` is an error
    /// instead of an empty directory.
    pub strict_listings: bool,
    /// When true, every `list_dir` fails.
    pub fail_listings: bool,
    /// Every path passed to `list_dir`, in order.
    pub list_calls: Vec<String>,
    /// `(path, data)` for every upload.
    pub stored: Vec<(String, Vec<u8>)>,
    /// Every deleted path.
    pub deleted: Vec<String>,
    /// Every created directory (also added to `listings`).
    pub created_dirs: Vec<String>,
}

impl MockFtpConnection {
    /// A mock serving `listings`, failing on any path not listed.
    pub fn strict(listings: HashMap<String, Vec<RawRemoteEntry>>) -> Self {
        Self { listings, strict_listings: true, ..Self::default() }
    }
}

impl FtpConnection for MockFtpConnection {
    fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
        self.list_calls.push(path.to_string());

        if self.fail_listings {
            return Err(FtpConnectionError(format!("listing failed for {path}")));
        }

        match self.listings.get(path) {
            Some(entries) => Ok(entries.clone()),
            None if self.strict_listings => Err(FtpConnectionError(format!("no listing for {path}"))),
            None => Ok(Vec::new()),
        }
    }

    fn try_hash(&mut self, _path: &str) -> Option<String> {
        self.hash.clone()
    }

    fn retr_to_buffer(&mut self, path: &str) -> Result<Vec<u8>, FtpConnectionError> {
        self.files
            .get(path)
            .or(self.default_content.as_ref())
            .cloned()
            .ok_or_else(|| FtpConnectionError(format!("no such remote file: {path}")))
    }

    fn store_from_buffer(&mut self, path: &str, data: &[u8]) -> Result<(), FtpConnectionError> {
        self.stored.push((path.to_string(), data.to_vec()));
        Ok(())
    }

    fn delete(&mut self, path: &str) -> Result<(), FtpConnectionError> {
        self.deleted.push(path.to_string());
        Ok(())
    }

    fn create_dir(&mut self, path: &str) -> Result<(), FtpConnectionError> {
        self.created_dirs.push(path.to_string());

        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        let parent = if parent.is_empty() { "/" } else { parent };

        self.listings
            .entry(parent.to_string())
            .or_default()
            .push(RawRemoteEntry { name: name.to_string(), is_dir: true, size: 0 });
        self.listings.entry(path.to_string()).or_default();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_mock_lists_unknown_paths_as_empty_and_strict_one_fails() {
        assert!(MockFtpConnection::default().list_dir("/x").unwrap().is_empty());
        assert!(MockFtpConnection::strict(HashMap::new()).list_dir("/x").is_err());
    }

    #[test]
    fn create_dir_registers_the_directory_in_its_parent_listing() {
        let mut mock = MockFtpConnection::default();

        mock.create_dir("/a/b").unwrap();

        assert!(mock.list_dir("/a").unwrap().iter().any(|e| e.is_dir && e.name == "b"));
        assert_eq!(mock.created_dirs, vec!["/a/b".to_string()]);
    }

    #[test]
    fn retrieval_prefers_files_then_default_content() {
        let mut mock = MockFtpConnection::default();
        mock.files.insert("/f".to_string(), b"file".to_vec());
        assert!(mock.retr_to_buffer("/other").is_err());

        mock.default_content = Some(b"default".to_vec());

        assert_eq!(mock.retr_to_buffer("/f").unwrap(), b"file");
        assert_eq!(mock.retr_to_buffer("/other").unwrap(), b"default");
    }
}
