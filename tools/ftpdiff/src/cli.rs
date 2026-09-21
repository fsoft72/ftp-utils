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
