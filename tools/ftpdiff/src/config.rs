//! JSON config loading and CLI/JSON/default merge logic.
//!
//! Precedence: CLI args > JSON config file > defaults. Boolean flags merge
//! as OR (CLI can enable, never disable, a JSON-set true). `--exclude`
//! patterns merge as a union of both sources.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::cli::Cli;

#[derive(Debug, Deserialize, Default)]
pub struct JsonConfig {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub remote_dir: Option<String>,
    pub local_dir: Option<PathBuf>,
    pub ftps: Option<bool>,
    pub hash: Option<bool>,
    pub insecure_tls: Option<bool>,
    pub verbose: Option<bool>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub remote_dir: String,
    pub local_dir: PathBuf,
    pub ftps: bool,
    pub hash: bool,
    pub insecure_tls: bool,
    pub verbose: bool,
    pub exclude: Vec<String>,
    pub csv: Option<PathBuf>,
}

#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConfigError {}

pub fn load_json_config(path: &Path) -> Result<JsonConfig, ConfigError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(format!("cannot read config file {}: {e}", path.display())))?;
    serde_json::from_str(&content)
        .map_err(|e| ConfigError(format!("invalid JSON in {}: {e}", path.display())))
}

pub fn merge(cli: &Cli, json: &JsonConfig) -> Result<EffectiveConfig, ConfigError> {
    let host = cli
        .host
        .clone()
        .or_else(|| json.host.clone())
        .ok_or_else(|| ConfigError("missing required setting: host (use --host or config file)".to_string()))?;

    let user = cli
        .user
        .clone()
        .or_else(|| json.user.clone())
        .ok_or_else(|| ConfigError("missing required setting: user (use --user or config file)".to_string()))?;

    let remote_dir = cli
        .remote_dir
        .clone()
        .or_else(|| json.remote_dir.clone())
        .ok_or_else(|| {
            ConfigError("missing required setting: remote-dir (use --remote-dir or config file)".to_string())
        })?;

    let local_dir = cli
        .local_dir
        .clone()
        .or_else(|| json.local_dir.clone())
        .ok_or_else(|| {
            ConfigError("missing required setting: local-dir (use --local-dir or config file)".to_string())
        })?;

    let mut exclude = json.exclude.clone();
    exclude.extend(cli.exclude.clone());

    Ok(EffectiveConfig {
        host,
        port: cli.port.or(json.port).unwrap_or(21),
        user,
        remote_dir,
        local_dir,
        ftps: cli.ftps || json.ftps.unwrap_or(false),
        hash: cli.hash || json.hash.unwrap_or(false),
        insecure_tls: cli.insecure_tls || json.insecure_tls.unwrap_or(false),
        verbose: cli.verbose || json.verbose.unwrap_or(false),
        exclude,
        csv: cli.csv.clone(),
    })
}

/// Resolves the FTP password from the CLI flag or the `FTPDIFF_PASSWORD`
/// environment variable, in that order. Returns `None` if neither is set,
/// meaning the caller should fall back to an interactive prompt.
fn password_from_cli_or_env(cli_password: Option<&str>) -> Option<String> {
    cli_password
        .map(|p| p.to_string())
        .or_else(|| std::env::var("FTPDIFF_PASSWORD").ok())
}

