# ftpdiff CSV-backed comparison sources - Design Spec

Date: 2026-09-21
Status: Approved

## Purpose

Let `ftpdiff` compare against a previously-recorded CSV snapshot (written
by an earlier `ftpdiff --csv` run) instead of a live filesystem scan
and/or a live FTP connection, independently for each side of the
comparison:

- `--local-csv <path>`: use this CSV's recorded local entries instead of
  scanning `--local-dir`.
- `--remote-csv <path>`: use this CSV's recorded remote entries instead
  of connecting to the FTP server and scanning `--remote-dir`.

Both flags are independent and optional. Using `--local-csv` alone still
connects to a live remote server. Using `--remote-csv` alone still scans
a live local directory. Using both means ftpdiff touches neither the
filesystem nor the network - it's a pure CSV-vs-CSV comparison.

Typical uses: diff a remote server against an old snapshot without
needing the actual local files on disk; diff the current local tree
against a recorded remote state without FTP credentials; or diff two
historical snapshots against each other to see what changed between them
(pass the same file as both `--local-csv` and `--remote-csv` on two
different runs, or diff two different report files this way in a small
wrapper script - ftpdiff itself only ever has one local-role input and
one remote-role input per run).

## CLI

Two new flags on `ftpdiff`, both independent of the existing connection
flags:

| Flag | Description |
|---|---|
| `--local-csv <path>` | Read the local side from this CSV's `local_size`/`local_md5` columns instead of scanning `--local-dir` |
| `--remote-csv <path>` | Read the remote side from this CSV's `remote_size`/`remote_md5` columns instead of connecting to the FTP server |

