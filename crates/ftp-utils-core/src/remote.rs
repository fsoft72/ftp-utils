//! Remote directory walking over an abstract `FtpConnection`, so the
//! comparison logic never depends directly on a concrete FTP library.

use std::collections::HashSet;
use std::io::{Read, Write};

use crate::exclude::ExcludeSet;

/// One entry as reported by a single remote directory listing (before
/// recursion resolves it into a full relative path).
#[derive(Debug, Clone)]
pub struct RawRemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// A file found while recursively walking the remote directory tree.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteEntry {
    pub relative_path: String,
    pub size: u64,
}

/// Error from any FTP operation, wrapping the underlying client's error text.
#[derive(Debug)]
pub struct FtpConnectionError(pub String);

impl std::fmt::Display for FtpConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for FtpConnectionError {}

/// Abstraction over an FTP/FTPS connection, so directory walking and hash
/// fallback logic can be unit-tested without a real network connection.
pub trait FtpConnection {
    /// Lists the direct children of `path` (files and directories).
    fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError>;
    /// Attempts to get a server-computed hash for `path` without
    /// downloading it. Returns `None` if the server doesn't support it.
    fn try_hash(&mut self, path: &str) -> Option<String>;
    /// Downloads the full contents of `path` into memory.
    fn retr_to_buffer(&mut self, path: &str) -> Result<Vec<u8>, FtpConnectionError>;
    /// Downloads `path` into `out`, returning the number of bytes written.
    /// The default goes through `retr_to_buffer`; real clients should
    /// override it to stream, so large files never sit fully in memory.
    fn retr_to_writer(&mut self, path: &str, out: &mut dyn Write) -> Result<u64, FtpConnectionError> {
        let data = self.retr_to_buffer(path)?;
        out.write_all(&data).map_err(|e| FtpConnectionError(format!("write error: {e}")))?;
        Ok(data.len() as u64)
    }
    /// Uploads `data` to `path`, overwriting any existing remote file.
    fn store_from_buffer(&mut self, path: &str, data: &[u8]) -> Result<(), FtpConnectionError>;
    /// Uploads everything `input` yields to `path`, overwriting any
    /// existing remote file. The default reads it all into memory and
    /// calls `store_from_buffer`; real clients should override it to
    /// stream.
    fn store_from_reader(&mut self, path: &str, input: &mut dyn Read) -> Result<(), FtpConnectionError> {
        let mut data = Vec::new();
        input.read_to_end(&mut data).map_err(|e| FtpConnectionError(format!("read error: {e}")))?;
        self.store_from_buffer(path, &data)
    }
    /// Deletes the remote file at `path`.
    fn delete(&mut self, path: &str) -> Result<(), FtpConnectionError>;
    /// Creates a single directory at `path`. Its parent must already exist.
    fn create_dir(&mut self, path: &str) -> Result<(), FtpConnectionError>;

    /// Ensures every directory component of `path`'s parent exists,
    /// creating any that are missing. `path` is the full remote path to a
    /// file; only its parent directories are created. Built generically
    /// on `list_dir` and `create_dir`, so any implementer gets it for
    /// free without needing its own recursive-mkdir logic.
    ///
    /// Directories recorded in `known_dirs` are assumed to exist and cost
    /// no round trip; every directory this call confirms or creates is
    /// added to it. Share one set across many calls (e.g. a bulk upload)
    /// so each directory is listed at most once.
    fn ensure_remote_dir_cached(
        &mut self,
        path: &str,
        known_dirs: &mut HashSet<String>,
    ) -> Result<(), FtpConnectionError> {
        let parent = match path.rfind('/') {
            Some(idx) if idx > 0 => &path[..idx],
            _ => return Ok(()),
        };

        let mut current = String::new();
        for component in parent.split('/').filter(|c| !c.is_empty()) {
            let listing_parent = if current.is_empty() { "/".to_string() } else { current.clone() };
            current.push('/');
            current.push_str(component);

            if known_dirs.contains(&current) {
                continue;
            }

            let existing = self.list_dir(&listing_parent)?;
            if !existing.iter().any(|e| e.is_dir && e.name == component) {
                self.create_dir(&current)?;
            }
            known_dirs.insert(current.clone());
        }

        Ok(())
    }

    /// Like `ensure_remote_dir_cached`, with no memory between calls.
    fn ensure_remote_dir(&mut self, path: &str) -> Result<(), FtpConnectionError> {
        self.ensure_remote_dir_cached(path, &mut HashSet::new())
    }
}

