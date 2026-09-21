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
| `--local-csv <path>` | Read the local side from a previous `ftpdiff --csv` report instead of scanning `--local-dir` |
| `--remote-csv <path>` | Read the remote side from a previous `ftpdiff --csv` report instead of connecting to the FTP server |
| `--build` | Scan exactly one side and write it to `--csv` without comparing; requires `--csv`, cannot combine with `--local-csv`/`--remote-csv` |
| `-h`, `--help` | Print help |

`host`, `user`, and `remote-dir` are required unless `--remote-csv` is
given; `local-dir` is required unless `--local-csv` is given. ftpdiff
exits with an error (code `2`) if a required setting is missing. If
`--local-dir`/`--local-csv` (or the remote equivalents) are both given,
the CSV flag takes precedence and the live setting is simply unused - not
an error, so a shared `--config` file can supply connection defaults used
by other invocations without breaking a CSV-sourced one. See "Comparing
against a CSV snapshot" below.

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

### Comparing against a CSV snapshot

Instead of scanning the local filesystem and/or connecting to the remote
server, either side of the comparison can be read from a CSV report
written by an earlier `ftpdiff --csv` run:

- `--local-csv <path>` reads that report's `local_size`/`local_md5`
  columns as the local side (rows without a `local_size` are skipped).
- `--remote-csv <path>` reads its `remote_size`/`remote_md5` columns as
  the remote side (rows without a `remote_size` are skipped).

Each flag is independent. Using one still performs a live scan/connection
for the other side. Using both compares two CSV files against each other
with no filesystem or network access at all. `--exclude` still applies to
entries loaded from a CSV, same as a live scan. With `--hash`, an MD5
already recorded in the source CSV is reused instead of being
recomputed; if it's missing (e.g. the original report wasn't generated
with `--hash`) and that side has no live source to fall back to, that
entry is left at its size-only `Match` result rather than erroring.

### Building a snapshot without comparing (`--build`)

`--build` scans exactly one side and writes it to `--csv`, skipping the
comparison entirely - the producer counterpart to `--local-csv`/
`--remote-csv`. Every entry gets status `Scan` rather than
`LocalOnly`/`RemoteOnly` (nothing was compared, so those labels don't
apply), but the CSV is still a fully valid `--local-csv`/`--remote-csv`
input for a later run - the reader only looks at whether a row has a
size for that side, not its status text.

Requires `--csv`; requires exactly one side's live settings (`--local-dir`
alone, or `--host`/`--user`/`--remote-dir` alone - not both, not
neither); cannot be combined with `--local-csv`/`--remote-csv`. With
`--hash`, each scanned file's own MD5 is computed and recorded (off by
default, since it's the slow path). Exit code is always `0` or `2`,
never `1` - nothing was compared, so "differences found" doesn't apply.

## Output

Always printed to stdout, one colored line per file:

- `+` (green) - local-only
- `-` (red) - remote-only
- `~` (yellow) - size mismatch or hash mismatch
- `=` (dim) - match
- `*` (cyan) - scanned by `--build`, not compared (status `Scan`)

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

### Scan a remote directory and generate a CSV report

`--build` scans one side only and writes it straight to `--csv`, with no
local directory needed at all:

```sh
ftpdiff --host ftp.example.com --user myuser \
  --remote-dir /var/www/site --build --csv report.csv
```

Add `--hash` to also record each file's MD5 in the snapshot:

```sh
ftpdiff --host ftp.example.com --user myuser \
  --remote-dir /var/www/site --build --hash --csv report.csv
```

`report.csv` now holds a full snapshot of the remote scan (its
`remote_size`/`remote_md5` columns; `status` is `Scan` for every row),
which can later be reused as a `--remote-csv` input - see the third
example below.

### Compare a local directory against a remote directory

The basic case: both sides scanned live, results printed to the
terminal, no CSV written.

```sh
ftpdiff --host ftp.example.com --user myuser \
  --remote-dir /var/www/site --local-dir ./site
```

Add `--hash` to also verify content (not just size) for same-size files,
and `--exclude` to skip VCS/temp files:

```sh
ftpdiff --host ftp.example.com --user myuser \
  --remote-dir /var/www/site --local-dir ./site \
  --hash --exclude '.git/*' --exclude '*.tmp'
```

### Compare a local directory against a remote CSV snapshot, writing a CSV diff report

Diff the current local filesystem against a previously recorded remote
state (e.g. the `report.csv` produced by the first example above), with
no FTP connection at all, and save the new diff as its own CSV:

```sh
ftpdiff --local-dir ./site --remote-csv report.csv \
  --csv diff.csv
```

Since `--remote-csv` is given, `--host`/`--user`/`--remote-dir`/password
are not needed - ftpdiff never connects to the server for this run.

Using a config file plus FTPS with a self-signed certificate:

```sh
ftpdiff --config ftp-utils.json --ftps --insecure-tls
```
