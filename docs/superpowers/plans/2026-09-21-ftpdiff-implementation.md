# ftpdiff Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `ftpdiff`, a CLI tool that recursively compares a local directory against a remote FTP/FTPS directory tree by file size (and optionally MD5 hash), reporting differences as colored text and an optional CSV file.

**Architecture:** `crates/ftp-utils-core` is a network-abstraction-free library: all FTP access goes through a small `FtpConnection` trait, so directory-walking, exclude-matching, diffing, and hash-fallback logic are unit-testable with an in-memory mock, with zero real network calls in tests. `tools/ftpdiff` is a thin binary crate: CLI parsing (clap), JSON config loading/merging, and output formatting, wired to a single real `FtpConnection` implementation (`SuppaFtpConnection`, backed by the `suppaftp` crate) in `main.rs`.

**Tech Stack:** Rust 2021, `suppaftp` 10 (FTP/FTPS client), `clap` 4 (derive, CLI), `walkdir` 2 (local directory walking), `glob` 0.3 (exclude patterns), `md5` 0.7 (hashing), `csv` 1 (CSV output), `colored` 2 (terminal colors), `serde`/`serde_json` (JSON config), `tempfile` (test fixtures, dev-only).

**Spec:** `docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md`

## Global Constraints

- Config precedence: CLI args > JSON config file > built-in defaults (spec "Configuration precedence").
- Password is read **only** from the `FTPDIFF_PASSWORD` environment variable; never accepted via CLI flag or JSON field.
- Boolean flags (`--ftps`, `--hash`) merge as OR between CLI and JSON: `effective = cli_flag || json_value` (a plain clap bool flag cannot represent "explicitly false", so CLI can enable but not disable a JSON-set `true`). This is a plan-level decision made explicit here since the spec doesn't specify flag-merge semantics.
- `--exclude` patterns merge as a union of CLI-provided and JSON-provided patterns (both apply), since exclusions are naturally additive.
- Default FTP port is 21 for both plain FTP and explicit FTPS (the design uses explicit AUTH TLS upgrade on the standard control port, not implicit FTPS on port 990).
- Exit codes: `0` = no differences, `1` = differences found, `2` = runtime error.
- **Known v1 limitation:** the `suppaftp` crate (v10, verified via its current docs) does not expose a public API for sending non-standard remote-hash commands (XMD5/MD5/HASH). `SuppaFtpConnection::try_hash` therefore always returns `None`, so when `--hash` is enabled the tool always falls back to downloading the remote file and computing MD5 locally. This still satisfies the spec's documented fallback behavior; it only forgoes the "skip download when server supports native hashing" optimization. Note this in `CHANGES.md` when Task 7 is committed.
- Files end with an empty line; comments/code in English only (per user's global instructions).

---

## File Structure

```
crates/ftp-utils-core/
├── Cargo.toml                  # add suppaftp, glob, walkdir, md5; dev-dep tempfile
└── src/
    ├── lib.rs                  # module wiring + CompareOptions + compare() + CompareError
    ├── diff.rs                 # DiffStatus, DiffEntry (extends existing stub)
    ├── exclude.rs               # is_excluded()
    ├── local.rs                 # LocalEntry, walk_local_dir()
    ├── remote.rs                 # RawRemoteEntry, RemoteEntry, FtpConnection trait, walk_remote()
    ├── compare.rs                # compare_entries()
    ├── hash.rs                    # apply_hash_comparison()
    └── ftp_client.rs               # SuppaFtpConnection (real suppaftp-backed FtpConnection)

tools/ftpdiff/
├── Cargo.toml                  # add clap, serde, serde_json, csv, colored; dev-dep tempfile
└── src/
    ├── main.rs                  # wiring: parse -> config -> connect -> compare -> output -> exit code
    ├── cli.rs                    # Cli (clap derive struct)
    ├── config.rs                  # JsonConfig, EffectiveConfig, load_json_config(), merge(), read_password()
    ├── output.rs                   # format_entry(), Summary, summarize(), format_summary()
    └── csv_report.rs                # write_csv()
```

---

### Task 1: Diff types

**Files:**
- Modify: `crates/ftp-utils-core/src/diff.rs` (already contains `DiffStatus` and `DiffEntry` from the initial scaffold commit; add `PartialEq` to `DiffEntry` so tests can assert equality)
- Test: same file, `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `pub enum DiffStatus { LocalOnly, RemoteOnly, SizeMismatch, HashMismatch, Match }` (already `Debug, Clone, Copy, PartialEq, Eq`), `pub struct DiffEntry { pub relative_path: String, pub status: DiffStatus, pub local_size: Option<u64>, pub remote_size: Option<u64>, pub local_md5: Option<String>, pub remote_md5: Option<String> }`.

- [ ] **Step 1: Write the failing test**

Add to `crates/ftp-utils-core/src/diff.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ftp-utils-core diff_entries_with_same_fields_are_equal`
Expected: FAIL with a compile error, `DiffEntry` doesn't implement `PartialEq`.

- [ ] **Step 3: Add PartialEq derive**

Change the `DiffEntry` derive line in `crates/ftp-utils-core/src/diff.rs` from:

```rust
#[derive(Debug, Clone)]
pub struct DiffEntry {
```

to:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ftp-utils-core diff_entries_with_same_fields_are_equal`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/ftp-utils-core/src/diff.rs
git commit -m "ftpdiff: add PartialEq to DiffEntry"
```

---

### Task 2: Exclude pattern matching

**Files:**
- Create: `crates/ftp-utils-core/src/exclude.rs`
- Modify: `crates/ftp-utils-core/src/lib.rs` (add `pub mod exclude;`)

**Interfaces:**
- Produces: `pub fn is_excluded(relative_path: &str, patterns: &[String]) -> bool`

- [ ] **Step 1: Write the failing test**

Create `crates/ftp-utils-core/src/exclude.rs`:

```rust
//! Glob-based exclude pattern matching for local and remote relative paths.

/// Returns true if `relative_path` matches any of `patterns` (glob syntax).
pub fn is_excluded(relative_path: &str, patterns: &[String]) -> bool {
    todo!()
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ftp-utils-core exclude::tests`
Expected: FAIL (panics with `not yet implemented` from `todo!()`)

- [ ] **Step 3: Implement**

Replace the `todo!()` body:

```rust
pub fn is_excluded(relative_path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        glob::Pattern::new(pattern)
            .map(|compiled| compiled.matches(relative_path))
            .unwrap_or(false)
    })
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ftp-utils-core exclude::tests`
Expected: PASS (3 tests)

- [ ] **Step 5: Wire the module**

In `crates/ftp-utils-core/src/lib.rs`, add `pub mod exclude;` next to the existing `pub mod diff;`.

- [ ] **Step 6: Commit**

```bash
git add crates/ftp-utils-core/src/exclude.rs crates/ftp-utils-core/src/lib.rs
git commit -m "ftpdiff: add glob exclude pattern matching"
```

---

### Task 3: Local directory walking

**Files:**
- Create: `crates/ftp-utils-core/src/local.rs`
- Modify: `crates/ftp-utils-core/src/lib.rs` (add `pub mod local;`)
- Modify: `crates/ftp-utils-core/Cargo.toml` (add `walkdir`, dev-dep `tempfile`)

**Interfaces:**
- Consumes: `exclude::is_excluded(relative_path: &str, patterns: &[String]) -> bool` (Task 2)
- Produces: `pub struct LocalEntry { pub relative_path: String, pub size: u64 }`, `pub fn walk_local_dir(root: &std::path::Path, excludes: &[String]) -> std::io::Result<Vec<LocalEntry>>`

- [ ] **Step 1: Update Cargo.toml**

In `crates/ftp-utils-core/Cargo.toml`:

```toml
[package]
name = "ftp-utils-core"
version = "0.1.0"
edition.workspace = true
license.workspace = true
description = "Shared FTP/FTPS client, diff, and comparison logic for the ftp-utils tool suite"

[dependencies]
suppaftp.workspace = true
glob.workspace = true
walkdir.workspace = true
md5.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

And in the root `Cargo.toml`, add to `[workspace.dependencies]`:

```toml
tempfile = "3"
```

(leave the other existing workspace dependency lines as they are)

- [ ] **Step 2: Write the failing test**

Create `crates/ftp-utils-core/src/local.rs`:

```rust
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
    todo!()
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
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p ftp-utils-core local::tests`
Expected: FAIL (`not yet implemented`)

- [ ] **Step 4: Implement**

Replace the `todo!()` body:

```rust
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
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p ftp-utils-core local::tests`
Expected: PASS (2 tests)

- [ ] **Step 6: Wire the module**

In `crates/ftp-utils-core/src/lib.rs`, add `pub mod local;`.

- [ ] **Step 7: Commit**

```bash
git add crates/ftp-utils-core/Cargo.toml Cargo.toml crates/ftp-utils-core/src/local.rs crates/ftp-utils-core/src/lib.rs
git commit -m "ftpdiff: add local directory walking"
```

---

### Task 4: Remote directory walking (FtpConnection trait)

**Files:**
- Create: `crates/ftp-utils-core/src/remote.rs`
- Modify: `crates/ftp-utils-core/src/lib.rs` (add `pub mod remote;`)

**Interfaces:**
- Consumes: `exclude::is_excluded` (Task 2)
- Produces: `pub struct RawRemoteEntry { pub name: String, pub is_dir: bool, pub size: u64 }`, `pub struct RemoteEntry { pub relative_path: String, pub size: u64 }`, `pub trait FtpConnection { fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError>; fn try_hash(&mut self, path: &str) -> Option<String>; fn retr_to_buffer(&mut self, path: &str) -> Result<Vec<u8>, FtpConnectionError>; }`, `pub struct FtpConnectionError(pub String)` (implements `Display`, `std::error::Error`), `pub fn walk_remote<C: FtpConnection>(conn: &mut C, root: &str, excludes: &[String]) -> Result<Vec<RemoteEntry>, FtpConnectionError>`

- [ ] **Step 1: Write the failing test**

Create `crates/ftp-utils-core/src/remote.rs`:

```rust
//! Remote directory walking over an abstract `FtpConnection`, so the
//! comparison logic never depends directly on a concrete FTP library.

use crate::exclude::is_excluded;

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
}

/// Recursively walks `root` on the remote server, returning one entry per
/// file, skipping any file whose relative path matches one of `excludes`.
pub fn walk_remote<C: FtpConnection>(
    conn: &mut C,
    root: &str,
    excludes: &[String],
) -> Result<Vec<RemoteEntry>, FtpConnectionError> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MockFtpConnection {
        listings: HashMap<String, Vec<RawRemoteEntry>>,
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
        let mut conn = MockFtpConnection { listings };

        let mut entries = walk_remote(&mut conn, "/remote", &[]).unwrap();
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
    fn applies_exclude_patterns() {
        let mut listings = HashMap::new();
        listings.insert(
            "/remote".to_string(),
            vec![
                RawRemoteEntry { name: "keep.txt".into(), is_dir: false, size: 1 },
                RawRemoteEntry { name: "skip.tmp".into(), is_dir: false, size: 1 },
            ],
        );
        let mut conn = MockFtpConnection { listings };

        let entries = walk_remote(&mut conn, "/remote", &["*.tmp".to_string()]).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].relative_path, "keep.txt");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ftp-utils-core remote::tests`
Expected: FAIL (`not yet implemented`)

- [ ] **Step 3: Implement**

Replace the `todo!()` body:

```rust
pub fn walk_remote<C: FtpConnection>(
    conn: &mut C,
    root: &str,
    excludes: &[String],
) -> Result<Vec<RemoteEntry>, FtpConnectionError> {
    let mut entries = Vec::new();
    let mut dirs_to_visit: Vec<String> = vec![String::new()]; // "" means root itself

    while let Some(relative_dir) = dirs_to_visit.pop() {
        let full_path = if relative_dir.is_empty() {
            root.to_string()
        } else {
            format!("{root}/{relative_dir}")
        };

        for item in conn.list_dir(&full_path)? {
            let relative_path = if relative_dir.is_empty() {
                item.name.clone()
            } else {
                format!("{relative_dir}/{}", item.name)
            };

            if item.is_dir {
                dirs_to_visit.push(relative_path);
                continue;
            }

            if is_excluded(&relative_path, excludes) {
                continue;
            }

            entries.push(RemoteEntry { relative_path, size: item.size });
        }
    }

    Ok(entries)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ftp-utils-core remote::tests`
Expected: PASS (2 tests)

- [ ] **Step 5: Wire the module**

In `crates/ftp-utils-core/src/lib.rs`, add `pub mod remote;`.

- [ ] **Step 6: Commit**

```bash
git add crates/ftp-utils-core/src/remote.rs crates/ftp-utils-core/src/lib.rs
git commit -m "ftpdiff: add remote directory walking via FtpConnection trait"
```

---

### Task 5: Diff engine

**Files:**
- Create: `crates/ftp-utils-core/src/compare.rs`
- Modify: `crates/ftp-utils-core/src/lib.rs` (add `pub mod compare;`)

**Interfaces:**
- Consumes: `diff::{DiffEntry, DiffStatus}` (Task 1), `local::LocalEntry` (Task 3), `remote::RemoteEntry` (Task 4)
- Produces: `pub fn compare_entries(local: &[LocalEntry], remote: &[RemoteEntry]) -> Vec<DiffEntry>`

- [ ] **Step 1: Write the failing test**

Create `crates/ftp-utils-core/src/compare.rs`:

```rust
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
    todo!()
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ftp-utils-core compare::tests`
Expected: FAIL (`not yet implemented`)

- [ ] **Step 3: Implement**

Replace the `todo!()` body:

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ftp-utils-core compare::tests`
Expected: PASS (3 tests)

- [ ] **Step 5: Wire the module**

In `crates/ftp-utils-core/src/lib.rs`, add `pub mod compare;`.

- [ ] **Step 6: Commit**

```bash
git add crates/ftp-utils-core/src/compare.rs crates/ftp-utils-core/src/lib.rs
git commit -m "ftpdiff: add size-based diff engine"
```

---

### Task 6: Hash fallback pass

**Files:**
- Create: `crates/ftp-utils-core/src/hash.rs`
- Modify: `crates/ftp-utils-core/src/lib.rs` (add `pub mod hash;`)

**Interfaces:**
- Consumes: `diff::{DiffEntry, DiffStatus}` (Task 1), `remote::FtpConnection` (Task 4)
- Produces: `pub fn apply_hash_comparison<C: FtpConnection>(conn: &mut C, remote_root: &str, local_root: &std::path::Path, entries: &mut [DiffEntry]) -> std::io::Result<()>`

- [ ] **Step 1: Write the failing test**

Create `crates/ftp-utils-core/src/hash.rs`:

```rust
//! Upgrades `Match` diff entries to a hash-verified `Match` or
//! `HashMismatch` by comparing MD5 hashes. Tries the connection's
//! server-side hash first; falls back to downloading the remote file and
//! hashing it locally if the server doesn't support that.

use std::path::Path;

use crate::diff::{DiffEntry, DiffStatus};
use crate::remote::FtpConnection;

/// For every entry currently marked `Match`, computes and compares MD5
/// hashes, updating `status` to `Match` or `HashMismatch` and filling in
/// `local_md5`/`remote_md5`. Entries with any other status are untouched.
pub fn apply_hash_comparison<C: FtpConnection>(
    conn: &mut C,
    remote_root: &str,
    local_root: &Path,
    entries: &mut [DiffEntry],
) -> std::io::Result<()> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{FtpConnectionError, RawRemoteEntry};

    struct MockConnection {
        hash: Option<String>,
        remote_bytes: Vec<u8>,
    }

    impl FtpConnection for MockConnection {
        fn list_dir(&mut self, _path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
            Ok(Vec::new())
        }

        fn try_hash(&mut self, _path: &str) -> Option<String> {
            self.hash.clone()
        }

        fn retr_to_buffer(&mut self, _path: &str) -> Result<Vec<u8>, FtpConnectionError> {
            Ok(self.remote_bytes.clone())
        }
    }

    fn entry(relative_path: &str) -> DiffEntry {
        DiffEntry {
            relative_path: relative_path.to_string(),
            status: DiffStatus::Match,
            local_size: Some(5),
            remote_size: Some(5),
            local_md5: None,
            remote_md5: None,
        }
    }

    #[test]
    fn uses_remote_hash_when_available() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();
        let expected_hash = format!("{:x}", md5::compute(b"hello"));

        let mut entries = vec![entry("f.txt")];
        let mut conn = MockConnection { hash: Some(expected_hash.clone()), remote_bytes: Vec::new() };

        apply_hash_comparison(&mut conn, "/remote", dir.path(), &mut entries).unwrap();

        assert_eq!(entries[0].status, DiffStatus::Match);
        assert_eq!(entries[0].remote_md5, Some(expected_hash));
    }

    #[test]
    fn falls_back_to_download_when_hash_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();

        let mut entries = vec![entry("f.txt")];
        let mut conn = MockConnection { hash: None, remote_bytes: b"different".to_vec() };

        apply_hash_comparison(&mut conn, "/remote", dir.path(), &mut entries).unwrap();

        assert_eq!(entries[0].status, DiffStatus::HashMismatch);
    }

    #[test]
    fn leaves_non_match_entries_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let mut entries = vec![DiffEntry {
            relative_path: "only-local.txt".to_string(),
            status: DiffStatus::LocalOnly,
            local_size: Some(5),
            remote_size: None,
            local_md5: None,
            remote_md5: None,
        }];
        let mut conn = MockConnection { hash: None, remote_bytes: Vec::new() };

        apply_hash_comparison(&mut conn, "/remote", dir.path(), &mut entries).unwrap();

        assert_eq!(entries[0].status, DiffStatus::LocalOnly);
        assert_eq!(entries[0].remote_md5, None);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ftp-utils-core hash::tests`
Expected: FAIL (`not yet implemented`)

- [ ] **Step 3: Implement**

Replace the `todo!()` body:

```rust
pub fn apply_hash_comparison<C: FtpConnection>(
    conn: &mut C,
    remote_root: &str,
    local_root: &Path,
    entries: &mut [DiffEntry],
) -> std::io::Result<()> {
    for entry in entries.iter_mut() {
        if entry.status != DiffStatus::Match {
            continue;
        }

        let remote_path = format!("{remote_root}/{}", entry.relative_path);
        let local_path = local_root.join(&entry.relative_path);

        let remote_md5 = match conn.try_hash(&remote_path) {
            Some(hash) => hash,
            None => {
                let bytes = conn
                    .retr_to_buffer(&remote_path)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
                format!("{:x}", md5::compute(&bytes))
            }
        };

        let local_bytes = std::fs::read(&local_path)?;
        let local_md5 = format!("{:x}", md5::compute(&local_bytes));

        entry.status = if local_md5 == remote_md5 {
            DiffStatus::Match
        } else {
            DiffStatus::HashMismatch
        };
        entry.local_md5 = Some(local_md5);
        entry.remote_md5 = Some(remote_md5);
    }

    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ftp-utils-core hash::tests`
Expected: PASS (3 tests)

- [ ] **Step 5: Wire the module**

In `crates/ftp-utils-core/src/lib.rs`, add `pub mod hash;`.

- [ ] **Step 6: Commit**

```bash
git add crates/ftp-utils-core/src/hash.rs crates/ftp-utils-core/src/lib.rs
git commit -m "ftpdiff: add MD5 hash fallback comparison pass"
```

---

### Task 7: Real suppaftp-backed FtpConnection

**Files:**
- Create: `crates/ftp-utils-core/src/ftp_client.rs`
- Modify: `crates/ftp-utils-core/src/lib.rs` (add `pub mod ftp_client;`)
- Modify: `CHANGES.md` (note the v1 hash-command limitation)

**Interfaces:**
- Consumes: `remote::{FtpConnection, FtpConnectionError, RawRemoteEntry}` (Task 4)
- Produces: `pub enum SuppaFtpConnection { Plain(suppaftp::FtpStream), Tls(suppaftp::NativeTlsFtpStream) }`, `impl SuppaFtpConnection { pub fn connect(host: &str, port: u16, user: &str, password: &str, ftps: bool) -> Result<Self, FtpConnectionError>; pub fn close(self); }`, `impl FtpConnection for SuppaFtpConnection`

This task has no automated network test (it requires a real or containerized
FTP server). It is verified by compiling and by the manual end-to-end check
in Task 14.

- [ ] **Step 1: Implement**

Create `crates/ftp-utils-core/src/ftp_client.rs`:

```rust
//! Real `FtpConnection` implementation backed by the `suppaftp` crate,
//! supporting both plain FTP and explicit FTPS (AUTH TLS).

use suppaftp::list::ListParser;
use suppaftp::native_tls::TlsConnector;
use suppaftp::{FtpStream, NativeTlsConnector, NativeTlsFtpStream};

use crate::remote::{FtpConnection, FtpConnectionError, RawRemoteEntry};

/// A live FTP or FTPS connection. Kept as an enum (rather than a trait
/// object) because `suppaftp`'s plain and TLS streams are distinct
/// concrete types selected once at connect time.
pub enum SuppaFtpConnection {
    Plain(FtpStream),
    Tls(NativeTlsFtpStream),
}

impl SuppaFtpConnection {
    /// Connects and authenticates. Uses explicit FTPS (AUTH TLS upgrade on
    /// the plain control channel) when `ftps` is true.
    pub fn connect(
        host: &str,
        port: u16,
        user: &str,
        password: &str,
        ftps: bool,
    ) -> Result<Self, FtpConnectionError> {
        let address = format!("{host}:{port}");

        if ftps {
            let stream = NativeTlsFtpStream::connect(&address)
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            let connector = TlsConnector::new().map_err(|e| FtpConnectionError(e.to_string()))?;
            let mut stream = stream
                .into_secure(NativeTlsConnector::from(connector), host)
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            stream
                .login(user, password)
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            Ok(SuppaFtpConnection::Tls(stream))
        } else {
            let mut stream =
                FtpStream::connect(&address).map_err(|e| FtpConnectionError(e.to_string()))?;
            stream
                .login(user, password)
                .map_err(|e| FtpConnectionError(e.to_string()))?;
            Ok(SuppaFtpConnection::Plain(stream))
        }
    }

    /// Sends QUIT and closes the connection. Errors are ignored: by the
    /// time this is called, the comparison has already finished.
    pub fn close(self) {
        match self {
            SuppaFtpConnection::Plain(stream) => {
                let _ = stream.quit();
            }
            SuppaFtpConnection::Tls(stream) => {
                let _ = stream.quit();
            }
        }
    }
}

impl FtpConnection for SuppaFtpConnection {
    fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
        let lines = match self {
            SuppaFtpConnection::Plain(stream) => stream.list(Some(path)),
            SuppaFtpConnection::Tls(stream) => stream.list(Some(path)),
        }
        .map_err(|e| FtpConnectionError(e.to_string()))?;

        let mut entries = Vec::new();
        for line in lines {
            let Ok(file) = ListParser::parse_posix(&line) else {
                continue;
            };
            let name = file.name();
            if name == "." || name == ".." {
                continue;
            }
            entries.push(RawRemoteEntry {
                name: name.to_string(),
                is_dir: file.is_directory(),
                size: file.size() as u64,
            });
        }
        Ok(entries)
    }

    fn try_hash(&mut self, _path: &str) -> Option<String> {
        // suppaftp (v10, as researched via its current docs) does not
        // expose a public API for sending non-standard hash commands
        // (XMD5/MD5/HASH). Always fall back to download + local MD5; see
        // hash::apply_hash_comparison.
        None
    }

    fn retr_to_buffer(&mut self, path: &str) -> Result<Vec<u8>, FtpConnectionError> {
        let cursor = match self {
            SuppaFtpConnection::Plain(stream) => stream.retr_as_buffer(path),
            SuppaFtpConnection::Tls(stream) => stream.retr_as_buffer(path),
        }
        .map_err(|e| FtpConnectionError(e.to_string()))?;
        Ok(cursor.into_inner())
    }
}
```

- [ ] **Step 2: Wire the module**

In `crates/ftp-utils-core/src/lib.rs`, add `pub mod ftp_client;`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p ftp-utils-core`
Expected: builds cleanly (suppaftp is already a workspace dependency from the initial scaffold commit; if the build reports it's missing a `native-tls` feature error, check `crates/ftp-utils-core/Cargo.toml` includes `suppaftp.workspace = true` and the root `Cargo.toml`'s `[workspace.dependencies]` has `suppaftp = { version = "10", features = ["native-tls"] }`)

