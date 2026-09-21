# ftpdiff CSV-backed Comparison Sources Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `--local-csv <path>` and `--remote-csv <path>` to `ftpdiff`, letting either side of the comparison come from a previously-written `ftpdiff --csv` report instead of a live filesystem scan / FTP connection, independently.

**Architecture:** `ftp-utils-core` gains a `csv_source` module that reads an existing CSV report and reconstructs one side's `LocalEntry`/`RemoteEntry` list plus a side-channel map of already-known MD5 hashes. `hash::apply_hash_comparison`'s signature becomes more permissive (`conn`/`local_root` become `Option`, plus the two known-MD5 maps), preferring a known hash over live access and gracefully leaving an entry unresolved (not erroring) when neither is available. `connection::merge_connection` (used as-is by `ftpops`) stays unchanged; a new `merge_connection_partial` is extracted from it (used internally, and directly by `ftpdiff` for its conditional requirements). `ftpdiff`'s `EffectiveConfig` gains `LocalSource`/`RemoteSource` enums describing where each side comes from, and `main.rs`'s orchestration branches on them instead of always calling the (now effectively legacy, still-present-for-compatibility) `ftp_utils_core::compare()` wrapper.

**Tech Stack:** Rust 2021, existing workspace dependencies only (`csv`, `md5`, `clap`, `serde`).

**Spec:** `docs/superpowers/specs/2026-09-21-ftpdiff-csv-source-design.md`. This plan amends one detail: the spec describes `--local-dir`+`--local-csv` (and the remote equivalents) given together as a hard error. During planning this was reconsidered - a shared `--config` JSON file legitimately containing both `local_dir` and other fields (reused across multiple invocations, some CSV-sourced, some not) would trip that error unnecessarily. **This plan implements precedence instead of an error**: the CSV flag, when given, is simply used and the corresponding live setting is ignored (not required, not validated, not touched) - no new error case. Task 8 updates the spec to reflect this.

## Global Constraints

