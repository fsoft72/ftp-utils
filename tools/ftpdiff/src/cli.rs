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

    /// Read the local side from a previous `ftpdiff --csv` report
    /// instead of scanning --local-dir.
    #[arg(long = "local-csv")]
    pub local_csv: Option<PathBuf>,

    /// Read the remote side from a previous `ftpdiff --csv` report
    /// instead of connecting to the FTP server.
    #[arg(long = "remote-csv")]
    pub remote_csv: Option<PathBuf>,

    /// Scan exactly one side (--local-dir, or --host/--user/--remote-dir)
    /// and write it to --csv without comparing. Cannot be combined with
    /// --local-csv/--remote-csv.
    #[arg(long)]
    pub build: bool,
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

    #[test]
    fn parses_local_csv_flag() {
        let cli = Cli::parse_from(["ftpdiff", "--local-csv", "snapshot.csv"]);

        assert_eq!(cli.local_csv, Some(PathBuf::from("snapshot.csv")));
    }

    #[test]
    fn parses_remote_csv_flag() {
        let cli = Cli::parse_from(["ftpdiff", "--remote-csv", "snapshot.csv"]);

        assert_eq!(cli.remote_csv, Some(PathBuf::from("snapshot.csv")));
    }

    #[test]
    fn csv_source_flags_default_to_none() {
        let cli = Cli::parse_from(["ftpdiff"]);

        assert_eq!(cli.local_csv, None);
        assert_eq!(cli.remote_csv, None);
    }

    #[test]
    fn build_flag_defaults_to_false() {
        let cli = Cli::parse_from(["ftpdiff"]);

        assert!(!cli.build);
    }

    #[test]
    fn parses_build_flag() {
        let cli = Cli::parse_from(["ftpdiff", "--build"]);

        assert!(cli.build);
    }
}
