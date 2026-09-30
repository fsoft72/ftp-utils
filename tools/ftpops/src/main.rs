//! ftpops: perform bulk copy/delete operations based on an ftpdiff CSV
//! report.
//!
//! See `docs/superpowers/specs/2026-09-21-ftpops-design.md` for the
//! design this binary implements.

mod app;
mod cli;
mod confirm;
mod filter;
mod ops;
mod output;
mod validate;

use clap::Parser;
use ftp_utils_core::exit::EXIT_ERROR;

use cli::Cli;

fn main() {
    let code = match app::run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e}");
            EXIT_ERROR
        }
    };

    std::process::exit(code);
}