/// Joins a remote directory and a relative path with exactly one `/`
/// between them: `("/", "a")` and `("/x/", "a")` give `/a` and `/x/a`
/// (a plain `format!("{root}/{rel}")` would give `//a` and `/x//a`). An
/// empty `root` yields `relative` unchanged.
pub fn join_remote(root: &str, relative: &str) -> String {
    if root.is_empty() {
        return relative.to_string();
    }
    format!("{}/{}", root.trim_end_matches('/'), relative.trim_start_matches('/'))
}

/// Recursively walks `root` on the remote server, returning one entry per
/// file, skipping any file whose relative path matches one of `excludes`.
/// Calls `progress` (if given) with a human-readable message for every
/// included file, for `--verbose` output.
pub fn walk_remote<C: FtpConnection>(
    conn: &mut C,
    root: &str,
    excludes: &ExcludeSet,
    mut progress: Option<&mut (dyn FnMut(&str) + '_)>,
) -> Result<Vec<RemoteEntry>, FtpConnectionError> {
    let mut entries = Vec::new();
    let mut dirs_to_visit: Vec<String> = vec![String::new()]; // "" means root itself

    while let Some(relative_dir) = dirs_to_visit.pop() {
        let full_path = if relative_dir.is_empty() {
            root.to_string()
        } else {
            join_remote(root, &relative_dir)
        };

        for item in conn.list_dir(&full_path)? {
            let relative_path = if relative_dir.is_empty() {
                item.name.clone()
            } else {
                format!("{relative_dir}/{}", item.name)
            };

            if item.is_dir {
                if !excludes.excludes_dir(&relative_path) {
                    dirs_to_visit.push(relative_path);
                }
                continue;
            }

            if excludes.is_excluded(&relative_path) {
                continue;
            }

            if let Some(cb) = progress.as_deref_mut() {
                cb(&format!("remote: {relative_path}"));
            }

            entries.push(RemoteEntry { relative_path, size: item.size });
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MockFtpConnection {
        listings: HashMap<String, Vec<RawRemoteEntry>>,
        created_dirs: Vec<String>,
    }

    impl FtpConnection for MockFtpConnection {
        fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
            self.listings
                .get(path)
                .cloned()
                .ok_or_else(|| FtpConnectionError(format!("no listing for {path}")))
        }

        fn try_hash(&mut self, _path: &str) -> Option<String> {
            None
        }

        fn retr_to_buffer(&mut self, _path: &str) -> Result<Vec<u8>, FtpConnectionError> {
            Ok(Vec::new())
        }

        fn store_from_buffer(&mut self, _path: &str, _data: &[u8]) -> Result<(), FtpConnectionError> {
            Ok(())
        }

        fn delete(&mut self, _path: &str) -> Result<(), FtpConnectionError> {
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

    #[test]
    fn join_remote_uses_exactly_one_separator() {
        assert_eq!(join_remote("/remote", "a.txt"), "/remote/a.txt");
        assert_eq!(join_remote("/remote/", "a.txt"), "/remote/a.txt");
        assert_eq!(join_remote("/", "a.txt"), "/a.txt");
        assert_eq!(join_remote("/remote", "sub/a.txt"), "/remote/sub/a.txt");
        assert_eq!(join_remote("", "a.txt"), "a.txt");
    }

    #[test]
    fn walks_nested_directories_under_root_with_trailing_slash() {
        let mut listings = HashMap::new();
        listings.insert("/".to_string(), vec![RawRemoteEntry { name: "sub".into(), is_dir: true, size: 0 }]);
        listings.insert("/sub".to_string(), vec![RawRemoteEntry { name: "b.txt".into(), is_dir: false, size: 1 }]);
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };

        let entries = walk_remote(&mut conn, "/", &ExcludeSet::default(), None).unwrap();

        assert_eq!(entries, vec![RemoteEntry { relative_path: "sub/b.txt".to_string(), size: 1 }]);
    }

    #[test]
    fn walks_nested_directories() {
        let mut listings = HashMap::new();
        listings.insert(
            "/remote".to_string(),
            vec![
                RawRemoteEntry { name: "a.txt".into(), is_dir: false, size: 10 },
                RawRemoteEntry { name: "sub".into(), is_dir: true, size: 0 },
            ],
        );
        listings.insert(
            "/remote/sub".to_string(),
            vec![RawRemoteEntry { name: "b.txt".into(), is_dir: false, size: 20 }],
        );
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };

        let mut entries = walk_remote(&mut conn, "/remote", &ExcludeSet::default(), None).unwrap();
        entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

        assert_eq!(
            entries,
            vec![
                RemoteEntry { relative_path: "a.txt".to_string(), size: 10 },
                RemoteEntry { relative_path: "sub/b.txt".to_string(), size: 20 },
            ]
        );
    }

    #[test]
    fn does_not_list_excluded_directories() {
        let mut listings = HashMap::new();
        listings.insert(
            "/remote".to_string(),
            vec![
                RawRemoteEntry { name: ".git".into(), is_dir: true, size: 0 },
                RawRemoteEntry { name: "keep.txt".into(), is_dir: false, size: 1 },
            ],
        );
        // No "/remote/.git" listing: MockFtpConnection errors if it is listed.
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };
        let excludes = ExcludeSet::new(&[".git/*".to_string()]).unwrap();

        let entries = walk_remote(&mut conn, "/remote", &excludes, None).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].relative_path, "keep.txt");
    }

    #[test]
    fn applies_exclude_patterns() {
        let mut listings = HashMap::new();
        listings.insert(
            "/remote".to_string(),
            vec![
                RawRemoteEntry { name: "keep.txt".into(), is_dir: false, size: 1 },
                RawRemoteEntry { name: "skip.tmp".into(), is_dir: false, size: 1 },
            ],
        );
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };

        let entries = walk_remote(&mut conn, "/remote", &ExcludeSet::new(&["*.tmp".to_string()]).unwrap(), None).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].relative_path, "keep.txt");
    }

    #[test]
    fn reports_progress_for_included_files_only() {
        let mut listings = HashMap::new();
        listings.insert(
            "/remote".to_string(),
            vec![
                RawRemoteEntry { name: "keep.txt".into(), is_dir: false, size: 1 },
                RawRemoteEntry { name: "skip.tmp".into(), is_dir: false, size: 1 },
            ],
        );
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };

        let mut messages = Vec::new();
        let mut progress = |msg: &str| messages.push(msg.to_string());

        walk_remote(&mut conn, "/remote", &ExcludeSet::new(&["*.tmp".to_string()]).unwrap(), Some(&mut progress)).unwrap();

        assert_eq!(messages, vec!["remote: keep.txt".to_string()]);
    }

    #[test]
    fn ensure_remote_dir_creates_missing_components() {
        let mut listings = HashMap::new();
        listings.insert("/".to_string(), vec![]);
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };

        conn.ensure_remote_dir("/a/b/file.txt").unwrap();

        assert_eq!(conn.created_dirs, vec!["/a".to_string(), "/a/b".to_string()]);
    }

    #[test]
    fn ensure_remote_dir_skips_components_that_already_exist() {
        let mut listings = HashMap::new();
        listings.insert("/".to_string(), vec![RawRemoteEntry { name: "a".into(), is_dir: true, size: 0 }]);
        listings.insert("/a".to_string(), vec![]);
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };

        conn.ensure_remote_dir("/a/b/file.txt").unwrap();

        assert_eq!(conn.created_dirs, vec!["/a/b".to_string()]);
    }

    #[test]
    fn ensure_remote_dir_cached_skips_directories_already_known() {
        let mut listings = HashMap::new();
        listings.insert("/".to_string(), vec![]);
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };
        let mut known = HashSet::new();

        conn.ensure_remote_dir_cached("/a/b/one.txt", &mut known).unwrap();
        conn.ensure_remote_dir_cached("/a/b/two.txt", &mut known).unwrap();
        conn.ensure_remote_dir_cached("/a/c/three.txt", &mut known).unwrap();

        assert_eq!(conn.created_dirs, vec!["/a".to_string(), "/a/b".to_string(), "/a/c".to_string()]);
        assert!(known.contains("/a") && known.contains("/a/b") && known.contains("/a/c"));
    }

    #[test]
    fn ensure_remote_dir_cached_does_not_relist_known_directories() {
        // "/a" has no listing entry: listing it would fail, so this only
        // passes if the cache avoids the round trip.
        let mut listings = HashMap::new();
        listings.insert("/a".to_string(), vec![]);
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };
        let mut known: HashSet<String> = ["/a".to_string()].into();

        conn.ensure_remote_dir_cached("/a/b/file.txt", &mut known).unwrap();

        assert_eq!(conn.created_dirs, vec!["/a/b".to_string()]);
    }

    #[test]
    fn ensure_remote_dir_is_a_noop_for_root_level_files() {
        let mut listings = HashMap::new();
        listings.insert("/".to_string(), vec![]);
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };

        conn.ensure_remote_dir("/file.txt").unwrap();

        assert!(conn.created_dirs.is_empty());
    }
}
