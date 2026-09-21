//! ftpdiff: compare a remote FTP/FTPS directory tree against a local copy.
//!
//! See `docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md`
//! for the design this binary implements.

mod cli;
mod config;
mod csv_report;
mod output;

use clap::Parser;
use ftp_utils_core::ftp_client::SuppaFtpConnection;
use ftp_utils_core::{compare, CompareOptions, DiffStatus};

use cli::Cli;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let cli = Cli::parse();

    let json_config = match &cli.config {
        Some(path) => match config::load_json_config(path) {
            Ok(loaded) => loaded,
            Err(e) => {
                eprintln!("Error: {e}");
                return 2;
            }
        },
        None => config::JsonConfig::default(),
    };

    let effective = match config::merge(&cli, &json_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {e}");
            return 2;
        }
    };

    let password = match config::read_password() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: {e}");
            return 2;
        }
    };

    let mut connection = match SuppaFtpConnection::connect(
        &effective.host,
        effective.port,
        &effective.user,
        &password,
        effective.ftps,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: failed to connect to {}:{}: {e}", effective.host, effective.port);
            return 2;
        }
    };

    let options = CompareOptions {
        local_dir: effective.local_dir.clone(),
        remote_dir: effective.remote_dir.clone(),
        excludes: effective.exclude.clone(),
        hash: effective.hash,
    };

    let entries = match compare(&mut connection, &options) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("Error: comparison failed: {e}");
            return 2;
        }
    };

    connection.close();

    for entry in &entries {
        println!("{}", output::format_entry(entry));
    }
    let summary = output::summarize(&entries);
    println!("{}", output::format_summary(&summary));

    if let Some(csv_path) = &effective.csv {
        if let Err(e) = csv_report::write_csv(csv_path, &entries) {
            eprintln!("Error: failed to write CSV to {}: {e}", csv_path.display());
            return 2;
        }
    }

    if entries.iter().any(|e| e.status != DiffStatus::Match) {
        1
    } else {
        0
    }
}
