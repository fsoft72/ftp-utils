# ftpops

Reads a CSV report produced by [`ftpdiff --csv`](ftpdiff.md) and performs
bulk `copy` (local<->remote) or `delete` (local/remote) operations,
filtered by diff status.

The CSV is trusted as-is: ftpops does not re-diff against live state
before acting. If the report is stale, re-run `ftpdiff --csv` first.

## Subcommands

### `ftpops copy`

```sh
ftpops copy --to <local|remote> --filter <remote-only|local-only> --csv <path> [OPTIONS]
```

Copies files between local and remote, in the direction given by `--to`.

| Flag | Description |
|---|---|
| `--to <local\|remote>` | Direction: download to local, or upload to remote |
| `--filter <remote-only\|local-only>` | Only operate on rows with this diff status |
| `--csv <path>` | Path to the `ftpdiff --csv` report to read |
| `--skip-existing` | Skip files whose destination already exists, instead of overwriting them (default: always overwrite) |
| `--dry-run` | Print what would be done without doing it |
| *(plus all connection flags - see below)* | |

### `ftpops delete`

```sh
ftpops delete --on <local|remote> --filter <remote-only|local-only> --csv <path> [OPTIONS]
```

Deletes files locally or remotely.

| Flag | Description |
|---|---|
| `--on <local\|remote>` | Which side to delete from |
| `--filter <remote-only\|local-only>` | Only operate on rows with this diff status |
| `--csv <path>` | Path to the `ftpdiff --csv` report to read |
| `--dry-run` | Print what would be done without doing it |
| *(plus all connection flags - see below)* | |

### Connection flags (both subcommands)

| Flag | Description |
|---|---|
| `--config <path>` | Optional JSON config file providing defaults for the other options |
| `--host <host>` | FTP server hostname |
| `--port <port>` | FTP server port (default `21`) |
| `--user <user>` | FTP username |
| `--remote-dir <path>` | Remote base directory (relative CSV paths are joined onto this) |
| `--local-dir <path>` | Local base directory (relative CSV paths are joined onto this) |
| `--ftps` | Use explicit FTPS (AUTH TLS) instead of plain FTP |
| `--insecure-tls` | With `--ftps`, accept any TLS certificate instead of validating it. Off by default; removes protection against man-in-the-middle attacks |
| `--password <password>` | FTP password (least preferred source - see below) |
| `-h`, `--help` | Print help |

See [`docs/json.md`](json.md) for the `--config` JSON field reference
(ftpops and ftpdiff can share the same config file).

`host`, `user`, `remote-dir`, and `local-dir` are required for every
invocation, even `delete --on local`, which doesn't otherwise need a
network connection - this keeps the connection-resolution logic uniform
across every subcommand.

## Password

Resolved in this order:

1. `--password <password>` flag (least preferred - can leak into shell
   history and process listings)
2. `FTPOPS_PASSWORD` environment variable (note: **not** `FTPDIFF_PASSWORD`
   - each tool has its own password env var, even though a `--config`
   JSON file's host/user/directory settings can be shared between them)
3. Interactive hidden-input terminal prompt

`--dry-run` never requires a password or a network connection - it only
reads the CSV and prints what would happen.

## `--filter` / `--to` / `--on` combinations

Only one `--filter` value makes sense per operation and direction; any
other combination is rejected immediately (before the CSV is even read),
with exit code `2`:

| Command | Valid `--filter` | Why |
|---|---|---|
| `copy --to local` | `remote-only` | Only `RemoteOnly` rows have a remote file to download and no local file to conflict with |
| `copy --to remote` | `local-only` | Only `LocalOnly` rows have a local file to upload and no remote file yet |
| `delete --on remote` | `remote-only` | Only `RemoteOnly` rows exist on the remote side to delete |
| `delete --on local` | `local-only` | Only `LocalOnly` rows exist locally to delete |

## What each operation does

- **`copy --to local`**: downloads each filtered row's remote file,
  creating missing local parent directories, and writes it under
  `--local-dir`. With `--skip-existing`, a row whose local destination
  already exists is skipped instead of overwritten.
- **`copy --to remote`**: reads each filtered row's local file, creates
  missing remote parent directories, and uploads it under `--remote-dir`.
  With `--skip-existing`, a row whose remote destination already exists
  (checked via a directory listing) is skipped instead of overwritten.
- **`delete --on remote`**: deletes each filtered row's file under
  `--remote-dir`.
- **`delete --on local`**: deletes each filtered row's file under
  `--local-dir`.

Every file is processed independently: one failure (e.g. a file already
removed by something else since the CSV was generated) doesn't stop the
rest - it's reported per-file and counted in the summary.

## Output

Without `--dry-run`, one colored line per file:

- `+` (green) - copied
- `-` (red) - deleted
- `~` (yellow) - skipped (destination already exists)
- `!` (bold red) - failed, with the error message

followed by a summary line: `Summary: N succeeded, N skipped, N failed`.

With `--dry-run`, one line per file that would be affected -
`Would copy: <path>` or `Would delete: <path>` - followed by a count
(`Would copy N file(s).` / `Would delete N file(s).`). No connection is
made and no file is touched.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | All operations succeeded (or, in `--dry-run`, the plan was printed) |
| `1` | At least one per-file operation failed (partial failure) |
| `2` | Runtime/setup error - unreadable/malformed CSV, connection failure, invalid `--filter`/`--to`/`--on` combination, missing required connection setting |

## Examples

Download every file marked `RemoteOnly` in a report:

```sh
ftpops copy --to local --filter remote-only --csv report.csv \
  --host ftp.example.com --user myuser --remote-dir /var/www/site --local-dir ./site
```

Preview an upload without doing it:

```sh
ftpops copy --to remote --filter local-only --csv report.csv \
  --host ftp.example.com --user myuser --remote-dir /var/www/site --local-dir ./site \
  --dry-run
```

Clean up files that only exist on the remote server:

```sh
ftpops delete --on remote --filter remote-only --csv report.csv \
  --host ftp.example.com --user myuser --remote-dir /var/www/site --local-dir ./site
```

Delete stray local-only files without touching the network (still
requires `--host`/`--user`/`--remote-dir` to be set, even though they're
unused for this operation):

```sh
ftpops delete --on local --filter local-only --csv report.csv \
  --host ftp.example.com --user myuser --remote-dir /var/www/site --local-dir ./site
```
