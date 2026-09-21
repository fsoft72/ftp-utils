# ftp-utils

A suite of FTP tools written in Rust, organized as a Cargo workspace.

## Structure

- `crates/ftp-utils-core` - shared FTP/FTPS client, directory walking, and
  diff logic used by all tools in the suite.
- `tools/` - one binary crate per tool.

## Tools

### ftpdiff

Compares a remote directory tree (over FTP/FTPS) against a local copy,
reporting differences by file size and, optionally, content hash.

See `docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md` for
the design spec.

### ftpops

Reads a CSV report produced by `ftpdiff --csv` and performs bulk `copy`
(local<->remote) or `delete` (local/remote) operations, filtered by diff
status (`RemoteOnly`/`LocalOnly`). Example: download every file marked
`RemoteOnly` in a report:

```sh
ftpops copy --to local --filter remote-only --csv report.csv \
  --host ftp.example.com --user myuser --remote-dir /var/www/site --local-dir ./site
```

See `docs/superpowers/specs/2026-09-21-ftpops-design.md` for the design
spec.

## Building

```sh
cargo build --workspace
```

To build release binaries for every tool and collect them in `bin/`:

```sh
scripts/build-all.sh
```

## License

MIT - see [LICENSE](LICENSE).