Both are CLI-only (like `ftpops`'s own `--csv`), not settable via the
`--config` JSON file - they describe what a single invocation compares
against, not a reusable connection default.

**Mutually exclusive with their live counterpart:** `--local-csv` and
`--local-dir` cannot both be given; same for `--remote-csv` and
`--remote-dir`/`--host`/`--user`/`--password`/`--ftps`/`--insecure-tls`.
Passing both is a hard error (exit code `2`) before anything runs -
consistent with how `ftpops` rejects invalid `--filter`/`--to`/`--on`
combinations up front.

**Conditional requirements:**

- `--local-dir` is required unless `--local-csv` is given.
- `--host`, `--user`, and `--remote-dir` are required unless
  `--remote-csv` is given. (Password resolution - flag, env var, prompt -
  is skipped entirely when `--remote-csv` is given, since no connection
  is made.)

`--exclude` still applies to entries loaded from a CSV source, exactly
as it does to a live scan - the exclude patterns filter the final entry
list regardless of where it came from.

## Core changes (`ftp-utils-core`)

### New module: `csv_source`

Reads an existing ftpdiff CSV report and reconstructs one side's entry
list plus a side-channel map of already-known MD5 hashes (so a later
`--hash` pass can reuse them instead of recomputing):

```rust
pub struct CsvSourceError(pub String); // Display, std::error::Error

/// Reads `local_size`/`local_md5` for every row that has a `local_size`
/// value, from a CSV report written by `ftpdiff --csv`.
pub fn read_local_entries(path: &Path) -> Result<(Vec<local::LocalEntry>, HashMap<String, String>), CsvSourceError>;

/// Reads `remote_size`/`remote_md5` for every row that has a
/// `remote_size` value, from a CSV report written by `ftpdiff --csv`.
pub fn read_remote_entries(path: &Path) -> Result<(Vec<remote::RemoteEntry>, HashMap<String, String>), CsvSourceError>;
```

A row with an empty size for that side is skipped (it means that side
didn't have a file there in the original report - e.g. a `RemoteOnly`
row has no `local_size`). `LocalEntry`/`RemoteEntry` are unchanged
(`relative_path`, `size`) - the recycled MD5, when present, goes only
into the returned `HashMap<relative_path, md5>`, not into the entry
struct itself, to avoid touching `compare_entries`' signature or the
existing entry types.

### `hash::apply_hash_comparison` signature change

Currently requires a live connection and a live local directory
unconditionally. Changes to make both optional, and to accept the
known-MD5 maps produced by `csv_source`:

```rust
pub fn apply_hash_comparison<C: FtpConnection>(
    conn: Option<&mut C>,
    remote_root: Option<&str>,
    local_root: Option<&Path>,
    local_known_md5: &HashMap<String, String>,
    remote_known_md5: &HashMap<String, String>,
    entries: &mut [DiffEntry],
    progress: Option<&mut (dyn FnMut(&str) + '_)>,
) -> std::io::Result<()>
```

For each `Match` entry, resolving each side's MD5 now follows this order:

1. **Known map first**: if `local_known_md5`/`remote_known_md5` has an
   entry for this path (recycled from a `--local-csv`/`--remote-csv`
   source), use it directly - no live access attempted for that side.
2. **Live fallback**: if not known and a live source is available for
   that side (`conn`+`remote_root` are `Some` for remote;
   `local_root` is `Some` for local), fetch/read it exactly as before
   (try_hash, then download, or read the local file).
3. **Unavailable**: if neither, that side's MD5 stays `None`. If *either*
   side ends up `None`, the entry is left at `Match` (its existing
   size-only result) without attempting a comparison - no error, just a
   less thorough result for that one entry. This only happens when
   `--hash` is combined with `--local-csv`/`--remote-csv` reports that
   were themselves generated without `--hash` (so they have no recorded
   MD5 to recycle) for entries the CSV-sourced side can't independently
   re-derive.

This is a superset of the current behavior: when `conn`/`local_root` are
always `Some` and both known-MD5 maps are empty (the default, fully-live
case), behavior is identical to today.

### `connection::merge_connection_partial` (new, additive)

`ftp-utils-core::connection::merge_connection` (used by `ftpops`
unconditionally, and by `ftpdiff` for the fully-live case) is **not**
changed - it keeps requiring host/user/remote-dir/local-dir, so `ftpops`
is unaffected by this feature. A new function is added alongside it,
which `merge_connection` itself is refactored to call internally (no
behavior change, just extracted):

```rust
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
/// field to be present (`host`/`user`/`remote_dir`/`local_dir` stay
/// `Option`). `merge_connection` builds on this and additionally
/// requires those four fields; callers that need conditional
/// requirements (like ftpdiff's CSV-source flags) use this directly.
pub fn merge_connection_partial(args: &ConnectionArgs, json: &ConnectionJsonConfig) -> PartialConnection;
```

## `tools/ftpdiff` changes

### `cli.rs`

Adds `local_csv: Option<PathBuf>` (`--local-csv`) and
`remote_csv: Option<PathBuf>` (`--remote-csv`) to `Cli`.

### `config.rs`

`EffectiveConfig` changes shape: instead of a single flat
`connection: EffectiveConnection`, it now has two independent source
descriptions:

```rust
pub enum LocalSource {
    Live(PathBuf),
    Csv(PathBuf),
}

pub enum RemoteSource {
    Live { host: String, port: u16, user: String, remote_dir: String, ftps: bool, insecure_tls: bool },
    Csv(PathBuf),
}

pub struct EffectiveConfig {
    pub local: LocalSource,
    pub remote: RemoteSource,
    pub hash: bool,
    pub verbose: bool,
    pub exclude: Vec<String>,
    pub csv: Option<PathBuf>,
}
```

`merge()` calls `connection::merge_connection_partial`, then:

1. Errors (exit code `2`, before Step 2) if `cli.local_csv.is_some() &&
   cli.connection.local_dir.is_some()` ("--local-dir and --local-csv are
   mutually exclusive"), and symmetrically if `cli.remote_csv.is_some()`
   and any of `remote_dir`/`host`/`user`/`password`/`ftps`/
   `insecure_tls` was explicitly set on the CLI or JSON side.
2. Builds `LocalSource`: `Csv(path)` if `--local-csv` given; else
   `Live(local_dir)`, erroring if `local_dir` couldn't be resolved.
3. Builds `RemoteSource`: `Csv(path)` if `--remote-csv` given; else
   `Live { host, user, remote_dir, port, ftps, insecure_tls }`, erroring
   if `host`/`user`/`remote_dir` couldn't be resolved.

### `main.rs`

Orchestration is restructured to branch on `LocalSource`/`RemoteSource`
instead of always calling the existing `ftp_utils_core::compare()`
convenience wrapper (which stays in `ftp-utils-core` unchanged, for any
future fully-live caller, but stops being used by ftpdiff once this
lands - `compare()`'s internal logic becomes ftpdiff's manual
orchestration, applied more flexibly):

1. Resolve `local_entries: Vec<LocalEntry>` and `local_known_md5:
   HashMap<String, String>`: from `csv_source::read_local_entries` if
   `LocalSource::Csv`, else `local::walk_local_dir` if `LocalSource::Live`
   (in which case `local_known_md5` stays empty). Apply `--exclude` to
   CSV-sourced entries the same way `walk_local_dir` already does for
   live ones (`exclude::is_excluded`).
2. If `RemoteSource::Live { .. }`: resolve the password (flag > env var >
   prompt) and connect via `SuppaFtpConnection`. If `RemoteSource::Csv`:
   skip password resolution and connection entirely.
3. Resolve `remote_entries: Vec<RemoteEntry>` and `remote_known_md5`:
   from `csv_source::read_remote_entries` if `RemoteSource::Csv` (filtered
   by `--exclude` the same way), else `remote::walk_remote` against the
   live connection if `RemoteSource::Live`.
4. `compare::compare_entries(&local_entries, &remote_entries)` - unchanged.
5. If `--hash`: `hash::apply_hash_comparison` with `conn` as
   `Some(&mut connection)` only when `RemoteSource::Live` (else `None`,
   using `SuppaFtpConnection` as the turbofish type so the call still
   type-checks with no live connection ever constructed),
   `remote_root`/`local_root` as `Some(..)` only for the `Live` variant of
   each side, and the two known-MD5 maps built in steps 1 and 3.
6. If `RemoteSource::Live`, close the connection.
7. Output (colored text, summary, optional `--csv` write) is unchanged -
   it only looks at the final `Vec<DiffEntry>`.

`--verbose` diagnostics adjust: for a `Live` side, the existing
"Connecting to..." / per-file "Checking local/remote: ..." messages stay
as they are. For a `Csv` side, ftpdiff instead prints one line when that
side is loaded: `Loaded N local entries from <path>.` /
`Loaded N remote entries from <path>.` - no per-file messages, since
nothing is being walked.

## Interaction with `--csv` (output)

Unaffected: `ftpdiff --csv <path>` always writes the final `Vec<DiffEntry>`
in the same format, regardless of whether either input side came from a
live scan or a CSV source. This is what makes chaining possible - the
output of one CSV-sourced run can be fed as the `--local-csv`/
`--remote-csv` input of a later run.

## Testing

- `ftp-utils-core::csv_source`: unit tests for `read_local_entries`/
  `read_remote_entries` - rows with/without a size for that side, rows
  with/without an md5, malformed size values (error), missing required
  columns (error).
- `ftp-utils-core::hash`: existing tests continue to pass with `Some`
  values substituted for the now-optional `conn`/`local_root` params and
  empty maps for the two new map params (no behavior change for the
  fully-live case). New tests: known-md5 map short-circuits live access
  (mock `conn`/local file absent, but comparison still resolves
  correctly from the map); missing local file *and* no known local md5
  leaves the entry at `Match` without erroring.
- `ftp-utils-core::connection`: new tests for `merge_connection_partial`
  (no required-field errors, still applies CLI>JSON>default precedence
  and OR-merge for booleans); existing `merge_connection` tests
  unchanged (it's a thin wrapper now, same external behavior).
- `ftpdiff::config`: tests for the new `LocalSource`/`RemoteSource`
  resolution logic - `--local-csv` alone (local-dir not required, still
  required for remote), `--remote-csv` alone (remote fields not
  required), both together (nothing required except the file paths
  themselves), the four mutual-exclusion error cases.
- `ftpdiff::main`: no new automated test (orchestration wiring, same
  pattern as the rest of `main.rs`); manual smoke test recommended for
  all four `Live`/`Csv` combinations before relying on this against
  production data, following the same pattern used for the original
  ftpdiff/ftpops manual smoke tests.

## Out of scope for v1

- Validating that a `--local-csv`/`--remote-csv` file was actually
  produced by `ftpdiff --csv` (any CSV with compatible column names is
  accepted).
- Re-validating a CSV-sourced side against live state (that's the
  opposite of the point of this feature).
- A dedicated "diff two CSV snapshots directly" subcommand/mode; the same
  effect is achieved by using both `--local-csv` and `--remote-csv` in
  one invocation.
