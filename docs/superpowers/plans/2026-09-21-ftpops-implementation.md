# ftpops Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `ftpops`, a second CLI tool in the `ftp-utils` monorepo that reads an `ftpdiff --csv` report and performs bulk `copy`/`delete` operations, filtered by diff status (`RemoteOnly`/`LocalOnly`).

**Architecture:** `tools/ftpops` is a `clap` binary with two subcommands (`copy`, `delete`), each flattening the shared `ftp_utils_core::connection::ConnectionArgs`. A small `validate` module rejects nonsensical `--filter`/`--to`/`--on` combinations before anything runs. A `csv_input` module reads and filters the CSV report. An `ops` module performs the actual file operations against the `FtpConnection` trait (generic, mock-testable, same pattern as `ftp-utils-core`'s `hash.rs`). `--dry-run` short-circuits before any connection or operation happens - it only reads the CSV and prints what would happen.

**Tech Stack:** Rust 2021, `clap` (derive, `Subcommand`, `ValueEnum`, `Args` flattening), `csv` (`Reader`), `colored`, reuses `ftp-utils-core`'s `connection` module and extended `FtpConnection` trait (`store_from_buffer`, `delete`, `create_dir`, `ensure_remote_dir`) from the prior refactor plan.

**Spec:** `docs/superpowers/specs/2026-09-21-ftpops-design.md` (section 3, `tools/ftpops`). Sections 1 and 2 of that spec were implemented by `docs/superpowers/plans/2026-09-21-shared-connection-refactor.md`, already complete.

## Global Constraints

- Password precedence: `--password` > `FTPOPS_PASSWORD` environment variable > interactive hidden-input prompt (spec "Password" section) - note the env var name is `FTPOPS_PASSWORD`, not `FTPDIFF_PASSWORD`.
- `--filter` values are `remote-only` | `local-only` (clap's default kebab-case `ValueEnum` casing of `RemoteOnly`/`LocalOnly` - no explicit renaming needed), mapped to the exact CSV `status` strings `"RemoteOnly"`/`"LocalOnly"` that `ftpdiff --csv` already writes (`format!("{:?}", DiffStatus)`).
- Combination validation table (spec section 3): `copy --to local` requires `--filter remote-only`; `copy --to remote` requires `--filter local-only`; `delete --on remote` requires `--filter remote-only`; `delete --on local` requires `--filter local-only`. Any other combination is a hard error, exit code 2, before the CSV is even read.
- `--dry-run` (both subcommands): prints `Would copy: <path>` / `Would delete: <path>` per filtered row plus a final count, performs no connection and no file operation.
- `--skip-existing` (copy only, default off): skip a file whose destination already exists instead of overwriting it.
- Exit codes: `0` success (or dry-run printed successfully), `1` at least one per-file operation failed, `2` runtime/setup error.
- Each file operation is attempted independently; one failing does not stop the rest.

---

## File Structure

```
tools/ftpops/
├── Cargo.toml
└── src/
    ├── main.rs        # wiring: parse -> validate -> (dry-run | connect+execute) -> output -> exit code
    ├── cli.rs          # Cli, Command (Copy/Delete), CopyTarget, DeleteTarget
    ├── filter.rs        # Filter (RemoteOnly/LocalOnly) + status_str()
    ├── csv_input.rs       # CsvRow, read_rows(), filter_by_status()
    ├── validate.rs         # validate_copy(), validate_delete()
    ├── ops.rs               # copy_to_local, copy_to_remote, delete_remote, delete_local
    └── output.rs              # format_result, format_dry_run_line, summarize, format_summary
```

---

### Task 1: Scaffold the `ftpops` crate

**Files:**
- Create: `tools/ftpops/Cargo.toml`
- Create: `tools/ftpops/src/main.rs`
- Modify: `Cargo.toml` (root workspace `members`)

**Interfaces:**
- Produces: a buildable `ftpops` binary crate member (stub `main` for now)

- [ ] **Step 1: Add the workspace member**

In the root `Cargo.toml`, change:

```toml
[workspace]
resolver = "2"
members = [
    "crates/ftp-utils-core",
    "tools/ftpdiff",
]
```

to:

```toml
[workspace]
resolver = "2"
members = [
    "crates/ftp-utils-core",
    "tools/ftpdiff",
    "tools/ftpops",
]
```

- [ ] **Step 2: Create the crate**

Create `tools/ftpops/Cargo.toml`:

```toml
[package]
name = "ftpops"
version = "0.1.0"
edition.workspace = true
license.workspace = true
description = "Perform bulk copy/delete operations based on an ftpdiff CSV report"

[dependencies]
ftp-utils-core = { path = "../../crates/ftp-utils-core" }
clap.workspace = true
csv.workspace = true
colored.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

Create `tools/ftpops/src/main.rs`:

```rust
//! ftpops: perform bulk copy/delete operations based on an ftpdiff CSV
//! report.
//!
//! See `docs/superpowers/specs/2026-09-21-ftpops-design.md` for the
//! design this binary implements.

fn main() {
    println!("ftpops: not yet implemented");
}
```

- [ ] **Step 3: Verify it builds**

Run: `cargo build --workspace`
Expected: builds cleanly, `ftpops` now appears in `cargo build`'s output

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml tools/ftpops/Cargo.toml tools/ftpops/src/main.rs
git commit -m "ftpops: scaffold crate"
```

---

### Task 2: `filter.rs` - the `--filter` value and its CSV status mapping

**Files:**
- Create: `tools/ftpops/src/filter.rs`
- Modify: `tools/ftpops/src/main.rs` (add `mod filter;`)

**Interfaces:**
- Produces: `pub enum Filter { RemoteOnly, LocalOnly }` (derives `clap::ValueEnum`, `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`), `impl Filter { pub fn status_str(self) -> &'static str }`

- [ ] **Step 1: Write the failing test**

Create `tools/ftpops/src/filter.rs`:

```rust
//! The `--filter` CLI value and its mapping to ftpdiff CSV status strings.

use clap::ValueEnum;

/// Which diff status to operate on. clap's default kebab-case casing
/// gives `--filter remote-only` / `--filter local-only`.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    RemoteOnly,
    LocalOnly,
}

impl Filter {
    /// The exact string this filter matches in the CSV `status` column
    /// (matches `ftp_utils_core::DiffStatus`'s `Debug` output, which is
    /// what `ftpdiff --csv` writes).
    pub fn status_str(self) -> &'static str {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_only_maps_to_csv_status() {
        assert_eq!(Filter::RemoteOnly.status_str(), "RemoteOnly");
    }

    #[test]
    fn local_only_maps_to_csv_status() {
        assert_eq!(Filter::LocalOnly.status_str(), "LocalOnly");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p ftpops filter::tests`
Expected: FAIL (`not yet implemented`) - note this will also fail to compile until Step 4 wires `mod filter;`; do Step 4 first if the compiler complains about an unused/undeclared module, then come back to this check.

- [ ] **Step 3: Implement**

Replace the `todo!()`:

```rust
    pub fn status_str(self) -> &'static str {
        match self {
            Filter::RemoteOnly => "RemoteOnly",
            Filter::LocalOnly => "LocalOnly",
        }
    }
```

- [ ] **Step 4: Wire the module**

In `tools/ftpops/src/main.rs`, add `mod filter;` above `fn main()`.

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p ftpops filter::tests`
Expected: PASS (2 tests)

- [ ] **Step 6: Commit**

```bash
git add tools/ftpops/src/filter.rs tools/ftpops/src/main.rs
git commit -m "ftpops: add --filter value and CSV status mapping"
```

---

### Task 3: `csv_input.rs` - reading and filtering the ftpdiff CSV report

**Files:**
- Create: `tools/ftpops/src/csv_input.rs`
- Modify: `tools/ftpops/src/main.rs` (add `mod csv_input;`)

**Interfaces:**
- Produces: `pub struct CsvRow { pub relative_path: String, pub status: String }` (derives `Debug`, `Clone`, `PartialEq`), `pub struct CsvError(pub String)` (`Display`, `std::error::Error`), `pub fn read_rows(path: &Path) -> Result<Vec<CsvRow>, CsvError>`, `pub fn filter_by_status<'a>(rows: &'a [CsvRow], status: &str) -> Vec<&'a CsvRow>`

- [ ] **Step 1: Write the failing test**

Create `tools/ftpops/src/csv_input.rs`:

```rust
//! Reads an `ftpdiff --csv` report and filters rows by status.

use std::path::Path;

/// One row from an ftpdiff CSV report, reduced to what ftpops needs.
#[derive(Debug, Clone, PartialEq)]
pub struct CsvRow {
    pub relative_path: String,
    pub status: String,
}

#[derive(Debug)]
pub struct CsvError(pub String);

impl std::fmt::Display for CsvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CsvError {}

/// Reads every row of the CSV at `path`, keyed by its `path` and `status`
/// columns (as written by `ftpdiff --csv`; extra columns are ignored).
pub fn read_rows(path: &Path) -> Result<Vec<CsvRow>, CsvError> {
    todo!()
}

/// Returns the rows whose `status` exactly matches `status`.
pub fn filter_by_status<'a>(rows: &'a [CsvRow], status: &str) -> Vec<&'a CsvRow> {
    rows.iter().filter(|r| r.status == status).collect()
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

    #[test]
    fn reads_path_and_status_columns() {
        let (_dir, path) = write_csv(
            "path,status,local_size,remote_size,local_md5,remote_md5\n\
             a.txt,RemoteOnly,,10,,\n\
             sub/b.txt,LocalOnly,5,,,\n",
        );

        let rows = read_rows(&path).unwrap();

        assert_eq!(
            rows,
            vec![
                CsvRow { relative_path: "a.txt".to_string(), status: "RemoteOnly".to_string() },
                CsvRow { relative_path: "sub/b.txt".to_string(), status: "LocalOnly".to_string() },
            ]
        );
    }

    #[test]
    fn errors_when_file_missing() {
        let result = read_rows(std::path::Path::new("/nonexistent/report.csv"));

        assert!(result.is_err());
    }

    #[test]
    fn filter_by_status_selects_matching_rows_only() {
        let rows = vec![
            CsvRow { relative_path: "a.txt".to_string(), status: "RemoteOnly".to_string() },
            CsvRow { relative_path: "b.txt".to_string(), status: "Match".to_string() },
            CsvRow { relative_path: "c.txt".to_string(), status: "RemoteOnly".to_string() },
        ];

        let filtered = filter_by_status(&rows, "RemoteOnly");

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].relative_path, "a.txt");
        assert_eq!(filtered[1].relative_path, "c.txt");
    }
}
```

- [ ] **Step 2: Run tests to verify the `read_rows` ones fail**

Run: `cargo test -p ftpops csv_input::tests`
Expected: FAIL (`not yet implemented`) for `reads_path_and_status_columns` and `errors_when_file_missing`; compile error until Step 4 wires the module.

- [ ] **Step 3: Implement**

Replace the `todo!()`:

```rust
pub fn read_rows(path: &Path) -> Result<Vec<CsvRow>, CsvError> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|e| CsvError(format!("cannot read CSV {}: {e}", path.display())))?;

    let headers = reader.headers().map_err(|e| CsvError(e.to_string()))?.clone();
    let path_idx = headers
        .iter()
        .position(|h| h == "path")
        .ok_or_else(|| CsvError("CSV missing 'path' column".to_string()))?;
    let status_idx = headers
        .iter()
        .position(|h| h == "status")
        .ok_or_else(|| CsvError("CSV missing 'status' column".to_string()))?;

    let mut rows = Vec::new();
    for result in reader.records() {
        let record = result.map_err(|e| CsvError(format!("invalid CSV row: {e}")))?;
        let relative_path = record
            .get(path_idx)
            .ok_or_else(|| CsvError("row missing 'path' value".to_string()))?
            .to_string();
        let status = record
            .get(status_idx)
            .ok_or_else(|| CsvError("row missing 'status' value".to_string()))?
            .to_string();
        rows.push(CsvRow { relative_path, status });
    }

    Ok(rows)
}
```

- [ ] **Step 4: Wire the module**

In `tools/ftpops/src/main.rs`, add `mod csv_input;`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p ftpops csv_input::tests`
Expected: PASS (3 tests)

- [ ] **Step 6: Commit**

```bash
git add tools/ftpops/src/csv_input.rs tools/ftpops/src/main.rs
git commit -m "ftpops: add CSV report reading and status filtering"
```

---

### Task 4: `validate.rs` - `--filter`/`--to`/`--on` combination validation

**Files:**
- Create: `tools/ftpops/src/validate.rs`
- Modify: `tools/ftpops/src/main.rs` (add `mod validate;`)

**Interfaces:**
- Consumes: `filter::Filter` (Task 2), `cli::{CopyTarget, DeleteTarget}` (defined in this task's tests as local stand-ins; the real `cli.rs` types arrive in Task 5 - see note below)
- Produces: `pub struct ValidationError(pub String)` (`Display`, `std::error::Error`), `pub fn validate_copy(to: CopyTarget, filter: Filter) -> Result<(), ValidationError>`, `pub fn validate_delete(on: DeleteTarget, filter: Filter) -> Result<(), ValidationError>`

**Note on ordering:** this task is written before `cli.rs` (Task 5) because `validate.rs`'s logic only depends on two small enums, not on `clap`. To keep `validate.rs` decoupled from the CLI layer (it's pure logic, more reusable and simpler to test), `CopyTarget` and `DeleteTarget` are defined *in this file*, and Task 5's `cli.rs` will `pub use` them from here rather than redefining them - avoiding duplicate/incompatible enum definitions.

- [ ] **Step 1: Write the failing test**

Create `tools/ftpops/src/validate.rs`:

```rust
//! Validates that `--filter` is compatible with the chosen `--to`/`--on`
//! direction, per the design spec's combination table: `RemoteOnly`
//! entries only make sense downloading to local or deleting from remote;
//! `LocalOnly` entries only make sense uploading to remote or deleting
//! from local.

use clap::ValueEnum;

use crate::filter::Filter;

/// Which side `copy` writes to.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyTarget {
    Local,
    Remote,
}

