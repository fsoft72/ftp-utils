//! ftpdiff: compare a remote FTP/FTPS directory tree against a local copy.
//!
//! See `docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md`
//! and `docs/superpowers/specs/2026-09-21-ftpdiff-csv-source-design.md`
//! for the design this binary implements.

mod app;
mod cli;
mod config;
mod output;

use clap::Parser;
use ftp_utils_core::exit::EXIT_ERROR;

use cli::Cli;

fn main() {
    let cli = Cli::parse();

    let code = match app::run(&cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e}");
            EXIT_ERROR
        }
    };

    std::process::exit(code);
}
