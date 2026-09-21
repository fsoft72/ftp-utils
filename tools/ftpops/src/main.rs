//! ftpops: perform bulk copy/delete operations based on an ftpdiff CSV
//! report.
//!
//! See `docs/superpowers/specs/2026-09-21-ftpops-design.md` for the
//! design this binary implements.

mod cli;
mod csv_input;
mod filter;
mod ops;
mod output;
mod validate;

use std::path::Path;

use clap::Parser;
use ftp_utils_core::connection::{self, ConnectionArgs, EffectiveConnection};
use ftp_utils_core::ftp_client::SuppaFtpConnection;

use cli::{Cli, Command};
use filter::Filter;
use validate::{CopyTarget, DeleteTarget};

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let cli = Cli::parse();

    match cli.command {
        Command::Copy { connection, to, filter, csv, skip_existing, dry_run } => {
            if let Err(e) = validate::validate_copy(to, filter) {
                eprintln!("Error: {e}");
                return 2;
            }
            run_copy(&connection, to, filter, &csv, skip_existing, dry_run)
        }
        Command::Delete { connection, on, filter, csv, dry_run } => {
            if let Err(e) = validate::validate_delete(on, filter) {
                eprintln!("Error: {e}");
                return 2;
            }
            run_delete(&connection, on, filter, &csv, dry_run)
        }
    }
}

fn load_effective_connection(args: &ConnectionArgs) -> Result<EffectiveConnection, i32> {
    let json_config = match &args.config {
        Some(path) => match connection::load_json_config::<connection::ConnectionJsonConfig>(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error: {e}");
                return Err(2);
            }
        },
        None => connection::ConnectionJsonConfig::default(),
    };

    connection::merge_connection(args, &json_config).map_err(|e| {
        eprintln!("Error: {e}");
        2
    })
}

fn connect(args: &ConnectionArgs, effective: &EffectiveConnection) -> Result<SuppaFtpConnection, i32> {
    let password = match connection::read_password(args.password.as_deref(), "FTPOPS_PASSWORD") {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: {e}");
            return Err(2);
        }
    };

    SuppaFtpConnection::connect(
        &effective.host,
        effective.port,
        &effective.user,
        &password,
        effective.ftps,
        effective.insecure_tls,
    )
    .map_err(|e| {
        eprintln!("Error: failed to connect to {}:{}: {e}", effective.host, effective.port);
        2
    })
}

fn read_filtered_rows(csv_path: &Path) -> Result<Vec<csv_input::CsvRow>, i32> {
    csv_input::read_rows(csv_path).map_err(|e| {
        eprintln!("Error: {e}");
        2
    })
}

fn run_copy(
    connection_args: &ConnectionArgs,
    to: CopyTarget,
    filter: Filter,
    csv_path: &Path,
    skip_existing: bool,
    dry_run: bool,
) -> i32 {
    let effective = match load_effective_connection(connection_args) {
        Ok(c) => c,
        Err(code) => return code,
    };

    let rows = match read_filtered_rows(csv_path) {
        Ok(r) => r,
        Err(code) => return code,
    };
    let filtered = csv_input::filter_by_status(&rows, filter.status_str());

    if dry_run {
        for row in &filtered {
            println!("{}", output::format_dry_run_line("copy", &row.relative_path));
        }
        println!("Would copy {} file(s).", filtered.len());
        return 0;
    }

    let mut connection = match connect(connection_args, &effective) {
        Ok(c) => c,
        Err(code) => return code,
    };

    let results = match to {
        CopyTarget::Local => {
            ops::copy_to_local(&mut connection, &effective.remote_dir, &effective.local_dir, &filtered, skip_existing)
        }
        CopyTarget::Remote => {
            ops::copy_to_remote(&mut connection, &effective.remote_dir, &effective.local_dir, &filtered, skip_existing)
        }
    };

    connection.close();

    for result in &results {
        println!("{}", output::format_result(result));
    }
    let summary = output::summarize(&results);
    println!("{}", output::format_summary(&summary));

    if summary.failed > 0 {
        1
    } else {
        0
    }
}

fn run_delete(connection_args: &ConnectionArgs, on: DeleteTarget, filter: Filter, csv_path: &Path, dry_run: bool) -> i32 {
    let effective = match load_effective_connection(connection_args) {
        Ok(c) => c,
        Err(code) => return code,
    };

    let rows = match read_filtered_rows(csv_path) {
        Ok(r) => r,
        Err(code) => return code,
    };
    let filtered = csv_input::filter_by_status(&rows, filter.status_str());

    if dry_run {
        for row in &filtered {
            println!("{}", output::format_dry_run_line("delete", &row.relative_path));
        }
        println!("Would delete {} file(s).", filtered.len());
        return 0;
    }

    let results = match on {
        DeleteTarget::Local => ops::delete_local(&effective.local_dir, &filtered),
        DeleteTarget::Remote => {
            let mut connection = match connect(connection_args, &effective) {
                Ok(c) => c,
                Err(code) => return code,
            };
            let results = ops::delete_remote(&mut connection, &effective.remote_dir, &filtered);
            connection.close();
            results
        }
    };

    for result in &results {
        println!("{}", output::format_result(result));
    }
    let summary = output::summarize(&results);
    println!("{}", output::format_summary(&summary));

    if summary.failed > 0 {
        1
    } else {
        0
    }
}