/// Which side `delete` removes from.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteTarget {
    Local,
    Remote,
}

#[derive(Debug)]
pub struct ValidationError(pub String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ValidationError {}

pub fn validate_copy(to: CopyTarget, filter: Filter) -> Result<(), ValidationError> {
    todo!()
}

pub fn validate_delete(on: DeleteTarget, filter: Filter) -> Result<(), ValidationError> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_to_local_with_remote_only_is_valid() {
        assert!(validate_copy(CopyTarget::Local, Filter::RemoteOnly).is_ok());
    }

    #[test]
    fn copy_to_remote_with_local_only_is_valid() {
        assert!(validate_copy(CopyTarget::Remote, Filter::LocalOnly).is_ok());
    }

    #[test]
    fn copy_to_local_with_local_only_is_invalid() {
        assert!(validate_copy(CopyTarget::Local, Filter::LocalOnly).is_err());
    }

    #[test]
    fn copy_to_remote_with_remote_only_is_invalid() {
        assert!(validate_copy(CopyTarget::Remote, Filter::RemoteOnly).is_err());
    }

    #[test]
    fn delete_on_remote_with_remote_only_is_valid() {
        assert!(validate_delete(DeleteTarget::Remote, Filter::RemoteOnly).is_ok());
    }

    #[test]
    fn delete_on_local_with_local_only_is_valid() {
        assert!(validate_delete(DeleteTarget::Local, Filter::LocalOnly).is_ok());
    }

    #[test]
    fn delete_on_remote_with_local_only_is_invalid() {
        assert!(validate_delete(DeleteTarget::Remote, Filter::LocalOnly).is_err());
    }

    #[test]
    fn delete_on_local_with_remote_only_is_invalid() {
        assert!(validate_delete(DeleteTarget::Local, Filter::RemoteOnly).is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ftpops validate::tests`
Expected: FAIL (`not yet implemented`); compile error until Step 4 wires the module and `filter::Filter` (Task 2) is available (it already is)

- [ ] **Step 3: Implement**

Replace the two `todo!()` bodies:

```rust
pub fn validate_copy(to: CopyTarget, filter: Filter) -> Result<(), ValidationError> {
    match (to, filter) {
        (CopyTarget::Local, Filter::RemoteOnly) => Ok(()),
        (CopyTarget::Remote, Filter::LocalOnly) => Ok(()),
        (CopyTarget::Local, Filter::LocalOnly) => Err(ValidationError(
            "--filter local-only doesn't apply to 'copy --to local': LocalOnly entries have no remote file to download from - did you mean --filter remote-only, or copy --to remote?".to_string(),
        )),
        (CopyTarget::Remote, Filter::RemoteOnly) => Err(ValidationError(
            "--filter remote-only doesn't apply to 'copy --to remote': RemoteOnly entries have no local file to upload from - did you mean --filter local-only, or copy --to local?".to_string(),
        )),
    }
}

pub fn validate_delete(on: DeleteTarget, filter: Filter) -> Result<(), ValidationError> {
    match (on, filter) {
        (DeleteTarget::Remote, Filter::RemoteOnly) => Ok(()),
        (DeleteTarget::Local, Filter::LocalOnly) => Ok(()),
        (DeleteTarget::Remote, Filter::LocalOnly) => Err(ValidationError(
            "--filter local-only doesn't apply to 'delete --on remote': LocalOnly entries don't exist on the remote side - did you mean --filter remote-only, or delete --on local?".to_string(),
        )),
        (DeleteTarget::Local, Filter::RemoteOnly) => Err(ValidationError(
            "--filter remote-only doesn't apply to 'delete --on local': RemoteOnly entries don't exist locally - did you mean --filter local-only, or delete --on remote?".to_string(),
        )),
    }
}
```

- [ ] **Step 4: Wire the module**

In `tools/ftpops/src/main.rs`, add `mod validate;`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p ftpops validate::tests`
Expected: PASS (8 tests)

- [ ] **Step 6: Commit**

```bash
git add tools/ftpops/src/validate.rs tools/ftpops/src/main.rs
git commit -m "ftpops: add --filter/--to/--on combination validation"
```

---

### Task 5: `cli.rs` - CLI argument definitions

**Files:**
- Create: `tools/ftpops/src/cli.rs`
- Modify: `tools/ftpops/src/main.rs` (add `mod cli;`)

**Interfaces:**
- Consumes: `ftp_utils_core::connection::ConnectionArgs`, `crate::filter::Filter` (Task 2), `crate::validate::{CopyTarget, DeleteTarget}` (Task 4)
- Produces: `pub struct Cli { pub command: Command }` (derives `clap::Parser`), `pub enum Command { Copy { connection: ConnectionArgs, to: CopyTarget, filter: Filter, csv: PathBuf, skip_existing: bool, dry_run: bool }, Delete { connection: ConnectionArgs, on: DeleteTarget, filter: Filter, csv: PathBuf, dry_run: bool } }` (derives `clap::Subcommand`)

- [ ] **Step 1: Write the failing test**

Create `tools/ftpops/src/cli.rs`:

```rust
//! Command-line argument definitions for ftpops.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ftp_utils_core::connection::ConnectionArgs;

pub use crate::validate::{CopyTarget, DeleteTarget};
use crate::filter::Filter;

/// Perform bulk copy/delete operations based on an ftpdiff CSV report.
#[derive(Parser, Debug)]
#[command(name = "ftpops", about = "Perform bulk copy/delete operations based on an ftpdiff CSV report")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Copy files between local and remote, filtered by diff status.
    Copy {
        #[command(flatten)]
        connection: ConnectionArgs,

        /// Direction to copy: download to local, or upload to remote.
        #[arg(long)]
        to: CopyTarget,

        /// Only operate on rows with this diff status.
        #[arg(long)]
        filter: Filter,

        /// Path to the ftpdiff --csv report to read.
        #[arg(long)]
        csv: PathBuf,

        /// Skip files whose destination already exists, instead of
        /// overwriting them.
        #[arg(long)]
        skip_existing: bool,

        /// Print what would be done without doing it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete files locally or remotely, filtered by diff status.
    Delete {
        #[command(flatten)]
        connection: ConnectionArgs,

        /// Which side to delete from.
        #[arg(long)]
        on: DeleteTarget,

        /// Only operate on rows with this diff status.
        #[arg(long)]
        filter: Filter,

        /// Path to the ftpdiff --csv report to read.
        #[arg(long)]
        csv: PathBuf,

        /// Print what would be done without doing it.
        #[arg(long)]
        dry_run: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_copy_subcommand() {
        let cli = Cli::parse_from([
            "ftpops", "copy",
            "--to", "local",
            "--filter", "remote-only",
            "--csv", "report.csv",
            "--host", "ftp.example.com",
            "--user", "bob",
            "--remote-dir", "/remote",
            "--local-dir", "./local",
        ]);

        match cli.command {
            Command::Copy { to, filter, csv, skip_existing, dry_run, connection } => {
                assert_eq!(to, CopyTarget::Local);
                assert_eq!(filter, Filter::RemoteOnly);
                assert_eq!(csv, PathBuf::from("report.csv"));
                assert!(!skip_existing);
                assert!(!dry_run);
                assert_eq!(connection.host.as_deref(), Some("ftp.example.com"));
            }
            Command::Delete { .. } => panic!("expected Copy"),
        }
    }

    #[test]
    fn parses_copy_optional_flags() {
        let cli = Cli::parse_from([
            "ftpops", "copy",
            "--to", "remote",
            "--filter", "local-only",
            "--csv", "report.csv",
            "--skip-existing",
            "--dry-run",
        ]);

        match cli.command {
            Command::Copy { skip_existing, dry_run, .. } => {
                assert!(skip_existing);
                assert!(dry_run);
            }
            Command::Delete { .. } => panic!("expected Copy"),
        }
    }

    #[test]
    fn parses_delete_subcommand() {
        let cli = Cli::parse_from([
            "ftpops", "delete",
            "--on", "remote",
            "--filter", "remote-only",
            "--csv", "report.csv",
        ]);

        match cli.command {
            Command::Delete { on, filter, csv, dry_run, .. } => {
                assert_eq!(on, DeleteTarget::Remote);
                assert_eq!(filter, Filter::RemoteOnly);
                assert_eq!(csv, PathBuf::from("report.csv"));
                assert!(!dry_run);
            }
            Command::Copy { .. } => panic!("expected Delete"),
        }
    }
}
```

- [ ] **Step 2: Wire the module (no red phase - this task only assembles existing pieces)**

In `tools/ftpops/src/main.rs`, add `mod cli;`.

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p ftpops cli::tests`
Expected: PASS (3 tests)

- [ ] **Step 4: Commit**

```bash
git add tools/ftpops/src/cli.rs tools/ftpops/src/main.rs
git commit -m "ftpops: add CLI argument definitions (copy/delete subcommands)"
```

---

### Task 6: `ops.rs` - copy and delete operations

**Files:**
- Create: `tools/ftpops/src/ops.rs`
- Modify: `tools/ftpops/src/main.rs` (add `mod ops;`)

**Interfaces:**
- Consumes: `ftp_utils_core::FtpConnection` (extended trait), `crate::csv_input::CsvRow` (Task 3)
- Produces: `pub enum OpOutcome { Copied, Deleted, Skipped, Failed(String) }` (derives `Debug`, `Clone`, `PartialEq`), `pub struct OpResult { pub relative_path: String, pub outcome: OpOutcome }` (derives `Debug`, `Clone`, `PartialEq`), `pub fn copy_to_local<C: FtpConnection>(conn: &mut C, remote_dir: &str, local_dir: &Path, rows: &[&CsvRow], skip_existing: bool) -> Vec<OpResult>`, `pub fn copy_to_remote<C: FtpConnection>(conn: &mut C, remote_dir: &str, local_dir: &Path, rows: &[&CsvRow], skip_existing: bool) -> Vec<OpResult>`, `pub fn delete_remote<C: FtpConnection>(conn: &mut C, remote_dir: &str, rows: &[&CsvRow]) -> Vec<OpResult>`, `pub fn delete_local(local_dir: &Path, rows: &[&CsvRow]) -> Vec<OpResult>`

- [ ] **Step 1: Write the failing tests**

Create `tools/ftpops/src/ops.rs`:

```rust
//! Copy and delete operations, executed against CSV rows already
//! filtered by status.

use std::path::Path;

use ftp_utils_core::FtpConnection;

use crate::csv_input::CsvRow;

/// What happened to one file.
#[derive(Debug, Clone, PartialEq)]
pub enum OpOutcome {
    Copied,
    Deleted,
    Skipped,
    Failed(String),
}

/// The outcome for one CSV row.
#[derive(Debug, Clone, PartialEq)]
pub struct OpResult {
    pub relative_path: String,
    pub outcome: OpOutcome,
}

/// Downloads each row's remote file into the local directory, creating
/// missing local parent directories. Skips (without overwriting) a row
/// whose local destination already exists when `skip_existing` is true.
pub fn copy_to_local<C: FtpConnection>(
    conn: &mut C,
    remote_dir: &str,
    local_dir: &Path,
    rows: &[&CsvRow],
    skip_existing: bool,
) -> Vec<OpResult> {
    todo!()
}

/// Uploads each row's local file to the remote directory, creating
/// missing remote parent directories. Skips (without overwriting) a row
/// whose remote destination already exists when `skip_existing` is true.
pub fn copy_to_remote<C: FtpConnection>(
    conn: &mut C,
    remote_dir: &str,
    local_dir: &Path,
    rows: &[&CsvRow],
    skip_existing: bool,
) -> Vec<OpResult> {
    todo!()
}

/// Deletes each row's remote file.
pub fn delete_remote<C: FtpConnection>(conn: &mut C, remote_dir: &str, rows: &[&CsvRow]) -> Vec<OpResult> {
    todo!()
}

/// Deletes each row's local file.
pub fn delete_local(local_dir: &Path, rows: &[&CsvRow]) -> Vec<OpResult> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ftp_utils_core::remote::{FtpConnectionError, RawRemoteEntry};
    use std::collections::HashMap;

    #[derive(Default)]
    struct MockConnection {
        listings: HashMap<String, Vec<RawRemoteEntry>>,
        remote_files: HashMap<String, Vec<u8>>,
        stored: Vec<(String, Vec<u8>)>,
        deleted: Vec<String>,
        created_dirs: Vec<String>,
    }

    impl FtpConnection for MockConnection {
        fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
            Ok(self.listings.get(path).cloned().unwrap_or_default())
        }

        fn try_hash(&mut self, _path: &str) -> Option<String> {
            None
        }

        fn retr_to_buffer(&mut self, path: &str) -> Result<Vec<u8>, FtpConnectionError> {
            self.remote_files
                .get(path)
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

    fn row(relative_path: &str) -> CsvRow {
        CsvRow { relative_path: relative_path.to_string(), status: "RemoteOnly".to_string() }
    }

    #[test]
    fn copy_to_local_downloads_and_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = MockConnection::default();
        conn.remote_files.insert("/remote/sub/file.txt".to_string(), b"hello".to_vec());

        let rows = vec![row("sub/file.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = copy_to_local(&mut conn, "/remote", dir.path(), &refs, false);

        assert_eq!(results, vec![OpResult { relative_path: "sub/file.txt".to_string(), outcome: OpOutcome::Copied }]);
        let written = std::fs::read(dir.path().join("sub/file.txt")).unwrap();
        assert_eq!(written, b"hello");
    }

    #[test]
    fn copy_to_local_skips_when_destination_exists_and_skip_existing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), b"already here").unwrap();
        let mut conn = MockConnection::default();
        conn.remote_files.insert("/remote/file.txt".to_string(), b"new content".to_vec());

        let rows = vec![row("file.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = copy_to_local(&mut conn, "/remote", dir.path(), &refs, true);

        assert_eq!(results, vec![OpResult { relative_path: "file.txt".to_string(), outcome: OpOutcome::Skipped }]);
        let content = std::fs::read(dir.path().join("file.txt")).unwrap();
        assert_eq!(content, b"already here");
    }

    #[test]
    fn copy_to_local_reports_failure_for_missing_remote_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = MockConnection::default();

        let rows = vec![row("missing.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = copy_to_local(&mut conn, "/remote", dir.path(), &refs, false);

        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, OpOutcome::Failed(_)));
    }

    #[test]
    fn copy_to_remote_uploads_and_ensures_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/file.txt"), b"hello").unwrap();
        let mut conn = MockConnection::default();
        conn.listings.insert("/".to_string(), vec![]);

        let rows = vec![row("sub/file.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = copy_to_remote(&mut conn, "/remote", dir.path(), &refs, false);

        assert_eq!(results, vec![OpResult { relative_path: "sub/file.txt".to_string(), outcome: OpOutcome::Copied }]);
        assert_eq!(conn.stored, vec![("/remote/sub/file.txt".to_string(), b"hello".to_vec())]);
        assert_eq!(conn.created_dirs, vec!["/remote".to_string(), "/remote/sub".to_string()]);
    }

    #[test]
    fn copy_to_remote_skips_when_destination_exists_and_skip_existing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), b"local content").unwrap();
        let mut conn = MockConnection::default();
        conn.listings.insert("/remote".to_string(), vec![RawRemoteEntry { name: "file.txt".into(), is_dir: false, size: 5 }]);

        let rows = vec![row("file.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = copy_to_remote(&mut conn, "/remote", dir.path(), &refs, true);

        assert_eq!(results, vec![OpResult { relative_path: "file.txt".to_string(), outcome: OpOutcome::Skipped }]);
        assert!(conn.stored.is_empty());
    }

    #[test]
    fn delete_remote_calls_delete_with_full_path() {
        let mut conn = MockConnection::default();

        let rows = vec![row("a.txt"), row("sub/b.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = delete_remote(&mut conn, "/remote", &refs);

        assert_eq!(
            results,
            vec![
                OpResult { relative_path: "a.txt".to_string(), outcome: OpOutcome::Deleted },
                OpResult { relative_path: "sub/b.txt".to_string(), outcome: OpOutcome::Deleted },
            ]
        );
        assert_eq!(conn.deleted, vec!["/remote/a.txt".to_string(), "/remote/sub/b.txt".to_string()]);
    }

    #[test]
    fn delete_local_removes_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();

        let rows = vec![row("a.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = delete_local(dir.path(), &refs);

        assert_eq!(results, vec![OpResult { relative_path: "a.txt".to_string(), outcome: OpOutcome::Deleted }]);
        assert!(!dir.path().join("a.txt").exists());
    }

    #[test]
    fn delete_local_reports_failure_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();

        let rows = vec![row("missing.txt")];
        let refs: Vec<&CsvRow> = rows.iter().collect();

        let results = delete_local(dir.path(), &refs);

        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, OpOutcome::Failed(_)));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ftpops ops::tests`
Expected: FAIL (`not yet implemented`); compile error until Step 4 wires the module

- [ ] **Step 3: Implement**

Replace the four `todo!()` bodies:

```rust
pub fn copy_to_local<C: FtpConnection>(
    conn: &mut C,
    remote_dir: &str,
    local_dir: &Path,
    rows: &[&CsvRow],
    skip_existing: bool,
) -> Vec<OpResult> {
    rows.iter()
        .map(|row| {
            let local_path = local_dir.join(&row.relative_path);
            if skip_existing && local_path.exists() {
                return OpResult { relative_path: row.relative_path.clone(), outcome: OpOutcome::Skipped };
            }

            let remote_path = format!("{remote_dir}/{}", row.relative_path);
            let outcome = (|| -> Result<(), String> {
                let data = conn.retr_to_buffer(&remote_path).map_err(|e| e.to_string())?;
                if let Some(parent) = local_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&local_path, data).map_err(|e| e.to_string())?;
                Ok(())
            })();

            OpResult {
                relative_path: row.relative_path.clone(),
                outcome: match outcome {
                    Ok(()) => OpOutcome::Copied,
                    Err(e) => OpOutcome::Failed(e),
                },
            }
        })
        .collect()
}

pub fn copy_to_remote<C: FtpConnection>(
    conn: &mut C,
    remote_dir: &str,
    local_dir: &Path,
    rows: &[&CsvRow],
    skip_existing: bool,
) -> Vec<OpResult> {
    rows.iter()
        .map(|row| {
            let remote_path = format!("{remote_dir}/{}", row.relative_path);

            if skip_existing {
                let (remote_parent, file_name) = remote_path.rsplit_once('/').unwrap_or(("", &remote_path));
                let remote_parent = if remote_parent.is_empty() { "/" } else { remote_parent };
                let exists = conn
                    .list_dir(remote_parent)
                    .map(|entries| entries.iter().any(|e| !e.is_dir && e.name == file_name))
                    .unwrap_or(false);
                if exists {
                    return OpResult { relative_path: row.relative_path.clone(), outcome: OpOutcome::Skipped };
                }
            }

            let local_path = local_dir.join(&row.relative_path);
            let outcome = (|| -> Result<(), String> {
                let data = std::fs::read(&local_path).map_err(|e| e.to_string())?;
                conn.ensure_remote_dir(&remote_path).map_err(|e| e.to_string())?;
                conn.store_from_buffer(&remote_path, &data).map_err(|e| e.to_string())?;
                Ok(())
            })();

            OpResult {
                relative_path: row.relative_path.clone(),
                outcome: match outcome {
                    Ok(()) => OpOutcome::Copied,
                    Err(e) => OpOutcome::Failed(e),
                },
            }
        })
        .collect()
}

pub fn delete_remote<C: FtpConnection>(conn: &mut C, remote_dir: &str, rows: &[&CsvRow]) -> Vec<OpResult> {
    rows.iter()
        .map(|row| {
            let remote_path = format!("{remote_dir}/{}", row.relative_path);
            let outcome = conn.delete(&remote_path).map_err(|e| e.to_string());
            OpResult {
                relative_path: row.relative_path.clone(),
                outcome: match outcome {
                    Ok(()) => OpOutcome::Deleted,
                    Err(e) => OpOutcome::Failed(e),
                },
            }
        })
        .collect()
}

pub fn delete_local(local_dir: &Path, rows: &[&CsvRow]) -> Vec<OpResult> {
    rows.iter()
        .map(|row| {
            let local_path = local_dir.join(&row.relative_path);
            let outcome = std::fs::remove_file(&local_path).map_err(|e| e.to_string());
            OpResult {
                relative_path: row.relative_path.clone(),
                outcome: match outcome {
                    Ok(()) => OpOutcome::Deleted,
                    Err(e) => OpOutcome::Failed(e),
                },
            }
        })
        .collect()
}
```

- [ ] **Step 4: Wire the module**

In `tools/ftpops/src/main.rs`, add `mod ops;`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p ftpops ops::tests`
Expected: PASS (9 tests)

- [ ] **Step 6: Commit**

```bash
git add tools/ftpops/src/ops.rs tools/ftpops/src/main.rs
git commit -m "ftpops: add copy_to_local, copy_to_remote, delete_remote, delete_local"
```

---

### Task 7: `output.rs` - result formatting

**Files:**
- Create: `tools/ftpops/src/output.rs`
- Modify: `tools/ftpops/src/main.rs` (add `mod output;`)

**Interfaces:**
- Consumes: `crate::ops::{OpOutcome, OpResult}` (Task 6)
- Produces: `pub fn format_result(result: &OpResult) -> String`, `pub fn format_dry_run_line(verb: &str, relative_path: &str) -> String`, `pub struct Summary { pub succeeded: usize, pub skipped: usize, pub failed: usize }`, `pub fn summarize(results: &[OpResult]) -> Summary`, `pub fn format_summary(summary: &Summary) -> String`

- [ ] **Step 1: Write the failing test**

Create `tools/ftpops/src/output.rs`:

```rust
//! Colored text output for ftpops operation results.

use colored::Colorize;

use crate::ops::{OpOutcome, OpResult};

/// Formats one result as a colored line: `+` copied, `-` deleted,
/// `~` skipped, `!` failed.
pub fn format_result(result: &OpResult) -> String {
    todo!()
}

/// Formats a single dry-run preview line: "Would <verb>: <path>".
pub fn format_dry_run_line(verb: &str, relative_path: &str) -> String {
    format!("Would {verb}: {relative_path}")
}

/// Counts of results by outcome.
pub struct Summary {
    pub succeeded: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Tallies `results` by outcome (`Copied`/`Deleted` both count as
/// succeeded).
pub fn summarize(results: &[OpResult]) -> Summary {
    let mut summary = Summary { succeeded: 0, skipped: 0, failed: 0 };
    for result in results {
        match result.outcome {
            OpOutcome::Copied | OpOutcome::Deleted => summary.succeeded += 1,
            OpOutcome::Skipped => summary.skipped += 1,
            OpOutcome::Failed(_) => summary.failed += 1,
        }
    }
    summary
}

/// Formats a one-line summary of the tallies.
pub fn format_summary(summary: &Summary) -> String {
    format!(
        "Summary: {} succeeded, {} skipped, {} failed",
        summary.succeeded, summary.skipped, summary.failed
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(relative_path: &str, outcome: OpOutcome) -> OpResult {
        OpResult { relative_path: relative_path.to_string(), outcome }
    }

    #[test]
    fn formats_each_outcome_with_the_relative_path() {
        colored::control::set_override(false);

        assert!(format_result(&result("a.txt", OpOutcome::Copied)).contains("a.txt"));
        assert!(format_result(&result("b.txt", OpOutcome::Deleted)).contains("b.txt"));
        assert!(format_result(&result("c.txt", OpOutcome::Skipped)).contains("c.txt"));
        assert!(format_result(&result("d.txt", OpOutcome::Failed("boom".to_string()))).contains("d.txt"));
        assert!(format_result(&result("d.txt", OpOutcome::Failed("boom".to_string()))).contains("boom"));
    }

    #[test]
    fn formats_dry_run_line() {
        let line = format_dry_run_line("copy", "a.txt");

        assert_eq!(line, "Would copy: a.txt");
    }

    #[test]
    fn summarizes_counts_by_outcome() {
        let results = vec![
            result("a", OpOutcome::Copied),
            result("b", OpOutcome::Deleted),
            result("c", OpOutcome::Skipped),
            result("d", OpOutcome::Failed("x".to_string())),
        ];

        let summary = summarize(&results);

        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.failed, 1);
    }

    #[test]
    fn formats_summary_line() {
        let summary = Summary { succeeded: 3, skipped: 1, failed: 2 };
        let line = format_summary(&summary);

        assert!(line.contains("3 succeeded"));
        assert!(line.contains("1 skipped"));
        assert!(line.contains("2 failed"));
    }
}
```

- [ ] **Step 2: Run tests to verify `format_result` tests fail**

Run: `cargo test -p ftpops output::tests`
Expected: FAIL (`not yet implemented` for `formats_each_outcome_with_the_relative_path`); compile error until Step 4 wires the module

- [ ] **Step 3: Implement**

Replace the `todo!()`:

```rust
pub fn format_result(result: &OpResult) -> String {
    match &result.outcome {
        OpOutcome::Copied => format!("{} {}", "+".green(), result.relative_path),
        OpOutcome::Deleted => format!("{} {}", "-".red(), result.relative_path),
        OpOutcome::Skipped => format!("{} {} (skipped, already exists)", "~".yellow(), result.relative_path),
        OpOutcome::Failed(e) => format!("{} {} ({e})", "!".red().bold(), result.relative_path),
    }
}
```

- [ ] **Step 4: Wire the module**

In `tools/ftpops/src/main.rs`, add `mod output;`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p ftpops output::tests`
Expected: PASS (4 tests)

- [ ] **Step 6: Commit**

```bash
git add tools/ftpops/src/output.rs tools/ftpops/src/main.rs
git commit -m "ftpops: add colored result output and summary formatting"
```

---

### Task 8: Wire `main.rs`, full verification, docs

**Files:**
- Modify: `tools/ftpops/src/main.rs`
- Modify: `README.md`
- Modify: `CHANGES.md`

**Interfaces:**
- Consumes: everything from Tasks 1-7

This task has no new automated test (it's wiring); it's verified by the
full workspace test suite plus a manual CLI smoke test (`--help`, error
paths, `--dry-run`).

- [ ] **Step 1: Implement**

Replace the full contents of `tools/ftpops/src/main.rs`:

```rust
//! ftpops: perform bulk copy/delete operations based on an ftpdiff CSV
//! report.
//!
//! See `docs/superpowers/specs/2026-09-21-ftpops-design.md` for the
//! design this binary implements.

mod cli;
mod csv_input;
mod filter;
mod ops;
mod output;
mod validate;

use std::path::Path;

use clap::Parser;
use ftp_utils_core::connection::{self, ConnectionArgs, EffectiveConnection};
use ftp_utils_core::ftp_client::SuppaFtpConnection;

use cli::{Cli, Command};
use filter::Filter;
use validate::{CopyTarget, DeleteTarget};

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let cli = Cli::parse();

    match cli.command {
        Command::Copy { connection, to, filter, csv, skip_existing, dry_run } => {
            if let Err(e) = validate::validate_copy(to, filter) {
                eprintln!("Error: {e}");
                return 2;
            }
            run_copy(&connection, to, filter, &csv, skip_existing, dry_run)
        }
        Command::Delete { connection, on, filter, csv, dry_run } => {
            if let Err(e) = validate::validate_delete(on, filter) {
                eprintln!("Error: {e}");
                return 2;
            }
            run_delete(&connection, on, filter, &csv, dry_run)
        }
    }
}

fn load_effective_connection(args: &ConnectionArgs) -> Result<EffectiveConnection, i32> {
    let json_config = match &args.config {
        Some(path) => match connection::load_json_config::<connection::ConnectionJsonConfig>(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error: {e}");
                return Err(2);
            }
        },
        None => connection::ConnectionJsonConfig::default(),
    };

    connection::merge_connection(args, &json_config).map_err(|e| {
        eprintln!("Error: {e}");
        2
    })
}

fn connect(args: &ConnectionArgs, effective: &EffectiveConnection) -> Result<SuppaFtpConnection, i32> {
    let password = match connection::read_password(args.password.as_deref(), "FTPOPS_PASSWORD") {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: {e}");
            return Err(2);
        }
    };

    SuppaFtpConnection::connect(
        &effective.host,
        effective.port,
        &effective.user,
        &password,
        effective.ftps,
        effective.insecure_tls,
    )
    .map_err(|e| {
        eprintln!("Error: failed to connect to {}:{}: {e}", effective.host, effective.port);
        2
    })
}

