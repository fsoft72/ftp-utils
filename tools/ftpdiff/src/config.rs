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
