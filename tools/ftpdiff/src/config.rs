//! JSON config loading and CLI/JSON/default merge logic for ftpdiff's own
//! fields (hash, verbose, exclude, local-csv, remote-csv). Connection
//! fields (host, port, user, password, local/remote dir, ftps,
//! insecure-tls) are handled by `ftp_utils_core::connection`.

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

/// Where the local side of the comparison comes from.
#[derive(Debug, Clone)]
pub enum LocalSource {
    Live(PathBuf),
    Csv(PathBuf),
}

/// Where the remote side of the comparison comes from.
#[derive(Debug, Clone)]
pub enum RemoteSource {
    Live { host: String, port: u16, user: String, remote_dir: String, ftps: bool, insecure_tls: bool },
    Csv(PathBuf),
}

#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub local: LocalSource,
    pub remote: RemoteSource,
    pub hash: bool,
    pub verbose: bool,
    pub exclude: Vec<String>,
    pub csv: Option<PathBuf>,
}

pub fn load_json_config(path: &Path) -> Result<JsonConfig, ConfigError> {
    connection::load_json_config(path)
}

/// Resolves `LocalSource`/`RemoteSource` and ftpdiff's own fields.
/// `--local-csv` makes `--local-dir` unnecessary; `--remote-csv` makes
/// `--host`/`--user`/`--remote-dir` (and password/FTPS settings)
/// unnecessary. If both a CSV flag and its live counterpart are given,
/// the CSV flag takes precedence - the live setting is simply unused,
/// not an error (so a shared `--config` file can supply connection
/// defaults used by other invocations without breaking a CSV-sourced
/// one).
pub fn merge(cli: &Cli, json: &JsonConfig) -> Result<EffectiveConfig, ConfigError> {
    let partial = connection::merge_connection_partial(&cli.connection, &json.connection);

    let local = if let Some(path) = &cli.local_csv {
        LocalSource::Csv(path.clone())
    } else {
        let local_dir = partial.local_dir.clone().ok_or_else(|| {
            ConfigError(
                "missing required setting: local-dir (use --local-dir, --local-csv, or config file)".to_string(),
            )
        })?;
        LocalSource::Live(local_dir)
    };

    let remote = if let Some(path) = &cli.remote_csv {
        RemoteSource::Csv(path.clone())
    } else {
        let host = partial.host.clone().ok_or_else(|| {
            ConfigError("missing required setting: host (use --host, config file, or --remote-csv)".to_string())
        })?;
        let user = partial.user.clone().ok_or_else(|| {
            ConfigError("missing required setting: user (use --user, config file, or --remote-csv)".to_string())
        })?;
        let remote_dir = partial.remote_dir.clone().ok_or_else(|| {
            ConfigError(
                "missing required setting: remote-dir (use --remote-dir, config file, or --remote-csv)".to_string(),
            )
        })?;
        RemoteSource::Live {
            host,
            port: partial.port,
            user,
            remote_dir,
            ftps: partial.ftps,
            insecure_tls: partial.insecure_tls,
        }
    };

    let mut exclude = json.exclude.clone();
    exclude.extend(cli.exclude.clone());

    Ok(EffectiveConfig {
        local,
        remote,
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
            local_csv: None,
            remote_csv: None,
        }
    }

    fn live_cli() -> Cli {
        let mut cli = empty_cli();
        cli.connection.host = Some("h".into());
        cli.connection.user = Some("u".into());
        cli.connection.remote_dir = Some("/r".into());
        cli.connection.local_dir = Some(PathBuf::from("./l"));
        cli
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
    fn fully_live_resolves_both_sides() {
        let cli = live_cli();

        let effective = merge(&cli, &JsonConfig::default()).unwrap();

        assert!(matches!(effective.local, LocalSource::Live(ref p) if p == &PathBuf::from("./l")));
        assert!(matches!(effective.remote, RemoteSource::Live { ref host, .. } if host == "h"));
    }

    #[test]
    fn local_csv_makes_local_dir_unnecessary() {
        let mut cli = empty_cli();
        cli.connection.host = Some("h".into());
        cli.connection.user = Some("u".into());
        cli.connection.remote_dir = Some("/r".into());
        cli.local_csv = Some(PathBuf::from("snapshot.csv"));

        let effective = merge(&cli, &JsonConfig::default()).unwrap();

        assert!(matches!(effective.local, LocalSource::Csv(ref p) if p == &PathBuf::from("snapshot.csv")));
    }

    #[test]
    fn remote_csv_makes_connection_fields_unnecessary() {
        let mut cli = empty_cli();
        cli.connection.local_dir = Some(PathBuf::from("./l"));
        cli.remote_csv = Some(PathBuf::from("snapshot.csv"));

        let effective = merge(&cli, &JsonConfig::default()).unwrap();

        assert!(matches!(effective.remote, RemoteSource::Csv(ref p) if p == &PathBuf::from("snapshot.csv")));
    }

    #[test]
    fn both_csv_sources_need_nothing_else() {
        let mut cli = empty_cli();
        cli.local_csv = Some(PathBuf::from("local.csv"));
        cli.remote_csv = Some(PathBuf::from("remote.csv"));

        assert!(merge(&cli, &JsonConfig::default()).is_ok());
    }

    #[test]
    fn local_csv_takes_precedence_over_local_dir_when_both_given() {
        let mut cli = live_cli();
        cli.local_csv = Some(PathBuf::from("snapshot.csv"));

        let effective = merge(&cli, &JsonConfig::default()).unwrap();

        assert!(matches!(effective.local, LocalSource::Csv(ref p) if p == &PathBuf::from("snapshot.csv")));
    }

    #[test]
    fn errors_when_local_dir_missing_and_no_local_csv() {
        let mut cli = empty_cli();
        cli.remote_csv = Some(PathBuf::from("remote.csv"));

        assert!(merge(&cli, &JsonConfig::default()).is_err());
    }

    #[test]
    fn errors_when_host_missing_and_no_remote_csv() {
        let mut cli = empty_cli();
        cli.local_csv = Some(PathBuf::from("local.csv"));

        assert!(merge(&cli, &JsonConfig::default()).is_err());
    }

    #[test]
    fn hash_and_verbose_merge_as_or() {
        let mut cli = live_cli();
        cli.hash = true;

        let json = JsonConfig { verbose: Some(true), ..Default::default() };

        let effective = merge(&cli, &json).unwrap();

        assert!(effective.hash); // from CLI
        assert!(effective.verbose); // from JSON
    }

    #[test]
    fn exclude_patterns_are_unioned() {
        let mut cli = live_cli();
        cli.exclude = vec!["*.tmp".to_string()];

        let json = JsonConfig { exclude: vec![".git/*".to_string()], ..Default::default() };

        let effective = merge(&cli, &json).unwrap();

        assert_eq!(effective.exclude, vec![".git/*".to_string(), "*.tmp".to_string()]);
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