fn read_filtered_rows(csv_path: &Path, filter: Filter) -> Result<Vec<csv_input::CsvRow>, i32> {
    csv_input::read_rows(csv_path).map_err(|e| {
        eprintln!("Error: {e}");
        2
    })
}

fn run_copy(
    connection_args: &ConnectionArgs,
    to: CopyTarget,
    filter: Filter,
    csv_path: &Path,
    skip_existing: bool,
    dry_run: bool,
) -> i32 {
    let effective = match load_effective_connection(connection_args) {
        Ok(c) => c,
        Err(code) => return code,
    };

    let rows = match read_filtered_rows(csv_path, filter) {
        Ok(r) => r,
        Err(code) => return code,
    };
    let filtered = csv_input::filter_by_status(&rows, filter.status_str());

    if dry_run {
        for row in &filtered {
            println!("{}", output::format_dry_run_line("copy", &row.relative_path));
        }
        println!("Would copy {} file(s).", filtered.len());
        return 0;
    }

    let mut connection = match connect(connection_args, &effective) {
        Ok(c) => c,
        Err(code) => return code,
    };

    let results = match to {
        CopyTarget::Local => {
            ops::copy_to_local(&mut connection, &effective.remote_dir, &effective.local_dir, &filtered, skip_existing)
        }
        CopyTarget::Remote => {
            ops::copy_to_remote(&mut connection, &effective.remote_dir, &effective.local_dir, &filtered, skip_existing)
        }
    };

    connection.close();

    for result in &results {
        println!("{}", output::format_result(result));
    }
    let summary = output::summarize(&results);
    println!("{}", output::format_summary(&summary));

    if summary.failed > 0 {
        1
    } else {
        0
    }
}

