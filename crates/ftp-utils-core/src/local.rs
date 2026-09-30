//! Recursive local directory walking, producing relative-path/size entries.

use std::path::Path;

use crate::exclude::ExcludeSet;

/// A file found while walking the local directory tree.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalEntry {
    pub relative_path: String,
    pub size: u64,
}

/// Recursively walks `root`, returning one entry per file (not directory),
/// skipping any file whose path (relative to `root`, using `/` separators)
/// matches one of `excludes`. Calls `progress` (if given) with a
/// human-readable message for every included file, for `--verbose` output.
pub fn walk_local_dir(
    root: &Path,
    excludes: &ExcludeSet,
    mut progress: Option<&mut (dyn FnMut(&str) + '_)>,
) -> std::io::Result<Vec<LocalEntry>> {
    let mut entries = Vec::new();

    // Skip whole excluded directories instead of walking into them.
    let walker = walkdir::WalkDir::new(root).into_iter().filter_entry(|entry| {
        if !entry.file_type().is_dir() || entry.depth() == 0 {
            return true;
        }
        let relative = entry.path().strip_prefix(root).map(|p| p.to_string_lossy().replace('\\', "/"));
        !relative.is_ok_and(|dir| excludes.excludes_dir(&dir))
    });

    for result in walker {
        let dir_entry = result.map_err(std::io::Error::other)?;
        if !dir_entry.file_type().is_file() {
            continue;
        }

        let relative = dir_entry
            .path()
            .strip_prefix(root)
            .expect("walkdir entries are always under root")
            .to_string_lossy()
            .replace('\\', "/");

        if excludes.is_excluded(&relative) {
            continue;
        }

        let size = dir_entry
            .metadata()
            .map_err(|e| std::io::Error::other(format!("{relative}: {e}")))?
            .len();

        if let Some(cb) = progress.as_deref_mut() {
            cb(&format!("local: {relative}"));
        }

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

        let mut entries = walk_local_dir(dir.path(), &ExcludeSet::default(), None).unwrap();
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

        let entries = walk_local_dir(dir.path(), &ExcludeSet::new(&["*.tmp".to_string()]).unwrap(), None).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].relative_path, "keep.txt");
    }

    #[test]
    fn skips_files_under_excluded_directories() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        fs::write(dir.path().join(".git/objects/blob"), b"x").unwrap();
        fs::write(dir.path().join("keep.txt"), b"y").unwrap();
        let excludes = ExcludeSet::new(&[".git/*".to_string()]).unwrap();

        let entries = walk_local_dir(dir.path(), &excludes, None).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].relative_path, "keep.txt");
    }

    #[test]
    fn excluded_directory_is_not_even_opened() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("secret");
        fs::create_dir(&secret).unwrap();
        fs::write(secret.join("f"), b"x").unwrap();
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
        let excludes = ExcludeSet::new(&["secret/*".to_string()]).unwrap();

        let result = walk_local_dir(dir.path(), &excludes, None);

        fs::set_permissions(&secret, fs::Permissions::from_mode(0o755)).unwrap();
        // Would fail with "permission denied" if the walker tried to read it
        // (unless running as root, where permissions don't apply).
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn reports_progress_for_included_files_only() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), b"x").unwrap();
        fs::write(dir.path().join("skip.tmp"), b"y").unwrap();

        let mut messages = Vec::new();
        let mut progress = |msg: &str| messages.push(msg.to_string());

        walk_local_dir(dir.path(), &ExcludeSet::new(&["*.tmp".to_string()]).unwrap(), Some(&mut progress)).unwrap();

        assert_eq!(messages, vec!["local: keep.txt".to_string()]);
    }
}
