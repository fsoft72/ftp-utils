# ftpops - Design Spec

Date: 2026-09-21
Status: Approved

## Purpose

Add a second tool, `ftpops`, to the `ftp-utils` monorepo. `ftpops` reads a
CSV report produced by `ftpdiff --csv` and performs bulk file operations
based on it: copying files between local and remote (in either direction)
and deleting files, filtered by diff status. Typical use case: "download
locally every file marked `RemoteOnly` in the ftpdiff report."

This spec also covers extracting the CLI/config/password logic that
`ftpdiff` already has into a shared module in `ftp-utils-core`, since
`ftpops` needs the same connection options and the user asked for the two
tools to be able to share one JSON config file.

## Workspace layout change

```
ftp-utils/
├── crates/
│   └── ftp-utils-core/
│       └── src/
│           ├── connection.rs   # NEW: shared CLI/config/password types
│           └── ... (existing modules, remote.rs extended - see below)
└── tools/
    ├── ftpdiff/                 # refactored to use ftp-utils-core::connection
    └── ftpops/                   # NEW
```

## 1. Shared connection/config module (`ftp-utils-core::connection`)

Extracted from `ftpdiff`'s existing `cli.rs`/`config.rs` (both tools need
identical host/port/user/password/local-dir/remote-dir/ftps/insecure-tls/
config-file handling):

- `pub struct ConnectionArgs` - a `clap::Args` (not `Parser`) struct with
  the shared flags, meant to be `#[command(flatten)]`ed into each tool's
  own `Cli` struct: `config: Option<PathBuf>`, `host: Option<String>`,
  `port: Option<u16>`, `user: Option<String>`, `remote_dir: Option<String>`,
  `local_dir: Option<PathBuf>`, `ftps: bool`, `insecure_tls: bool`,
  `password: Option<String>`.
- `pub struct ConnectionJsonConfig` (`serde::Deserialize`, `Default`) -
  the shared JSON fields: `host`, `port`, `user`, `remote_dir`,
  `local_dir`, `ftps`, `insecure_tls`. Tool-specific JSON fields (ftpdiff's
  `hash`, `verbose`, `exclude`; ftpops's own, if any - none needed for v1)
  stay in each tool's own JSON config struct, which embeds
  `ConnectionJsonConfig` via `#[serde(flatten)]`.
- `pub struct EffectiveConnection { host: String, port: u16, user: String,
  remote_dir: String, local_dir: PathBuf, ftps: bool, insecure_tls: bool }`
- `pub fn load_json_config<T: DeserializeOwned>(path: &Path) -> Result<T,
  ConnectionError>` - generic loader, used by both tools with their own
  JSON struct type.
- `pub fn merge_connection(args: &ConnectionArgs, json: &
  ConnectionJsonConfig) -> Result<EffectiveConnection, ConnectionError>` -
  same precedence rules as today (CLI > JSON > default; booleans OR;
  port defaults to 21).
- `pub fn read_password(cli_password: Option<&str>, env_var: &str) ->
  Result<String, ConnectionError>` - same three-source resolution
  (`--password` > env var > interactive `rpassword` prompt). `env_var` is
  now a parameter instead of a hardcoded name, since `ftpops` uses its own
  environment variable (see below) rather than reusing `FTPDIFF_PASSWORD`
  for a differently-named binary.
- `pub struct ConnectionError(pub String)` (`Display`, `std::error::Error`).

**ftpdiff changes:** `tools/ftpdiff/src/cli.rs` keeps only `hash: bool`,
`exclude: Vec<String>`, `csv: Option<PathBuf>`, `verbose: bool`, plus
`#[command(flatten)] connection: ConnectionArgs`. `tools/ftpdiff/src/
config.rs` keeps only `JsonConfig { #[serde(flatten)] connection:
ConnectionJsonConfig, hash: Option<bool>, verbose: Option<bool>, exclude:
Vec<String> }` and a thin `merge()` that calls `merge_connection` plus
resolves the ftpdiff-only fields. Behavior is unchanged; this is a pure
refactor. `read_password` call site passes `"FTPDIFF_PASSWORD"` as before.

Both tools' JSON config files remain compatible with each other: a single
JSON file with `host`/`user`/`remote_dir`/`local_dir`/etc. works for both
`ftpdiff --config` and `ftpops --config`, since the shared fields use the
same names; each tool simply ignores JSON fields it doesn't declare
(`serde` default behavior, no `deny_unknown_fields`).

## 2. `FtpConnection` trait extension (`ftp-utils-core::remote`)

Three new methods, implemented for `SuppaFtpConnection` in `ftp_client.rs`
and added to the test `MockFtpConnection`s wherever they're defined:

```rust
pub trait FtpConnection {
    // ... existing methods unchanged ...

    /// Uploads `data` to `path`, overwriting any existing remote file.
    fn store_from_buffer(&mut self, path: &str, data: &[u8]) -> Result<(), FtpConnectionError>;

    /// Deletes the remote file at `path`.
    fn delete(&mut self, path: &str) -> Result<(), FtpConnectionError>;

    /// Ensures every directory component of `path` exists on the server,
    /// creating any that are missing. `path` is the full remote path to a
    /// file; only its parent directories are created.
    fn ensure_remote_dir(&mut self, path: &str) -> Result<(), FtpConnectionError>;
}
```

`SuppaFtpConnection` implements:
- `store_from_buffer` via `suppaftp`'s `put_file` (or `put_with_stream`
  with a `Cursor<&[u8]>`).
- `delete` via `suppaftp`'s `rm`.
- `ensure_remote_dir` by splitting the parent path into components and
  calling `mkdir` on each intermediate path in order, treating a "already
  exists" error response as success (suppaftp has no native recursive
  mkdir). If a `mkdir` fails for a reason other than "already exists",
  the whole call fails with that error.

