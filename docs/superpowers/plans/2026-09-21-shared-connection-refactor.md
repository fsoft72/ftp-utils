# Shared Connection Module + FtpConnection Extension Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract ftpdiff's CLI/config/password logic into a shared `ftp-utils-core::connection` module (so `ftpops`, planned next, can reuse it without duplication), and extend the `FtpConnection` trait with upload/delete/mkdir primitives that `ftpops` will need. This plan changes no user-visible ftpdiff behavior - it's a pure refactor plus additive trait capability.

**Architecture:** `ftp-utils-core` gains a `connection` module holding the host/port/user/password/local-dir/remote-dir/ftps/insecure-tls CLI args (as a `clap::Args` struct meant to be flattened), the matching JSON config fields, the merge logic, and password resolution - all generalized to take an environment variable name instead of hardcoding `FTPDIFF_PASSWORD`. `ftpdiff`'s own `cli.rs`/`config.rs` shrink to just its tool-specific fields (`hash`, `exclude`, `csv`, `verbose`) plus a flattened/embedded connection. Separately, `FtpConnection` gains `store_from_buffer`, `delete`, and `create_dir` (all implemented by `SuppaFtpConnection` via `suppaftp`), plus a default-implemented `ensure_remote_dir` built generically on `list_dir` + `create_dir` (so it's unit-testable with the existing mock pattern, no real network needed).

**Tech Stack:** Rust 2021, `clap` (derive, `Args` flattening), `serde`/`serde_json` (`#[serde(flatten)]`), `suppaftp` (`put_file`, `rm`, `mkdir`), `rpassword`.

**Spec:** `docs/superpowers/specs/2026-09-21-ftpops-design.md` (sections 1 and 2 - the shared connection module and the `FtpConnection` extension). This plan also amends that spec's `ensure_remote_dir` description: instead of being implemented independently per-connection-type, it is a **default trait method** built on a new `create_dir` primitive plus the existing `list_dir`, so its component-splitting/already-exists logic is unit-testable via the mock, matching the spec's stated testing intent.

## Global Constraints

- No user-visible behavior change for `ftpdiff` in this plan - CLI flags, JSON config shape, precedence rules, and password resolution stay identical; only their implementation moves.
- Boolean flags merge as OR (CLI can enable, never disable, a JSON-set true) - unchanged rule, now lives in `connection::merge_connection`.
- Password precedence unchanged: `--password` > environment variable > interactive hidden-input prompt. The environment variable name becomes a parameter (`ftpdiff` keeps passing `"FTPDIFF_PASSWORD"`).
- Every step ends with a passing test; the full workspace test suite must pass at the end of Task 4 (ftpdiff refactor) and again at the end of Task 7 (trait extension), with zero test count regression from before this plan.

---

## File Structure

```
crates/ftp-utils-core/
├── Cargo.toml                  # add clap, serde, serde_json, rpassword
└── src/
    ├── lib.rs                  # add `pub mod connection;`
    ├── connection.rs           # NEW: ConnectionArgs, ConnectionJsonConfig,
    │                            #      EffectiveConnection, ConnectionError,
    │                            #      load_json_config, merge_connection,
    │                            #      read_password
    └── remote.rs                # FtpConnection: + store_from_buffer, delete,
                                   #                create_dir, ensure_remote_dir (default)

tools/ftpdiff/
└── src/
    ├── cli.rs                  # Cli.connection: ConnectionArgs (flattened);
    │                            # keeps hash/exclude/csv/verbose
    ├── config.rs                # JsonConfig.connection: ConnectionJsonConfig
    │                             # (flattened); EffectiveConfig.connection:
    │                             # EffectiveConnection; delegates to
    │                             # ftp_utils_core::connection
    └── main.rs                   # field access updated to effective.connection.*
```

---

### Task 1: `ftp-utils-core::connection` module

**Files:**
- Create: `crates/ftp-utils-core/src/connection.rs`
- Modify: `crates/ftp-utils-core/src/lib.rs` (add `pub mod connection;`)
- Modify: `crates/ftp-utils-core/Cargo.toml` (add `clap`, `serde`, `serde_json`, `rpassword`)

**Interfaces:**
- Produces: `pub struct ConnectionArgs { pub config: Option<PathBuf>, pub host: Option<String>, pub port: Option<u16>, pub user: Option<String>, pub remote_dir: Option<String>, pub local_dir: Option<PathBuf>, pub ftps: bool, pub insecure_tls: bool, pub password: Option<String> }` (derives `clap::Args`), `pub struct ConnectionJsonConfig { pub host: Option<String>, pub port: Option<u16>, pub user: Option<String>, pub remote_dir: Option<String>, pub local_dir: Option<PathBuf>, pub ftps: Option<bool>, pub insecure_tls: Option<bool> }` (derives `serde::Deserialize`, `Default`), `pub struct EffectiveConnection { pub host: String, pub port: u16, pub user: String, pub remote_dir: String, pub local_dir: PathBuf, pub ftps: bool, pub insecure_tls: bool }`, `pub struct ConnectionError(pub String)` (`Display`, `std::error::Error`), `pub fn load_json_config<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, ConnectionError>`, `pub fn merge_connection(args: &ConnectionArgs, json: &ConnectionJsonConfig) -> Result<EffectiveConnection, ConnectionError>`, `pub fn read_password(cli_password: Option<&str>, env_var: &str) -> Result<String, ConnectionError>`

- [ ] **Step 1: Add dependencies**

In `crates/ftp-utils-core/Cargo.toml`, change `[dependencies]` to:

```toml
[dependencies]
suppaftp.workspace = true
glob.workspace = true
walkdir.workspace = true
md5.workspace = true
clap.workspace = true
serde.workspace = true
serde_json.workspace = true
rpassword.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 2: Write the failing tests**

Create `crates/ftp-utils-core/src/connection.rs`:

```rust
//! Shared CLI/config/password handling for tools that connect to an
//! FTP/FTPS server: host, port, user, password, local/remote directory,
//! and TLS options. Meant to be flattened/embedded into each tool's own
//! `clap::Parser` CLI struct and JSON config struct.

use std::path::{Path, PathBuf};

use clap::Args;
use serde::Deserialize;

