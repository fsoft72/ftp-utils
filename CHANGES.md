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