- No behavior change for `ftpdiff` invocations that use neither `--local-csv` nor `--remote-csv` - every existing flag, error message, and exit code stays identical.
- No behavior change for `ftpops` or `ftp_utils_core::connection::merge_connection` at all - `ftpops` doesn't know this feature exists.
- `--local-csv`/`--remote-csv` are CLI-only, not JSON config fields (matches `ftpops`'s own `--csv` precedent).
- `--exclude` applies to CSV-sourced entries the same way it applies to live-scanned ones.
- With `--hash`, a side's already-known MD5 (recycled from its CSV source) is preferred over live computation; if unavailable on a side with no live fallback, that entry's hash stays unresolved (status stays `Match`, no error).

---

## File Structure

```
crates/ftp-utils-core/
└── src/
    ├── lib.rs           # add `pub mod csv_source;`
    ├── connection.rs      # add PartialConnection + merge_connection_partial
    ├── csv_source.rs       # NEW: read_local_entries, read_remote_entries
    └── hash.rs               # apply_hash_comparison: conn/local_root -> Option,
                                # + two known-md5 map params

tools/ftpdiff/
└── src/
    ├── cli.rs           # + local_csv, remote_csv fields
    ├── config.rs          # LocalSource, RemoteSource, restructured EffectiveConfig/merge()
    └── main.rs              # orchestration branches on LocalSource/RemoteSource
```

---

### Task 1: `connection::merge_connection_partial`

**Files:**
- Modify: `crates/ftp-utils-core/src/connection.rs`

**Interfaces:**
- Produces: `pub struct PartialConnection { pub host: Option<String>, pub port: u16, pub user: Option<String>, pub remote_dir: Option<String>, pub local_dir: Option<PathBuf>, pub ftps: bool, pub insecure_tls: bool }` (derives `Debug`, `Clone`), `pub fn merge_connection_partial(args: &ConnectionArgs, json: &ConnectionJsonConfig) -> PartialConnection`
- Modifies (internal refactor, same external signature/behavior): `merge_connection` now calls `merge_connection_partial` then applies required-field checks - existing callers (`ftpops`, `ftpdiff`'s live-only tests) see no difference.

- [ ] **Step 1: Write the failing test**

In `crates/ftp-utils-core/src/connection.rs`, add to the `tests` module (after the existing `default_port_is_21` test):

```rust
    #[test]
    fn partial_merge_never_errors_and_leaves_missing_fields_none() {
        let effective = merge_connection_partial(&empty_args(), &ConnectionJsonConfig::default());

        assert_eq!(effective.host, None);
        assert_eq!(effective.user, None);
        assert_eq!(effective.remote_dir, None);
        assert_eq!(effective.local_dir, None);
        assert_eq!(effective.port, 21);
        assert!(!effective.ftps);
        assert!(!effective.insecure_tls);
    }

    #[test]
    fn partial_merge_applies_cli_over_json_precedence() {
        let mut args = empty_args();
        args.host = Some("cli-host".to_string());

        let json = ConnectionJsonConfig { host: Some("json-host".to_string()), ..Default::default() };

        let effective = merge_connection_partial(&args, &json);

        assert_eq!(effective.host, Some("cli-host".to_string()));
    }

    #[test]
    fn partial_merge_or_merges_booleans() {
        let mut args = empty_args();
        args.ftps = true;

        let json = ConnectionJsonConfig { insecure_tls: Some(true), ..Default::default() };

        let effective = merge_connection_partial(&args, &json);

        assert!(effective.ftps);
        assert!(effective.insecure_tls);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ftp-utils-core connection::tests::partial_merge`
Expected: FAIL to compile (`merge_connection_partial`/`PartialConnection` don't exist yet)

- [ ] **Step 3: Implement, refactoring `merge_connection` to build on it**

In `crates/ftp-utils-core/src/connection.rs`, replace the existing `merge_connection` function:

```rust
pub fn merge_connection(
    args: &ConnectionArgs,
    json: &ConnectionJsonConfig,
) -> Result<EffectiveConnection, ConnectionError> {
    let host = args
        .host
        .clone()
        .or_else(|| json.host.clone())
        .ok_or_else(|| ConnectionError("missing required setting: host (use --host or config file)".to_string()))?;

    let user = args
        .user
        .clone()
        .or_else(|| json.user.clone())
        .ok_or_else(|| ConnectionError("missing required setting: user (use --user or config file)".to_string()))?;

    let remote_dir = args
        .remote_dir
        .clone()
        .or_else(|| json.remote_dir.clone())
        .ok_or_else(|| {
            ConnectionError("missing required setting: remote-dir (use --remote-dir or config file)".to_string())
        })?;

    let local_dir = args
        .local_dir
        .clone()
        .or_else(|| json.local_dir.clone())
        .ok_or_else(|| {
            ConnectionError("missing required setting: local-dir (use --local-dir or config file)".to_string())
        })?;

    Ok(EffectiveConnection {
        host,
        port: args.port.or(json.port).unwrap_or(21),
        user,
        remote_dir,
        local_dir,
        ftps: args.ftps || json.ftps.unwrap_or(false),
        insecure_tls: args.insecure_tls || json.insecure_tls.unwrap_or(false),
    })
}
```

with:

```rust
/// Resolved connection settings that don't require any field to be
/// present - `host`/`user`/`remote_dir`/`local_dir` stay `Option`.
/// `merge_connection` builds on this and additionally requires those
/// four fields; callers with conditional requirements (like ftpdiff's
/// `--local-csv`/`--remote-csv`) use this directly.
#[derive(Debug, Clone)]
pub struct PartialConnection {
    pub host: Option<String>,
    pub port: u16,
    pub user: Option<String>,
    pub remote_dir: Option<String>,
    pub local_dir: Option<PathBuf>,
    pub ftps: bool,
    pub insecure_tls: bool,
}

/// Merges CLI args, JSON config, and defaults, without requiring any
/// field to be present. Precedence: CLI > JSON > default. Boolean flags
/// merge as OR (CLI can enable, never disable, a JSON-set true).
pub fn merge_connection_partial(args: &ConnectionArgs, json: &ConnectionJsonConfig) -> PartialConnection {
    PartialConnection {
        host: args.host.clone().or_else(|| json.host.clone()),
        port: args.port.or(json.port).unwrap_or(21),
        user: args.user.clone().or_else(|| json.user.clone()),
        remote_dir: args.remote_dir.clone().or_else(|| json.remote_dir.clone()),
        local_dir: args.local_dir.clone().or_else(|| json.local_dir.clone()),
        ftps: args.ftps || json.ftps.unwrap_or(false),
        insecure_tls: args.insecure_tls || json.insecure_tls.unwrap_or(false),
    }
}

/// Merges CLI args, JSON config, and defaults into `EffectiveConnection`,
/// requiring `host`/`user`/`remote_dir`/`local_dir` to be resolvable from
/// one of the two sources.
pub fn merge_connection(
    args: &ConnectionArgs,
    json: &ConnectionJsonConfig,
) -> Result<EffectiveConnection, ConnectionError> {
    let partial = merge_connection_partial(args, json);

    let host = partial
        .host
        .ok_or_else(|| ConnectionError("missing required setting: host (use --host or config file)".to_string()))?;
    let user = partial
        .user
        .ok_or_else(|| ConnectionError("missing required setting: user (use --user or config file)".to_string()))?;
    let remote_dir = partial.remote_dir.ok_or_else(|| {
        ConnectionError("missing required setting: remote-dir (use --remote-dir or config file)".to_string())
    })?;
    let local_dir = partial.local_dir.ok_or_else(|| {
        ConnectionError("missing required setting: local-dir (use --local-dir or config file)".to_string())
    })?;

    Ok(EffectiveConnection {
        host,
        port: partial.port,
        user,
        remote_dir,
        local_dir,
        ftps: partial.ftps,
        insecure_tls: partial.insecure_tls,
    })
}
```

- [ ] **Step 4: Run tests to verify everything passes**

Run: `cargo test -p ftp-utils-core connection::tests`
Expected: PASS (12 tests: the existing 9 plus the 3 new ones; all pre-existing `merge_connection` tests still pass unchanged, since its external behavior is identical)

- [ ] **Step 5: Commit**

```bash
git add crates/ftp-utils-core/src/connection.rs
git commit -m "ftp-utils-core: extract merge_connection_partial from merge_connection"
```

---

### Task 2: `csv_source` module

**Files:**
- Create: `crates/ftp-utils-core/src/csv_source.rs`
- Modify: `crates/ftp-utils-core/src/lib.rs` (add `pub mod csv_source;`)
- Modify: `crates/ftp-utils-core/Cargo.toml` (add `csv`)

**Interfaces:**
- Produces: `pub struct CsvSourceError(pub String)` (`Display`, `std::error::Error`), `pub fn read_local_entries(path: &Path) -> Result<(Vec<local::LocalEntry>, HashMap<String, String>), CsvSourceError>`, `pub fn read_remote_entries(path: &Path) -> Result<(Vec<remote::RemoteEntry>, HashMap<String, String>), CsvSourceError>`

- [ ] **Step 1: Add the `csv` dependency**

In `crates/ftp-utils-core/Cargo.toml`, change `[dependencies]` to:

```toml
[dependencies]
suppaftp.workspace = true
glob.workspace = true
walkdir.workspace = true
md5.workspace = true
clap.workspace = true
serde.workspace = true
serde_json.workspace = true
rpassword.workspace = true
csv.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 2: Write the failing tests**

Create `crates/ftp-utils-core/src/csv_source.rs`:

```rust
//! Reads a previously-written `ftpdiff --csv` report as a substitute for
//! a live filesystem/FTP directory listing, for one side of a comparison
//! (`--local-csv` / `--remote-csv`).

use std::collections::HashMap;
use std::path::Path;

use crate::local::LocalEntry;
use crate::remote::RemoteEntry;

#[derive(Debug)]
pub struct CsvSourceError(pub String);

impl std::fmt::Display for CsvSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CsvSourceError {}

struct RawRow {
    relative_path: String,
    size: Option<u64>,
    md5: Option<String>,
}

fn read_side(path: &Path, size_column: &str, md5_column: &str) -> Result<Vec<RawRow>, CsvSourceError> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|e| CsvSourceError(format!("cannot read CSV {}: {e}", path.display())))?;

    let headers = reader.headers().map_err(|e| CsvSourceError(e.to_string()))?.clone();
    let path_idx = headers
        .iter()
        .position(|h| h == "path")
        .ok_or_else(|| CsvSourceError("CSV missing 'path' column".to_string()))?;
    let size_idx = headers
        .iter()
        .position(|h| h == size_column)
        .ok_or_else(|| CsvSourceError(format!("CSV missing '{size_column}' column")))?;
    let md5_idx = headers
        .iter()
        .position(|h| h == md5_column)
        .ok_or_else(|| CsvSourceError(format!("CSV missing '{md5_column}' column")))?;

    let mut rows = Vec::new();
    for result in reader.records() {
        let record = result.map_err(|e| CsvSourceError(format!("invalid CSV row: {e}")))?;

        let relative_path = record
            .get(path_idx)
            .ok_or_else(|| CsvSourceError("row missing 'path' value".to_string()))?
            .to_string();

        let size_str = record.get(size_idx).unwrap_or("");
        let size = if size_str.is_empty() {
            None
        } else {
            Some(size_str.parse::<u64>().map_err(|e| {
                CsvSourceError(format!("row for '{relative_path}' has invalid {size_column} '{size_str}': {e}"))
            })?)
        };

        let md5_str = record.get(md5_idx).unwrap_or("");
        let md5 = if md5_str.is_empty() { None } else { Some(md5_str.to_string()) };

        rows.push(RawRow { relative_path, size, md5 });
    }

    Ok(rows)
}

/// Reads `local_size`/`local_md5` for every row that has a `local_size`
/// value, from a CSV report written by `ftpdiff --csv`. Rows without a
/// `local_size` are skipped (they had no local file in the original
/// report).
pub fn read_local_entries(path: &Path) -> Result<(Vec<LocalEntry>, HashMap<String, String>), CsvSourceError> {
    let rows = read_side(path, "local_size", "local_md5")?;

    let mut entries = Vec::new();
    let mut known_md5 = HashMap::new();
    for row in rows {
        if let Some(size) = row.size {
            if let Some(md5) = row.md5 {
                known_md5.insert(row.relative_path.clone(), md5);
            }
            entries.push(LocalEntry { relative_path: row.relative_path, size });
        }
    }

    Ok((entries, known_md5))
}

/// Reads `remote_size`/`remote_md5` for every row that has a
/// `remote_size` value, from a CSV report written by `ftpdiff --csv`.
/// Rows without a `remote_size` are skipped (they had no remote file in
/// the original report).
pub fn read_remote_entries(path: &Path) -> Result<(Vec<RemoteEntry>, HashMap<String, String>), CsvSourceError> {
    let rows = read_side(path, "remote_size", "remote_md5")?;

    let mut entries = Vec::new();
    let mut known_md5 = HashMap::new();
    for row in rows {
        if let Some(size) = row.size {
            if let Some(md5) = row.md5 {
                known_md5.insert(row.relative_path.clone(), md5);
            }
            entries.push(RemoteEntry { relative_path: row.relative_path, size });
        }
    }

    Ok((entries, known_md5))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_csv(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.csv");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    const HEADER: &str = "path,status,local_size,remote_size,local_md5,remote_md5\n";

    #[test]
    fn reads_local_entries_and_skips_rows_without_local_size() {
        let (_dir, path) = write_csv(&format!(
            "{HEADER}\
             a.txt,Match,10,10,abc,abc\n\
             b.txt,RemoteOnly,,20,,def\n"
        ));

        let (entries, known_md5) = read_local_entries(&path).unwrap();

        assert_eq!(entries, vec![LocalEntry { relative_path: "a.txt".to_string(), size: 10 }]);
        assert_eq!(known_md5.get("a.txt"), Some(&"abc".to_string()));
        assert_eq!(known_md5.get("b.txt"), None);
    }

    #[test]
    fn reads_remote_entries_and_skips_rows_without_remote_size() {
        let (_dir, path) = write_csv(&format!(
            "{HEADER}\
             a.txt,Match,10,10,abc,abc\n\
             c.txt,LocalOnly,5,,ghi,\n"
        ));

        let (entries, known_md5) = read_remote_entries(&path).unwrap();

        assert_eq!(entries, vec![RemoteEntry { relative_path: "a.txt".to_string(), size: 10 }]);
        assert_eq!(known_md5.get("a.txt"), Some(&"abc".to_string()));
        assert_eq!(known_md5.get("c.txt"), None);
    }

    #[test]
    fn row_without_md5_is_not_added_to_known_map() {
        let (_dir, path) = write_csv(&format!("{HEADER}a.txt,SizeMismatch,10,20,,\n"));

        let (entries, known_md5) = read_local_entries(&path).unwrap();

        assert_eq!(entries, vec![LocalEntry { relative_path: "a.txt".to_string(), size: 10 }]);
        assert!(known_md5.is_empty());
    }

    #[test]
    fn errors_on_malformed_size_value() {
        let (_dir, path) = write_csv(&format!("{HEADER}a.txt,Match,not-a-number,10,,\n"));

        let result = read_local_entries(&path);

        assert!(result.is_err());
    }

    #[test]
    fn errors_when_required_column_missing() {
        let (_dir, path) = write_csv("path,status\na.txt,Match\n");

        let result = read_local_entries(&path);

        assert!(result.is_err());
    }

    #[test]
    fn errors_when_file_missing() {
        let result = read_local_entries(std::path::Path::new("/nonexistent/report.csv"));

        assert!(result.is_err());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p ftp-utils-core csv_source::tests`
Expected: FAIL to compile (module not wired in yet)

- [ ] **Step 4: Wire the module**

In `crates/ftp-utils-core/src/lib.rs`, add `pub mod csv_source;` alongside the other module declarations.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p ftp-utils-core csv_source::tests`
Expected: PASS (6 tests)

- [ ] **Step 6: Commit**

```bash
git add crates/ftp-utils-core/Cargo.toml crates/ftp-utils-core/src/csv_source.rs crates/ftp-utils-core/src/lib.rs
git commit -m "ftp-utils-core: add csv_source module for CSV-backed comparison sides"
```

---

### Task 3: `hash::apply_hash_comparison` - optional live sources, known-MD5 reuse

**Files:**
- Modify: `crates/ftp-utils-core/src/hash.rs`

**Interfaces:**
- Produces (changed signature): `pub fn apply_hash_comparison<C: FtpConnection>(conn: Option<&mut C>, remote_root: Option<&str>, local_root: Option<&Path>, local_known_md5: &HashMap<String, String>, remote_known_md5: &HashMap<String, String>, entries: &mut [DiffEntry], progress: Option<&mut (dyn FnMut(&str) + '_)>) -> std::io::Result<()>`

- [ ] **Step 1: Replace the implementation**

Replace the full contents of `crates/ftp-utils-core/src/hash.rs`:

```rust
//! Upgrades `Match` diff entries to a hash-verified `Match` or
//! `HashMismatch` by comparing MD5 hashes. For each side, prefers an
//! already-known hash (e.g. recycled from a `--local-csv`/`--remote-csv`
//! source) over live access; if a side has neither, that entry is left
//! at its size-only `Match` result rather than erroring.

use std::collections::HashMap;
use std::path::Path;

use crate::diff::{DiffEntry, DiffStatus};
use crate::remote::FtpConnection;

/// For every entry currently marked `Match`, resolves and compares each
/// side's MD5 hash, updating `status` to `Match` or `HashMismatch` (only
/// when both sides' hashes could be resolved) and filling in whichever of
/// `local_md5`/`remote_md5` were resolved. Entries with any other status
/// are untouched.
///
/// Each side's hash is resolved in this order: `local_known_md5`/
/// `remote_known_md5` first (an already-known hash, e.g. recycled from a
/// CSV source); then, if that side has a live source (`conn`+
/// `remote_root` for remote, `local_root` for local), fetched live
/// exactly as before. If neither is available for a side, that side's
/// hash stays unresolved and the entry's status is left unchanged.
///
/// Calls `progress` (if given) with a human-readable message for every
/// entry a hash resolution is attempted for, for `--verbose` output.
pub fn apply_hash_comparison<C: FtpConnection>(
    mut conn: Option<&mut C>,
    remote_root: Option<&str>,
    local_root: Option<&Path>,
    local_known_md5: &HashMap<String, String>,
    remote_known_md5: &HashMap<String, String>,
    entries: &mut [DiffEntry],
    mut progress: Option<&mut (dyn FnMut(&str) + '_)>,
) -> std::io::Result<()> {
    for entry in entries.iter_mut() {
        if entry.status != DiffStatus::Match {
            continue;
        }

        if let Some(cb) = progress.as_deref_mut() {
            cb(&format!("hash: {}", entry.relative_path));
        }

        let remote_md5 = if let Some(md5) = remote_known_md5.get(&entry.relative_path) {
            Some(md5.clone())
        } else if let (Some(conn), Some(remote_root)) = (conn.as_deref_mut(), remote_root) {
            let remote_path = format!("{remote_root}/{}", entry.relative_path);
            match conn.try_hash(&remote_path) {
                Some(hash) => Some(hash),
                None => {
                    let bytes = conn
                        .retr_to_buffer(&remote_path)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
                    Some(format!("{:x}", md5::compute(&bytes)))
                }
            }
        } else {
            None
        };

        let local_md5 = if let Some(md5) = local_known_md5.get(&entry.relative_path) {
            Some(md5.clone())
        } else if let Some(local_root) = local_root {
            let local_path = local_root.join(&entry.relative_path);
            let bytes = std::fs::read(&local_path)?;
            Some(format!("{:x}", md5::compute(&bytes)))
        } else {
            None
        };

        if let (Some(local_md5), Some(remote_md5)) = (&local_md5, &remote_md5) {
            entry.status = if local_md5 == remote_md5 { DiffStatus::Match } else { DiffStatus::HashMismatch };
        }
        entry.local_md5 = local_md5;
        entry.remote_md5 = remote_md5;
    }

    Ok(())
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

        fn store_from_buffer(&mut self, _path: &str, _data: &[u8]) -> Result<(), FtpConnectionError> {
            Ok(())
        }

        fn delete(&mut self, _path: &str) -> Result<(), FtpConnectionError> {
            Ok(())
        }

        fn create_dir(&mut self, _path: &str) -> Result<(), FtpConnectionError> {
            Ok(())
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

        apply_hash_comparison(
            Some(&mut conn),
            Some("/remote"),
            Some(dir.path()),
            &HashMap::new(),
            &HashMap::new(),
            &mut entries,
            None,
        )
        .unwrap();

        assert_eq!(entries[0].status, DiffStatus::Match);
        assert_eq!(entries[0].remote_md5, Some(expected_hash));
    }

    #[test]
    fn falls_back_to_download_when_hash_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();

        let mut entries = vec![entry("f.txt")];
        let mut conn = MockConnection { hash: None, remote_bytes: b"different".to_vec() };

        apply_hash_comparison(
            Some(&mut conn),
            Some("/remote"),
            Some(dir.path()),
            &HashMap::new(),
            &HashMap::new(),
            &mut entries,
            None,
        )
        .unwrap();

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

        apply_hash_comparison(
            Some(&mut conn),
            Some("/remote"),
            Some(dir.path()),
            &HashMap::new(),
            &HashMap::new(),
            &mut entries,
            None,
        )
        .unwrap();

        assert_eq!(entries[0].status, DiffStatus::LocalOnly);
        assert_eq!(entries[0].remote_md5, None);
    }

    #[test]
    fn reports_progress_only_for_hashed_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();

        let mut entries = vec![
            entry("f.txt"),
            DiffEntry {
                relative_path: "only-local.txt".to_string(),
                status: DiffStatus::LocalOnly,
                local_size: Some(5),
                remote_size: None,
                local_md5: None,
                remote_md5: None,
            },
        ];
        let mut conn = MockConnection { hash: None, remote_bytes: b"hello".to_vec() };

        let mut messages = Vec::new();
        let mut progress = |msg: &str| messages.push(msg.to_string());

        apply_hash_comparison(
            Some(&mut conn),
            Some("/remote"),
            Some(dir.path()),
            &HashMap::new(),
            &HashMap::new(),
            &mut entries,
            Some(&mut progress),
        )
        .unwrap();

        assert_eq!(messages, vec!["hash: f.txt".to_string()]);
    }

    #[test]
    fn known_md5_short_circuits_live_access() {
        // No live connection or local file backing this entry at all -
        // both known-md5 maps must be consulted first.
        let mut entries = vec![entry("f.txt")];
        let mut local_known = HashMap::new();
        local_known.insert("f.txt".to_string(), "same-hash".to_string());
        let mut remote_known = HashMap::new();
        remote_known.insert("f.txt".to_string(), "same-hash".to_string());

        apply_hash_comparison::<MockConnection>(None, None, None, &local_known, &remote_known, &mut entries, None)
            .unwrap();

        assert_eq!(entries[0].status, DiffStatus::Match);
        assert_eq!(entries[0].local_md5, Some("same-hash".to_string()));
        assert_eq!(entries[0].remote_md5, Some("same-hash".to_string()));
    }

    #[test]
    fn leaves_entry_at_match_when_hash_unresolvable_on_either_side() {
        let mut entries = vec![entry("f.txt")];

        apply_hash_comparison::<MockConnection>(
            None,
            None,
            None,
            &HashMap::new(),
            &HashMap::new(),
            &mut entries,
            None,
        )
        .unwrap();

        assert_eq!(entries[0].status, DiffStatus::Match);
        assert_eq!(entries[0].local_md5, None);
        assert_eq!(entries[0].remote_md5, None);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p ftp-utils-core hash::tests`
Expected: PASS (6 tests) - this is a full-file replacement rather than a red/green cycle, since the signature change and the tests that exercise it must land together for the file to compile at all

- [ ] **Step 3: Commit**

```bash
git add crates/ftp-utils-core/src/hash.rs
git commit -m "ftp-utils-core: make apply_hash_comparison's live sources optional, reuse known MD5s"
```

---

### Task 4: Adapt `lib.rs`'s `compare()` to the new `apply_hash_comparison` signature

**Files:**
- Modify: `crates/ftp-utils-core/src/lib.rs`

**Interfaces:**
- `compare()`'s own public signature and behavior are unchanged; only its internal call to `apply_hash_comparison` adapts.

- [ ] **Step 1: Update the internal call**

In `crates/ftp-utils-core/src/lib.rs`, replace:

```rust
    if opts.hash {
        hash::apply_hash_comparison(conn, &opts.remote_dir, &opts.local_dir, &mut entries, progress.as_deref_mut())?;
    }
```

with:

```rust
    if opts.hash {
        hash::apply_hash_comparison(
            Some(conn),
            Some(opts.remote_dir.as_str()),
            Some(opts.local_dir.as_path()),
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &mut entries,
            progress.as_deref_mut(),
        )?;
    }
```

- [ ] **Step 2: Run the full core test suite**

Run: `cargo test -p ftp-utils-core`
Expected: PASS - `compare()`'s own tests (`compares_local_and_remote_end_to_end`,
`reports_progress_across_local_and_remote_walks`) are unaffected since neither
uses `--hash`; this step is verification that the adaptation compiles and
nothing else regressed, alongside every test from Tasks 1-3

- [ ] **Step 3: Commit**

```bash
git add crates/ftp-utils-core/src/lib.rs
git commit -m "ftp-utils-core: adapt compare()'s internal hash call to the new signature"
```

---

### Task 5: ftpdiff `cli.rs` - `--local-csv` / `--remote-csv` flags

**Files:**
- Modify: `tools/ftpdiff/src/cli.rs`

**Interfaces:**
- Produces (added to `Cli`): `pub local_csv: Option<PathBuf>`, `pub remote_csv: Option<PathBuf>`

- [ ] **Step 1: Write the failing test**

In `tools/ftpdiff/src/cli.rs`, add the two fields to `Cli` (after `verbose`):

```rust
    /// Print progress diagnostics (connecting, directory walk counts,
    /// hashing) to stderr as the comparison runs.
    #[arg(long)]
    pub verbose: bool,

    /// Read the local side from a previous `ftpdiff --csv` report
    /// instead of scanning --local-dir.
    #[arg(long = "local-csv")]
    pub local_csv: Option<PathBuf>,

    /// Read the remote side from a previous `ftpdiff --csv` report
    /// instead of connecting to the FTP server.
    #[arg(long = "remote-csv")]
    pub remote_csv: Option<PathBuf>,
}
```

(the closing `}` above replaces the previous one that ended the struct right
after `verbose`)

Then add tests at the end of the `tests` module:

```rust
    #[test]
    fn parses_local_csv_flag() {
        let cli = Cli::parse_from(["ftpdiff", "--local-csv", "snapshot.csv"]);

        assert_eq!(cli.local_csv, Some(PathBuf::from("snapshot.csv")));
    }

    #[test]
    fn parses_remote_csv_flag() {
        let cli = Cli::parse_from(["ftpdiff", "--remote-csv", "snapshot.csv"]);

        assert_eq!(cli.remote_csv, Some(PathBuf::from("snapshot.csv")));
    }

    #[test]
    fn csv_source_flags_default_to_none() {
        let cli = Cli::parse_from(["ftpdiff"]);

        assert_eq!(cli.local_csv, None);
        assert_eq!(cli.remote_csv, None);
    }
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p ftpdiff cli::tests`
Expected: PASS (11 tests: 8 existing + 3 new) - no red phase, this is additive
struct fields plus their parser tests, nothing to break first

- [ ] **Step 3: Commit**

```bash
git add tools/ftpdiff/src/cli.rs
git commit -m "ftpdiff: add --local-csv and --remote-csv flags"
```

---

### Task 6: ftpdiff `config.rs` - `LocalSource`/`RemoteSource`, conditional requirements

**Files:**
- Modify: `tools/ftpdiff/src/config.rs`

**Interfaces:**
- Consumes: `ftp_utils_core::connection::merge_connection_partial` (Task 1), `crate::cli::Cli` (Task 5)
- Produces: `pub enum LocalSource { Live(PathBuf), Csv(PathBuf) }` (derives `Debug`, `Clone`), `pub enum RemoteSource { Live { host: String, port: u16, user: String, remote_dir: String, ftps: bool, insecure_tls: bool }, Csv(PathBuf) }` (derives `Debug`, `Clone`), `pub struct EffectiveConfig { pub local: LocalSource, pub remote: RemoteSource, pub hash: bool, pub verbose: bool, pub exclude: Vec<String>, pub csv: Option<PathBuf> }`

- [ ] **Step 1: Replace the file**

Replace the full contents of `tools/ftpdiff/src/config.rs`:

```rust
//! JSON config loading and CLI/JSON/default merge logic for ftpdiff's own
//! fields (hash, verbose, exclude, local-csv, remote-csv). Connection
//! fields (host, port, user, password, local/remote dir, ftps,
//! insecure-tls) are handled by `ftp_utils_core::connection`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use ftp_utils_core::connection::{self, ConnectionJsonConfig};

use crate::cli::Cli;

pub use ftp_utils_core::connection::ConnectionError as ConfigError;

#[derive(Debug, Deserialize, Default)]
pub struct JsonConfig {
    #[serde(flatten)]
    pub connection: ConnectionJsonConfig,
    pub hash: Option<bool>,
    pub verbose: Option<bool>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// Where the local side of the comparison comes from.
#[derive(Debug, Clone)]
pub enum LocalSource {
    Live(PathBuf),
    Csv(PathBuf),
}

/// Where the remote side of the comparison comes from.
#[derive(Debug, Clone)]
pub enum RemoteSource {
    Live { host: String, port: u16, user: String, remote_dir: String, ftps: bool, insecure_tls: bool },
    Csv(PathBuf),
}

#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub local: LocalSource,
    pub remote: RemoteSource,
    pub hash: bool,
    pub verbose: bool,
    pub exclude: Vec<String>,
    pub csv: Option<PathBuf>,
}

pub fn load_json_config(path: &Path) -> Result<JsonConfig, ConfigError> {
    connection::load_json_config(path)
}

/// Resolves `LocalSource`/`RemoteSource` and ftpdiff's own fields.
/// `--local-csv` makes `--local-dir` unnecessary; `--remote-csv` makes
/// `--host`/`--user`/`--remote-dir` (and password/FTPS settings)
/// unnecessary. If both a CSV flag and its live counterpart are given,
/// the CSV flag takes precedence - the live setting is simply unused,
/// not an error (so a shared `--config` file can supply connection
/// defaults used by other invocations without breaking a CSV-sourced
/// one).
pub fn merge(cli: &Cli, json: &JsonConfig) -> Result<EffectiveConfig, ConfigError> {
    let partial = connection::merge_connection_partial(&cli.connection, &json.connection);

    let local = if let Some(path) = &cli.local_csv {
        LocalSource::Csv(path.clone())
    } else {
        let local_dir = partial.local_dir.clone().ok_or_else(|| {
            ConfigError(
                "missing required setting: local-dir (use --local-dir, --local-csv, or config file)".to_string(),
            )
        })?;
        LocalSource::Live(local_dir)
    };

    let remote = if let Some(path) = &cli.remote_csv {
        RemoteSource::Csv(path.clone())
    } else {
        let host = partial.host.clone().ok_or_else(|| {
            ConfigError("missing required setting: host (use --host, config file, or --remote-csv)".to_string())
        })?;
        let user = partial.user.clone().ok_or_else(|| {
            ConfigError("missing required setting: user (use --user, config file, or --remote-csv)".to_string())
        })?;
        let remote_dir = partial.remote_dir.clone().ok_or_else(|| {
            ConfigError(
                "missing required setting: remote-dir (use --remote-dir, config file, or --remote-csv)".to_string(),
            )
        })?;
        RemoteSource::Live {
            host,
            port: partial.port,
            user,
            remote_dir,
            ftps: partial.ftps,
            insecure_tls: partial.insecure_tls,
        }
    };

    let mut exclude = json.exclude.clone();
    exclude.extend(cli.exclude.clone());

    Ok(EffectiveConfig {
        local,
        remote,
        hash: cli.hash || json.hash.unwrap_or(false),
        verbose: cli.verbose || json.verbose.unwrap_or(false),
        exclude,
        csv: cli.csv.clone(),
    })
}

/// Resolves the FTP password using the `FTPDIFF_PASSWORD` environment
/// variable as ftpdiff's fallback source.
pub fn read_password(cli_password: Option<&str>) -> Result<String, ConfigError> {
    connection::read_password(cli_password, "FTPDIFF_PASSWORD")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_cli() -> Cli {
        Cli {
            connection: ftp_utils_core::connection::ConnectionArgs {
                config: None,
                host: None,
                port: None,
                user: None,
                remote_dir: None,
                local_dir: None,
                ftps: false,
                insecure_tls: false,
                password: None,
            },
            hash: false,
            exclude: Vec::new(),
            csv: None,
            verbose: false,
            local_csv: None,
            remote_csv: None,
        }
    }

    fn live_cli() -> Cli {
        let mut cli = empty_cli();
        cli.connection.host = Some("h".into());
        cli.connection.user = Some("u".into());
        cli.connection.remote_dir = Some("/r".into());
        cli.connection.local_dir = Some(PathBuf::from("./l"));
        cli
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

        assert_eq!(config.connection.host.as_deref(), Some("ftp.example.com"));
        assert_eq!(config.connection.user.as_deref(), Some("bob"));
        assert_eq!(config.exclude, vec!["*.tmp".to_string()]);
    }

    #[test]
    fn fully_live_resolves_both_sides() {
        let cli = live_cli();

        let effective = merge(&cli, &JsonConfig::default()).unwrap();

        assert!(matches!(effective.local, LocalSource::Live(ref p) if p == &PathBuf::from("./l")));
        assert!(matches!(effective.remote, RemoteSource::Live { ref host, .. } if host == "h"));
    }

    #[test]
    fn local_csv_makes_local_dir_unnecessary() {
        let mut cli = empty_cli();
        cli.connection.host = Some("h".into());
        cli.connection.user = Some("u".into());
        cli.connection.remote_dir = Some("/r".into());
        cli.local_csv = Some(PathBuf::from("snapshot.csv"));

        let effective = merge(&cli, &JsonConfig::default()).unwrap();

        assert!(matches!(effective.local, LocalSource::Csv(ref p) if p == &PathBuf::from("snapshot.csv")));
    }

    #[test]
    fn remote_csv_makes_connection_fields_unnecessary() {
        let mut cli = empty_cli();
        cli.connection.local_dir = Some(PathBuf::from("./l"));
        cli.remote_csv = Some(PathBuf::from("snapshot.csv"));

        let effective = merge(&cli, &JsonConfig::default()).unwrap();

        assert!(matches!(effective.remote, RemoteSource::Csv(ref p) if p == &PathBuf::from("snapshot.csv")));
    }

    #[test]
    fn both_csv_sources_need_nothing_else() {
        let mut cli = empty_cli();
        cli.local_csv = Some(PathBuf::from("local.csv"));
        cli.remote_csv = Some(PathBuf::from("remote.csv"));

        assert!(merge(&cli, &JsonConfig::default()).is_ok());
    }

    #[test]
    fn local_csv_takes_precedence_over_local_dir_when_both_given() {
        let mut cli = live_cli();
        cli.local_csv = Some(PathBuf::from("snapshot.csv"));

        let effective = merge(&cli, &JsonConfig::default()).unwrap();

        assert!(matches!(effective.local, LocalSource::Csv(ref p) if p == &PathBuf::from("snapshot.csv")));
    }

    #[test]
    fn errors_when_local_dir_missing_and_no_local_csv() {
        let mut cli = empty_cli();
        cli.remote_csv = Some(PathBuf::from("remote.csv"));

        assert!(merge(&cli, &JsonConfig::default()).is_err());
    }

    #[test]
    fn errors_when_host_missing_and_no_remote_csv() {
        let mut cli = empty_cli();
        cli.local_csv = Some(PathBuf::from("local.csv"));

        assert!(merge(&cli, &JsonConfig::default()).is_err());
    }

    #[test]
    fn hash_and_verbose_merge_as_or() {
        let mut cli = live_cli();
        cli.hash = true;

        let json = JsonConfig { verbose: Some(true), ..Default::default() };

        let effective = merge(&cli, &json).unwrap();

        assert!(effective.hash); // from CLI
        assert!(effective.verbose); // from JSON
    }

    #[test]
    fn exclude_patterns_are_unioned() {
        let mut cli = live_cli();
        cli.exclude = vec!["*.tmp".to_string()];

        let json = JsonConfig { exclude: vec![".git/*".to_string()], ..Default::default() };

        let effective = merge(&cli, &json).unwrap();

        assert_eq!(effective.exclude, vec![".git/*".to_string(), "*.tmp".to_string()]);
    }

    #[test]
    fn read_password_uses_ftpdiff_env_var() {
        use std::sync::Mutex;
        static ENV_GUARD: Mutex<()> = Mutex::new(());
        let _guard = ENV_GUARD.lock().unwrap();

        std::env::set_var("FTPDIFF_PASSWORD", "from-env");
        let result = read_password(None);
        std::env::remove_var("FTPDIFF_PASSWORD");

        assert_eq!(result.unwrap(), "from-env");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p ftpdiff config::tests`
Expected: PASS (12 tests) - full-file replacement (the old flat
`EffectiveConfig`/`EffectiveConnection` shape no longer exists, so this
can't be a red/green cycle on the old file; the new tests are the
correctness check)

- [ ] **Step 3: Commit**

```bash
git add tools/ftpdiff/src/config.rs
git commit -m "ftpdiff: add LocalSource/RemoteSource with conditional connection requirements"
```

---

### Task 7: ftpdiff `main.rs` - orchestration

**Files:**
- Modify: `tools/ftpdiff/src/main.rs`

**Interfaces:**
- Consumes: everything from Tasks 1-6

This task has no new automated test (it's wiring, same pattern as
`ftpops`'s `main.rs`). Verified by the full workspace test suite plus a
manual smoke test covering all four `Live`/`Csv` combinations.

- [ ] **Step 1: Replace the file**

Replace the full contents of `tools/ftpdiff/src/main.rs`:

```rust
//! ftpdiff: compare a remote FTP/FTPS directory tree against a local copy.
//!
//! See `docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md`
//! and `docs/superpowers/specs/2026-09-21-ftpdiff-csv-source-design.md`
//! for the design this binary implements.

mod cli;
mod config;
mod csv_report;
mod output;

use std::collections::HashMap;

use clap::Parser;
use ftp_utils_core::compare::compare_entries;
use ftp_utils_core::ftp_client::SuppaFtpConnection;
use ftp_utils_core::{csv_source, exclude, hash, local, remote, DiffStatus};

use cli::Cli;
use config::{LocalSource, RemoteSource};

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let cli = Cli::parse();

    let json_config = match &cli.connection.config {
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

    let mut progress: Option<Box<dyn FnMut(&str)>> = if effective.verbose {
        Some(Box::new(|msg: &str| eprintln!("Checking {msg}")))
    } else {
        None
    };

    let (local_entries, local_known_md5) = match &effective.local {
        LocalSource::Live(dir) => match local::walk_local_dir(dir, &effective.exclude, progress.as_deref_mut()) {
            Ok(entries) => (entries, HashMap::new()),
            Err(e) => {
                eprintln!("Error: {e}");
                return 2;
            }
        },
        LocalSource::Csv(path) => {
            let (entries, known_md5) = match csv_source::read_local_entries(path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return 2;
                }
            };
            if effective.verbose {
                eprintln!("Loaded {} local entries from {}.", entries.len(), path.display());
            }
            let filtered = entries
                .into_iter()
                .filter(|e| !exclude::is_excluded(&e.relative_path, &effective.exclude))
                .collect();
            (filtered, known_md5)
        }
    };

    let mut connection: Option<SuppaFtpConnection> = None;

    let (remote_entries, remote_known_md5) = match &effective.remote {
        RemoteSource::Live { host, port, user, remote_dir, ftps, insecure_tls } => {
            let password = match config::read_password(cli.connection.password.as_deref()) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return 2;
                }
            };

            if effective.verbose {
                eprintln!("Connecting to {host}:{port} as {user} ({})...", if *ftps { "FTPS" } else { "FTP" });
            }

            let mut conn = match SuppaFtpConnection::connect(host, *port, user, &password, *ftps, *insecure_tls) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error: failed to connect to {host}:{port}: {e}");
                    return 2;
                }
            };

            let entries = match remote::walk_remote(&mut conn, remote_dir, &effective.exclude, progress.as_deref_mut()) {
                Ok(entries) => entries,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return 2;
                }
            };

            connection = Some(conn);
            (entries, HashMap::new())
        }
        RemoteSource::Csv(path) => {
            let (entries, known_md5) = match csv_source::read_remote_entries(path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return 2;
                }
            };
            if effective.verbose {
                eprintln!("Loaded {} remote entries from {}.", entries.len(), path.display());
            }
            let filtered = entries
                .into_iter()
                .filter(|e| !exclude::is_excluded(&e.relative_path, &effective.exclude))
                .collect();
            (filtered, known_md5)
        }
    };

    let mut entries = compare_entries(&local_entries, &remote_entries);

    if effective.hash {
        let (conn_opt, remote_root_opt) = match (&mut connection, &effective.remote) {
            (Some(conn), RemoteSource::Live { remote_dir, .. }) => (Some(conn), Some(remote_dir.as_str())),
            _ => (None, None),
        };
        let local_root_opt = match &effective.local {
            LocalSource::Live(dir) => Some(dir.as_path()),
            LocalSource::Csv(_) => None,
        };

        if let Err(e) = hash::apply_hash_comparison(
            conn_opt,
            remote_root_opt,
            local_root_opt,
            &local_known_md5,
            &remote_known_md5,
            &mut entries,
            progress.as_deref_mut(),
        ) {
            eprintln!("Error: hash comparison failed: {e}");
            return 2;
        }
    }

    if let Some(conn) = connection {
        conn.close();
    }

    if effective.verbose {
        eprintln!("Comparison done: {} entries.", entries.len());
    }

    for entry in &entries {
        println!("{}", output::format_entry(entry));
    }
    let summary = output::summarize(&entries);
    println!("{}", output::format_summary(&summary));

    if let Some(csv_path) = &effective.csv {
        if effective.verbose {
            eprintln!("Writing CSV report to {}...", csv_path.display());
        }
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
Expected: PASS - every test from Tasks 1-6, unchanged behavior for the
fully-live case

- [ ] **Step 3: Build and manually smoke-test all four combinations**

Run: `cargo build --workspace`
Expected: builds cleanly, no warnings (check specifically for an unused
`ftp_utils_core::CompareOptions`/`compare` import warning - there should
be none, since `main.rs` no longer imports the top-level `compare()`
wrapper)

Prepare a local CSV report and a local directory for the smoke test:

```sh
mkdir -p /tmp/ftpdiff-csv-smoke
printf 'path,status,local_size,remote_size,local_md5,remote_md5\na.txt,Match,5,5,%s,%s\nb.txt,RemoteOnly,,3,,\n' \
  "$(printf hello | md5sum | cut -d' ' -f1)" "$(printf hello | md5sum | cut -d' ' -f1)" \
  > /tmp/ftpdiff-csv-smoke/snapshot.csv
mkdir -p /tmp/ftpdiff-csv-smoke/local
printf hello > /tmp/ftpdiff-csv-smoke/local/a.txt
```

Run (local live, remote CSV - no FTP connection, no password prompt):
```sh
cargo run -p ftpdiff -- --local-dir /tmp/ftpdiff-csv-smoke/local --remote-csv /tmp/ftpdiff-csv-smoke/snapshot.csv --hash
```
Expected: reports `a.txt` as `Match` (hash reused from the CSV, `local_root`
still live so its own MD5 is computed fresh and compared against the
recycled remote one) and `b.txt` as `RemoteOnly`; exits with code `1`; no
`FTPDIFF_PASSWORD`/connection attempted

Run (both CSV - use the same file for both, fully offline):
```sh
cargo run -p ftpdiff -- --local-csv /tmp/ftpdiff-csv-smoke/snapshot.csv --remote-csv /tmp/ftpdiff-csv-smoke/snapshot.csv --hash --verbose
```
Expected: verbose output shows `Loaded 1 local entries from ...` and
`Loaded 2 remote entries from ...` (local-role rows only include `a.txt`,
remote-role rows include `a.txt` and `b.txt`); no connection attempted;
`a.txt` matches (known MD5s equal since same file used for both), `b.txt`
appears `RemoteOnly`

Run (missing required setting - `--remote-csv` not given, no `--host`):
```sh
cargo run -p ftpdiff -- --local-dir /tmp/ftpdiff-csv-smoke/local
```
Expected: `Error: missing required setting: host (use --host, config file, or --remote-csv)`, exit code `2`

- [ ] **Step 4: Clean up the smoke-test files**

Remove `/tmp/ftpdiff-csv-smoke` (or leave it - it's outside the repo and
harmless either way).

- [ ] **Step 5: Commit**

```bash
git add tools/ftpdiff/src/main.rs
git commit -m "ftpdiff: orchestrate comparison from LocalSource/RemoteSource"
```

---

### Task 8: Spec amendment, CHANGES.md, final verification

**Files:**
- Modify: `docs/superpowers/specs/2026-09-21-ftpdiff-csv-source-design.md`
- Modify: `CHANGES.md`
- Modify: `docs/ftpdiff.md` (only if the manual smoke test in Task 7 revealed wording that needs correcting; otherwise no change - the examples were already added)

- [ ] **Step 1: Amend the spec's CLI section**

In `docs/superpowers/specs/2026-09-21-ftpdiff-csv-source-design.md`, find
the paragraph starting `**Mutually exclusive with their live
counterpart:**` and replace it with:

```markdown
**Precedence, not an error, when both are given (revised during
implementation, 2026-09-21):** the original design called passing both
`--local-csv` and `--local-dir` (or the remote equivalents) together a
hard error. This was reconsidered while implementing: a shared
`--config` JSON file legitimately supplies `local_dir`/`host`/etc. for
multiple invocations, some of which may be CSV-sourced - erroring on
their mere presence would break that reuse. Instead, the CSV flag simply
takes precedence: the corresponding live setting, if also present, is
unused (not validated, not required).
```

- [ ] **Step 2: Run the full workspace test suite one more time**

Run: `cargo test --workspace && cargo build --workspace`
Expected: PASS, no warnings

- [ ] **Step 3: Update CHANGES.md**

Append to the `## Unreleased` section of `CHANGES.md`:

```markdown
- Added `--local-csv <path>` and `--remote-csv <path>` to ftpdiff: either
  side of the comparison can now be read from a previous `ftpdiff --csv`
  report instead of a live filesystem scan / FTP connection,
  independently. `--local-dir`/`--host`+`--user`+`--remote-dir` become
  unnecessary for the corresponding side (and, for `--remote-csv`, no
  password is resolved and no connection is made at all). With `--hash`,
  an MD5 already recorded in the source CSV is reused instead of being
  recomputed; if unavailable and that side has no live fallback, the
  entry is left at its size-only `Match` result rather than erroring.
  Required extending `ftp-utils-core`'s `apply_hash_comparison` to make
  its live connection/local-directory parameters optional and accept two
  known-MD5 maps, and extracting `connection::merge_connection_partial`
  (used only by ftpdiff; `ftpops` and `merge_connection` itself are
  unaffected). Manually smoke-tested all four Live/Csv combinations for
  local/remote plus the missing-setting error path.
```

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-09-21-ftpdiff-csv-source-design.md CHANGES.md
git commit -m "Amend ftpdiff CSV-source spec (precedence not error); update CHANGES"
```

---

## Self-Review Notes

- **Spec coverage:** shared `merge_connection_partial` extraction (Task 1) - `csv_source` module (Task 2) - `apply_hash_comparison` optionality and known-MD5 reuse (Task 3) - `compare()` internal adaptation, unchanged public behavior (Task 4) - CLI flags (Task 5) - `LocalSource`/`RemoteSource` and conditional requirements (Task 6) - orchestration (Task 7) - spec amendment for the precedence-not-error decision (Task 8). All sections of the design spec are covered, with the one deliberate, documented deviation (precedence instead of a hard error for the "both given" case).
- **Placeholder scan:** no `todo!()`/TBD/TODO remain outside plan-writing scaffolding; every task in this plan ships complete code.
- **Type consistency:** `LocalEntry`/`RemoteEntry` (unchanged, from Task 2 onward) flow unmodified from `csv_source::read_local_entries`/`read_remote_entries` and `local::walk_local_dir`/`remote::walk_remote` into `compare_entries` exactly as before. `PartialConnection` (Task 1) fields match `ConnectionArgs`/`ConnectionJsonConfig`'s existing field names/types. `LocalSource`/`RemoteSource` (Task 6) are consumed with identical variant shapes in `main.rs` (Task 7) - `RemoteSource::Live`'s five named fields match between definition and every match arm that destructures it.