- [ ] **Step 4: Update CHANGES.md**

Append to the `## Unreleased` section of `CHANGES.md`:

```markdown
- Implemented ftp-utils-core: local/remote directory walking, exclude
  patterns, size-based diff engine, MD5 hash fallback comparison, and a
  real suppaftp-backed `FtpConnection` (FTP + explicit FTPS). Note: server-
  side remote hash commands (XMD5/MD5/HASH) are not attempted in v1, since
  suppaftp does not expose a public API for them; `--hash` always falls
  back to download + local MD5.
```

- [ ] **Step 5: Commit**

```bash
git add crates/ftp-utils-core/src/ftp_client.rs crates/ftp-utils-core/src/lib.rs CHANGES.md
git commit -m "ftpdiff: add suppaftp-backed FtpConnection (FTP + FTPS)"
```

---

### Task 8: Core library wiring (compare())

**Files:**
- Modify: `crates/ftp-utils-core/src/lib.rs`

**Interfaces:**
- Consumes: `local::walk_local_dir` (Task 3), `remote::{walk_remote, FtpConnection, FtpConnectionError}` (Task 4), `compare::compare_entries` (Task 5), `hash::apply_hash_comparison` (Task 6)
- Produces: `pub struct CompareOptions { pub local_dir: std::path::PathBuf, pub remote_dir: String, pub excludes: Vec<String>, pub hash: bool }`, `pub enum CompareError { Io(std::io::Error), Ftp(FtpConnectionError) }` (implements `Display`, `std::error::Error`, `From<std::io::Error>`, `From<FtpConnectionError>`), `pub fn compare<C: FtpConnection>(conn: &mut C, opts: &CompareOptions) -> Result<Vec<DiffEntry>, CompareError>`

