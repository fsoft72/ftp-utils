//! Loads a site's `config.json`: the shared connection fields plus the
//! list of exclude globs.

use std::path::Path;

use serde::Deserialize;

use ftp_utils_core::connection::{
    self, ConnectionError, ConnectionJsonConfig, RemoteParams, DEFAULT_FTP_PORT, DEFAULT_TIMEOUT_SECS,
};
use ftp_utils_core::exclude::ExcludeSet;

/// Raw shape of config.json.
#[derive(Debug, Deserialize, Default)]
struct SiteJson {
    #[serde(flatten)]
    connection: ConnectionJsonConfig,
    #[serde(default)]
    exclude: Vec<String>,
}

/// Everything `init` and `check` need from `config.json`.
pub struct SiteSettings {
    pub remote: RemoteParams,
    pub exclude: ExcludeSet,
}

/// Returns the value or a "missing required setting" error.
fn _require(value: Option<String>, name: &str, path: &Path) -> Result<String, ConnectionError> {
    value.ok_or_else(|| ConnectionError(format!("missing required setting '{name}' in {}", path.display())))
}

/// Reads and validates the site config at `path`: `host`, `user` and
/// `remote_dir` are required; `exclude` globs must be valid.
pub fn load(path: &Path) -> Result<SiteSettings, ConnectionError> {
    let json: SiteJson = connection::load_json_config(path)?;
    let c = json.connection;

    let remote = RemoteParams {
        host: _require(c.host, "host", path)?,
        port: c.port.unwrap_or(DEFAULT_FTP_PORT),
        user: _require(c.user, "user", path)?,
        remote_dir: _require(c.remote_dir, "remote_dir", path)?,
        ftps: c.ftps.unwrap_or(false),
        insecure_tls: c.insecure_tls.unwrap_or(false),
        timeout_secs: c.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS),
    };
    let exclude = ExcludeSet::new(&json.exclude)
        .map_err(|e| ConnectionError(format!("invalid exclude pattern in {}: {e}", path.display())))?;

    Ok(SiteSettings { remote, exclude })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a config.json into a fresh temp dir.
    fn write_config(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn loads_connection_and_excludes_with_defaults() {
        let (_dir, path) = write_config(
            r#"{"host":"ftp.test.com","user":"bob","remote_dir":"/public_html","exclude":["wp-content/uploads/**"]}"#,
        );

        let settings = load(&path).unwrap();

        assert_eq!(settings.remote.host, "ftp.test.com");
        assert_eq!(settings.remote.port, 21);
        assert_eq!(settings.remote.timeout_secs, 30);
        assert!(!settings.remote.ftps);
        assert!(settings.exclude.is_excluded("wp-content/uploads/2026/a.jpg"));
        assert!(!settings.exclude.is_excluded("wp-config.php"));
    }

    #[test]
    fn missing_required_field_names_the_field_and_file() {
        let (_dir, path) = write_config(r#"{"host":"h","user":"u"}"#);

        let err = load(&path).err().unwrap();

        assert!(err.to_string().contains("remote_dir"), "{err}");
    }

    #[test]
    fn invalid_glob_and_invalid_json_are_errors() {
        let (_d1, bad_glob) = write_config(r#"{"host":"h","user":"u","remote_dir":"/","exclude":["[unclosed"]}"#);
        assert!(load(&bad_glob).is_err());

        let (_d2, bad_json) = write_config("{not json");
        assert!(load(&bad_json).is_err());

        assert!(load(Path::new("/nonexistent/config.json")).is_err());
    }
}