fn run_delete(connection_args: &ConnectionArgs, on: DeleteTarget, filter: Filter, csv_path: &Path, dry_run: bool) -> i32 {
    let effective = match load_effective_connection(connection_args) {
        Ok(c) => c,
        Err(code) => return code,
    };

    let rows = match read_filtered_rows(csv_path, filter) {
        Ok(r) => r,
        Err(code) => return code,
    };
    let filtered = csv_input::filter_by_status(&rows, filter.status_str());

    if dry_run {
        for row in &filtered {
            println!("{}", output::format_dry_run_line("delete", &row.relative_path));
        }
        println!("Would delete {} file(s).", filtered.len());
        return 0;
    }

    let results = match on {
        DeleteTarget::Local => ops::delete_local(&effective.local_dir, &filtered),
        DeleteTarget::Remote => {
            let mut connection = match connect(connection_args, &effective) {
                Ok(c) => c,
                Err(code) => return code,
            };
            let results = ops::delete_remote(&mut connection, &effective.remote_dir, &filtered);
            connection.close();
            results
        }
    };

    for result in &results {
        println!("{}", output::format_result(result));
    }
    let summary = output::summarize(&results);
    println!("{}", output::format_summary(&summary));

    if summary.failed > 0 {
        1
    } else {
        0
    }
}
```

- [ ] **Step 2: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS (all tests from Tasks 1-7, plus everything from the prior refactor plan)

- [ ] **Step 3: Build and manually smoke-test the CLI**

Run: `cargo build --workspace`
Expected: builds cleanly, no warnings

Run: `cargo run -p ftpops -- --help` and `cargo run -p ftpops -- copy --help`
Expected: shows the `copy`/`delete` subcommands and their flags

Run (validation error path, no CSV needed since validation happens first):
```sh
cargo run -p ftpops -- copy --to local --filter local-only --csv /nonexistent.csv --host h --user u --remote-dir /r --local-dir .
```
Expected: prints the `--filter local-only doesn't apply to 'copy --to local'...` error and exits 2 - confirms validation runs before the CSV is touched

