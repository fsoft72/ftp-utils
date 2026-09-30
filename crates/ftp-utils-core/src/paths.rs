//! Validation of relative paths read from untrusted input (CSV reports),
//! so they can never escape the local or remote root they are joined to.

use std::path::{Component, Path};

/// Checks that `relative_path` is a non-empty, relative path made only of
/// normal components: no absolute paths, no `..`, no drive prefixes.
/// Both `/` and `\` are treated as separators, so a path that is only
/// dangerous on another platform is rejected too.
pub fn validate_relative_path(relative_path: &str) -> Result<(), String> {
    if relative_path.is_empty() {
        return Err("empty path".to_string());
    }
    if relative_path.contains('\0') {
        return Err(format!("path '{relative_path}' contains a NUL byte"));
    }
    if relative_path.starts_with('/') || relative_path.starts_with('\\') {
        return Err(format!("path '{relative_path}' is absolute"));
    }
    if relative_path.split(['/', '\\']).any(|part| part == "..") {
        return Err(format!("path '{relative_path}' contains '..'"));
    }
    let only_normal =
        Path::new(relative_path).components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if !only_normal {
        return Err(format!("path '{relative_path}' is not a plain relative path"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_relative_paths() {
        assert!(validate_relative_path("a.txt").is_ok());
        assert!(validate_relative_path("sub/dir/b.txt").is_ok());
        assert!(validate_relative_path("..hidden/file").is_ok());
    }

    #[test]
    fn rejects_empty_path() {
        assert!(validate_relative_path("").is_err());
    }

    #[test]
    fn rejects_absolute_paths() {
        assert!(validate_relative_path("/etc/passwd").is_err());
        assert!(validate_relative_path("\\windows\\x").is_err());
    }

    #[test]
    fn rejects_parent_traversal() {
        assert!(validate_relative_path("../x").is_err());
        assert!(validate_relative_path("a/../../x").is_err());
        assert!(validate_relative_path("a\\..\\x").is_err());
    }

    #[test]
    fn rejects_nul_byte() {
        assert!(validate_relative_path("a\0b").is_err());
    }
}