/// Resolves the FTP password: `--password` flag, then `FTPDIFF_PASSWORD`
/// environment variable, then an interactive hidden-input prompt. Prefer
/// the environment variable or the prompt over the flag: a CLI argument
/// can leak into shell history and process listings.
pub fn read_password(cli_password: Option<&str>) -> Result<String, ConfigError> {
    if let Some(password) = password_from_cli_or_env(cli_password) {
        return Ok(password);
    }

    rpassword::prompt_password("FTP password: ")
        .map_err(|e| ConfigError(format!("failed to read password from terminal: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // FTPDIFF_PASSWORD is process-global state; serialize the two tests
    // that touch it so they can't race under parallel test execution.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    #[test]
    fn resolves_password_from_env_var_when_cli_absent() {
        let _guard = ENV_GUARD.lock().unwrap();
        std::env::set_var("FTPDIFF_PASSWORD", "s3cr3t");

        let result = password_from_cli_or_env(None);

        assert_eq!(result, Some("s3cr3t".to_string()));
        std::env::remove_var("FTPDIFF_PASSWORD");
    }

    #[test]
    fn cli_password_takes_precedence_over_env_var() {
        let _guard = ENV_GUARD.lock().unwrap();
        std::env::set_var("FTPDIFF_PASSWORD", "env-password");

        let result = password_from_cli_or_env(Some("cli-password"));

        assert_eq!(result, Some("cli-password".to_string()));
        std::env::remove_var("FTPDIFF_PASSWORD");
    }

    #[test]
    fn returns_none_when_neither_cli_nor_env_password_set() {
        let _guard = ENV_GUARD.lock().unwrap();
        std::env::remove_var("FTPDIFF_PASSWORD");

        let result = password_from_cli_or_env(None);

        assert_eq!(result, None);
    }

    fn empty_cli() -> Cli {
        Cli {
            config: None,
            host: None,
            port: None,
            user: None,
            remote_dir: None,
            local_dir: None,
            ftps: false,
            hash: false,
            insecure_tls: false,
            verbose: false,
            exclude: Vec::new(),
            csv: None,
            password: None,
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

        assert_eq!(config.host.as_deref(), Some("ftp.example.com"));
        assert_eq!(config.user.as_deref(), Some("bob"));
        assert_eq!(config.exclude, vec!["*.tmp".to_string()]);
    }

    #[test]
    fn cli_overrides_json_for_scalar_fields() {
        let mut cli = empty_cli();
        cli.host = Some("cli-host".to_string());
        cli.user = Some("cli-user".to_string());
        cli.remote_dir = Some("/cli-remote".to_string());
        cli.local_dir = Some(PathBuf::from("./cli-local"));

        let json = JsonConfig {
            host: Some("json-host".to_string()),
            user: Some("json-user".to_string()),
            remote_dir: Some("/json-remote".to_string()),
            local_dir: Some(PathBuf::from("./json-local")),
            ..Default::default()
        };

        let effective = merge(&cli, &json).unwrap();

        assert_eq!(effective.host, "cli-host");
        assert_eq!(effective.user, "cli-user");
        assert_eq!(effective.remote_dir, "/cli-remote");
        assert_eq!(effective.local_dir, PathBuf::from("./cli-local"));
    }

    #[test]
    fn falls_back_to_json_when_cli_field_absent() {
        let cli = empty_cli();
        let json = JsonConfig {
            host: Some("json-host".to_string()),
            user: Some("json-user".to_string()),
            remote_dir: Some("/json-remote".to_string()),
            local_dir: Some(PathBuf::from("./json-local")),
            ..Default::default()
        };

        let effective = merge(&cli, &json).unwrap();

        assert_eq!(effective.host, "json-host");
        assert_eq!(effective.user, "json-user");
    }

    #[test]
    fn errors_when_required_field_missing_everywhere() {
        let cli = empty_cli();
        let json = JsonConfig::default();

        let result = merge(&cli, &json);

        assert!(result.is_err());
    }

    #[test]
    fn boolean_flags_merge_as_or() {
        let mut cli = empty_cli();
        cli.host = Some("h".into());
        cli.user = Some("u".into());
        cli.remote_dir = Some("/r".into());
        cli.local_dir = Some(PathBuf::from("./l"));
        cli.ftps = true;

        let json = JsonConfig { hash: Some(true), ..Default::default() };

        let effective = merge(&cli, &json).unwrap();

        assert!(effective.ftps); // from CLI
        assert!(effective.hash); // from JSON
    }

    #[test]
    fn insecure_tls_and_verbose_merge_as_or() {
        let mut cli = empty_cli();
        cli.host = Some("h".into());
        cli.user = Some("u".into());
        cli.remote_dir = Some("/r".into());
        cli.local_dir = Some(PathBuf::from("./l"));
        cli.insecure_tls = true;

        let json = JsonConfig { verbose: Some(true), ..Default::default() };

        let effective = merge(&cli, &json).unwrap();

        assert!(effective.insecure_tls); // from CLI
        assert!(effective.verbose); // from JSON
    }

    #[test]
    fn exclude_patterns_are_unioned() {
        let mut cli = empty_cli();
        cli.host = Some("h".into());
        cli.user = Some("u".into());
        cli.remote_dir = Some("/r".into());
        cli.local_dir = Some(PathBuf::from("./l"));
        cli.exclude = vec!["*.tmp".to_string()];

        let json = JsonConfig { exclude: vec![".git/*".to_string()], ..Default::default() };

        let effective = merge(&cli, &json).unwrap();

        assert_eq!(effective.exclude, vec![".git/*".to_string(), "*.tmp".to_string()]);
    }

    #[test]
    fn default_port_is_21() {
        let mut cli = empty_cli();
        cli.host = Some("h".into());
        cli.user = Some("u".into());
        cli.remote_dir = Some("/r".into());
        cli.local_dir = Some(PathBuf::from("./l"));

        let effective = merge(&cli, &JsonConfig::default()).unwrap();

        assert_eq!(effective.port, 21);
    }
}
