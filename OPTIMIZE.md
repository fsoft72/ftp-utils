# Optimization TODO

> Generated on 2026-09-30. Items sorted by importance.

## Critical

- [x] **Validate CSV `relative_path` against path traversal** - `ftpops` joins the CSV `path` column straight onto `local_dir`/`remote_dir`, so a row like `../../etc/x` or an absolute path (`Path::join` replaces the base for absolute paths) lets a tampered or hand-edited CSV read, overwrite or delete files outside the target directory (`delete --on local` calls `remove_file` on it). Reject absolute paths, `..` components and empty paths when reading rows (and in `csv_source`), and fail fast with the offending row.
  - File(s): `tools/ftpops/src/csv_input.rs`, `tools/ftpops/src/ops.rs`, `crates/ftp-utils-core/src/csv_source.rs`
- [x] **Do not silently drop unparseable FTP listing lines** - `list_dir` does `let Ok(..) = parse_posix(..) else { continue }`, so any line the parser rejects (unusual server format, symlink, odd filename) vanishes from the remote tree. The diff then reports false `LocalOnly` entries, which `ftpops copy --to remote --filter local-only` will happily re-upload or `delete --on local` will remove. Return an error (or at least a warning counted in the summary) for lines that cannot be parsed, and consider `MLSD` when the server supports it.
  - File(s): `crates/ftp-utils-core/src/ftp_client.rs`
- [x] **`--skip-existing` overwrites on listing errors** - in `copy_to_remote`, `list_dir(..).unwrap_or(false)` treats a failed listing as "file does not exist", so a transient error defeats the very flag meant to prevent overwrites. Propagate the error as `OpOutcome::Failed` instead of swallowing it.
  - File(s): `tools/ftpops/src/ops.rs`

## High

- [x] **Destructive `delete` has no confirmation and trusts a possibly stale CSV** - `ftpops delete` acts immediately on rows from a report that may be days old, without re-checking that the file still has the recorded status. Add an interactive confirmation (with a `--yes` flag for scripts) and print the count and target before deleting.
  - File(s): `tools/ftpops/src/main.rs`
- [x] **Stream transfers and hashes instead of buffering whole files** - `retr_to_buffer`, `store_from_buffer`, `std::fs::read` + `md5::compute` load each file completely into memory, so a multi-GB file exhausts RAM. Use `md5::Context` with chunked reads locally, and stream remote downloads/uploads (`retr` with a reader, `put_file` from a `File`).
  - File(s): `crates/ftp-utils-core/src/hash.rs`, `crates/ftp-utils-core/src/ftp_client.rs`, `crates/ftp-utils-core/src/remote.rs`, `tools/ftpops/src/ops.rs`, `tools/ftpdiff/src/main.rs`
- [x] **`try_hash` is a stub, every remote hash downloads the whole file** - the trait method always returns `None`, so `--hash` re-downloads every size-matching file. Implement `XMD5`/`MD5`/`XSHA`-style commands via suppaftp's custom command support (or remove the method and document the download cost) and cache the capability probe per connection.
  - File(s): `crates/ftp-utils-core/src/ftp_client.rs`, `crates/ftp-utils-core/src/remote.rs`
- [x] **`ensure_remote_dir` issues a `LIST` per path component per uploaded file** - uploading N files of depth D costs O(N*D) round trips. Keep a `HashSet` of directories already known to exist (per run) and only list/create on a miss, or just try `MKD` and ignore "already exists".
  - File(s): `crates/ftp-utils-core/src/remote.rs`, `tools/ftpops/src/ops.rs`
- [x] **`copy_to_remote --skip-existing` lists the parent directory for every row** - the same directory is listed again for each file. Group rows by parent and list each directory once.
  - File(s): `tools/ftpops/src/ops.rs`
- [ ] **Split `ftpdiff::run` / `run_build` god functions** - `run` (~170 lines) mixes config loading, live/CSV source loading, connecting, hashing, printing and CSV writing with a repeated `match ... { Err(e) => { eprintln!; return 2 } }` pattern. Extract `load_local_side`, `load_remote_side`, `report` and use a `Result<_, CliError>` with `?` and a single exit-code mapping in `main`.
  - File(s): `tools/ftpdiff/src/main.rs`
