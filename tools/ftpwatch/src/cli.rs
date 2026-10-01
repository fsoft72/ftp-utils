//! Command-line definitions for ftpwatch.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Monitor FTP/FTPS sites for added, deleted and modified files.
#[derive(Parser, Debug)]
#[command(name = "ftpwatch", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// First run: download the site and record the baseline snapshot.
    Init(InitArgs),
    /// Scan the site (listing only, no downloads), log what changed and
    /// record a new snapshot. Exit code 1 means changes were found.
    Check(CheckArgs),
}

/// Arguments shared by every command.
#[derive(Args, Debug)]
pub struct CommonArgs {
    /// The site directory (holds config.json, files/, snapshots/, logs/).
    pub site_dir: PathBuf,

    /// FTP password. Prefer the FTPWATCH_PASSWORD environment variable or
    /// the interactive prompt: a CLI argument can leak into shell history
    /// and process listings.
    #[arg(long)]
    pub password: Option<String>,

    /// Print progress diagnostics to stderr: the connection and one line per
    /// scanned file (`Checking remote: <path>`), plus each download in `init`.
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// Start over even if a snapshot already exists. Records a new baseline
    /// but keeps the existing files/ tree: same-size files are not downloaded
    /// again and files deleted on the server are not removed locally. Delete
    /// files/ by hand for a clean restart.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct CheckArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}
