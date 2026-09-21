//! Recursive local directory walking, producing relative-path/size entries.

use std::path::Path;

use crate::exclude::is_excluded;

/// A file found while walking the local directory tree.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalEntry {
    pub relative_path: String,
    pub size: u64,
}

/// Recursively walks `root`, returning one entry per file (not directory),
/// skipping any file whose path (relative to `root`, using `/` separators)
/// matches one of `excludes`.
pub fn walk_local_dir(root: &Path, excludes: &[String]) -> std::io::Result<Vec<LocalEntry>> {
    let mut entries = Vec::new();

    for result in walkdir::WalkDir::new(root) {
        let dir_entry = result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        if !dir_entry.file_type().is_file() {
            continue;
        }

        let relative = dir_entry
            .path()
            .strip_prefix(root)
            .expect("walkdir entries are always under root")
            .to_string_lossy()
            .replace('\\', "/");

        if is_excluded(&relative, excludes) {
            continue;
        }

        let size = dir_entry
            .metadata()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?
            .len();

        entries.push(LocalEntry { relative_path: relative, size });
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/b.txt"), b"world!").unwrap();

        let mut entries = walk_local_dir(dir.path(), &[]).unwrap();
        entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

        assert_eq!(
            entries,
            vec![
                LocalEntry { relative_path: "a.txt".to_string(), size: 5 },
                LocalEntry { relative_path: "sub/b.txt".to_string(), size: 6 },
            ]
        );
    }

    #[test]
    fn skips_excluded_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), b"x").unwrap();
        fs::write(dir.path().join("skip.tmp"), b"y").unwrap();

        let entries = walk_local_dir(dir.path(), &["*.tmp".to_string()]).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].relative_path, "keep.txt");
    }
}