- [ ] **Step 1: Write the failing test**

Replace the full contents of `crates/ftp-utils-core/src/lib.rs` with:

```rust
//! Shared FTP/FTPS client, directory comparison, and diff logic used by
//! all tools in the ftp-utils suite.
//!
//! See `docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md`
//! for the design this crate implements.

pub mod compare;
pub mod diff;
pub mod exclude;
pub mod ftp_client;
pub mod hash;
pub mod local;
pub mod remote;

pub use diff::{DiffEntry, DiffStatus};
pub use remote::{FtpConnection, FtpConnectionError, RawRemoteEntry};

use std::path::PathBuf;

/// Inputs to a single `compare` run.
pub struct CompareOptions {
    pub local_dir: PathBuf,
    pub remote_dir: String,
    pub excludes: Vec<String>,
    pub hash: bool,
}

/// Error from a top-level `compare` call.
#[derive(Debug)]
pub enum CompareError {
    Io(std::io::Error),
    Ftp(FtpConnectionError),
}

impl std::fmt::Display for CompareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompareError::Io(e) => write!(f, "{e}"),
            CompareError::Ftp(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CompareError {}

impl From<std::io::Error> for CompareError {
    fn from(e: std::io::Error) -> Self {
        CompareError::Io(e)
    }
}

impl From<FtpConnectionError> for CompareError {
    fn from(e: FtpConnectionError) -> Self {
        CompareError::Ftp(e)
    }
}

/// Walks the local and remote directory trees, diffs them by size, and
/// (if `opts.hash` is set) upgrades same-size matches with an MD5 check.
pub fn compare<C: FtpConnection>(
    conn: &mut C,
    opts: &CompareOptions,
) -> Result<Vec<DiffEntry>, CompareError> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::RawRemoteEntry;
    use std::collections::HashMap;

    struct MockConnection {
        listings: HashMap<String, Vec<RawRemoteEntry>>,
    }

    impl FtpConnection for MockConnection {
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
    }

    #[test]
    fn compares_local_and_remote_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("only_local.txt"), b"local").unwrap();
        std::fs::write(dir.path().join("shared.txt"), b"12345").unwrap();

        let mut listings = HashMap::new();
        listings.insert(
            "/remote".to_string(),
            vec![
                RawRemoteEntry { name: "shared.txt".into(), is_dir: false, size: 5 },
                RawRemoteEntry { name: "only_remote.txt".into(), is_dir: false, size: 3 },
            ],
        );
        let mut conn = MockConnection { listings };

        let opts = CompareOptions {
            local_dir: dir.path().to_path_buf(),
            remote_dir: "/remote".to_string(),
            excludes: Vec::new(),
            hash: false,
        };

        let entries = compare(&mut conn, &opts).unwrap();

        assert!(entries
            .iter()
            .any(|e| e.relative_path == "only_local.txt" && e.status == DiffStatus::LocalOnly));
        assert!(entries
            .iter()
            .any(|e| e.relative_path == "only_remote.txt" && e.status == DiffStatus::RemoteOnly));
        assert!(entries
            .iter()
            .any(|e| e.relative_path == "shared.txt" && e.status == DiffStatus::Match));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ftp-utils-core compares_local_and_remote_end_to_end`