- [ ] **Duplicated connect/password/hash logic between `run`, `run_build` and core** - the connect block appears twice in `main.rs`, and the "try_hash, else download and MD5" logic exists in both `hash.rs` and `run_build`. Add a single `connect_remote(...)` helper and a shared `hash::remote_md5(conn, path)` / `hash::local_md5(path)` in core.
  - File(s): `tools/ftpdiff/src/main.rs`, `crates/ftp-utils-core/src/hash.rs`
- [x] **Unused public API in core duplicates the tool's pipeline** - `ftp_utils_core::compare`, `CompareOptions` and `CompareError` are not used by any tool (ftpdiff reimplements the flow in `main.rs`), so the tested path is not the shipped path. Either make `ftpdiff` call `compare` (extended to accept CSV sources) or delete it.
  - File(s): `crates/ftp-utils-core/src/lib.rs`, `tools/ftpdiff/src/main.rs`

## Medium

- [x] **Invalid exclude globs are silently ignored and recompiled per file** - `is_excluded` calls `glob::Pattern::new` for every pattern on every path and maps an invalid pattern to `false`, so a typo silently excludes nothing. Compile patterns once into an `ExcludeSet` at startup and fail with a clear error on invalid ones.
  - File(s): `crates/ftp-utils-core/src/exclude.rs`, `crates/ftp-utils-core/src/local.rs`, `crates/ftp-utils-core/src/remote.rs`
- [x] **Excluded directories are still traversed** - excludes are only tested on files, so `--exclude ".git/*"` still lists/walks the entire `.git` tree (a lot of FTP round trips remotely). Test directories too and skip descending (`WalkDir::filter_entry`, and check before pushing to `dirs_to_visit`).
  - File(s): `crates/ftp-utils-core/src/local.rs`, `crates/ftp-utils-core/src/remote.rs`
- [x] **Repeated string-newtype error types** - `ConnectionError`, `FtpConnectionError`, `CsvSourceError`, `CsvError` and `ValidationError` are identical `pub struct X(pub String)` with copy-pasted `Display`/`Error` impls, and context is lost by stringifying (`e.to_string()`). Introduce one error enum (e.g. with `thiserror`, or a small hand-written one) with source chaining. *Done: deduplicated with the `message_error!` macro. Source chaining was deliberately not added: every error is terminal and printed once with its full context, and no dependency (`thiserror`) is justified for that.*
  - File(s): `crates/ftp-utils-core/src/connection.rs`, `crates/ftp-utils-core/src/remote.rs`, `crates/ftp-utils-core/src/csv_source.rs`, `tools/ftpops/src/csv_input.rs`, `tools/ftpops/src/validate.rs`
- [ ] **CSV header lookup duplicated and the CSV schema is coupled to `Debug` output** - `csv_source::read_side` and `csv_input::read_rows` repeat the same column-position logic, and the status column relies on `format!("{:?}", status)` in the writer and string literals in `filter.rs`. Move CSV read/write (and a `Display`/`FromStr` for `DiffStatus`) into core so writer and readers share one definition, and parse the status into the enum in `ftpops`.
  - File(s): `crates/ftp-utils-core/src/csv_source.rs`, `tools/ftpops/src/csv_input.rs`, `tools/ftpops/src/filter.rs`, `tools/ftpdiff/src/csv_report.rs`
- [x] **Remote and connection settings duplicated across types** - `RemoteSource::Live`, `BuildSide::Remote`, `PartialConnection` and `EffectiveConnection` all repeat host/port/user/remote_dir/ftps/insecure_tls, and `merge`/`merge_build` repeat the "require host, user, remote-dir" checks. Extract a `RemoteParams` struct and one `require_remote(&PartialConnection)` function.
  - File(s): `tools/ftpdiff/src/config.rs`, `crates/ftp-utils-core/src/connection.rs`
- [x] **`DiffEntry` construction repeated four times** - `compare_entries` builds nearly identical literals with `None` md5 fields for each case, and tests do the same. Add `DiffEntry::new(path, status, local_size, remote_size)` (or per-status constructors).
  - File(s): `crates/ftp-utils-core/src/compare.rs`, `crates/ftp-utils-core/src/diff.rs`, `tools/ftpdiff/src/main.rs`
