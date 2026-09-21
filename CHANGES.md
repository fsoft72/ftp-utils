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
