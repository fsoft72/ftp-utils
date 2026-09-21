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
    let content = std::fs::read_to_string(path)
        .map_err(|e| ConnectionError(format!("cannot read config file {}: {e}", path.display())))?;
    serde_json::from_str(&content)
        .map_err(|e| ConnectionError(format!("invalid JSON in {}: {e}", path.display())))
}

/// Merges CLI args, JSON config, and defaults into `EffectiveConnection`.
/// Precedence: CLI > JSON > default. Boolean flags merge as OR (CLI can
/// enable, never disable, a JSON-set true).
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
