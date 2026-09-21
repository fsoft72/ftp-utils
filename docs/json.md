# JSON config file reference

Every tool in `ftp-utils` accepts an optional JSON config file via
`--config <path>`, providing default values for the other CLI flags.
Precedence is always:

```
CLI flags > JSON config file > built-in defaults
```

Boolean fields merge as OR between the CLI flag and the JSON value: the
CLI flag can turn a boolean on even if the JSON file doesn't set it, but
it can never turn off a boolean the JSON file set to `true`. `--exclude`
(ftpdiff only) merges as a union of the CLI-provided and JSON-provided
patterns.

Since `ftpdiff` and `ftpops` share the same connection fields (same
names, same types), a single JSON file can be passed to both tools via
`--config`; each tool simply ignores any field it doesn't declare.

## Shared fields (ftpdiff and ftpops)

These fields come from `ftp-utils-core`'s shared connection module and
are accepted by every tool's `--config` file.

| Field | Type | CLI equivalent | Default | Notes |
|---|---|---|---|---|
| `host` | string | `--host` | *(required)* | FTP server hostname. Must be provided by either the CLI or the JSON file. |
| `port` | number | `--port` | `21` | FTP server port. Same default for plain FTP and explicit FTPS (the connection starts on the control port, then upgrades to TLS). |
| `user` | string | `--user` | *(required)* | FTP username. |
| `remote_dir` | string | `--remote-dir` | *(required)* | Remote base directory. |
| `local_dir` | string | `--local-dir` | *(required)* | Local base directory. |
| `ftps` | boolean | `--ftps` | `false` | Use explicit FTPS (AUTH TLS) instead of plain FTP. |
| `insecure_tls` | boolean | `--insecure-tls` | `false` | With `ftps: true`, accept any TLS certificate (expired, self-signed, hostname mismatch) instead of validating it. Removes protection against man-in-the-middle attacks - only use for servers whose certificate you can't otherwise validate. |

**Not accepted in JSON, for any tool:** the FTP password. It is resolved
at runtime from (in order) the `--password` CLI flag, an environment
variable (`FTPDIFF_PASSWORD` for ftpdiff, `FTPOPS_PASSWORD` for ftpops),
or an interactive hidden-input terminal prompt. This is deliberate: a
JSON config file is often kept on disk or committed, and a password field
there would be a credential leak waiting to happen.

## ftpdiff-only fields

In addition to the shared fields above, `ftpdiff --config` also accepts:

| Field | Type | CLI equivalent | Default | Notes |
|---|---|---|---|---|
| `hash` | boolean | `--hash` | `false` | Compare files by MD5 hash (in addition to size) for same-size entries. |
| `verbose` | boolean | `--verbose` | `false` | Print progress diagnostics to stderr as the comparison runs, including each file as it's checked. |
| `exclude` | array of strings | `--exclude` (repeatable) | `[]` | Glob patterns to exclude from the comparison. Merges as a union with any `--exclude` flags passed on the CLI. |

`ftpdiff` example config:

```json
{
  "host": "ftp.example.com",
  "port": 21,
  "user": "myuser",
  "remote_dir": "/var/www/site",
  "local_dir": "./site",
  "ftps": false,
  "insecure_tls": false,
  "hash": true,
  "verbose": false,
  "exclude": [".git/*", "*.tmp"]
}
```

## ftpops-only fields

None. `ftpops --config` accepts only the shared fields above - there is
no JSON equivalent for `--filter`, `--to`, `--on`, `--csv`,
`--skip-existing`, or `--dry-run` (all of those describe a single
invocation's operation, not a reusable connection default).

`ftpops` example config (can be the same file used for `ftpdiff`):

```json
{
  "host": "ftp.example.com",
  "port": 21,
  "user": "myuser",
  "remote_dir": "/var/www/site",
  "local_dir": "./site"
}
```

## Unknown fields

Extra fields in the JSON file that a tool doesn't recognize are silently
ignored (no `deny_unknown_fields`). This is what makes sharing one config
file between `ftpdiff` and `ftpops` possible - `ftpops` ignores
`ftpdiff`'s `hash`/`verbose`/`exclude` fields if they're present in the
same file, and vice versa.
