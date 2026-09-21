# ftp-utils monorepo + ftpdiff - Design Spec

Date: 2026-09-21
Status: Approved

## Purpose

Create a Rust monorepo (`ftp-utils`) that will host a suite of FTP-related
CLI tools. The first tool, `ftpdiff`, compares a remote directory tree
(over FTP/FTPS) against a local directory tree and reports differences
based on file size and, optionally, content hash.

## Workspace layout

```
ftp-utils/
├── Cargo.toml              # workspace root (resolver = "2", edition 2021)
├── CHANGES.md
├── LICENSE                  # MIT
├── README.md
├── .gitignore
├── crates/
│   └── ftp-utils-core/       # shared library
└── tools/
    └── ftpdiff/               # first binary tool
```

- `crates/` holds shared library crates usable by any future tool.
- `tools/` holds binary crates, one per CLI tool.
- Future tools (e.g. `ftpsync`, `ftpwatch`) will be added as new members
  under `tools/`, reusing `ftp-utils-core`.

## crates/ftp-utils-core

Shared library responsible for:

- FTP/FTPS client wrapper built on top of the `suppaftp` crate (supports
  both plain FTP and explicit FTPS/AUTH TLS).
- Domain types: `LocalEntry`, `RemoteEntry`, `DiffEntry`, `DiffStatus`.
- Recursive directory walking:
  - Local: via `std::fs` / `walkdir`.
  - Remote: via FTP `LIST`/`MLSD`, recursing into subdirectories.
- Exclude pattern matching using glob patterns (`glob` crate), applied to
  relative paths on both local and remote sides before comparison.
- Diff/comparison logic producing a list of `DiffEntry { relative_path,
  status, local_size, remote_size, local_md5, remote_md5 }`.
- Hash strategy (used only when hashing is enabled by the caller):
  1. For files with matching size, attempt non-standard FTP hash
     commands (XMD5, MD5, HASH) on the remote file.
  2. If the server does not support any of these commands, download the
     remote file into memory/temp and compute MD5 locally, comparing
     against the local file's MD5.
  3. If hashing is disabled entirely, comparison is size-only.

This crate has no CLI/output concerns - it exposes a programmatic API
that `ftpdiff` (and future tools) call into.

## tools/ftpdiff

Thin binary crate providing the CLI, built with `clap`.

### Configuration precedence

`CLI args > JSON config file > built-in defaults`

- `--config <path>`: optional path to a JSON config file providing
  defaults for host, port, user, remote-dir, local-dir, exclude
  patterns, ftps, hash.
- Any CLI flag explicitly provided overrides the corresponding JSON
  value.
- The FTP password is **never** accepted via JSON config (to avoid
  leaking credentials via config files committed to disk). It is
  resolved in this order: `--password` CLI flag > `FTPDIFF_PASSWORD`
  environment variable > interactive hidden-input terminal prompt.
  **Amendment (2026-09-21):** the original spec disallowed a CLI flag
  entirely; a `--password` flag was added afterward at the user's
  request, documented as the least preferred source since it can leak
  into shell history and process listings - the env var and the prompt
  remain the recommended ways to supply credentials.

### CLI flags

| Flag | Description |
|---|---|
| `--config <path>` | Optional JSON config file with defaults |
| `--host <host>` | FTP server hostname |
| `--port <port>` | FTP server port (default 21, or 990 for FTPS if unset) |
| `--user <user>` | FTP username |
| `--remote-dir <path>` | Remote directory to compare |
| `--local-dir <path>` | Local directory to compare |
| `--ftps` | Use FTPS (explicit AUTH TLS) instead of plain FTP |
| `--hash` | Enable hash comparison (remote hash command, fallback to download+MD5) |
| `--exclude <glob>` | Glob pattern to exclude from comparison (repeatable) |
| `--csv <path>` | Also write a structured CSV report to this path |

Password: `FTPDIFF_PASSWORD` environment variable (required at runtime,
not a flag).

### JSON config file shape

```json
{
  "host": "ftp.example.com",
  "port": 21,
  "user": "myuser",
  "remote_dir": "/var/www/site",
  "local_dir": "./site",
  "ftps": false,
  "hash": false,
  "exclude": [".git/*", "*.tmp"]
}
```

All fields optional; CLI flags override any field present here.

### Comparison flow

1. Load config (JSON defaults, then apply CLI overrides).
2. Read password from `FTPDIFF_PASSWORD`; fail fast with a clear error
   if missing.
3. Connect to the FTP/FTPS server via `ftp-utils-core`.
4. Recursively walk local dir and remote dir.
5. Apply `--exclude` glob patterns to both sides.
6. Compute diff entries via `ftp-utils-core`, categorized as:
   - `LocalOnly` - file exists locally but not remotely.
   - `RemoteOnly` - file exists remotely but not locally.
   - `SizeMismatch` - file exists on both sides, sizes differ.
   - `HashMismatch` - sizes match but hashes differ (`--hash` only).
   - `Match` - file matches (size, or size+hash when `--hash` is set).
7. Output:
   - Always: human-readable colored text on stdout, with a summary
     count per status at the end.
   - If `--csv <path>` given: also write a CSV file with columns
     `path,status,local_size,remote_size,local_md5,remote_md5`.

### Exit codes

- `0`: no differences found (all `Match`).
- `1`: differences found (any non-`Match` entry).
- `2`: runtime error (connection failure, missing password, invalid
  config, etc).

## Error handling

- Fail fast with descriptive, contextual error messages (connection
  errors, auth failures, missing local/remote dir, unsupported hash
  command with `--hash` falling back automatically rather than erroring).
- No silent failures: any file that can't be listed/read is reported as
  an error entry in the output, not skipped silently.

## Testing

- `ftp-utils-core`: unit tests for diff logic, exclude pattern matching,
  hash fallback logic, using in-memory/mock data structures (no real
  FTP server required for these).
- `ftpdiff`: integration tests using a local FTP server for CI (to be
  decided in the implementation plan - candidates: `pyftpdlib` in a
  test fixture, or a Rust-based embedded test FTP server).

## Out of scope for v1

- Sync/upload capability (this is a diff/report tool only, read-only).
- Tools other than `ftpdiff` (future work, same monorepo).
- SFTP/SCP support (FTP/FTPS only).