/// Shared connection flags. Embed via `#[command(flatten)]` in a tool's
/// own `clap::Parser` struct.
#[derive(Args, Debug, Clone)]
pub struct ConnectionArgs {
    /// Optional JSON config file providing defaults for the other options.
    #[arg(long)]
    pub config: Option<PathBuf>,

    #[arg(long)]
    pub host: Option<String>,

    #[arg(long)]
    pub port: Option<u16>,

    #[arg(long)]
    pub user: Option<String>,

    #[arg(long = "remote-dir")]
    pub remote_dir: Option<String>,

    #[arg(long = "local-dir")]
    pub local_dir: Option<PathBuf>,

    /// Use explicit FTPS (AUTH TLS) instead of plain FTP.
    #[arg(long)]
    pub ftps: bool,

    /// Accept any TLS certificate (expired, self-signed, hostname
    /// mismatch) when using --ftps, instead of validating it. Only use
    /// this for servers whose certificate you can't otherwise validate:
    /// it removes protection against man-in-the-middle attacks.
    #[arg(long = "insecure-tls")]
    pub insecure_tls: bool,

    /// FTP password. Prefer the environment variable or the interactive
    /// prompt over this flag: a CLI argument can leak into shell history
    /// and process listings.
    #[arg(long)]
    pub password: Option<String>,
}

/// Shared JSON config fields. Embed via `#[serde(flatten)]` in a tool's
/// own JSON config struct.
#[derive(Debug, Deserialize, Default)]
pub struct ConnectionJsonConfig {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub remote_dir: Option<String>,
    pub local_dir: Option<PathBuf>,
    pub ftps: Option<bool>,
    pub insecure_tls: Option<bool>,
}

/// Resolved connection settings after merging CLI args, JSON config, and
/// defaults.
#[derive(Debug, Clone)]
pub struct EffectiveConnection {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub remote_dir: String,
    pub local_dir: PathBuf,
    pub ftps: bool,
    pub insecure_tls: bool,
}

/// Error from config loading, merging, or password resolution.
#[derive(Debug)]
pub struct ConnectionError(pub String);

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConnectionError {}

/// Loads and parses a JSON config file into any `Deserialize` type.
pub fn load_json_config<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, ConnectionError> {
    todo!()
}

/// Merges CLI args, JSON config, and defaults into `EffectiveConnection`.
/// Precedence: CLI > JSON > default. Boolean flags merge as OR (CLI can
/// enable, never disable, a JSON-set true).
pub fn merge_connection(
    args: &ConnectionArgs,
    json: &ConnectionJsonConfig,
) -> Result<EffectiveConnection, ConnectionError> {
    todo!()
}

fn password_from_cli_or_env(cli_password: Option<&str>, env_var: &str) -> Option<String> {
    cli_password
        .map(|p| p.to_string())
        .or_else(|| std::env::var(env_var).ok())
}

/// Resolves the FTP password: `--password` flag, then the environment
/// variable named `env_var`, then an interactive hidden-input prompt.
/// Prefer the environment variable or the prompt over the flag: a CLI
/// argument can leak into shell history and process listings.
pub fn read_password(cli_password: Option<&str>, env_var: &str) -> Result<String, ConnectionError> {
    if let Some(password) = password_from_cli_or_env(cli_password, env_var) {
        return Ok(password);
    }

    rpassword::prompt_password("FTP password: ")
        .map_err(|e| ConnectionError(format!("failed to read password from terminal: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_GUARD: Mutex<()> = Mutex::new(());

    fn empty_args() -> ConnectionArgs {
        ConnectionArgs {
            config: None,
            host: None,
            port: None,
            user: None,
            remote_dir: None,
            local_dir: None,
            ftps: false,
            insecure_tls: false,
            password: None,
        }
    }

    #[test]
    fn loads_json_config_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"host": "ftp.example.com", "user": "bob"}"#).unwrap();

        let config: ConnectionJsonConfig = load_json_config(&path).unwrap();

        assert_eq!(config.host.as_deref(), Some("ftp.example.com"));
        assert_eq!(config.user.as_deref(), Some("bob"));
    }

    #[test]
    fn cli_overrides_json_for_scalar_fields() {
        let mut args = empty_args();
        args.host = Some("cli-host".to_string());
        args.user = Some("cli-user".to_string());
        args.remote_dir = Some("/cli-remote".to_string());
        args.local_dir = Some(PathBuf::from("./cli-local"));

        let json = ConnectionJsonConfig {
            host: Some("json-host".to_string()),
            user: Some("json-user".to_string()),
            remote_dir: Some("/json-remote".to_string()),
            local_dir: Some(PathBuf::from("./json-local")),
            ..Default::default()
        };

        let effective = merge_connection(&args, &json).unwrap();

        assert_eq!(effective.host, "cli-host");
        assert_eq!(effective.user, "cli-user");
        assert_eq!(effective.remote_dir, "/cli-remote");
        assert_eq!(effective.local_dir, PathBuf::from("./cli-local"));
    }

    #[test]
    fn falls_back_to_json_when_cli_field_absent() {
        let args = empty_args();
        let json = ConnectionJsonConfig {
            host: Some("json-host".to_string()),
            user: Some("json-user".to_string()),
            remote_dir: Some("/json-remote".to_string()),
            local_dir: Some(PathBuf::from("./json-local")),
            ..Default::default()
        };

        let effective = merge_connection(&args, &json).unwrap();

        assert_eq!(effective.host, "json-host");
        assert_eq!(effective.user, "json-user");
    }

    #[test]
    fn errors_when_required_field_missing_everywhere() {
        let args = empty_args();
        let json = ConnectionJsonConfig::default();

        let result = merge_connection(&args, &json);

        assert!(result.is_err());
    }

    #[test]
    fn ftps_and_insecure_tls_merge_as_or() {
        let mut args = empty_args();
        args.host = Some("h".into());
        args.user = Some("u".into());
        args.remote_dir = Some("/r".into());
        args.local_dir = Some(PathBuf::from("./l"));
        args.ftps = true;

        let json = ConnectionJsonConfig { insecure_tls: Some(true), ..Default::default() };

        let effective = merge_connection(&args, &json).unwrap();

        assert!(effective.ftps); // from CLI
        assert!(effective.insecure_tls); // from JSON
    }

    #[test]
    fn default_port_is_21() {
        let mut args = empty_args();
        args.host = Some("h".into());
        args.user = Some("u".into());
        args.remote_dir = Some("/r".into());
        args.local_dir = Some(PathBuf::from("./l"));

        let effective = merge_connection(&args, &ConnectionJsonConfig::default()).unwrap();

        assert_eq!(effective.port, 21);
    }

    #[test]
    fn resolves_password_from_env_var_when_cli_absent() {
        let _guard = ENV_GUARD.lock().unwrap();
        std::env::set_var("CONNECTION_TEST_PASSWORD_1", "s3cr3t");

        let result = password_from_cli_or_env(None, "CONNECTION_TEST_PASSWORD_1");

        assert_eq!(result, Some("s3cr3t".to_string()));
        std::env::remove_var("CONNECTION_TEST_PASSWORD_1");
    }

    #[test]
    fn cli_password_takes_precedence_over_env_var() {
        let _guard = ENV_GUARD.lock().unwrap();
        std::env::set_var("CONNECTION_TEST_PASSWORD_2", "env-password");

        let result = password_from_cli_or_env(Some("cli-password"), "CONNECTION_TEST_PASSWORD_2");

        assert_eq!(result, Some("cli-password".to_string()));
        std::env::remove_var("CONNECTION_TEST_PASSWORD_2");
    }

    #[test]
    fn returns_none_when_neither_cli_nor_env_password_set() {
        let _guard = ENV_GUARD.lock().unwrap();
        std::env::remove_var("CONNECTION_TEST_PASSWORD_3");

        let result = password_from_cli_or_env(None, "CONNECTION_TEST_PASSWORD_3");

        assert_eq!(result, None);
    }
}
```

- [ ] **Step 3: Run tests to verify the two `todo!()`-backed tests fail**

Run: `cargo test -p ftp-utils-core connection::tests`
Expected: FAIL (`not yet implemented`) for `loads_json_config_from_file`, `cli_overrides_json_for_scalar_fields`, and the other `merge_connection`-based tests; the three password tests pass already since they only touch `password_from_cli_or_env`, which has no `todo!()`.

- [ ] **Step 4: Implement `load_json_config` and `merge_connection`**

Replace the two `todo!()` bodies:

```rust
pub fn load_json_config<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, ConnectionError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| ConnectionError(format!("cannot read config file {}: {e}", path.display())))?;
    serde_json::from_str(&content)
        .map_err(|e| ConnectionError(format!("invalid JSON in {}: {e}", path.display())))
}

