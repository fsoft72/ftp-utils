//! ftpwatch: monitor FTP/FTPS sites for added, deleted and modified files.
//!
//! See `docs/superpowers/specs/2026-10-01-ftpwatch-design.md`.

mod app;
mod cli;
mod compare;
mod config;
mod log;
mod store;
mod timefmt;

use clap::Parser;
use ftp_utils_core::exit::EXIT_ERROR;

use cli::Cli;

/// Parses the command line, runs the command and exits with its code.
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
