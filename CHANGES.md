# Changes

## Unreleased

- Set up Cargo workspace structure for the ftp-utils monorepo
  (`crates/ftp-utils-core`, `tools/ftpdiff`).
- Added design spec for the monorepo layout and the ftpdiff tool
  (`docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md`).
- Implemented ftp-utils-core: local/remote directory walking, exclude
  patterns, size-based diff engine, MD5 hash fallback comparison, and a
  real suppaftp-backed `FtpConnection` (FTP + explicit FTPS). Note: server-
  side remote hash commands (XMD5/MD5/HASH) are not attempted in v1, since
  suppaftp does not expose a public API for them; `--hash` always falls
  back to download + local MD5.
- Wired ftpdiff's CLI, config loading, output formatting, and CSV report
  into `main.rs`. ftpdiff v1 is now feature-complete per the design spec:
  recursive local/remote diff by size, optional `--hash` (MD5, always via
  download since suppaftp doesn't expose remote hash commands), `--exclude`
  glob patterns, `--csv` report, FTP/FTPS support, `FTPDIFF_PASSWORD`-only
  credentials, and exit codes 0/1/2.
- Fixed a data-corruption bug found during manual end-to-end testing
  against a local FTP server: the connection defaulted to ASCII transfer
  mode, which rewrites line endings in transit and produced false
  hash/size mismatches for any downloaded file containing `\n`. Fixed by
  switching to binary (`TYPE I`) mode right after login. Verified against
  a local pyftpdlib server: local-only/remote-only/size-mismatch/match/
  hash-match all reported correctly, colored output, CSV report, and exit
  code 1 all worked as expected. `--ftps` was not exercised against a real
  TLS-enabled server (the local test fixture didn't have one configured);
  the code path builds and is covered by the trait-level design, but
  should get a real FTPS smoke test before relying on it in production.
- Added a `--password` CLI flag and an interactive hidden-input prompt
  (via `rpassword`) as a third password source. Precedence is now
  `--password` > `FTPDIFF_PASSWORD` > interactive prompt. The flag is
  documented as the least preferred option (shell history / process
  listing exposure); the env var and prompt remain the recommended ways
  to supply credentials.
- Added `--insecure-tls`: when set with `--ftps`, accepts any TLS
  certificate (expired, self-signed, hostname mismatch) instead of
  validating it. Off by default; documented as removing protection
  against man-in-the-middle attacks, for use only with servers whose
  certificate can't otherwise be validated.
- Added `--verbose`: prints progress diagnostics (connecting, comparison
  start/end with entry count, CSV write) to stderr as ftpdiff runs.
- Extended `--verbose` to also print each file as it's checked: `walk_local_dir`,
  `walk_remote`, and `apply_hash_comparison` in ftp-utils-core now take an
  optional `progress: Option<&mut (dyn FnMut(&str) + '_)>` callback, called
  per included file ("local: <path>", "remote: <path>", "hash: <path>"),
  threaded through `compare()`. Verified via 7 new unit tests (message
  content and ordering across local/remote/hash phases); not re-verified
  against a live FTP server for this change (no local test server
  available in this session) - the earlier live-server smoke test already
  covered the underlying compare() call, and this change is additive to
  its signature only.
- Refactored ftpdiff's connection/config/password handling into a shared
  `ftp_utils_core::connection` module (`ConnectionArgs`, `ConnectionJsonConfig`,
  `merge_connection`, `read_password`), in preparation for the `ftpops`
  tool which needs the same host/user/password/local-dir/remote-dir/ftps/
  insecure-tls handling. No user-visible change to ftpdiff's CLI or config
  file format.
- Extended `FtpConnection` with `store_from_buffer`, `delete`, and
  `create_dir`, plus a default-implemented `ensure_remote_dir` built on
  `list_dir` + `create_dir` (unit-tested via the mock; no live-server
  test yet for `SuppaFtpConnection`'s implementation - recommended before
  the upcoming `ftpops` tool relies on it against production data).
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
- Added `--build` to ftpdiff: scans exactly one side (`--local-dir`, or
  `--host`/`--user`/`--remote-dir`) and writes it to `--csv` without
  comparing - the producer counterpart to `--local-csv`/`--remote-csv`.
  Requires `--csv`; requires exactly one live side; cannot combine with
  `--local-csv`/`--remote-csv`. Entries get a new `DiffStatus::Scan`
  status; with `--hash`, each file's own MD5 is computed unconditionally
  (not just for matches, since nothing is being matched). Exit code is
  always `0` or `2`, never `1`. Manually smoke-tested: local build with
  `--hash`, round-tripping the output back in as `--local-csv`+
  `--remote-csv`, and both error paths (missing `--csv`, both sides
  given).
- Added `OPTIMIZE.md`: prioritized code review TODO list (security, performance, refactoring).
- Reject absolute, `..` and empty paths in CSV rows (ftpops and ftpdiff CSV sources) via new `ftp_utils_core::paths`.
- FTP listing parser now errors on unparseable lines (skipping only blank, `total`, `.`, `..`) and falls back to the DOS format.
- `ftpops copy --skip-existing` now reports a failure instead of overwriting when the remote existence check fails.
- `ftpops delete` now asks for confirmation (`--yes` to skip); aborts with exit code 2 otherwise.
- `ftpops copy --to remote` caches known remote directories and per-directory listings (`ensure_remote_dir_cached`), and creates parents before the `--skip-existing` check.
- Transfers and hashing now stream: added `FtpConnection::retr_to_writer`/`store_from_reader`, `hash::local_md5`/`remote_md5`; hash errors name the file.
- Implemented server-side hashing in `SuppaFtpConnection::try_hash` (`XMD5`, then `MD5`, probed once per connection, digest validated); falls back to streamed download. Stream dispatch now uses a `with_stream!` macro.
- Deduplicated the plain/FTPS connect paths in `SuppaFtpConnection::connect` (generic login helper).
- Added `--timeout` (JSON `timeout`, default 30s): connect, control-channel and data-channel read/write timeouts; added `DEFAULT_FTP_PORT`/`DEFAULT_TIMEOUT_SECS` constants.
- `ftpops copy --to local` downloads to a temporary `.<name>.ftpops-part` file and renames it on success; failures keep the previous file.
- Added `remote::join_remote` (no more `//` for `--remote-dir /` or trailing slashes); local metadata errors name the file.
- Exclude patterns are compiled once into `ExcludeSet`; invalid globs are a config error (exit 2). Directories fully covered by a `<dir>/*` or `<dir>/**` pattern are no longer walked (local and remote).
- Added `DiffEntry::new` and replaced repeated struct literals; refreshed the stale `diff.rs` module doc.
- Introduced `RemoteParams` and `PartialConnection::require_remote`; `RemoteSource::Live`, `BuildSide::Remote` and `EffectiveConnection` now share them; added `SuppaFtpConnection::connect_params`.
- Removed the unused `compare`/`CompareOptions`/`CompareError` pipeline from core (tools compose the tested building blocks; its 2 tests went with it).
- Replaced six copy-pasted message error types with the `message_error!` macro (`ftp_utils_core::error`).
- CSV report reading/writing now lives in core (`csv_source::read_status_rows`/`write_report`) with shared header lookup; `DiffStatus` has `Display`/`FromStr` and ftpops parses statuses into the enum (unknown statuses are errors).
- Split ftpdiff's `run`/`run_build` into small testable functions in `app.rs` (`Result<_, CliError>` with one exit-code mapping in `main`), added shared `connect_remote`, `exit` constants in core, and end-to-end CLI tests (`tools/ftpdiff/tests/cli.rs`). Dropped unused `csv` deps from the tools.
- Refactored ftpops into `app.rs` (shared `load_job`/`preview`/`report`, `Result<_, CliError>`, `EXIT_*` constants) and added end-to-end CLI tests (`tools/ftpops/tests/cli.rs`); a declined delete prompt now reports `Error: aborted, ...`.
- Added integration tests for `SuppaFtpConnection` against an in-process fake FTP server (listing, streaming transfers, delete/mkdir, hash probing, recursion).
- Replaced four hand-written test mocks with one `ftp_utils_core::testing::MockFtpConnection` (behind the `test-utils` feature for the tools' tests).
- Split `Scan` out of `DiffStatus`: new `ReportStatus`/`ReportEntry` in `csv_source` model report rows (comparison or scan); the CSV format is unchanged.
- Added `rustfmt.toml` and a GitHub Actions workflow running `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test`.
- Added `ftpdiff --download-dir <PATH>` (JSON `download_dir`): downloads every non-excluded remote file in binary mode while the remote walk runs (new `remote::walk_remote_with` callback), streamed via a `.part` file renamed on success, skipping files already present with the same size; rejects `..`/absolute paths; needs a live remote side (also works with `--build` on the remote).
- Git hooks: `pre-commit` now only refreshes the codegraph index (optional, never blocks); removed dprint formatting and pnpm lockfile regeneration from the hook and its README.
- Added design spec for `ftpwatch` (`docs/superpowers/specs/2026-10-01-ftpwatch-design.md`): site monitor with `init` (full download + baseline snapshot) and `check` (listing-only scan, size+mtime comparison, text log).
- Remote listings now carry the file modification time (`RawRemoteEntry::modified`, `RemoteEntry::modified`, Unix seconds UTC).