Run (dry-run path, using a real CSV like the one `ftpdiff --csv` produces):
```sh
mkdir -p /tmp/ftpops-smoke
printf 'path,status,local_size,remote_size,local_md5,remote_md5\na.txt,RemoteOnly,,10,,\n' > /tmp/ftpops-smoke/report.csv
cargo run -p ftpops -- copy --to local --filter remote-only --csv /tmp/ftpops-smoke/report.csv --dry-run --host h --user u --remote-dir /r --local-dir /tmp/ftpops-smoke
```
Expected: prints `Would copy: a.txt` and `Would copy 1 file(s).`, exits 0, without needing `FTPOPS_PASSWORD` set or any network connection (dry-run short-circuits before `connect`)

- [ ] **Step 4: Update README.md**

In `README.md`, add a new subsection under `## Tools` (after the existing `### ftpdiff` subsection):

```markdown
### ftpops

Reads a CSV report produced by `ftpdiff --csv` and performs bulk `copy`
(local<->remote) or `delete` (local/remote) operations, filtered by diff
status (`RemoteOnly`/`LocalOnly`). Example: download every file marked
`RemoteOnly` in a report:

```sh
ftpops copy --to local --filter remote-only --csv report.csv \
  --host ftp.example.com --user myuser --remote-dir /var/www/site --local-dir ./site
