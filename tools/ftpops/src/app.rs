//! ftpops' orchestration: load the connection settings and CSV report,
//! select the rows for the requested filter, then preview or execute the
//! operation and report the results.

use std::path::Path;

use ftp_utils_core::connection::{self, ConnectionArgs, ConnectionError, EffectiveConnection};
use ftp_utils_core::csv_source::{self, CsvSourceError, StatusRow};
use ftp_utils_core::exit::{EXIT_FAILURES, EXIT_OK};
use ftp_utils_core::ftp_client::SuppaFtpConnection;

use crate::cli::{Cli, Command};
use crate::confirm;
use crate::filter::{self, Filter};
use crate::ops::{self, OpResult};
use crate::output;
use crate::validate::{self, CopyTarget, DeleteTarget, ValidationError};

/// Environment variable consulted for the FTP password.
const PASSWORD_ENV_VAR: &str = "FTPOPS_PASSWORD";

ftp_utils_core::message_error! {
    /// A fatal ftpops error: printed once as `Error: <message>`, exit code 2.
    pub CliError
}

impl From<ConnectionError> for CliError {
    fn from(e: ConnectionError) -> Self {
        CliError(e.to_string())
    }
}

impl From<CsvSourceError> for CliError {
    fn from(e: CsvSourceError) -> Self {
        CliError(e.to_string())
    }
}

impl From<ValidationError> for CliError {
    fn from(e: ValidationError) -> Self {
        CliError(e.to_string())
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError(e.to_string())
    }
}

/// Runs the requested subcommand and returns the process exit code:
/// `EXIT_OK` on success, `EXIT_FAILURES` if any operation failed.
pub fn run(cli: Cli) -> Result<i32, CliError> {
    match cli.command {
        Command::Copy { connection, to, filter, csv, skip_existing, dry_run } => {
            validate::validate_copy(to, filter)?;
            run_copy(&connection, to, filter, &csv, skip_existing, dry_run)
        }
        Command::Delete { connection, on, filter, csv, dry_run, yes } => {
            validate::validate_delete(on, filter)?;
            run_delete(&connection, on, filter, &csv, dry_run, yes)
        }
    }
}

/// Loads the JSON config (if any), merges it with the CLI flags, and reads
/// the CSV report.
fn load_job(args: &ConnectionArgs, csv_path: &Path) -> Result<(EffectiveConnection, Vec<StatusRow>), CliError> {
    let json_config = match &args.config {
        Some(path) => connection::load_json_config::<connection::ConnectionJsonConfig>(path)?,
        None => connection::ConnectionJsonConfig::default(),
    };
    let effective = connection::merge_connection(args, &json_config)?;
    let rows = csv_source::read_status_rows(csv_path)?;
    Ok((effective, rows))
}

/// Connects to the server (asking for the password if needed).
fn connect(args: &ConnectionArgs, effective: &EffectiveConnection) -> Result<SuppaFtpConnection, CliError> {
    let password = connection::read_password(args.password.as_deref(), PASSWORD_ENV_VAR)?;

    SuppaFtpConnection::connect_params(&effective.remote, &password)
        .map_err(|e| CliError(format!("failed to connect to {}:{}: {e}", effective.remote.host, effective.remote.port)))
}

/// Prints what `--dry-run` would do; touches nothing.
fn preview(verb: &str, rows: &[&StatusRow]) -> i32 {
    for row in rows {
        println!("{}", output::format_dry_run_line(verb, &row.relative_path));
    }
    println!("Would {verb} {} file(s).", rows.len());
    EXIT_OK
}

/// Prints one line per result and the summary; the exit code reflects
/// whether any operation failed.
fn report(results: &[OpResult]) -> i32 {
    for result in results {
        println!("{}", output::format_result(result));
    }
    let summary = output::summarize(results);
    println!("{}", output::format_summary(&summary));

    if summary.failed > 0 {
        EXIT_FAILURES
    } else {
        EXIT_OK
    }
}

fn run_copy(
    connection_args: &ConnectionArgs,
    to: CopyTarget,
    filter: Filter,
    csv_path: &Path,
    skip_existing: bool,
    dry_run: bool,
) -> Result<i32, CliError> {
    let (effective, rows) = load_job(connection_args, csv_path)?;
    let selected = filter::filter_by_status(&rows, filter.status());

    if dry_run {
        return Ok(preview("copy", &selected));
    }

    let remote_dir = &effective.remote.remote_dir;
    let mut connection = connect(connection_args, &effective)?;
    let results = match to {
        CopyTarget::Local => {
            ops::copy_to_local(&mut connection, remote_dir, &effective.local_dir, &selected, skip_existing)
        }
        CopyTarget::Remote => {
            ops::copy_to_remote(&mut connection, remote_dir, &effective.local_dir, &selected, skip_existing)
        }
    };
    connection.close();

    Ok(report(&results))
}

fn run_delete(
    connection_args: &ConnectionArgs,
    on: DeleteTarget,
    filter: Filter,
    csv_path: &Path,
    dry_run: bool,
    yes: bool,
) -> Result<i32, CliError> {
    let (effective, rows) = load_job(connection_args, csv_path)?;
    let selected = filter::filter_by_status(&rows, filter.status());

    if dry_run {
        return Ok(preview("delete", &selected));
    }

    if !yes && !selected.is_empty() {
        let (side, dir) = match on {
            DeleteTarget::Local => ("local", effective.local_dir.display().to_string()),
            DeleteTarget::Remote => ("remote", effective.remote.remote_dir.clone()),
        };
        let prompt = format!(
            "About to delete {} file(s) on the {side} side under {dir}, based on {}. Continue?",
            selected.len(),
            csv_path.display()
        );
        if !confirm::confirm(&prompt)? {
            return Err(CliError("aborted, nothing was deleted (use --yes to skip the confirmation)".to_string()));
        }
    }

    let results = match on {
        DeleteTarget::Local => ops::delete_local(&effective.local_dir, &selected),
        DeleteTarget::Remote => {
            let mut connection = connect(connection_args, &effective)?;
            let results = ops::delete_remote(&mut connection, &effective.remote.remote_dir, &selected);
            connection.close();
            results
        }
    };

    Ok(report(&results))
}