Expected: FAIL (`not yet implemented`)

- [ ] **Step 3: Implement**

Replace the `todo!()` body of `compare`:

```rust
pub fn compare<C: FtpConnection>(
    conn: &mut C,
    opts: &CompareOptions,
) -> Result<Vec<DiffEntry>, CompareError> {
    let local_entries = local::walk_local_dir(&opts.local_dir, &opts.excludes)?;
    let remote_entries = remote::walk_remote(conn, &opts.remote_dir, &opts.excludes)?;
    let mut entries = compare::compare_entries(&local_entries, &remote_entries);

    if opts.hash {
        hash::apply_hash_comparison(conn, &opts.remote_dir, &opts.local_dir, &mut entries)?;
    }

    Ok(entries)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ftp-utils-core compares_local_and_remote_end_to_end`
Expected: PASS

- [ ] **Step 5: Run the full core test suite**

Run: `cargo test -p ftp-utils-core`
Expected: PASS (all tests from Tasks 1-8)

- [ ] **Step 6: Commit**

```bash
git add crates/ftp-utils-core/src/lib.rs
git commit -m "ftpdiff: wire ftp-utils-core's top-level compare() function"
```

---

### Task 9: ftpdiff CLI argument parsing

**Files:**
- Create: `tools/ftpdiff/src/cli.rs`
- Modify: `tools/ftpdiff/Cargo.toml` (add `clap`)
- Modify: `tools/ftpdiff/src/main.rs` (add `mod cli;`, keep the existing stub `fn main()` for now)

