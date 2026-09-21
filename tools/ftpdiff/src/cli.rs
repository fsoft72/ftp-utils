//! Command-line argument definitions for ftpdiff.

use std::path::PathBuf;

use clap::Parser;

/// Compare a remote FTP/FTPS directory tree against a local copy.
#[derive(Parser, Debug, Clone)]
#[command(name = "ftpdiff", about = "Compare a remote FTP/FTPS directory tree against a local copy")]
pub struct Cli {
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

    /// Compare files by MD5 hash in addition to size.
    #[arg(long)]
    pub hash: bool,

    /// Glob pattern to exclude from comparison; can be repeated.
    #[arg(long = "exclude")]
    pub exclude: Vec<String>,

    /// Also write a structured CSV report to this path.
    #[arg(long)]
    pub csv: Option<PathBuf>,

    /// FTP password. Prefer the FTPDIFF_PASSWORD environment variable or
    /// the interactive prompt over this flag: a CLI argument can leak into
    /// shell history and process listings.
    #[arg(long)]
    pub password: Option<String>,
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

        assert_eq!(cli.host.as_deref(), Some("ftp.example.com"));
        assert_eq!(cli.user.as_deref(), Some("bob"));
        assert_eq!(cli.remote_dir.as_deref(), Some("/remote"));
        assert!(!cli.ftps);
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
        assert!(cli.ftps);
        assert!(cli.hash);
    }

    #[test]
    fn parses_password_flag() {
        let cli = Cli::parse_from(["ftpdiff", "--password", "s3cr3t"]);

        assert_eq!(cli.password.as_deref(), Some("s3cr3t"));
    }

    #[test]
    fn password_defaults_to_none() {
        let cli = Cli::parse_from(["ftpdiff"]);

        assert_eq!(cli.password, None);
    }
}