## 3. `tools/ftpops`

### CLI

Two subcommands, both taking `#[command(flatten)] connection:
ConnectionArgs` plus `--csv <path>` (required) and `--dry-run`:

```
ftpops copy --to <local|remote> --filter <remote-only|local-only> --csv <path> [--skip-existing] [--dry-run] <connection args>
ftpops delete --on <local|remote> --filter <remote-only|local-only> --csv <path> [--dry-run] <connection args>
```

- `--filter` is a small enum: `remote-only` | `local-only`, mapped to the
  ftpdiff CSV `status` column values `"RemoteOnly"` / `"LocalOnly"`
  (exact string match against the `Debug` format `ftp-utils-core`
  already writes).
- `--skip-existing` (copy only): if the destination already exists, skip
  the file (print a message) instead of overwriting it. Default (flag
  absent): always overwrite.
- `--dry-run`: print every action that would be taken (`Would copy:
  <path>`, `Would delete: <path>`) and a final count, without performing
  any of them. Default (flag absent): execute immediately.

### Combination validation

Only one filter is valid per operation+direction; any other combination
is a hard error before anything runs:

| Command | Valid `--filter` | Why |
|---|---|---|
| `copy --to local` | `remote-only` | only `RemoteOnly` entries have a remote file to download and no local file to conflict with |
| `copy --to remote` | `local-only` | only `LocalOnly` entries have a local file to upload and no remote file yet |
| `delete --on remote` | `remote-only` | only `RemoteOnly` entries exist on the remote side to delete |
| `delete --on local` | `local-only` | only `LocalOnly` entries exist locally to delete |

Passing e.g. `copy --to local --filter local-only` fails immediately with
an error explaining the mismatch (`"--filter local-only doesn't apply to
'copy --to local': LocalOnly entries have no remote file to download from
- did you mean --filter remote-only, or copy --to remote?"`), exit code 2,
no CSV is even read.

### CSV parsing

Reads the file at `--csv` with the `csv` crate, expecting the header
`path,status,local_size,remote_size,local_md5,remote_md5` that `ftpdiff
--csv` writes. Rows are filtered by exact match on the `status` column
against the value implied by `--filter`. The CSV is trusted as-is at
execution time - ftpops does not re-diff against live state before
acting; if the report is stale, the user is expected to re-run `ftpdiff
--csv` first (documented in `--help` and the README).

### Operations

- **`copy --to local`**: for each filtered row, download the remote file
  (`retr_to_buffer` at `<remote-dir>/<relative_path>`), create local
  parent directories as needed (`std::fs::create_dir_all`), write the
  file at `<local-dir>/<relative_path>`. If `--skip-existing` and the
  local file already exists, skip with a message instead.
- **`copy --to remote`**: for each filtered row, read the local file at
  `<local-dir>/<relative_path>`, ensure remote parent directories exist
  (`ensure_remote_dir`), upload (`store_from_buffer`) to
  `<remote-dir>/<relative_path>`. If `--skip-existing`, first check
  whether the remote file exists (via `list_dir` on its parent directory)
  and skip with a message if so.
- **`delete --on remote`**: for each filtered row, delete the remote file
  at `<remote-dir>/<relative_path>` (`delete`).
- **`delete --on local`**: for each filtered row, delete the local file at
  `<local-dir>/<relative_path>` (`std::fs::remove_file`).

Each operation is attempted independently; one file failing (e.g. deleted
by something else between the CSV being generated and ftpops running)
does not stop the rest - it's reported as a per-file error and counted in
the summary.

### Output

Colored text per file (`+` copied, `-` deleted, `!` failed, reusing the
same color conventions as `ftpdiff`'s output module where sensible), plus
a final summary line with counts (succeeded / skipped / failed). In
`--dry-run` mode, the same per-file lines are prefixed `Would `.

### Exit codes

- `0`: all operations succeeded (or, in `--dry-run`, the plan was printed
  successfully).
- `1`: at least one per-file operation failed (partial failure).
- `2`: runtime/setup error (CSV unreadable or malformed, connection
  failure, invalid `--filter`/`--to`/`--on` combination, missing
  required connection setting).

### Password

Same three-source resolution as `ftpdiff` (`--password` > env var >
interactive prompt), using its own environment variable `FTPOPS_PASSWORD`
(not `FTPDIFF_PASSWORD` - different binary, own env var, consistent with
each tool documenting its own credential source; a shared JSON config
file can still supply `host`/`user`/etc. to both).

## Testing

- `ftp-utils-core::connection`: unit tests for `merge_connection`
  precedence (mirroring the existing `ftpdiff` config tests) and
  `read_password` precedence, moved from `ftpdiff`'s current test suite.
- `ftp-utils-core::remote`: unit tests for `ensure_remote_dir`'s
  component-splitting and "already exists" tolerance, using an extended
  `MockFtpConnection`.
- `ftpops`: unit tests for CSV filtering (`status` matching), the
  combination-validation table above, and the copy/delete operations
  themselves against a mock `FtpConnection` and a temp local directory
  (no real FTP server needed, same pattern as `ftpdiff`'s `hash.rs`
  tests). A manual end-to-end smoke test against a local FTP server
  (same pattern used for `ftpdiff`) is recommended before relying on
  `ftpops` against production data, but is not required to complete the
  implementation plan.

## Out of scope for v1

- Filters other than `RemoteOnly`/`LocalOnly` (e.g. re-copying
  `SizeMismatch`/`HashMismatch` entries) - not requested, can be added
  later by extending the `--filter` enum.
- Re-validating the CSV against live state before acting.
- Parallelism (operations run sequentially, one file at a time).
