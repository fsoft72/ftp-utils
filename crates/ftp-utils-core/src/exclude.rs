//! Glob-based exclude pattern matching for local and remote relative paths.

/// Returns true if `relative_path` matches any of `patterns` (glob syntax).
pub fn is_excluded(relative_path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        glob::Pattern::new(pattern)
            .map(|compiled| compiled.matches(relative_path))
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_simple_glob() {
        let patterns = vec!["*.tmp".to_string()];
        assert!(is_excluded("file.tmp", &patterns));
        assert!(!is_excluded("file.txt", &patterns));
    }

    #[test]
    fn matches_nested_path_pattern() {
        let patterns = vec![".git/*".to_string()];
        assert!(is_excluded(".git/config", &patterns));
        assert!(!is_excluded("src/.git/config", &patterns));
    }

    #[test]
    fn no_patterns_excludes_nothing() {
        assert!(!is_excluded("anything.txt", &[]));
    }
}
