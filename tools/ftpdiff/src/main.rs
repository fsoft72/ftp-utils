//! ftpdiff: compare a remote FTP/FTPS directory tree against a local copy.
//!
//! See `docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md`
//! and `docs/superpowers/specs/2026-09-21-ftpdiff-csv-source-design.md`
//! for the design this binary implements.

mod cli;
mod config;
mod csv_report;
mod output;

use std::collections::HashMap;

use clap::Parser;
use ftp_utils_core::compare::compare_entries;
use ftp_utils_core::ftp_client::SuppaFtpConnection;
use ftp_utils_core::{csv_source, exclude, hash, local, remote, DiffStatus, FtpConnection};

use cli::Cli;
use config::{LocalSource, RemoteSource};

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let cli = Cli::parse();

    let json_config = match &cli.connection.config {
        Some(path) => match config::load_json_config(path) {
            Ok(loaded) => loaded,
            Err(e) => {
                eprintln!("Error: {e}");
                return 2;
            }
        },
        None => config::JsonConfig::default(),
    };

    if cli.build {
        return run_build(&cli, &json_config);
    }

    let effective = match config::merge(&cli, &json_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {e}");
            return 2;
        }
    };

    let mut progress: Option<Box<dyn FnMut(&str)>> = if effective.verbose {
        Some(Box::new(|msg: &str| eprintln!("Checking {msg}")))
    } else {
        None
    };

    let (local_entries, local_known_md5) = match &effective.local {
        LocalSource::Live(dir) => match local::walk_local_dir(dir, &effective.exclude, progress.as_deref_mut()) {
            Ok(entries) => (entries, HashMap::new()),
            Err(e) => {
                eprintln!("Error: {e}");
                return 2;
            }
        },
        LocalSource::Csv(path) => {
            let (entries, known_md5) = match csv_source::read_local_entries(path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return 2;
                }
            };
            if effective.verbose {
                eprintln!("Loaded {} local entries from {}.", entries.len(), path.display());
            }
            let filtered = entries
                .into_iter()
                .filter(|e| !exclude::is_excluded(&e.relative_path, &effective.exclude))
                .collect();
            (filtered, known_md5)
        }
    };

    let mut connection: Option<SuppaFtpConnection> = None;

    let (remote_entries, remote_known_md5) = match &effective.remote {
        RemoteSource::Live { host, port, user, remote_dir, ftps, insecure_tls } => {
            let password = match config::read_password(cli.connection.password.as_deref()) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return 2;
                }
            };

            if effective.verbose {
                eprintln!("Connecting to {host}:{port} as {user} ({})...", if *ftps { "FTPS" } else { "FTP" });
            }

            let mut conn = match SuppaFtpConnection::connect(host, *port, user, &password, *ftps, *insecure_tls) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error: failed to connect to {host}:{port}: {e}");
                    return 2;
                }
            };

            let entries = match remote::walk_remote(&mut conn, remote_dir, &effective.exclude, progress.as_deref_mut()) {
                Ok(entries) => entries,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return 2;
                }
            };

            connection = Some(conn);
            (entries, HashMap::new())
        }
        RemoteSource::Csv(path) => {
            let (entries, known_md5) = match csv_source::read_remote_entries(path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return 2;
                }
            };
            if effective.verbose {
                eprintln!("Loaded {} remote entries from {}.", entries.len(), path.display());
            }
            let filtered = entries
                .into_iter()
                .filter(|e| !exclude::is_excluded(&e.relative_path, &effective.exclude))
                .collect();
            (filtered, known_md5)
        }
    };

    let mut entries = compare_entries(&local_entries, &remote_entries);

    if effective.hash {
        let (conn_opt, remote_root_opt) = match (&mut connection, &effective.remote) {
            (Some(conn), RemoteSource::Live { remote_dir, .. }) => (Some(conn), Some(remote_dir.as_str())),
            _ => (None, None),
        };
        let local_root_opt = match &effective.local {
            LocalSource::Live(dir) => Some(dir.as_path()),
            LocalSource::Csv(_) => None,
        };

        if let Err(e) = hash::apply_hash_comparison(
            conn_opt,
            remote_root_opt,
            local_root_opt,
            &local_known_md5,
            &remote_known_md5,
            &mut entries,
            progress.as_deref_mut(),
        ) {
            eprintln!("Error: hash comparison failed: {e}");
            return 2;
        }
    }

    if let Some(conn) = connection {
        conn.close();
    }

    if effective.verbose {
        eprintln!("Comparison done: {} entries.", entries.len());
    }

    for entry in &entries {
        println!("{}", output::format_entry(entry));
    }
    let summary = output::summarize(&entries);
    println!("{}", output::format_summary(&summary));

    if let Some(csv_path) = &effective.csv {
        if effective.verbose {
            eprintln!("Writing CSV report to {}...", csv_path.display());
        }
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

fn run_build(cli: &Cli, json_config: &config::JsonConfig) -> i32 {
    let build = match config::merge_build(cli, json_config) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: {e}");
            return 2;
        }
    };

    let mut progress: Option<Box<dyn FnMut(&str)>> = if build.verbose {
        Some(Box::new(|msg: &str| eprintln!("Checking {msg}")))
    } else {
        None
    };

    let entries: Vec<ftp_utils_core::DiffEntry> = match &build.side {
        config::BuildSide::Local(dir) => {
            let scanned = match local::walk_local_dir(dir, &build.exclude, progress.as_deref_mut()) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return 2;
                }
            };

            let mut out = Vec::new();
            for item in scanned {
                let local_md5 = if build.hash {
                    let bytes = match std::fs::read(dir.join(&item.relative_path)) {
                        Ok(b) => b,
                        Err(err) => {
                            eprintln!("Error: failed to read {} for hashing: {err}", item.relative_path);
                            return 2;
                        }
                    };
                    Some(format!("{:x}", md5::compute(&bytes)))
                } else {
                    None
                };

                out.push(ftp_utils_core::DiffEntry {
                    relative_path: item.relative_path,
                    status: DiffStatus::Scan,
                    local_size: Some(item.size),
                    remote_size: None,
                    local_md5,
                    remote_md5: None,
                });
            }
            out
        }
        config::BuildSide::Remote { host, port, user, remote_dir, ftps, insecure_tls } => {
            let password = match config::read_password(cli.connection.password.as_deref()) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return 2;
                }
            };

            if build.verbose {
                eprintln!("Connecting to {host}:{port} as {user} ({})...", if *ftps { "FTPS" } else { "FTP" });
            }

            let mut conn = match SuppaFtpConnection::connect(host, *port, user, &password, *ftps, *insecure_tls) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error: failed to connect to {host}:{port}: {e}");
                    return 2;
                }
            };

            let scanned = match remote::walk_remote(&mut conn, remote_dir, &build.exclude, progress.as_deref_mut()) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return 2;
                }
            };

            let mut out = Vec::new();
            for item in scanned {
                let remote_md5 = if build.hash {
                    let remote_path = format!("{remote_dir}/{}", item.relative_path);
                    match conn.try_hash(&remote_path) {
                        Some(hash) => Some(hash),
                        None => match conn.retr_to_buffer(&remote_path) {
                            Ok(bytes) => Some(format!("{:x}", md5::compute(&bytes))),
                            Err(err) => {
                                eprintln!("Error: failed to download {} for hashing: {err}", item.relative_path);
                                return 2;
                            }
                        },
                    }
                } else {
                    None
                };

                out.push(ftp_utils_core::DiffEntry {
                    relative_path: item.relative_path,
                    status: DiffStatus::Scan,
                    local_size: None,
                    remote_size: Some(item.size),
                    local_md5: None,
                    remote_md5,
                });
            }

            conn.close();
            out
        }
    };

    for entry in &entries {
        println!("{}", output::format_entry(entry));
    }
    println!("Scanned {} entries.", entries.len());

    if let Err(e) = csv_report::write_csv(&build.csv, &entries) {
        eprintln!("Error: failed to write CSV to {}: {e}", build.csv.display());
        return 2;
    }

    0
}
