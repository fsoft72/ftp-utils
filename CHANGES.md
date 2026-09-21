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
