//! Command-line argument definitions for ftpops.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ftp_utils_core::connection::ConnectionArgs;

pub use crate::validate::{CopyTarget, DeleteTarget};
use crate::filter::Filter;

/// Perform bulk copy/delete operations based on an ftpdiff CSV report.
#[derive(Parser, Debug)]
#[command(name = "ftpops", about = "Perform bulk copy/delete operations based on an ftpdiff CSV report")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Copy files between local and remote, filtered by diff status.
    Copy {
        #[command(flatten)]
        connection: ConnectionArgs,

        /// Direction to copy: download to local, or upload to remote.
        #[arg(long)]
        to: CopyTarget,

        /// Only operate on rows with this diff status.
        #[arg(long)]
        filter: Filter,

        /// Path to the ftpdiff --csv report to read.
        #[arg(long)]
        csv: PathBuf,

        /// Skip files whose destination already exists, instead of
        /// overwriting them.
        #[arg(long)]
        skip_existing: bool,

        /// Print what would be done without doing it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete files locally or remotely, filtered by diff status.
    Delete {
        #[command(flatten)]
        connection: ConnectionArgs,

        /// Which side to delete from.
        #[arg(long)]
        on: DeleteTarget,

        /// Only operate on rows with this diff status.
        #[arg(long)]
        filter: Filter,

        /// Path to the ftpdiff --csv report to read.
        #[arg(long)]
        csv: PathBuf,

        /// Print what would be done without doing it.
        #[arg(long)]
        dry_run: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_copy_subcommand() {
        let cli = Cli::parse_from([
            "ftpops", "copy",
            "--to", "local",
            "--filter", "remote-only",
            "--csv", "report.csv",
            "--host", "ftp.example.com",
            "--user", "bob",
            "--remote-dir", "/remote",
            "--local-dir", "./local",
        ]);

        match cli.command {
            Command::Copy { to, filter, csv, skip_existing, dry_run, connection } => {
                assert_eq!(to, CopyTarget::Local);
                assert_eq!(filter, Filter::RemoteOnly);
                assert_eq!(csv, PathBuf::from("report.csv"));
                assert!(!skip_existing);
                assert!(!dry_run);
                assert_eq!(connection.host.as_deref(), Some("ftp.example.com"));
            }
            Command::Delete { .. } => panic!("expected Copy"),
        }
    }

    #[test]
    fn parses_copy_optional_flags() {
        let cli = Cli::parse_from([
            "ftpops", "copy",
            "--to", "remote",
            "--filter", "local-only",
            "--csv", "report.csv",
            "--skip-existing",
            "--dry-run",
        ]);

        match cli.command {
            Command::Copy { skip_existing, dry_run, .. } => {
                assert!(skip_existing);
                assert!(dry_run);
            }
            Command::Delete { .. } => panic!("expected Copy"),
        }
    }

    #[test]
    fn parses_delete_subcommand() {
        let cli = Cli::parse_from([
            "ftpops", "delete",
            "--on", "remote",
            "--filter", "remote-only",
            "--csv", "report.csv",
        ]);

        match cli.command {
            Command::Delete { on, filter, csv, dry_run, .. } => {
                assert_eq!(on, DeleteTarget::Remote);
                assert_eq!(filter, Filter::RemoteOnly);
                assert_eq!(csv, PathBuf::from("report.csv"));
                assert!(!dry_run);
            }
            Command::Copy { .. } => panic!("expected Delete"),
        }
    }
}
