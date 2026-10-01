# ftpwatch - Design Spec

## Purpose

Monitor WordPress sites over FTP/FTPS for file changes, without downloading
anything after the first run.

- **First run (`init`)**: download every file of the site (except the
  excluded ones, typically `wp-content/uploads/**`) and write a baseline
  snapshot CSV containing path, size and modification time.
- **Daily runs (`check`)**: scan the remote tree (listing only, no
  downloads), compare it with the previous snapshot, write a human-readable
  log of what changed and store today's snapshot. New files are reported but
  never downloaded.

## Decisions (agreed with the user)

- Change detection uses **size + modification time (mtime)**, taken from the
  FTP listing. No hashing, no downloads during `check`.
- Each `check` compares against the **previous snapshot** (the latest one in
  `snapshots/`), so a change is reported once, not every day.
- The log is **plain text** only (CSV/JSON log is out of scope for v1).
- Delivered as a **new tool `tools/ftpwatch`** in the workspace, next to
  `ftpdiff` and `ftpops`, reusing `ftp-utils-core`.

## Site directory layout

One directory per site, passed as `--site-dir` (or the positional argument):

```
sites/test.com/
  config.json          host, user, remote_dir, exclude, ftps, timeout, ...
  files/               copy downloaded by `init`
  snapshots/
    2026-10-01.csv     baseline written by `init`
    2026-10-02.csv     one per `check` run
  logs/
    2026-10-02.log     one per `check` run
```

`config.json` uses the same connection fields as the other tools (see
`docs/json.md`), plus `exclude` (list of globs). The password is never stored
in the file: `FTPWATCH_PASSWORD` env var or interactive prompt (same pattern
as `ftpdiff`/`ftpops`; `--password` is the least preferred source).

## 1. `ftp-utils-core` changes

### mtime in remote listings

- `RawRemoteEntry` and `RemoteEntry` gain `modified: Option<i64>` (Unix
  seconds, UTC). `None` when the server did not provide a usable date.
- `SuppaFtpConnection::parse_listing` fills it from `ListParser`'s
  `modified()`.
- Known limitation of `LIST`: old files carry only a date (no time of day),
  so their mtime has day precision. Comparison must therefore treat mtimes
  as equal when both fall on the same day **and** the source listing lacked a
  time (see "Comparison rules"). Preferring `MLSD` (second precision) when
  the server supports it is a possible follow-up, not part of v1.
- Local walking is unchanged (`ftpwatch` never scans the local tree).

### CSV format

- The report/snapshot CSV gains an optional trailing column `remote_mtime`
  (Unix seconds, empty when unknown).
- Readers stay compatible with old CSVs: a missing column means "no mtime".
  Existing tools (`ftpdiff`, `ftpops`) ignore the new column.
- Writers always emit the column.

## 2. `tools/ftpwatch`

### CLI

```
ftpwatch init  <SITE_DIR> [--password <pw>] [--verbose]
ftpwatch check <SITE_DIR> [--password <pw>] [--verbose]
```

### `init`

1. Refuse to run if `snapshots/` already contains a CSV (exit 2), unless
   `--force` is given. This prevents overwriting the baseline by mistake.
2. Walk the remote tree honoring `exclude`, downloading each non-excluded
   file into `files/` (reuses `walk_remote_with` and the `.part`-then-rename
   download logic of `ftpdiff --download-dir`).
3. Write `snapshots/<YYYY-MM-DD>.csv` with path, size, mtime.

### `check`

1. Require an existing snapshot (otherwise error: run `init` first).
2. Walk the remote tree honoring `exclude`; **no download**.
3. Compare with the latest snapshot (see rules below).
4. Write `logs/<YYYY-MM-DD>.log` and `snapshots/<YYYY-MM-DD>.csv`.
5. If today's snapshot already exists (second run on the same day), the log
   and snapshot get a `-HHMMSS` suffix instead of being overwritten.

The new snapshot is written **only after** the log was written successfully,
so a failed run never loses changes.

### Comparison rules

Given the previous snapshot P and the current scan C, keyed by relative path:

- `NEW`: in C, not in P.
- `DELETED`: in P, not in C.
- `MODIFIED`: in both and size differs, or mtime differs.
  - If either mtime is unknown (`None`), only size is compared.
  - If the listing carried no time of day for the file, mtimes are compared
    at day precision.
- Everything else is unchanged and not logged.

### Log format

```
ftpwatch check - test.com - 2026-10-02 03:00:12
Previous snapshot: snapshots/2026-10-01.csv
Summary: 2 new, 1 deleted, 3 modified, 1840 unchanged

NEW       wp-content/plugins/foo/bar.php   size=1204  mtime=2026-10-02 02:11:09
DELETED   wp-content/themes/old/style.css  size=88210
MODIFIED  wp-config.php                    size 3021 -> 3050, mtime 2026-09-12 -> 2026-10-02
```

Lines are sorted by status then path, so diffs between logs stay readable.
When nothing changed, the log contains only the header and
`Summary: no changes`.

### Exit codes

Consistent with the `exit` constants in core:

| Code | Meaning |
|---|---|
| `0` | `check`: no changes. `init`: success |
| `1` | `check`: at least one change |
| `2` | Error (config, connection, I/O, missing snapshot) |

This lets a cron job send a mail only when the exit code is `1`.

## Scheduling

Not handled by the tool. Example crontab entry:

```
0 3 * * * FTPWATCH_PASSWORD=... ftpwatch check /srv/sites/test.com
```

## Testing

- Core: unit tests for mtime parsing from listings (with and without time of
  day), CSV round-trip with and without the `remote_mtime` column, old-CSV
  compatibility.
- `ftpwatch` comparison logic as a pure function (previous rows + current
  rows -> NEW/DELETED/MODIFIED), tested for: empty previous, identical,
  size-only change, mtime-only change, unknown mtime, day-precision mtime.
- End-to-end CLI tests (like `tools/ftpdiff/tests/cli.rs`) using the
  in-process fake FTP server: `init` downloads and writes the snapshot;
  `check` after adding/removing/editing files writes the expected log, exit
  code 1, and does not download anything.
- Same-day second run and refusal to re-`init`.

## Out of scope for v1

- Downloading new/changed files during `check`.
- Hash-based change detection.
- `MLSD` support.
- CSV/JSON log output, e-mail notification, built-in scheduler.
- Monitoring several sites in one invocation (use one cron line per site).
- Retention/cleanup of old snapshots and logs.
