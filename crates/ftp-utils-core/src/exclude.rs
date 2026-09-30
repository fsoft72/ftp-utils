//! Glob-based exclude pattern matching for local and remote relative paths.

use glob::Pattern;

/// A set of exclude patterns, compiled once and reused for every path.
#[derive(Debug, Clone, Default)]
pub struct ExcludeSet {
    patterns: Vec<Pattern>,
    /// For every pattern ending in `/*` or `/**`, the pattern without that
    /// suffix. A directory matching one of these has all of its contents
    /// excluded, so walkers can skip descending into it.
    dir_patterns: Vec<Pattern>,
}

/// An exclude pattern that is not valid glob syntax.
#[derive(Debug)]
pub struct ExcludeError(pub String);

impl std::fmt::Display for ExcludeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ExcludeError {}

impl ExcludeSet {
    /// Compiles `patterns` (glob syntax). Fails on the first invalid
    /// pattern, so a typo is reported instead of silently excluding
    /// nothing.
    pub fn new(patterns: &[String]) -> Result<Self, ExcludeError> {
        let mut set = ExcludeSet::default();
        for pattern in patterns {
            let compiled = Pattern::new(pattern)
                .map_err(|e| ExcludeError(format!("invalid exclude pattern '{pattern}': {e}")))?;
            set.patterns.push(compiled);

            let prefix = pattern.strip_suffix("/**").or_else(|| pattern.strip_suffix("/*"));
            if let Some(prefix) = prefix.filter(|p| !p.is_empty()) {
                let compiled_prefix = Pattern::new(prefix)
                    .map_err(|e| ExcludeError(format!("invalid exclude pattern '{pattern}': {e}")))?;
                set.dir_patterns.push(compiled_prefix);
            }
        }
        Ok(set)
    }

    /// Returns true if `relative_path` matches any pattern.
    pub fn is_excluded(&self, relative_path: &str) -> bool {
        self.patterns.iter().any(|pattern| pattern.matches(relative_path))
    }

    /// Returns true if everything under the directory `relative_dir` is
    /// excluded by a `<dir>/*` or `<dir>/**` pattern, so it need not be
    /// visited at all. Conservative: `false` never hides a file that
    /// `is_excluded` would keep.
    pub fn excludes_dir(&self, relative_dir: &str) -> bool {
        self.dir_patterns.iter().any(|pattern| pattern.matches(relative_dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(patterns: &[&str]) -> ExcludeSet {
        ExcludeSet::new(&patterns.iter().map(|p| p.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn matches_simple_glob() {
        let excludes = set(&["*.tmp"]);
        assert!(excludes.is_excluded("file.tmp"));
        assert!(!excludes.is_excluded("file.txt"));
    }

    #[test]
    fn matches_nested_path_pattern() {
        let excludes = set(&[".git/*"]);
        assert!(excludes.is_excluded(".git/config"));
        assert!(!excludes.is_excluded("src/.git/config"));
    }

    #[test]
    fn no_patterns_excludes_nothing() {
        assert!(!ExcludeSet::default().is_excluded("anything.txt"));
        assert!(!ExcludeSet::new(&[]).unwrap().is_excluded("anything.txt"));
    }

    #[test]
    fn invalid_pattern_is_an_error_naming_the_pattern() {
        let err = ExcludeSet::new(&["ok/*".to_string(), "[unclosed".to_string()]).unwrap_err();

        assert!(err.to_string().contains("[unclosed"), "{err}");
    }

    #[test]
    fn dir_wildcard_patterns_exclude_the_whole_directory() {
        let excludes = set(&[".git/*", "build/**", "*/cache/*"]);

        assert!(excludes.excludes_dir(".git"));
        assert!(excludes.excludes_dir("build"));
        assert!(excludes.excludes_dir("app/cache"));
        assert!(!excludes.excludes_dir("src"));
        assert!(!excludes.excludes_dir("src/.git"));
    }

    #[test]
    fn file_only_patterns_never_exclude_a_directory() {
        let excludes = set(&["*.tmp", ".git/config", ".git/a*"]);

        assert!(!excludes.excludes_dir(".git"));
        assert!(!excludes.excludes_dir("x.tmp"));
    }
}