**Interfaces:**
- Produces: `pub struct Cli { pub config: Option<PathBuf>, pub host: Option<String>, pub port: Option<u16>, pub user: Option<String>, pub remote_dir: Option<String>, pub local_dir: Option<PathBuf>, pub ftps: bool, pub hash: bool, pub exclude: Vec<String>, pub csv: Option<PathBuf> }` (derives `clap::Parser`)

- [ ] **Step 1: Update Cargo.toml**

Replace `tools/ftpdiff/Cargo.toml`:

```toml
[package]
name = "ftpdiff"
version = "0.1.0"
edition.workspace = true
license.workspace = true
description = "Compare a remote FTP/FTPS directory tree against a local copy"

[dependencies]
ftp-utils-core = { path = "../../crates/ftp-utils-core" }
clap.workspace = true
serde.workspace = true
serde_json.workspace = true
csv.workspace = true
colored.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

And add to the root `Cargo.toml`'s `[workspace.dependencies]`:

```toml
serde_json = "1"
csv = "1"
colored = "2"
```

(keep the `clap` and `serde` lines already there; the root `Cargo.toml` should now list: `suppaftp`, `clap`, `glob`, `walkdir`, `serde`, `serde_json`, `md5`, `csv`, `colored`, `tempfile`)

- [ ] **Step 2: Write the failing test**

Create `tools/ftpdiff/src/cli.rs`:

```rust
//! Command-line argument definitions for ftpdiff.

use std::path::PathBuf;

use clap::Parser;

/// Compare a remote FTP/FTPS directory tree against a local copy.
#[derive(Parser, Debug, Clone)]
#[command(name = "ftpdiff", about = "Compare a remote FTP/FTPS directory tree against a local copy")]
pub struct Cli {
    /// Optional JSON config file providing defaults for the other options.
    #[arg(long)]
    pub config: Option<PathBuf>,

    #[arg(long)]
    pub host: Option<String>,

    #[arg(long)]
    pub port: Option<u16>,

    #[arg(long)]
    pub user: Option<String>,

    #[arg(long = "remote-dir")]
    pub remote_dir: Option<String>,

    #[arg(long = "local-dir")]
    pub local_dir: Option<PathBuf>,

    /// Use explicit FTPS (AUTH TLS) instead of plain FTP.
    #[arg(long)]
    pub ftps: bool,

    /// Compare files by MD5 hash in addition to size.
    #[arg(long)]
    pub hash: bool,

    /// Glob pattern to exclude from comparison; can be repeated.
    #[arg(long = "exclude")]
    pub exclude: Vec<String>,

    /// Also write a structured CSV report to this path.
    #[arg(long)]
    pub csv: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_args() {
        let cli = Cli::parse_from([
            "ftpdiff",
            "--host", "ftp.example.com",
            "--user", "bob",
            "--remote-dir", "/remote",
            "--local-dir", "./local",
        ]);

        assert_eq!(cli.host.as_deref(), Some("ftp.example.com"));
        assert_eq!(cli.user.as_deref(), Some("bob"));
        assert_eq!(cli.remote_dir.as_deref(), Some("/remote"));
        assert!(!cli.ftps);
        assert!(!cli.hash);
        assert!(cli.exclude.is_empty());
    }

    #[test]
    fn parses_repeated_exclude_and_flags() {
        let cli = Cli::parse_from([
            "ftpdiff",
            "--exclude", "*.tmp",
            "--exclude", ".git/*",
            "--ftps",
            "--hash",
        ]);

        assert_eq!(cli.exclude, vec!["*.tmp".to_string(), ".git/*".to_string()]);
        assert!(cli.ftps);
        assert!(cli.hash);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p ftpdiff cli::tests`
Expected: FAIL (compile error: `mod cli;` not yet declared in `main.rs`, or `clap` not yet a dependency if Step 1 wasn't applied)

- [ ] **Step 4: Wire the module**

Update `tools/ftpdiff/src/main.rs` to add the module declaration, keeping the existing stub body:

```rust
//! ftpdiff: compare a remote FTP/FTPS directory tree against a local copy.
//!
//! CLI, config loading, and comparison flow will be implemented according
//! to the implementation plan derived from
//! `docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md`.

mod cli;

fn main() {
    println!("ftpdiff: not yet implemented");
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p ftpdiff cli::tests`
Expected: PASS (2 tests; expect an "unused" warning on `cli`, which is fine and will go away once `main.rs` uses it in Task 14)

- [ ] **Step 6: Commit**

```bash
git add tools/ftpdiff/Cargo.toml Cargo.toml tools/ftpdiff/src/cli.rs tools/ftpdiff/src/main.rs
git commit -m "ftpdiff: add CLI argument parsing"
```

---

### Task 10: Config loading and merge

**Files:**
- Create: `tools/ftpdiff/src/config.rs`
- Modify: `tools/ftpdiff/src/main.rs` (add `mod config;`)

**Interfaces:**
- Consumes: `cli::Cli` (Task 9)
- Produces: `pub struct JsonConfig { pub host: Option<String>, pub port: Option<u16>, pub user: Option<String>, pub remote_dir: Option<String>, pub local_dir: Option<PathBuf>, pub ftps: Option<bool>, pub hash: Option<bool>, pub exclude: Vec<String> }` (derives `serde::Deserialize`, `Default`), `pub struct EffectiveConfig { pub host: String, pub port: u16, pub user: String, pub remote_dir: String, pub local_dir: PathBuf, pub ftps: bool, pub hash: bool, pub exclude: Vec<String>, pub csv: Option<PathBuf> }`, `pub struct ConfigError(pub String)` (implements `Display`, `std::error::Error`), `pub fn load_json_config(path: &Path) -> Result<JsonConfig, ConfigError>`, `pub fn merge(cli: &Cli, json: &JsonConfig) -> Result<EffectiveConfig, ConfigError>`

- [ ] **Step 1: Write the failing test**

Create `tools/ftpdiff/src/config.rs`:

```rust
//! JSON config loading and CLI/JSON/default merge logic.
//!
//! Precedence: CLI args > JSON config file > defaults. Boolean flags merge
//! as OR (CLI can enable, never disable, a JSON-set true). `--exclude`
//! patterns merge as a union of both sources.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::cli::Cli;

#[derive(Debug, Deserialize, Default)]
pub struct JsonConfig {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub remote_dir: Option<String>,
    pub local_dir: Option<PathBuf>,
    pub ftps: Option<bool>,
    pub hash: Option<bool>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub remote_dir: String,
    pub local_dir: PathBuf,
    pub ftps: bool,
    pub hash: bool,
    pub exclude: Vec<String>,
    pub csv: Option<PathBuf>,
}

#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConfigError {}

pub fn load_json_config(path: &Path) -> Result<JsonConfig, ConfigError> {
    todo!()
}

pub fn merge(cli: &Cli, json: &JsonConfig) -> Result<EffectiveConfig, ConfigError> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_cli() -> Cli {
        Cli {
            config: None,
            host: None,
            port: None,
            user: None,
            remote_dir: None,
            local_dir: None,
            ftps: false,
            hash: false,
            exclude: Vec::new(),
            csv: None,
        }
    }

    #[test]
    fn loads_json_config_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"host": "ftp.example.com", "user": "bob", "exclude": ["*.tmp"]}"#,
        )
        .unwrap();

