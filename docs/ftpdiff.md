# ftpdiff

Compares a remote directory tree (over FTP/FTPS) against a local copy,
recursively, and reports differences by file size and, optionally,
content (MD5) hash.

## Usage

```sh
ftpdiff --host <host> --user <user> --remote-dir <path> --local-dir <path> [OPTIONS]
```

All connection settings can also come from a `--config` JSON file - see
[`docs/json.md`](json.md) for the full field reference. CLI flags always
override the JSON file, which overrides built-in defaults.

## Flags

| Flag | Description |
|---|---|
| `--config <path>` | Optional JSON config file providing defaults for the other options |
| `--host <host>` | FTP server hostname |
| `--port <port>` | FTP server port (default `21`) |
| `--user <user>` | FTP username |
| `--remote-dir <path>` | Remote directory to compare |
| `--local-dir <path>` | Local directory to compare |
| `--ftps` | Use explicit FTPS (AUTH TLS) instead of plain FTP |
| `--insecure-tls` | With `--ftps`, accept any TLS certificate instead of validating it. Off by default; removes protection against man-in-the-middle attacks - only use for servers whose certificate you can't otherwise validate |
| `--password <password>` | FTP password. Prefer the environment variable or the interactive prompt over this flag: a CLI argument can leak into shell history and process listings |
| `--hash` | Compare files by MD5 hash in addition to size, for entries that already match by size |
| `--exclude <glob>` | Glob pattern to exclude from comparison; repeatable |
| `--csv <path>` | Also write a structured CSV report to this path |
| `--verbose` | Print progress diagnostics to stderr as the comparison runs, including each file as it's checked |
| `-h`, `--help` | Print help |

`host`, `user`, `remote-dir`, and `local-dir` are required (via CLI flag
or config file); ftpdiff exits with an error (code `2`) if any is
missing.

## Password

Resolved in this order:

1. `--password <password>` flag (least preferred - can leak into shell
   history and process listings)
2. `FTPDIFF_PASSWORD` environment variable
3. Interactive hidden-input terminal prompt (if neither of the above is
   set, and a terminal is available)

## What gets compared

ftpdiff recursively walks both `--local-dir` and `--remote-dir`, applies
any `--exclude` glob patterns to both sides, then compares by relative
path. Each file is categorized as:

- **LocalOnly** - exists locally, not on the remote server.
- **RemoteOnly** - exists on the remote server, not locally.
- **SizeMismatch** - exists on both sides, sizes differ.
- **HashMismatch** - sizes match but MD5 hashes differ (only computed
  when `--hash` is set).
- **Match** - sizes match (and, with `--hash`, MD5 hashes match too).

### Hash strategy (`--hash`)

When `--hash` is set, ftpdiff tries to get a server-computed hash for
same-size files first, falling back to downloading the file and computing
MD5 locally if the server doesn't support that. In the current version,
the underlying `suppaftp` client doesn't expose a public API for
non-standard remote-hash commands (XMD5/MD5/HASH), so in practice every
`--hash` comparison downloads the file - correct, just not optimized to
skip the download when the server could have answered without it.

## Output

Always printed to stdout, one colored line per file:

- `+` (green) - local-only
- `-` (red) - remote-only
- `~` (yellow) - size mismatch or hash mismatch
- `=` (dim) - match

followed by a summary line with counts per status.

With `--csv <path>`, the same results are also written as a CSV file with
columns:

```
path,status,local_size,remote_size,local_md5,remote_md5
```

This CSV is the expected input format for [`ftpops`](ftpops.md).

With `--verbose`, additional diagnostic lines go to stderr: connecting,
each file as it's checked during the local walk (`Checking local: <path>`),
the remote walk (`Checking remote: <path>`), and hashing
(`Checking hash: <path>`), plus a final entry count and (if `--csv` is
set) a note before writing the report.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | No differences found (every entry is `Match`) |
| `1` | Differences found (at least one non-`Match` entry) |
| `2` | Runtime error - connection failure, missing required setting, unreadable/invalid config or CSV target, etc. |

## Examples

Basic comparison:

```sh
ftpdiff --host ftp.example.com --user myuser \
  --remote-dir /var/www/site --local-dir ./site
```

Comparison with hashing, excluding VCS/temp files, writing a CSV report:

```sh
ftpdiff --host ftp.example.com --user myuser \
  --remote-dir /var/www/site --local-dir ./site \
  --hash --exclude '.git/*' --exclude '*.tmp' \
  --csv report.csv
```

Using a config file plus FTPS with a self-signed certificate:

```sh
ftpdiff --config ftp-utils.json --ftps --insecure-tls
```