- [x] **FTPS and plain connect branches are copy-pasted** - the two arms of `SuppaFtpConnection::connect` (login, `transfer_type`) and every `match self { Plain(..) => .., Tls(..) => .. }` in the trait impl duplicate code. Use a small macro or a helper that dispatches on the enum once.
  - File(s): `crates/ftp-utils-core/src/ftp_client.rs`
- [x] **No connect/read timeouts** - `FtpStream::connect` blocks indefinitely on a dead host or stalled transfer. Use `connect_timeout` and set read/write timeouts on the control and data channels, exposed as a `--timeout` option.
  - File(s): `crates/ftp-utils-core/src/ftp_client.rs`, `crates/ftp-utils-core/src/connection.rs`
- [x] **Non-atomic local writes in `copy_to_local`** - a failed or interrupted download leaves a truncated file at the final path, which later looks like a size mismatch or, with `--skip-existing`, is skipped as "already exists". Write to a temporary file in the same directory and `rename` on success.
  - File(s): `tools/ftpops/src/ops.rs`
- [x] **Remote path joining produces `//` and loses error context** - `format!("{root}/{rel}")` yields `//a` for `--remote-dir /` and `/x//a` for a trailing slash; I/O errors in hashing (`fs::read`, `walkdir`) do not name the file. Add a `join_remote(root, rel)` helper and wrap errors with the path.
  - File(s): `crates/ftp-utils-core/src/remote.rs`, `crates/ftp-utils-core/src/hash.rs`, `crates/ftp-utils-core/src/local.rs`, `tools/ftpops/src/ops.rs`

## Low / Nice to have

- [ ] **Share the mock `FtpConnection` across tests** - the same `MockConnection` (all six trait methods) is re-implemented in `remote.rs`, `hash.rs`, `lib.rs` and `ops.rs` tests. Provide one configurable mock behind a `test-utils` feature or `#[cfg(test)]` module.
  - File(s): `crates/ftp-utils-core/src/remote.rs`, `crates/ftp-utils-core/src/hash.rs`, `crates/ftp-utils-core/src/lib.rs`, `tools/ftpops/src/ops.rs`
- [ ] **Untested orchestration and FTP client** - `ftp_client.rs`, `ftpdiff/main.rs` and `ftpops/main.rs` have no tests, so exit-code behavior and the CSV/live wiring are only covered manually. Once `run` is split, add integration tests (e.g. against an in-process FTP server such as `libunftp`) and CLI tests with `assert_cmd`.
  - File(s): `crates/ftp-utils-core/src/ftp_client.rs`, `tools/ftpdiff/src/main.rs`, `tools/ftpops/src/main.rs`
- [ ] **Duplicated `run_copy` / `run_delete` skeleton and magic exit codes** - both load config, read/filter rows, handle dry-run, print results and map failures to `0/1`, and the numbers `0/1/2` are scattered literals across both binaries. Define `const EXIT_OK/EXIT_DIFF/EXIT_ERROR` (shared in core) and factor the shared flow.
  - File(s): `tools/ftpops/src/main.rs`, `tools/ftpdiff/src/main.rs`
- [x] **Stale module doc and default port literal** - `diff.rs` still says comparison "will be implemented according to the implementation plan", and the default port `21` is a bare literal in `merge_connection_partial`. Update the doc and add `const DEFAULT_FTP_PORT: u16 = 21;`.
  - File(s): `crates/ftp-utils-core/src/diff.rs`, `crates/ftp-utils-core/src/connection.rs`
- [ ] **`DiffStatus::Scan` leaks into compare-only code** - `summarize` needs a dead `Scan => {}` arm and `format_entry` handles a status that `compare_entries` never produces. Model build output as its own type (or split the enum) so each mode only sees valid states.
  - File(s): `crates/ftp-utils-core/src/diff.rs`, `tools/ftpdiff/src/output.rs`
- [ ] **No CI or lint/format configuration** - no `.github/` workflows, `rustfmt.toml` or clippy settings, and `io::Error::new(ErrorKind::Other, ..)` (replaceable with `io::Error::other`) is the kind of thing clippy would flag. Add a CI job running `cargo fmt --check`, `cargo clippy -- -D warnings` and `cargo test`.
  - File(s): repository root
