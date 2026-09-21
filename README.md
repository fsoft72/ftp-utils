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

## Building

```sh
cargo build --workspace
```

## License

MIT - see [LICENSE](LICENSE).