```

See `docs/superpowers/specs/2026-09-21-ftpops-design.md` for the design
spec.
```

- [ ] **Step 5: Update CHANGES.md**

Append to the `## Unreleased` section of `CHANGES.md`:

```markdown
- Added `ftpops`, a second tool in the monorepo: reads an `ftpdiff --csv`
  report and performs `copy` (local<->remote) or `delete` (local/remote)
  operations filtered by diff status (`RemoteOnly`/`LocalOnly`), with
  `--dry-run` and `--skip-existing` (copy only). Validates the
  `--filter`/`--to`/`--on` combination before touching the CSV or
  connecting. Password via `--password` > `FTPOPS_PASSWORD` env var >
  interactive prompt, same pattern as ftpdiff. Manually smoke-tested for
  `--help`, the validation error path, and `--dry-run`; not yet
  smoke-tested end-to-end against a real FTP server for the actual
  copy/delete network calls - recommended before relying on it against
  production data.
```

- [ ] **Step 6: Commit**

```bash
git add tools/ftpops/src/main.rs README.md CHANGES.md
git commit -m "ftpops: wire CLI, validation, connection, and operations into main"
```

---

## Self-Review Notes

- **Spec coverage:** CLI shape (Task 5) - `--filter` mapping (Task 2) - combination validation table (Task 4) - CSV parsing/trust-as-is (Task 3) - copy/delete operations incl. `--skip-existing` and `ensure_remote_dir` (Task 6) - colored output + summary (Task 7) - `--dry-run` short-circuit, exit codes, `FTPOPS_PASSWORD` (Task 8). All spec section 3 requirements are covered. "Out of scope for v1" items (other filters, live re-validation, parallelism) are correctly not implemented.
- **Placeholder scan:** all `todo!()` markers are intentional TDD red-step scaffolding, each replaced within the same task. No TBD/TODO comments remain.
- **Type consistency:** `Filter` (Task 2) is used unchanged in `validate.rs` (Task 4), `cli.rs` (Task 5), `main.rs` (Task 8). `CopyTarget`/`DeleteTarget` are defined once in `validate.rs` (Task 4) and re-exported (`pub use`) from `cli.rs` (Task 5) rather than being redefined - avoids two incompatible enums with the same name. `CsvRow` (Task 3) is used unchanged in `ops.rs` (Task 6) and `main.rs` (Task 8). `OpResult`/`OpOutcome` (Task 6) are used unchanged in `output.rs` (Task 7) and `main.rs` (Task 8).