pub fn merge_connection(
    args: &ConnectionArgs,
    json: &ConnectionJsonConfig,
) -> Result<EffectiveConnection, ConnectionError> {
    let host = args
        .host
        .clone()
        .or_else(|| json.host.clone())
        .ok_or_else(|| ConnectionError("missing required setting: host (use --host or config file)".to_string()))?;

    let user = args
        .user
        .clone()
        .or_else(|| json.user.clone())
        .ok_or_else(|| ConnectionError("missing required setting: user (use --user or config file)".to_string()))?;

    let remote_dir = args
        .remote_dir
        .clone()
        .or_else(|| json.remote_dir.clone())
        .ok_or_else(|| {
            ConnectionError("missing required setting: remote-dir (use --remote-dir or config file)".to_string())
        })?;

    let local_dir = args
        .local_dir
        .clone()
        .or_else(|| json.local_dir.clone())
        .ok_or_else(|| {
            ConnectionError("missing required setting: local-dir (use --local-dir or config file)".to_string())
        })?;

    Ok(EffectiveConnection {
        host,
        port: args.port.or(json.port).unwrap_or(21),
        user,
        remote_dir,
        local_dir,
        ftps: args.ftps || json.ftps.unwrap_or(false),
        insecure_tls: args.insecure_tls || json.insecure_tls.unwrap_or(false),
    })
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p ftp-utils-core connection::tests`
Expected: PASS (9 tests)

- [ ] **Step 6: Wire the module**

In `crates/ftp-utils-core/src/lib.rs`, add `pub mod connection;` alongside the existing module declarations.

- [ ] **Step 7: Verify the whole core crate still builds and tests pass**

Run: `cargo test -p ftp-utils-core`
Expected: PASS (all existing tests plus the 9 new ones)

- [ ] **Step 8: Commit**

```bash
git add crates/ftp-utils-core/Cargo.toml crates/ftp-utils-core/src/connection.rs crates/ftp-utils-core/src/lib.rs
git commit -m "ftp-utils-core: add shared connection/config/password module"
```

---

### Task 2: Refactor ftpdiff's `cli.rs` to flatten `ConnectionArgs`

**Files:**
- Modify: `tools/ftpdiff/src/cli.rs`

**Interfaces:**
- Consumes: `ftp_utils_core::connection::ConnectionArgs` (Task 1)
- Produces: `pub struct Cli { pub connection: ConnectionArgs, pub hash: bool, pub exclude: Vec<String>, pub csv: Option<PathBuf>, pub verbose: bool }` (derives `clap::Parser`; `connection` is `#[command(flatten)]`ed so its flags appear as top-level CLI flags exactly as before - `--host`, `--password`, etc. are unchanged from the user's perspective)

- [ ] **Step 1: Replace the file**

Replace the full contents of `tools/ftpdiff/src/cli.rs`:

```rust
//! Command-line argument definitions for ftpdiff.

use std::path::PathBuf;

use clap::Parser;
use ftp_utils_core::connection::ConnectionArgs;

/// Compare a remote FTP/FTPS directory tree against a local copy.
#[derive(Parser, Debug, Clone)]
#[command(name = "ftpdiff", about = "Compare a remote FTP/FTPS directory tree against a local copy")]
pub struct Cli {
    #[command(flatten)]
    pub connection: ConnectionArgs,

    /// Compare files by MD5 hash in addition to size.
    #[arg(long)]
    pub hash: bool,

    /// Glob pattern to exclude from comparison; can be repeated.
    #[arg(long = "exclude")]
    pub exclude: Vec<String>,

    /// Also write a structured CSV report to this path.
    #[arg(long)]
    pub csv: Option<PathBuf>,

    /// Print progress diagnostics (connecting, directory walk counts,
    /// hashing) to stderr as the comparison runs.
    #[arg(long)]
    pub verbose: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_args() {
        let cli = Cli::parse_from([
            "ftpdiff",
            "--host", "ftp.example.com",
            "--user", "bob",
            "--remote-dir", "/remote",
            "--local-dir", "./local",
        ]);

        assert_eq!(cli.connection.host.as_deref(), Some("ftp.example.com"));
        assert_eq!(cli.connection.user.as_deref(), Some("bob"));
        assert_eq!(cli.connection.remote_dir.as_deref(), Some("/remote"));
        assert!(!cli.connection.ftps);
        assert!(!cli.hash);
        assert!(cli.exclude.is_empty());
    }

    #[test]
    fn parses_repeated_exclude_and_flags() {
        let cli = Cli::parse_from([
            "ftpdiff",
            "--exclude", "*.tmp",
            "--exclude", ".git/*",
            "--ftps",
            "--hash",
        ]);

        assert_eq!(cli.exclude, vec!["*.tmp".to_string(), ".git/*".to_string()]);
        assert!(cli.connection.ftps);
        assert!(cli.hash);
    }

    #[test]
    fn parses_password_flag() {
        let cli = Cli::parse_from(["ftpdiff", "--password", "s3cr3t"]);

        assert_eq!(cli.connection.password.as_deref(), Some("s3cr3t"));
    }

    #[test]
    fn password_defaults_to_none() {
        let cli = Cli::parse_from(["ftpdiff"]);

        assert_eq!(cli.connection.password, None);
    }

    #[test]
    fn parses_insecure_tls_flag() {
        let cli = Cli::parse_from(["ftpdiff", "--insecure-tls"]);

        assert!(cli.connection.insecure_tls);
    }

    #[test]
    fn insecure_tls_defaults_to_false() {
        let cli = Cli::parse_from(["ftpdiff"]);

        assert!(!cli.connection.insecure_tls);
    }

    #[test]
    fn parses_verbose_flag() {
        let cli = Cli::parse_from(["ftpdiff", "--verbose"]);

        assert!(cli.verbose);
    }

    #[test]
    fn verbose_defaults_to_false() {
        let cli = Cli::parse_from(["ftpdiff"]);

        assert!(!cli.verbose);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p ftpdiff cli::tests`
Expected: PASS (8 tests) - this step has no red phase since it's a mechanical field-path update (`cli.host` -> `cli.connection.host`) applied together with the struct change; both change atomically because the old field paths won't compile otherwise.

- [ ] **Step 3: Commit**

```bash
git add tools/ftpdiff/src/cli.rs
git commit -m "ftpdiff: flatten ConnectionArgs into Cli"
```

---

### Task 3: Refactor ftpdiff's `config.rs` to delegate to `ftp_utils_core::connection`

**Files:**
- Modify: `tools/ftpdiff/src/config.rs`

**Interfaces:**
- Consumes: `ftp_utils_core::connection::{self, ConnectionError, ConnectionJsonConfig, EffectiveConnection}` (Task 1), `crate::cli::Cli` (Task 2)
- Produces: `pub type ConfigError = ConnectionError;`, `pub struct JsonConfig { pub connection: ConnectionJsonConfig, pub hash: Option<bool>, pub verbose: Option<bool>, pub exclude: Vec<String> }` (derives `serde::Deserialize`, `Default`; `connection` is `#[serde(flatten)]`ed), `pub struct EffectiveConfig { pub connection: EffectiveConnection, pub hash: bool, pub verbose: bool, pub exclude: Vec<String>, pub csv: Option<PathBuf> }`, `pub fn load_json_config(path: &Path) -> Result<JsonConfig, ConfigError>`, `pub fn merge(cli: &Cli, json: &JsonConfig) -> Result<EffectiveConfig, ConfigError>`, `pub fn read_password(cli_password: Option<&str>) -> Result<String, ConfigError>`

- [ ] **Step 1: Replace the file**

Replace the full contents of `tools/ftpdiff/src/config.rs`:

```rust
//! JSON config loading and CLI/JSON/default merge logic for ftpdiff's own
//! fields (hash, verbose, exclude). Connection fields (host, port, user,
//! password, local/remote dir, ftps, insecure-tls) are handled by
//! `ftp_utils_core::connection`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use ftp_utils_core::connection::{self, ConnectionJsonConfig};

use crate::cli::Cli;

pub use ftp_utils_core::connection::ConnectionError as ConfigError;

#[derive(Debug, Deserialize, Default)]
pub struct JsonConfig {
    #[serde(flatten)]
    pub connection: ConnectionJsonConfig,
    pub hash: Option<bool>,
    pub verbose: Option<bool>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub connection: connection::EffectiveConnection,
    pub hash: bool,
    pub verbose: bool,
    pub exclude: Vec<String>,
    pub csv: Option<PathBuf>,
}

pub fn load_json_config(path: &Path) -> Result<JsonConfig, ConfigError> {
    connection::load_json_config(path)
}

pub fn merge(cli: &Cli, json: &JsonConfig) -> Result<EffectiveConfig, ConfigError> {
    let effective_connection = connection::merge_connection(&cli.connection, &json.connection)?;

    let mut exclude = json.exclude.clone();
    exclude.extend(cli.exclude.clone());

    Ok(EffectiveConfig {
        connection: effective_connection,
        hash: cli.hash || json.hash.unwrap_or(false),
        verbose: cli.verbose || json.verbose.unwrap_or(false),
        exclude,
        csv: cli.csv.clone(),
    })
}

/// Resolves the FTP password using the `FTPDIFF_PASSWORD` environment
/// variable as ftpdiff's fallback source.
pub fn read_password(cli_password: Option<&str>) -> Result<String, ConfigError> {
    connection::read_password(cli_password, "FTPDIFF_PASSWORD")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_cli() -> Cli {
        Cli {
            connection: ftp_utils_core::connection::ConnectionArgs {
                config: None,
                host: None,
                port: None,
                user: None,
                remote_dir: None,
                local_dir: None,
                ftps: false,
                insecure_tls: false,
                password: None,
            },
            hash: false,
            exclude: Vec::new(),
            csv: None,
            verbose: false,
        }
    }

    #[test]
    fn loads_json_config_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"host": "ftp.example.com", "user": "bob", "exclude": ["*.tmp"]}"#,
        )
        .unwrap();

        let config = load_json_config(&path).unwrap();

        assert_eq!(config.connection.host.as_deref(), Some("ftp.example.com"));
        assert_eq!(config.connection.user.as_deref(), Some("bob"));
        assert_eq!(config.exclude, vec!["*.tmp".to_string()]);
    }

    #[test]
    fn hash_and_verbose_merge_as_or() {
        let mut cli = empty_cli();
        cli.connection.host = Some("h".into());
        cli.connection.user = Some("u".into());
        cli.connection.remote_dir = Some("/r".into());
        cli.connection.local_dir = Some(PathBuf::from("./l"));
        cli.hash = true;

        let json = JsonConfig { verbose: Some(true), ..Default::default() };

        let effective = merge(&cli, &json).unwrap();

        assert!(effective.hash); // from CLI
        assert!(effective.verbose); // from JSON
    }

    #[test]
    fn exclude_patterns_are_unioned() {
        let mut cli = empty_cli();
        cli.connection.host = Some("h".into());
        cli.connection.user = Some("u".into());
        cli.connection.remote_dir = Some("/r".into());
        cli.connection.local_dir = Some(PathBuf::from("./l"));
        cli.exclude = vec!["*.tmp".to_string()];

        let json = JsonConfig { exclude: vec![".git/*".to_string()], ..Default::default() };

        let effective = merge(&cli, &json).unwrap();

        assert_eq!(effective.exclude, vec![".git/*".to_string(), "*.tmp".to_string()]);
    }

    #[test]
    fn merge_propagates_connection_errors() {
        let cli = empty_cli();
        let json = JsonConfig::default();

        let result = merge(&cli, &json);

        assert!(result.is_err());
    }

    #[test]
    fn read_password_uses_ftpdiff_env_var() {
        use std::sync::Mutex;
        static ENV_GUARD: Mutex<()> = Mutex::new(());
        let _guard = ENV_GUARD.lock().unwrap();

        std::env::set_var("FTPDIFF_PASSWORD", "from-env");
        let result = read_password(None);
        std::env::remove_var("FTPDIFF_PASSWORD");

        assert_eq!(result.unwrap(), "from-env");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p ftpdiff config::tests`
Expected: PASS (5 tests)

- [ ] **Step 3: Commit**

```bash
git add tools/ftpdiff/src/config.rs
git commit -m "ftpdiff: delegate connection config to ftp_utils_core::connection"
```

---

### Task 4: Update ftpdiff's `main.rs` field access; full verification

**Files:**
- Modify: `tools/ftpdiff/src/main.rs`

**Interfaces:**
- Consumes: `EffectiveConfig.connection: EffectiveConnection` (Task 3), `Cli.connection: ConnectionArgs` (Task 2)

- [ ] **Step 1: Update field access**

In `tools/ftpdiff/src/main.rs`, every reference to a connection field on `effective` gains a `.connection` segment, and `cli.password` becomes `cli.connection.password`. Replace the full contents:

```rust
//! ftpdiff: compare a remote FTP/FTPS directory tree against a local copy.
//!
//! See `docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md`
//! for the design this binary implements.

mod cli;
mod config;
mod csv_report;
mod output;

use clap::Parser;
use ftp_utils_core::ftp_client::SuppaFtpConnection;
use ftp_utils_core::{compare, CompareOptions, DiffStatus};

use cli::Cli;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let cli = Cli::parse();

    let json_config = match &cli.connection.config {
        Some(path) => match config::load_json_config(path) {
            Ok(loaded) => loaded,
            Err(e) => {
                eprintln!("Error: {e}");
                return 2;
            }
        },
        None => config::JsonConfig::default(),
    };

    let effective = match config::merge(&cli, &json_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {e}");
            return 2;
        }
    };

    let password = match config::read_password(cli.connection.password.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: {e}");
            return 2;
        }
    };

    if effective.verbose {
        eprintln!(
            "Connecting to {}:{} as {} ({})...",
            effective.connection.host,
            effective.connection.port,
            effective.connection.user,
            if effective.connection.ftps { "FTPS" } else { "FTP" }
        );
    }

    let mut connection = match SuppaFtpConnection::connect(
        &effective.connection.host,
        effective.connection.port,
        &effective.connection.user,
        &password,
        effective.connection.ftps,
        effective.connection.insecure_tls,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "Error: failed to connect to {}:{}: {e}",
                effective.connection.host, effective.connection.port
            );
            return 2;
        }
    };

    if effective.verbose {
        eprintln!(
            "Connected. Comparing {} against {}...",
            effective.connection.local_dir.display(),
            effective.connection.remote_dir
        );
    }

    let options = CompareOptions {
        local_dir: effective.connection.local_dir.clone(),
        remote_dir: effective.connection.remote_dir.clone(),
        excludes: effective.exclude.clone(),
        hash: effective.hash,
    };

    let mut progress: Option<Box<dyn FnMut(&str)>> = if effective.verbose {
        Some(Box::new(|msg: &str| eprintln!("Checking {msg}")))
    } else {
        None
    };

    let entries = match compare(&mut connection, &options, progress.as_deref_mut()) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("Error: comparison failed: {e}");
            return 2;
        }
    };

    connection.close();

    if effective.verbose {
        eprintln!("Comparison done: {} entries.", entries.len());
    }

    for entry in &entries {
        println!("{}", output::format_entry(entry));
    }
    let summary = output::summarize(&entries);
    println!("{}", output::format_summary(&summary));

    if let Some(csv_path) = &effective.csv {
        if effective.verbose {
            eprintln!("Writing CSV report to {}...", csv_path.display());
        }
        if let Err(e) = csv_report::write_csv(csv_path, &entries) {
            eprintln!("Error: failed to write CSV to {}: {e}", csv_path.display());
            return 2;
        }
    }

    if entries.iter().any(|e| e.status != DiffStatus::Match) {
        1
    } else {
        0
    }
}
```

- [ ] **Step 2: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS, same total test count as before this plan started (the refactor moves tests, it doesn't add or remove coverage)

- [ ] **Step 3: Build and smoke-test the CLI**

Run: `cargo build --workspace && cargo run -p ftpdiff -- --help`
Expected: builds cleanly; `--help` output is unchanged from before this refactor (same flags, same descriptions) - confirms the `#[command(flatten)]` didn't alter the CLI surface

- [ ] **Step 4: Update CHANGES.md**

Append to the `## Unreleased` section of `CHANGES.md`:

```markdown
- Refactored ftpdiff's connection/config/password handling into a shared
  `ftp_utils_core::connection` module (`ConnectionArgs`, `ConnectionJsonConfig`,
  `merge_connection`, `read_password`), in preparation for the `ftpops`
  tool which needs the same host/user/password/local-dir/remote-dir/ftps/
  insecure-tls handling. No user-visible change to ftpdiff's CLI or config
  file format.
```

- [ ] **Step 5: Commit**

```bash
git add tools/ftpdiff/src/main.rs CHANGES.md
git commit -m "ftpdiff: update main.rs for the connection module refactor"
```

---

### Task 5: Extend `FtpConnection` with upload/delete/mkdir primitives

**Files:**
- Modify: `crates/ftp-utils-core/src/remote.rs`

**Interfaces:**
- Produces (added to the existing `pub trait FtpConnection`): `fn store_from_buffer(&mut self, path: &str, data: &[u8]) -> Result<(), FtpConnectionError>`, `fn delete(&mut self, path: &str) -> Result<(), FtpConnectionError>`, `fn create_dir(&mut self, path: &str) -> Result<(), FtpConnectionError>`, and a **default-implemented** `fn ensure_remote_dir(&mut self, path: &str) -> Result<(), FtpConnectionError>` built on `list_dir` + `create_dir` (so any `FtpConnection` implementer gets it for free and it's unit-testable via the mock).

- [ ] **Step 1: Write the failing tests**

In `crates/ftp-utils-core/src/remote.rs`, add the three new required methods to the `FtpConnection` trait and the default `ensure_remote_dir` method. First, locate the existing trait definition:

```rust
pub trait FtpConnection {
    /// Lists the direct children of `path` (files and directories).
    fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError>;
    /// Attempts to get a server-computed hash for `path` without
    /// downloading it. Returns `None` if the server doesn't support it.
    fn try_hash(&mut self, path: &str) -> Option<String>;
    /// Downloads the full contents of `path` into memory.
    fn retr_to_buffer(&mut self, path: &str) -> Result<Vec<u8>, FtpConnectionError>;
}
```

Replace it with:

```rust
pub trait FtpConnection {
    /// Lists the direct children of `path` (files and directories).
    fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError>;
    /// Attempts to get a server-computed hash for `path` without
    /// downloading it. Returns `None` if the server doesn't support it.
    fn try_hash(&mut self, path: &str) -> Option<String>;
    /// Downloads the full contents of `path` into memory.
    fn retr_to_buffer(&mut self, path: &str) -> Result<Vec<u8>, FtpConnectionError>;
    /// Uploads `data` to `path`, overwriting any existing remote file.
    fn store_from_buffer(&mut self, path: &str, data: &[u8]) -> Result<(), FtpConnectionError>;
    /// Deletes the remote file at `path`.
    fn delete(&mut self, path: &str) -> Result<(), FtpConnectionError>;
    /// Creates a single directory at `path`. Its parent must already exist.
    fn create_dir(&mut self, path: &str) -> Result<(), FtpConnectionError>;

    /// Ensures every directory component of `path`'s parent exists,
    /// creating any that are missing. `path` is the full remote path to a
    /// file; only its parent directories are created. Built generically
    /// on `list_dir` and `create_dir`, so any implementer gets it for
    /// free without needing its own recursive-mkdir logic.
    fn ensure_remote_dir(&mut self, path: &str) -> Result<(), FtpConnectionError> {
        let parent = match path.rfind('/') {
            Some(idx) if idx > 0 => &path[..idx],
            _ => return Ok(()),
        };

        let mut current = String::new();
        for component in parent.split('/').filter(|c| !c.is_empty()) {
            let listing_parent = if current.is_empty() { "/".to_string() } else { current.clone() };
            current.push('/');
            current.push_str(component);

            let existing = self.list_dir(&listing_parent)?;
            if existing.iter().any(|e| e.is_dir && e.name == component) {
                continue;
            }

            self.create_dir(&current)?;
        }

        Ok(())
    }
}
```

Then extend the test module's `MockFtpConnection` (already in this file) to implement the three new required methods, and add new tests. Find the existing `impl FtpConnection for MockFtpConnection` block:

```rust
    impl FtpConnection for MockFtpConnection {
        fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
            self.listings
                .get(path)
                .cloned()
                .ok_or_else(|| FtpConnectionError(format!("no listing for {path}")))
        }

        fn try_hash(&mut self, _path: &str) -> Option<String> {
            None
        }

        fn retr_to_buffer(&mut self, _path: &str) -> Result<Vec<u8>, FtpConnectionError> {
            Ok(Vec::new())
        }
    }
```

Replace it with a version that tracks created directories (needed by the
new `ensure_remote_dir` tests) and also updates `listings` when a
directory is created, so a second `ensure_remote_dir` call sees it as
already existing:

```rust
    impl FtpConnection for MockFtpConnection {
        fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
            self.listings
                .get(path)
                .cloned()
                .ok_or_else(|| FtpConnectionError(format!("no listing for {path}")))
        }

        fn try_hash(&mut self, _path: &str) -> Option<String> {
            None
        }

        fn retr_to_buffer(&mut self, _path: &str) -> Result<Vec<u8>, FtpConnectionError> {
            Ok(Vec::new())
        }

        fn store_from_buffer(&mut self, _path: &str, _data: &[u8]) -> Result<(), FtpConnectionError> {
            Ok(())
        }

        fn delete(&mut self, _path: &str) -> Result<(), FtpConnectionError> {
            Ok(())
        }

        fn create_dir(&mut self, path: &str) -> Result<(), FtpConnectionError> {
            self.created_dirs.push(path.to_string());

            let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
            let parent = if parent.is_empty() { "/" } else { parent };

            self.listings
                .entry(parent.to_string())
                .or_default()
                .push(RawRemoteEntry { name: name.to_string(), is_dir: true, size: 0 });
            self.listings.entry(path.to_string()).or_default();

            Ok(())
        }
    }
```

Add the `created_dirs` field to `MockFtpConnection`'s definition (find `struct MockFtpConnection { listings: HashMap<String, Vec<RawRemoteEntry>> }` and add a field):

```rust
    struct MockFtpConnection {
        listings: HashMap<String, Vec<RawRemoteEntry>>,
        created_dirs: Vec<String>,
    }
```

Every existing test constructing `MockFtpConnection { listings }` now needs `created_dirs: Vec::new()` too - update each of the three existing construction sites (`walks_nested_directories`, `applies_exclude_patterns`, `reports_progress_for_included_files_only`) from:

```rust
        let mut conn = MockFtpConnection { listings };
```

to:

```rust
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };
```

Then add new tests at the end of the `tests` module (before the closing `}`):

```rust
    #[test]
    fn ensure_remote_dir_creates_missing_components() {
        let mut listings = HashMap::new();
        listings.insert("/".to_string(), vec![]);
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };

        conn.ensure_remote_dir("/a/b/file.txt").unwrap();

        assert_eq!(conn.created_dirs, vec!["/a".to_string(), "/a/b".to_string()]);
    }

    #[test]
    fn ensure_remote_dir_skips_components_that_already_exist() {
        let mut listings = HashMap::new();
        listings.insert("/".to_string(), vec![RawRemoteEntry { name: "a".into(), is_dir: true, size: 0 }]);
        listings.insert("/a".to_string(), vec![]);
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };

        conn.ensure_remote_dir("/a/b/file.txt").unwrap();

        assert_eq!(conn.created_dirs, vec!["/a/b".to_string()]);
    }

    #[test]
    fn ensure_remote_dir_is_a_noop_for_root_level_files() {
        let mut listings = HashMap::new();
        listings.insert("/".to_string(), vec![]);
        let mut conn = MockFtpConnection { listings, created_dirs: Vec::new() };

        conn.ensure_remote_dir("/file.txt").unwrap();

        assert!(conn.created_dirs.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail (compile error first, since the trait now has unimplemented required methods; then logic failures once it compiles)**

Run: `cargo test -p ftp-utils-core remote::tests`
Expected: FAIL to compile until the `MockFtpConnection` impl block above is in place; once in place, `ensure_remote_dir_*` tests should already pass because `ensure_remote_dir`'s implementation was written directly (it's a default trait method, not a `todo!()` - there's no separate red step for it here since its correctness is exactly what step 1's tests check)

- [ ] **Step 3: Run tests to confirm everything passes**

Run: `cargo test -p ftp-utils-core remote::tests`
Expected: PASS (5 existing + 3 new = 8 tests)

- [ ] **Step 4: Update the other two `FtpConnection` mocks so the crate compiles**

`crates/ftp-utils-core/src/hash.rs`'s `MockConnection` and
`crates/ftp-utils-core/src/lib.rs`'s `MockConnection` each need the three
new methods. In `crates/ftp-utils-core/src/hash.rs`, find:

```rust
    impl FtpConnection for MockConnection {
        fn list_dir(&mut self, _path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
            Ok(Vec::new())
        }

        fn try_hash(&mut self, _path: &str) -> Option<String> {
            self.hash.clone()
        }

        fn retr_to_buffer(&mut self, _path: &str) -> Result<Vec<u8>, FtpConnectionError> {
            Ok(self.remote_bytes.clone())
        }
    }
```

Add three methods to it:

```rust
    impl FtpConnection for MockConnection {
        fn list_dir(&mut self, _path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
            Ok(Vec::new())
        }

        fn try_hash(&mut self, _path: &str) -> Option<String> {
            self.hash.clone()
        }

        fn retr_to_buffer(&mut self, _path: &str) -> Result<Vec<u8>, FtpConnectionError> {
            Ok(self.remote_bytes.clone())
        }

        fn store_from_buffer(&mut self, _path: &str, _data: &[u8]) -> Result<(), FtpConnectionError> {
            Ok(())
        }

        fn delete(&mut self, _path: &str) -> Result<(), FtpConnectionError> {
            Ok(())
        }

        fn create_dir(&mut self, _path: &str) -> Result<(), FtpConnectionError> {
            Ok(())
        }
    }
```

In `crates/ftp-utils-core/src/lib.rs`, find the analogous block:

```rust
    impl FtpConnection for MockConnection {
        fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
            self.listings
                .get(path)
                .cloned()
                .ok_or_else(|| FtpConnectionError(format!("no listing for {path}")))
        }

        fn try_hash(&mut self, _path: &str) -> Option<String> {
            None
        }

        fn retr_to_buffer(&mut self, _path: &str) -> Result<Vec<u8>, FtpConnectionError> {
            Ok(Vec::new())
        }
    }
```

Add the same three methods:

```rust
    impl FtpConnection for MockConnection {
        fn list_dir(&mut self, path: &str) -> Result<Vec<RawRemoteEntry>, FtpConnectionError> {
            self.listings
                .get(path)
                .cloned()
                .ok_or_else(|| FtpConnectionError(format!("no listing for {path}")))
        }

        fn try_hash(&mut self, _path: &str) -> Option<String> {
            None
        }

        fn retr_to_buffer(&mut self, _path: &str) -> Result<Vec<u8>, FtpConnectionError> {
            Ok(Vec::new())
        }

        fn store_from_buffer(&mut self, _path: &str, _data: &[u8]) -> Result<(), FtpConnectionError> {
            Ok(())
        }

        fn delete(&mut self, _path: &str) -> Result<(), FtpConnectionError> {
            Ok(())
        }

        fn create_dir(&mut self, _path: &str) -> Result<(), FtpConnectionError> {
            Ok(())
        }
    }
```

- [ ] **Step 5: Run the full core test suite**

Run: `cargo test -p ftp-utils-core`
Expected: PASS (everything compiles and passes, including `hash.rs` and `lib.rs` tests that don't exercise the three new no-op methods directly)

- [ ] **Step 6: Commit**

```bash
git add crates/ftp-utils-core/src/remote.rs crates/ftp-utils-core/src/hash.rs crates/ftp-utils-core/src/lib.rs
git commit -m "ftp-utils-core: extend FtpConnection with store/delete/create_dir + ensure_remote_dir"
```

---

### Task 6: Implement the three new methods in `SuppaFtpConnection`

**Files:**
- Modify: `crates/ftp-utils-core/src/ftp_client.rs`

**Interfaces:**
- Consumes: `remote::FtpConnection` (Task 5, extended)
- Produces: `SuppaFtpConnection`'s `impl FtpConnection` gains `store_from_buffer`, `delete`, `create_dir` (real network calls; `ensure_remote_dir` is inherited from the trait's default implementation, unchanged)

This task has no automated test (it requires a real or containerized FTP
server that also supports STOR/DELE/MKD, none of which the current test
fixture setup covers). It's verified by compiling and should get a manual
smoke test (documented below) before `ftpops` relies on it in Task... of
the next plan.

- [ ] **Step 1: Implement**

In `crates/ftp-utils-core/src/ftp_client.rs`, find the closing of the
existing `impl FtpConnection for SuppaFtpConnection` block (the
`retr_to_buffer` method and its closing `}`):

```rust
    fn retr_to_buffer(&mut self, path: &str) -> Result<Vec<u8>, FtpConnectionError> {
        let cursor = match self {
            SuppaFtpConnection::Plain(stream) => stream.retr_as_buffer(path),
            SuppaFtpConnection::Tls(stream) => stream.retr_as_buffer(path),
        }
        .map_err(|e| FtpConnectionError(e.to_string()))?;
        Ok(cursor.into_inner())
    }
}
```

Replace it with (adding three methods before the closing `}`):

```rust
    fn retr_to_buffer(&mut self, path: &str) -> Result<Vec<u8>, FtpConnectionError> {
        let cursor = match self {
            SuppaFtpConnection::Plain(stream) => stream.retr_as_buffer(path),
            SuppaFtpConnection::Tls(stream) => stream.retr_as_buffer(path),
        }
        .map_err(|e| FtpConnectionError(e.to_string()))?;
        Ok(cursor.into_inner())
    }

    fn store_from_buffer(&mut self, path: &str, data: &[u8]) -> Result<(), FtpConnectionError> {
        let mut reader = std::io::Cursor::new(data);
        match self {
            SuppaFtpConnection::Plain(stream) => stream.put_file(path, &mut reader),
            SuppaFtpConnection::Tls(stream) => stream.put_file(path, &mut reader),
        }
        .map_err(|e| FtpConnectionError(e.to_string()))?;
        Ok(())
    }

    fn delete(&mut self, path: &str) -> Result<(), FtpConnectionError> {
        match self {
            SuppaFtpConnection::Plain(stream) => stream.rm(path),
            SuppaFtpConnection::Tls(stream) => stream.rm(path),
        }
        .map_err(|e| FtpConnectionError(e.to_string()))
    }

    fn create_dir(&mut self, path: &str) -> Result<(), FtpConnectionError> {
        match self {
            SuppaFtpConnection::Plain(stream) => stream.mkdir(path),
            SuppaFtpConnection::Tls(stream) => stream.mkdir(path),
        }
        .map_err(|e| FtpConnectionError(e.to_string()))
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p ftp-utils-core`
Expected: builds cleanly

- [ ] **Step 3: Commit**

```bash
git add crates/ftp-utils-core/src/ftp_client.rs
git commit -m "ftp-utils-core: implement store/delete/create_dir for SuppaFtpConnection"
```

---

### Task 7: Final verification and spec amendment

**Files:**
- Modify: `docs/superpowers/specs/2026-09-21-ftpops-design.md`
- Modify: `CHANGES.md`

- [ ] **Step 1: Run the full workspace test suite and build**

Run: `cargo test --workspace && cargo build --workspace`
Expected: PASS, no warnings

- [ ] **Step 2: Amend the ftpops spec's section 2**

In `docs/superpowers/specs/2026-09-21-ftpops-design.md`, under `## 2.
FtpConnection trait extension`, add a note after the existing code block
(before `SuppaFtpConnection implements:`) explaining the refinement made
during implementation:

```markdown
**Refinement made during implementation (2026-09-21):** `ensure_remote_dir`
is a **default-implemented** trait method built on `list_dir` and a new
`create_dir` primitive (single-directory `mkdir`, parent must already
exist), rather than something each `FtpConnection` implementer writes
independently. This makes its component-splitting and already-exists
logic unit-testable via the mock, matching this spec's stated testing
intent, and means `SuppaFtpConnection` only needs to implement the
simpler `create_dir`.
```

- [ ] **Step 3: Update CHANGES.md**

Append to the `## Unreleased` section of `CHANGES.md`:

```markdown
- Extended `FtpConnection` with `store_from_buffer`, `delete`, and
  `create_dir`, plus a default-implemented `ensure_remote_dir` built on
  `list_dir` + `create_dir` (unit-tested via the mock; no live-server
  test yet for `SuppaFtpConnection`'s implementation - recommended before
  the upcoming `ftpops` tool relies on it against production data).
```

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-09-21-ftpops-design.md CHANGES.md
git commit -m "Amend ftpops spec with ensure_remote_dir refinement; update CHANGES"
```

---

## Self-Review Notes

- **Spec coverage:** section 1 (shared connection module) - Tasks 1-4. Section 2 (FtpConnection extension) - Tasks 5-6, with the `ensure_remote_dir`-as-default-method refinement documented and back-ported into the spec in Task 7. Section 3 (`ftpops` itself) is intentionally out of scope for this plan - it's a separate plan per the user's decomposition decision.
- **Placeholder scan:** the two `todo!()` markers in Task 1 are the intentional TDD red-step scaffolding, replaced within the same task.
- **Type consistency:** `ConnectionArgs`/`ConnectionJsonConfig`/`EffectiveConnection`/`ConnectionError` (Task 1) are used unchanged through ftpdiff's `cli.rs` (Task 2), `config.rs` (Task 3), and `main.rs` (Task 4). `FtpConnection`'s three new methods plus `ensure_remote_dir` (Task 5) are implemented identically (by name and signature) in all three existing mocks (`remote.rs`, `hash.rs`, `lib.rs`, Task 5) and in `SuppaFtpConnection` (Task 6).
