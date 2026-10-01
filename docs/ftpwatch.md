# ftpwatch

Monitors an FTP/FTPS site (typically a WordPress install) for added, deleted
and modified files. `init` downloads the site once and records a baseline
snapshot; every later `check` only lists the remote tree (no downloads),
compares it with the previous snapshot and writes a plain-text log of what
changed.

## Usage

```
ftpwatch init  <SITE_DIR> [--password <pw>] [--verbose] [--force]
ftpwatch check <SITE_DIR> [--password <pw>] [--verbose]
```

## Site directory

One directory per site:

```
sites/test.com/
  config.json                      connection settings (you write this)
  files/                           copy downloaded by `init`
  snapshots/
    2026-10-01_030000.csv          baseline written by `init`
    2026-10-02_030004.csv          one per `check` run
  logs/
    2026-10-02_030004.log          one per `check` run
```

Snapshot and log names are `YYYY-MM-DD_HHMMSS` in **UTC**. The latest
snapshot is the one with the greatest name. Files are written atomically, so
an interrupted run never leaves a half-written snapshot.

## config.json

| Field | Required | Default | Description |
|---|---|---|---|
| `host` | yes | - | FTP server host name |
| `port` | no | `21` | FTP port |
| `user` | yes | - | FTP user name |
| `remote_dir` | yes | - | Remote directory to monitor |
| `ftps` | no | `false` | Use explicit FTPS |
| `insecure_tls` | no | `false` | Accept invalid TLS certificates |
| `timeout` | no | `30` | Connection timeout in seconds |
| `exclude` | no | `[]` | Glob patterns (relative to `remote_dir`) to skip |

The password is never stored in the file. Unknown fields are ignored.

WordPress example:

```json
{
  "host": "ftp.test.com",
  "user": "deploy",
  "remote_dir": "/public_html",
  "ftps": true,
  "exclude": ["wp-content/uploads/**", "wp-content/cache/**"]
}
```

## Password

In order of preference: the interactive prompt, the `FTPWATCH_PASSWORD`
environment variable, the `--password` flag (least preferred: it can leak
into shell history and process listings).

## init

1. Fails with exit code 2 if the site already has a snapshot, unless
   `--force` is given (so the baseline is not replaced by mistake).
2. Walks the remote tree honoring `exclude` and downloads every file into
   `files/`.
3. Writes the baseline snapshot. If anything fails, no snapshot is written.

## check

1. Requires an existing snapshot (otherwise: run `ftpwatch init` first). This
   is verified before connecting.
2. Walks the remote tree honoring `exclude`. Nothing is downloaded; new files
   are only reported.
3. Compares with the latest snapshot, writes the log, then writes the new
   snapshot. A failed run writes neither, so changes are never lost.

Each `check` compares against the previous snapshot, so a change is reported
once. Rows of the previous snapshot that match the current `exclude` globs are
ignored, so adding an exclude does not report those files as deleted.

## Comparison rules

Files are keyed by relative path:

- `NEW`: present now, absent from the previous snapshot.
- `DELETED`: in the previous snapshot, absent now.
- `MODIFIED`: the size differs, or the mtime differs.

Modification times are always compared at **day precision**, because FTP
listings give old files a date without a time of day. If either mtime is
unknown, only the size is compared.

## Safeguard

If the remote listing is empty while the previous snapshot has files, `check`
fails (exit code 2) and writes nothing, instead of recording every file as
deleted (a typical symptom of a wrong `remote_dir` or a broken server).

## Log format

```
ftpwatch check - test.com - 2026-10-02 03:00:12 UTC
Previous snapshot: 2026-10-01_030000.csv
Summary: 2 new, 1 deleted, 3 modified, 1840 unchanged

NEW       wp-content/plugins/foo/bar.php  size=1204  mtime=2026-10-02 02:11:09
DELETED   wp-content/themes/old/style.css  size=88210  mtime=2026-08-14 10:00:00
MODIFIED  wp-config.php  size 3021 -> 3050, mtime 2026-09-12 10:00:00 -> 2026-10-02 02:00:00
```

When nothing changed, the log contains only the header, the previous snapshot
and `Summary: no changes`. All times are UTC.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | `init` succeeded, or `check` found no changes |
| `1` | `check` found at least one change |
| `2` | Error (config, connection, I/O, missing snapshot, empty listing) |

## Scheduling

ftpwatch has no built-in scheduler. Example crontab entry that sends a mail
only when something changed:

```
0 3 * * * FTPWATCH_PASSWORD=... ftpwatch check /srv/sites/test.com; [ $? -eq 1 ] && mail -s "test.com changed" me@example.com < "$(ls -1 /srv/sites/test.com/logs/*.log | tail -1)"
```