        let config = load_json_config(&path).unwrap();

        assert_eq!(config.host.as_deref(), Some("ftp.example.com"));
        assert_eq!(config.user.as_deref(), Some("bob"));
        assert_eq!(config.exclude, vec!["*.tmp".to_string()]);
    }

    #[test]
    fn cli_overrides_json_for_scalar_fields() {
        let mut cli = empty_cli();
        cli.host = Some("cli-host".to_string());
        cli.user = Some("cli-user".to_string());
        cli.remote_dir = Some("/cli-remote".to_string());
        cli.local_dir = Some(PathBuf::from("./cli-local"));

        let json = JsonConfig {
            host: Some("json-host".to_string()),
            user: Some("json-user".to_string()),
            remote_dir: Some("/json-remote".to_string()),
            local_dir: Some(PathBuf::from("./json-local")),
            ..Default::default()
        };

        let effective = merge(&cli, &json).unwrap();

        assert_eq!(effective.host, "cli-host");
        assert_eq!(effective.user, "cli-user");
        assert_eq!(effective.remote_dir, "/cli-remote");
        assert_eq!(effective.local_dir, PathBuf::from("./cli-local"));
    }

    #[test]
    fn falls_back_to_json_when_cli_field_absent() {
        let cli = empty_cli();
        let json = JsonConfig {
            host: Some("json-host".to_string()),
            user: Some("json-user".to_string()),
            remote_dir: Some("/json-remote".to_string()),
            local_dir: Some(PathBuf::from("./json-local")),
            ..Default::default()
        };

        let effective = merge(&cli, &json).unwrap();

        assert_eq!(effective.host, "json-host");
        assert_eq!(effective.user, "json-user");
    }

    #[test]
    fn errors_when_required_field_missing_everywhere() {
        let cli = empty_cli();
        let json = JsonConfig::default();

        let result = merge(&cli, &json);

        assert!(result.is_err());
    }

    #[test]
    fn boolean_flags_merge_as_or() {
        let mut cli = empty_cli();
        cli.host = Some("h".into());
        cli.user = Some("u".into());
        cli.remote_dir = Some("/r".into());
        cli.local_dir = Some(PathBuf::from("./l"));
        cli.ftps = true;

        let json = JsonConfig { hash: Some(true), ..Default::default() };

        let effective = merge(&cli, &json).unwrap();

        assert!(effective.ftps); // from CLI
        assert!(effective.hash); // from JSON
    }

    #[test]
    fn exclude_patterns_are_unioned() {
        let mut cli = empty_cli();
        cli.host = Some("h".into());
        cli.user = Some("u".into());
        cli.remote_dir = Some("/r".into());
        cli.local_dir = Some(PathBuf::from("./l"));
        cli.exclude = vec!["*.tmp".to_string()];

        let json = JsonConfig { exclude: vec![".git/*".to_string()], ..Default::default() };

        let effective = merge(&cli, &json).unwrap();

        assert_eq!(effective.exclude, vec![".git/*".to_string(), "*.tmp".to_string()]);
    }

    #[test]
    fn default_port_is_21() {
        let mut cli = empty_cli();
        cli.host = Some("h".into());
        cli.user = Some("u".into());
        cli.remote_dir = Some("/r".into());
        cli.local_dir = Some(PathBuf::from("./l"));

        let effective = merge(&cli, &JsonConfig::default()).unwrap();

        assert_eq!(effective.port, 21);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ftpdiff config::tests`
Expected: FAIL (`not yet implemented`, and a compile error since `mod config;` isn't wired yet)

- [ ] **Step 3: Wire the module (before implementing, so tests can compile)**

Update `tools/ftpdiff/src/main.rs`:

```rust
mod cli;
mod config;

fn main() {
    println!("ftpdiff: not yet implemented");
}
```

- [ ] **Step 4: Implement**

Replace the two `todo!()` bodies in `tools/ftpdiff/src/config.rs`:

```rust
pub fn load_json_config(path: &Path) -> Result<JsonConfig, ConfigError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(format!("cannot read config file {}: {e}", path.display())))?;
    serde_json::from_str(&content)
        .map_err(|e| ConfigError(format!("invalid JSON in {}: {e}", path.display())))
}

pub fn merge(cli: &Cli, json: &JsonConfig) -> Result<EffectiveConfig, ConfigError> {
    let host = cli
        .host
        .clone()
        .or_else(|| json.host.clone())
        .ok_or_else(|| ConfigError("missing required setting: host (use --host or config file)".to_string()))?;

    let user = cli
        .user
        .clone()
        .or_else(|| json.user.clone())
        .ok_or_else(|| ConfigError("missing required setting: user (use --user or config file)".to_string()))?;

    let remote_dir = cli
        .remote_dir
        .clone()
        .or_else(|| json.remote_dir.clone())
        .ok_or_else(|| {
            ConfigError("missing required setting: remote-dir (use --remote-dir or config file)".to_string())
        })?;

    let local_dir = cli
        .local_dir
        .clone()
        .or_else(|| json.local_dir.clone())
        .ok_or_else(|| {
            ConfigError("missing required setting: local-dir (use --local-dir or config file)".to_string())
        })?;

    let mut exclude = json.exclude.clone();
    exclude.extend(cli.exclude.clone());

    Ok(EffectiveConfig {
        host,
        port: cli.port.or(json.port).unwrap_or(21),
        user,
        remote_dir,
        local_dir,
        ftps: cli.ftps || json.ftps.unwrap_or(false),
        hash: cli.hash || json.hash.unwrap_or(false),
        exclude,
        csv: cli.csv.clone(),
    })
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p ftpdiff config::tests`
Expected: PASS (7 tests)

- [ ] **Step 6: Commit**

```bash
git add tools/ftpdiff/src/config.rs tools/ftpdiff/src/main.rs
git commit -m "ftpdiff: add JSON config loading and CLI/JSON merge"
```

---

### Task 11: Password from environment

**Files:**
- Modify: `tools/ftpdiff/src/config.rs`

**Interfaces:**
- Produces: `pub fn read_password() -> Result<String, ConfigError>`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `tools/ftpdiff/src/config.rs`:

```rust
    use std::sync::Mutex;

    // FTPDIFF_PASSWORD is process-global state; serialize the two tests
    // that touch it so they can't race under parallel test execution.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    #[test]
    fn reads_password_from_env_var() {
        let _guard = ENV_GUARD.lock().unwrap();
        std::env::set_var("FTPDIFF_PASSWORD", "s3cr3t");

        let result = read_password();

        assert_eq!(result.unwrap(), "s3cr3t");
        std::env::remove_var("FTPDIFF_PASSWORD");
    }

    #[test]
    fn errors_when_password_env_var_missing() {
        let _guard = ENV_GUARD.lock().unwrap();
        std::env::remove_var("FTPDIFF_PASSWORD");

        let result = read_password();

        assert!(result.is_err());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ftpdiff config::tests::reads_password_from_env_var config::tests::errors_when_password_env_var_missing`
Expected: FAIL (`read_password` not defined)

- [ ] **Step 3: Implement**

Add to `tools/ftpdiff/src/config.rs` (after `merge`):

```rust
/// Reads the FTP password from the `FTPDIFF_PASSWORD` environment
/// variable. Never accepted via CLI flag or JSON config, to avoid leaking
/// it into shell history or a config file on disk.
pub fn read_password() -> Result<String, ConfigError> {
    std::env::var("FTPDIFF_PASSWORD")
        .map_err(|_| ConfigError("FTPDIFF_PASSWORD environment variable is not set".to_string()))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ftpdiff config::tests`
Expected: PASS (9 tests total)

- [ ] **Step 5: Commit**

```bash
git add tools/ftpdiff/src/config.rs
git commit -m "ftpdiff: read FTP password from FTPDIFF_PASSWORD env var"
```

---

### Task 12: Text output formatting

**Files:**
- Create: `tools/ftpdiff/src/output.rs`
- Modify: `tools/ftpdiff/src/main.rs` (add `mod output;`)

**Interfaces:**
- Consumes: `ftp_utils_core::{DiffEntry, DiffStatus}` (Task 1/8)
- Produces: `pub fn format_entry(entry: &DiffEntry) -> String`, `pub struct Summary { pub local_only: usize, pub remote_only: usize, pub size_mismatch: usize, pub hash_mismatch: usize, pub matched: usize }`, `pub fn summarize(entries: &[DiffEntry]) -> Summary`, `pub fn format_summary(summary: &Summary) -> String`

- [ ] **Step 1: Write the failing test**

Create `tools/ftpdiff/src/output.rs`:

```rust
//! Human-readable colored text output for diff results.

use colored::Colorize;
use ftp_utils_core::{DiffEntry, DiffStatus};

/// Formats one diff entry as a single colored line.
pub fn format_entry(entry: &DiffEntry) -> String {
    todo!()
}

/// Counts of entries by status.
pub struct Summary {
    pub local_only: usize,
    pub remote_only: usize,
    pub size_mismatch: usize,
    pub hash_mismatch: usize,
    pub matched: usize,
}

/// Tallies `entries` by status.
pub fn summarize(entries: &[DiffEntry]) -> Summary {
    let mut summary = Summary { local_only: 0, remote_only: 0, size_mismatch: 0, hash_mismatch: 0, matched: 0 };
    for entry in entries {
        match entry.status {
            DiffStatus::LocalOnly => summary.local_only += 1,
            DiffStatus::RemoteOnly => summary.remote_only += 1,
            DiffStatus::SizeMismatch => summary.size_mismatch += 1,
            DiffStatus::HashMismatch => summary.hash_mismatch += 1,
            DiffStatus::Match => summary.matched += 1,
        }
    }
    summary
}

/// Formats a one-line summary of the tallies.
pub fn format_summary(summary: &Summary) -> String {
    format!(
        "Summary: {} matched, {} local-only, {} remote-only, {} size mismatch, {} hash mismatch",
        summary.matched, summary.local_only, summary.remote_only, summary.size_mismatch, summary.hash_mismatch
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(relative_path: &str, status: DiffStatus) -> DiffEntry {
        DiffEntry {
            relative_path: relative_path.to_string(),
            status,
            local_size: None,
            remote_size: None,
            local_md5: None,
            remote_md5: None,
        }
    }

    #[test]
    fn formats_each_status_with_the_relative_path() {
        colored::control::set_override(false);

        assert!(format_entry(&entry("a.txt", DiffStatus::LocalOnly)).contains("a.txt"));
        assert!(format_entry(&entry("b.txt", DiffStatus::RemoteOnly)).contains("b.txt"));
        assert!(format_entry(&entry("c.txt", DiffStatus::SizeMismatch)).contains("c.txt"));
        assert!(format_entry(&entry("d.txt", DiffStatus::HashMismatch)).contains("d.txt"));
        assert!(format_entry(&entry("e.txt", DiffStatus::Match)).contains("e.txt"));
    }

    #[test]
    fn summarizes_counts_by_status() {
        let entries = vec![
            entry("a", DiffStatus::LocalOnly),
            entry("b", DiffStatus::LocalOnly),
            entry("c", DiffStatus::RemoteOnly),
            entry("d", DiffStatus::Match),
        ];

        let summary = summarize(&entries);

        assert_eq!(summary.local_only, 2);
        assert_eq!(summary.remote_only, 1);
        assert_eq!(summary.matched, 1);
        assert_eq!(summary.size_mismatch, 0);
        assert_eq!(summary.hash_mismatch, 0);
    }

    #[test]
    fn formats_summary_line() {
        let summary = Summary { local_only: 1, remote_only: 2, size_mismatch: 3, hash_mismatch: 4, matched: 5 };
        let line = format_summary(&summary);

        assert!(line.contains("5 matched"));
        assert!(line.contains("1 local-only"));
        assert!(line.contains("2 remote-only"));
        assert!(line.contains("3 size mismatch"));
        assert!(line.contains("4 hash mismatch"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ftpdiff output::tests`
Expected: FAIL (`not yet implemented`, plus a compile error until `mod output;` is wired)

- [ ] **Step 3: Wire the module**

Update `tools/ftpdiff/src/main.rs`:

```rust
mod cli;
mod config;
mod output;

fn main() {
    println!("ftpdiff: not yet implemented");
}
```

- [ ] **Step 4: Implement**

Replace the `todo!()` body of `format_entry`:

```rust
pub fn format_entry(entry: &DiffEntry) -> String {
    match entry.status {
        DiffStatus::LocalOnly => format!("{} {}", "+".green(), entry.relative_path),
        DiffStatus::RemoteOnly => format!("{} {}", "-".red(), entry.relative_path),
        DiffStatus::SizeMismatch => format!(
            "{} {} (local={:?}, remote={:?})",
            "~".yellow(),
            entry.relative_path,
            entry.local_size,
            entry.remote_size
        ),
        DiffStatus::HashMismatch => format!("{} {} (hash differs)", "~".yellow(), entry.relative_path),
        DiffStatus::Match => format!("{} {}", "=".dimmed(), entry.relative_path),
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p ftpdiff output::tests`
Expected: PASS (3 tests)

- [ ] **Step 6: Commit**

```bash
git add tools/ftpdiff/src/output.rs tools/ftpdiff/src/main.rs
git commit -m "ftpdiff: add colored text output formatting"
```

---

### Task 13: CSV report output

**Files:**
- Create: `tools/ftpdiff/src/csv_report.rs`
- Modify: `tools/ftpdiff/src/main.rs` (add `mod csv_report;`)

**Interfaces:**
- Consumes: `ftp_utils_core::DiffEntry` (Task 1/8)
- Produces: `pub fn write_csv(path: &std::path::Path, entries: &[DiffEntry]) -> std::io::Result<()>`

- [ ] **Step 1: Write the failing test**

Create `tools/ftpdiff/src/csv_report.rs`:

```rust
//! CSV report output for diff results.

use std::path::Path;

use ftp_utils_core::DiffEntry;

/// Writes `entries` to `path` as CSV with columns:
/// path,status,local_size,remote_size,local_md5,remote_md5
pub fn write_csv(path: &Path, entries: &[DiffEntry]) -> std::io::Result<()> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ftp_utils_core::DiffStatus;

    #[test]
    fn writes_header_and_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.csv");

        let entries = vec![DiffEntry {
            relative_path: "a.txt".to_string(),
            status: DiffStatus::SizeMismatch,
            local_size: Some(10),
            remote_size: Some(20),
            local_md5: None,
            remote_md5: None,
        }];

        write_csv(&path, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let mut lines = content.lines();
        assert_eq!(lines.next().unwrap(), "path,status,local_size,remote_size,local_md5,remote_md5");
        assert_eq!(lines.next().unwrap(), "a.txt,SizeMismatch,10,20,,");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ftpdiff csv_report::tests`
Expected: FAIL (`not yet implemented`, plus a compile error until `mod csv_report;` is wired)

- [ ] **Step 3: Wire the module**

Update `tools/ftpdiff/src/main.rs`:

```rust
mod cli;
mod config;
mod csv_report;
mod output;

fn main() {
    println!("ftpdiff: not yet implemented");
}
```

- [ ] **Step 4: Implement**

Replace the `todo!()` body:

```rust
pub fn write_csv(path: &Path, entries: &[DiffEntry]) -> std::io::Result<()> {
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record(["path", "status", "local_size", "remote_size", "local_md5", "remote_md5"])?;

    for entry in entries {
        writer.write_record([
            entry.relative_path.clone(),
            format!("{:?}", entry.status),
            entry.local_size.map(|v| v.to_string()).unwrap_or_default(),
            entry.remote_size.map(|v| v.to_string()).unwrap_or_default(),
            entry.local_md5.clone().unwrap_or_default(),
            entry.remote_md5.clone().unwrap_or_default(),
        ])?;
    }

    writer.flush()?;
    Ok(())
}
```

Note: `csv::Writer::from_path` and `write_record` return `csv::Result<_>`, which converts to `std::io::Result` via `?` because `csv::Error` implements `From<csv::Error> for std::io::Error`... if that conversion doesn't hold in the installed `csv` version, change the function signature to return `Result<(), csv::Error>` instead and propagate that type unchanged through `main.rs`'s error handling in Task 14 (display it the same way via `{e}`).

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p ftpdiff csv_report::tests`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add tools/ftpdiff/src/csv_report.rs tools/ftpdiff/src/main.rs
git commit -m "ftpdiff: add CSV report output"
```

---

### Task 14: Wire main.rs, exit codes, manual end-to-end verification

**Files:**
- Modify: `tools/ftpdiff/src/main.rs`
- Modify: `CHANGES.md`

**Interfaces:**
- Consumes: everything produced by Tasks 1-13: `cli::Cli`, `config::{JsonConfig, merge, load_json_config, read_password}`, `ftp_utils_core::{compare, CompareOptions, DiffStatus, ftp_client::SuppaFtpConnection}`, `output::{format_entry, summarize, format_summary}`, `csv_report::write_csv`

This task has no automated test (it drives a real network connection). It's
verified by a manual run against a local test FTP server.

- [ ] **Step 1: Implement**

Replace the full contents of `tools/ftpdiff/src/main.rs`:

```rust
//! ftpdiff: compare a remote FTP/FTPS directory tree against a local copy.
//!
//! See `docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md`
//! for the design this binary implements.

mod cli;
mod config;
mod csv_report;
mod output;

use clap::Parser;
use ftp_utils_core::ftp_client::SuppaFtpConnection;
use ftp_utils_core::{compare, CompareOptions, DiffStatus};

use cli::Cli;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let cli = Cli::parse();

    let json_config = match &cli.config {
        Some(path) => match config::load_json_config(path) {
            Ok(loaded) => loaded,
            Err(e) => {
                eprintln!("Error: {e}");
                return 2;
            }
        },
        None => config::JsonConfig::default(),
    };

    let effective = match config::merge(&cli, &json_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {e}");
            return 2;
        }
    };

    let password = match config::read_password() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: {e}");
            return 2;
        }
    };

    let mut connection = match SuppaFtpConnection::connect(
        &effective.host,
        effective.port,
        &effective.user,
        &password,
        effective.ftps,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: failed to connect to {}:{}: {e}", effective.host, effective.port);
            return 2;
        }
    };

    let options = CompareOptions {
        local_dir: effective.local_dir.clone(),
        remote_dir: effective.remote_dir.clone(),
        excludes: effective.exclude.clone(),
        hash: effective.hash,
    };

    let entries = match compare(&mut connection, &options) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("Error: comparison failed: {e}");
            return 2;
        }
    };

    connection.close();

    for entry in &entries {
        println!("{}", output::format_entry(entry));
    }
    let summary = output::summarize(&entries);
    println!("{}", output::format_summary(&summary));

    if let Some(csv_path) = &effective.csv {
        if let Err(e) = csv_report::write_csv(csv_path, &entries) {
            eprintln!("Error: failed to write CSV to {}: {e}", csv_path.display());
            return 2;
        }
    }

    if entries.iter().any(|e| e.status != DiffStatus::Match) {
        1
    } else {
        0
    }
}
```

- [ ] **Step 2: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS (all tests from Tasks 1-13)

- [ ] **Step 3: Build the release binary**

Run: `cargo build --workspace`
Expected: builds cleanly, no warnings about unused code (every module is now wired into `main.rs`)

- [ ] **Step 4: Manual end-to-end verification**

Start a local test FTP server (any of these work; pick whichever is already
available):

```bash
# Option A: Python's built-in test FTP server (if pyftpdlib is installed)
python3 -m pyftpdlib -p 2121 -w &

# Option B: any other local FTP server listening on 127.0.0.1:2121
# with anonymous or test credentials
```

Then, from the repo root:

```bash
mkdir -p /tmp/ftpdiff-manual-test
echo "hello" > /tmp/ftpdiff-manual-test/match.txt
echo "only local" > /tmp/ftpdiff-manual-test/local-only.txt

FTPDIFF_PASSWORD=anonymous cargo run -p ftpdiff -- \
  --host 127.0.0.1 --port 2121 --user anonymous \
  --remote-dir / --local-dir /tmp/ftpdiff-manual-test \
  --hash --csv /tmp/ftpdiff-manual-test/report.csv
```

Verify:
- Colored text output lists entries with correct `+`/`-`/`~`/`=` markers.
- The summary line's counts match what's printed above it.
- `/tmp/ftpdiff-manual-test/report.csv` exists and has a header row plus one
  row per entry.
- The exit code (`echo $?`) is `1` if any entry isn't `Match`, `0` otherwise.
- Running with `--ftps` against a test server that supports explicit FTPS
  connects successfully (skip this check if no FTPS test server is
  available; note that in the commit message instead of blocking on it).

- [ ] **Step 5: Update CHANGES.md**

Append to the `## Unreleased` section of `CHANGES.md`:

```markdown
- Wired ftpdiff's CLI, config loading, output formatting, and CSV report
  into `main.rs`. ftpdiff v1 is now feature-complete per the design spec:
  recursive local/remote diff by size, optional `--hash` (MD5, always via
  download since suppaftp doesn't expose remote hash commands), `--exclude`
  glob patterns, `--csv` report, FTP/FTPS support, `FTPDIFF_PASSWORD`-only
  credentials, and exit codes 0/1/2.
```

- [ ] **Step 6: Commit**

```bash
git add tools/ftpdiff/src/main.rs CHANGES.md
git commit -m "ftpdiff: wire CLI, config, comparison, and output into main"
```

---

## Self-Review Notes

- **Spec coverage:** workspace layout (Task 0, already scaffolded before this plan) - crate boundaries (Tasks 1-8) - CLI flags (Task 9) - JSON config + precedence (Task 10) - password via env var (Task 11) - text+CSV output (Tasks 12-13) - FTP/FTPS connection (Task 7) - exclude patterns (Task 2) - hash strategy with fallback (Tasks 6-7, with the documented suppaftp API limitation) - exit codes (Task 14). All spec sections are covered.
- **Placeholder scan:** no TBD/TODO remain outside of `todo!()` markers that are the intentional TDD red-step scaffolding, each immediately replaced within the same task.
- **Type consistency:** `DiffEntry`/`DiffStatus` (Task 1) are used unchanged through `compare.rs`, `hash.rs`, `lib.rs`, `output.rs`, and `csv_report.rs`. `FtpConnection`/`FtpConnectionError`/`RawRemoteEntry` (Task 4) are used unchanged in `ftp_client.rs` (Task 7) and `lib.rs` (Task 8). `Cli` (Task 9) and `JsonConfig`/`EffectiveConfig` (Task 10) field names match between `cli.rs`, `config.rs`, and `main.rs`.
