//! ftpdiff's orchestration: loading each side, comparing, hashing and
//! reporting. Kept out of `main.rs` and split into small functions (generic
//! over `FtpConnection` where a network is involved) so they can be tested
//! without a server.

use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use ftp_utils_core::compare::compare_entries;
use ftp_utils_core::connection::{ConnectionError, RemoteParams};
use ftp_utils_core::csv_source::{self, CsvSourceError, ReportEntry};
use ftp_utils_core::exclude::ExcludeSet;
use ftp_utils_core::exit::{EXIT_FAILURES, EXIT_OK};
use ftp_utils_core::ftp_client::SuppaFtpConnection;
use ftp_utils_core::local::LocalEntry;
use ftp_utils_core::paths::validate_relative_path;
use ftp_utils_core::remote::RemoteEntry;
use ftp_utils_core::{hash, local, remote, DiffEntry, DiffStatus, FtpConnection, FtpConnectionError};

use crate::cli::Cli;
use crate::config::{self, BuildConfig, BuildSide, EffectiveConfig, JsonConfig, LocalSource, RemoteSource};
use crate::output;

ftp_utils_core::message_error! {
    /// A fatal ftpdiff error: printed once as `Error: <message>`, exit code 2.
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

impl From<FtpConnectionError> for CliError {
    fn from(e: FtpConnectionError) -> Self {
        CliError(e.to_string())
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError(e.to_string())
    }
}

/// Optional `--verbose` progress sink, called with one message per file.
pub type Progress = Option<Box<dyn FnMut(&str)>>;

/// Entries of one side plus the MD5s already known for them (from a CSV).
type Side<E> = (Vec<E>, HashMap<String, String>);

/// Runs ftpdiff for parsed arguments and returns the process exit code:
/// `EXIT_OK` if everything matched (or a build succeeded), `EXIT_FAILURES`
/// if there are differences.
pub fn run(cli: &Cli) -> Result<i32, CliError> {
    let json_config = match &cli.connection.config {
        Some(path) => config::load_json_config(path)?,
        None => JsonConfig::default(),
    };

    if cli.build {
        return run_build(cli, &json_config);
    }
    run_compare(cli, &json_config)
}

/// Returns the `--verbose` progress printer, or `None` when not verbose.
fn make_progress(verbose: bool) -> Progress {
    if !verbose {
        return None;
    }
    Some(Box::new(|msg: &str| eprintln!("Checking {msg}")))
}

/// Connects to the server (asking for the password if needed).
fn connect_remote(
    params: &RemoteParams,
    cli_password: Option<&str>,
    verbose: bool,
) -> Result<SuppaFtpConnection, CliError> {
    let password = config::read_password(cli_password)?;

    if verbose {
        eprintln!(
            "Connecting to {}:{} as {} ({})...",
            params.host,
            params.port,
            params.user,
            if params.ftps { "FTPS" } else { "FTP" }
        );
    }

    SuppaFtpConnection::connect_params(params, &password)
        .map_err(|e| CliError(format!("failed to connect to {}:{}: {e}", params.host, params.port)))
}

/// Loads the local side: a live directory walk, or a CSV report filtered
/// through `exclude`.
fn load_local_side(
    source: &LocalSource,
    exclude: &ExcludeSet,
    verbose: bool,
    progress: &mut Progress,
) -> Result<Side<LocalEntry>, CliError> {
    match source {
        LocalSource::Live(dir) => Ok((local::walk_local_dir(dir, exclude, progress.as_deref_mut())?, HashMap::new())),
        LocalSource::Csv(path) => {
            let (entries, known_md5) = csv_source::read_local_entries(path)?;
            if verbose {
                eprintln!("Loaded {} local entries from {}.", entries.len(), path.display());
            }
            let kept = entries.into_iter().filter(|e| !exclude.is_excluded(&e.relative_path)).collect();
            Ok((kept, known_md5))
        }
    }
}

/// Loads the remote side from a CSV report, filtered through `exclude`.
fn load_remote_csv(path: &Path, exclude: &ExcludeSet, verbose: bool) -> Result<Side<RemoteEntry>, CliError> {
    let (entries, known_md5) = csv_source::read_remote_entries(path)?;
    if verbose {
        eprintln!("Loaded {} remote entries from {}.", entries.len(), path.display());
    }
    let kept = entries.into_iter().filter(|e| !exclude.is_excluded(&e.relative_path)).collect();
    Ok((kept, known_md5))
}

/// Upgrades size-matching entries to hash-verified ones, using known MD5s
/// first and live sources (`conn` + remote root, local root) otherwise.
fn resolve_hashes<C: FtpConnection>(
    conn: Option<&mut C>,
    remote_root: Option<&str>,
    local_source: &LocalSource,
    known: (&HashMap<String, String>, &HashMap<String, String>),
    entries: &mut [DiffEntry],
    progress: &mut Progress,
) -> Result<(), CliError> {
    let local_root = match local_source {
        LocalSource::Live(dir) => Some(dir.as_path()),
        LocalSource::Csv(_) => None,
    };
    let (local_known_md5, remote_known_md5) = known;

    hash::apply_hash_comparison(
        conn,
        remote_root,
        local_root,
        local_known_md5,
        remote_known_md5,
        entries,
        progress.as_deref_mut(),
    )
    .map_err(|e| CliError(format!("hash comparison failed: {e}")))
}

/// Prints one line per entry plus the summary.
fn print_report(entries: &[DiffEntry]) {
    for entry in entries {
        println!("{}", output::format_entry(entry));
    }
    println!("{}", output::format_summary(&output::summarize(entries)));
}

/// Writes the CSV report to `path`.
fn write_csv(path: &Path, entries: &[ReportEntry], verbose: bool) -> Result<(), CliError> {
    if verbose {
        eprintln!("Writing CSV report to {}...", path.display());
    }
    csv_source::write_report(path, entries)
        .map_err(|e| CliError(format!("failed to write CSV to {}: {e}", path.display())))
}

/// Downloads every entry of `entries` (binary mode, streamed) from
/// `remote_root` into `dest`, recreating subdirectories. A file already in
/// `dest` with the same size is skipped. Each file is written to a
/// temporary `.<name>.ftpdiff-part` file and renamed on success, so a
/// failed transfer never leaves a truncated file under its final name.
/// Returns `(downloaded, skipped)` counts.
fn download_remote_files<C: FtpConnection>(
    conn: &mut C,
    remote_root: &str,
    entries: &[RemoteEntry],
    dest: &Path,
    progress: &mut Progress,
) -> Result<(usize, usize), CliError> {
    let (mut downloaded, mut skipped) = (0, 0);

    for entry in entries {
        validate_relative_path(&entry.relative_path)
            .map_err(|e| CliError(format!("refusing to download {}: {e}", entry.relative_path)))?;

        let target = dest.join(&entry.relative_path);
        if fs::metadata(&target).is_ok_and(|m| m.is_file() && m.len() == entry.size) {
            skipped += 1;
            continue;
        }

        if let Some(report) = progress.as_deref_mut() {
            report(&format!("download {}", entry.relative_path));
        }
        download_one(conn, &remote::join_remote(remote_root, &entry.relative_path), &target)
            .map_err(|e| CliError(format!("failed to download {}: {e}", entry.relative_path)))?;
        downloaded += 1;
    }

    Ok((downloaded, skipped))
}

/// Streams `remote_path` to `target` through a temporary part file.
fn download_one<C: FtpConnection>(conn: &mut C, remote_path: &str, target: &Path) -> Result<(), CliError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let part: PathBuf = target.with_file_name(format!(".{name}.ftpdiff-part"));

    let result = File::create(&part)
        .map_err(CliError::from)
        .and_then(|mut file| conn.retr_to_writer(remote_path, &mut file).map(|_| ()).map_err(CliError::from));

    match result {
        Ok(()) => Ok(fs::rename(&part, target)?),
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

/// Runs the download and prints its summary line.
fn download_and_report<C: FtpConnection>(
    conn: &mut C,
    remote_root: &str,
    entries: &[RemoteEntry],
    dest: &Path,
    progress: &mut Progress,
) -> Result<(), CliError> {
    let (downloaded, skipped) = download_remote_files(conn, remote_root, entries, dest, progress)?;
    println!("Downloaded {downloaded} files to {} ({skipped} skipped, same size).", dest.display());
    Ok(())
}

/// The compare mode: load both sides, diff, optionally hash, report.
fn run_compare(cli: &Cli, json_config: &JsonConfig) -> Result<i32, CliError> {
    let effective = config::merge(cli, json_config)?;
    let mut progress = make_progress(effective.verbose);

    let (local_entries, local_known_md5) =
        load_local_side(&effective.local, &effective.exclude, effective.verbose, &mut progress)?;

    let mut connection: Option<SuppaFtpConnection> = None;
    let (remote_entries, remote_known_md5) = match &effective.remote {
        RemoteSource::Live(params) => {
            let mut conn = connect_remote(params, cli.connection.password.as_deref(), effective.verbose)?;
            let entries =
                remote::walk_remote(&mut conn, &params.remote_dir, &effective.exclude, progress.as_deref_mut())?;
            connection = Some(conn);
            (entries, HashMap::new())
        }
        RemoteSource::Csv(path) => load_remote_csv(path, &effective.exclude, effective.verbose)?,
    };

    let mut entries = compare_entries(&local_entries, &remote_entries);

    if effective.hash {
        hash_compared_entries(
            &effective,
            &mut connection,
            (&local_known_md5, &remote_known_md5),
            &mut entries,
            &mut progress,
        )?;
    }

    if let (Some(dest), Some(conn), RemoteSource::Live(params)) =
        (&effective.download_dir, connection.as_mut(), &effective.remote)
    {
        download_and_report(conn, &params.remote_dir, &remote_entries, dest, &mut progress)?;
    }

    if let Some(conn) = connection {
        conn.close();
    }

    if effective.verbose {
        eprintln!("Comparison done: {} entries.", entries.len());
    }

    print_report(&entries);

    if let Some(csv_path) = &effective.csv {
        let rows: Vec<ReportEntry> = entries.iter().map(ReportEntry::from).collect();
        write_csv(csv_path, &rows, effective.verbose)?;
    }

    Ok(exit_code_for(&entries))
}

/// Runs `resolve_hashes` with the live connection (if any) of `effective`.
fn hash_compared_entries(
    effective: &EffectiveConfig,
    connection: &mut Option<SuppaFtpConnection>,
    known: (&HashMap<String, String>, &HashMap<String, String>),
    entries: &mut [DiffEntry],
    progress: &mut Progress,
) -> Result<(), CliError> {
    let (conn, remote_root) = match (connection.as_mut(), &effective.remote) {
        (Some(conn), RemoteSource::Live(params)) => (Some(conn), Some(params.remote_dir.as_str())),
        _ => (None, None),
    };
    resolve_hashes(conn, remote_root, &effective.local, known, entries, progress)
}

/// `EXIT_OK` if every entry matches, `EXIT_FAILURES` otherwise.
fn exit_code_for(entries: &[DiffEntry]) -> i32 {
    if entries.iter().any(|e| e.status != DiffStatus::Match) {
        EXIT_FAILURES
    } else {
        EXIT_OK
    }
}

/// Scans a local directory into `Scan` entries, hashing each file when
/// `hash` is set.
fn scan_local(
    dir: &Path,
    exclude: &ExcludeSet,
    hash: bool,
    progress: &mut Progress,
) -> Result<Vec<ReportEntry>, CliError> {
    let mut entries = Vec::new();
    for item in local::walk_local_dir(dir, exclude, progress.as_deref_mut())? {
        let mut entry = ReportEntry::scan(item.relative_path, Some(item.size), None);
        if hash {
            let md5 = hash::local_md5(&dir.join(&entry.relative_path))
                .map_err(|e| CliError(format!("failed to read {} for hashing: {e}", entry.relative_path)))?;
            entry.local_md5 = Some(md5);
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// Scans a remote directory into `Scan` entries, hashing each file when
/// `hash` is set.
fn scan_remote<C: FtpConnection>(
    conn: &mut C,
    remote_dir: &str,
    exclude: &ExcludeSet,
    hash: bool,
    progress: &mut Progress,
) -> Result<Vec<ReportEntry>, CliError> {
    let mut entries = Vec::new();
    for item in remote::walk_remote(conn, remote_dir, exclude, progress.as_deref_mut())? {
        let mut entry = ReportEntry::scan(item.relative_path, None, Some(item.size));
        if hash {
            let remote_path = remote::join_remote(remote_dir, &entry.relative_path);
            let md5 = hash::remote_md5(conn, &remote_path)
                .map_err(|e| CliError(format!("failed to hash {}: {e}", entry.relative_path)))?;
            entry.remote_md5 = Some(md5);
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// The `--build` mode: scan exactly one side and write it to `--csv`.
fn run_build(cli: &Cli, json_config: &JsonConfig) -> Result<i32, CliError> {
    let build: BuildConfig = config::merge_build(cli, json_config)?;
    let mut progress = make_progress(build.verbose);

    let entries = match &build.side {
        BuildSide::Local(dir) => scan_local(dir, &build.exclude, build.hash, &mut progress)?,
        BuildSide::Remote(params) => {
            let mut conn = connect_remote(params, cli.connection.password.as_deref(), build.verbose)?;
            let entries = scan_remote(&mut conn, &params.remote_dir, &build.exclude, build.hash, &mut progress)?;
            if let Some(dest) = &build.download_dir {
                let files: Vec<RemoteEntry> = entries
                    .iter()
                    .filter_map(|e| Some(RemoteEntry { relative_path: e.relative_path.clone(), size: e.remote_size? }))
                    .collect();
                download_and_report(&mut conn, &params.remote_dir, &files, dest, &mut progress)?;
            }
            conn.close();
            entries
        }
    };

    for entry in &entries {
        println!("{}", output::format_scan_entry(entry));
    }
    println!("Scanned {} entries.", entries.len());

    write_csv(&build.csv, &entries, false)?;
    Ok(EXIT_OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ftp_utils_core::testing::MockFtpConnection;
    use ftp_utils_core::RawRemoteEntry;

    /// MD5 of the bytes `hello`.
    const HELLO_MD5: &str = "5d41402abc4b2a76b9719d911017c592";

    /// A server with `/remote/a.txt` ("hello") and `/remote/skip.tmp`.
    fn server() -> MockFtpConnection {
        let mut listings = HashMap::new();
        listings.insert(
            "/remote".to_string(),
            vec![
                RawRemoteEntry { name: "a.txt".into(), is_dir: false, size: 5 },
                RawRemoteEntry { name: "skip.tmp".into(), is_dir: false, size: 1 },
            ],
        );
        let mut files = HashMap::new();
        files.insert("/remote/a.txt".to_string(), b"hello".to_vec());
        MockFtpConnection { files, ..MockFtpConnection::strict(listings) }
    }

    fn excludes(patterns: &[&str]) -> ExcludeSet {
        ExcludeSet::new(&patterns.iter().map(|p| p.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn scan_local_lists_files_as_scan_entries_with_optional_hash() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        std::fs::write(dir.path().join("skip.tmp"), b"x").unwrap();
        let exclude = excludes(&["*.tmp"]);

        let plain = scan_local(dir.path(), &exclude, false, &mut None).unwrap();
        assert_eq!(plain, vec![ReportEntry::scan("a.txt", Some(5), None)]);

        let hashed = scan_local(dir.path(), &exclude, true, &mut None).unwrap();
        assert_eq!(hashed[0].local_md5.as_deref(), Some(HELLO_MD5));
        assert_eq!(hashed[0].remote_md5, None);
    }

    #[test]
    fn scan_remote_lists_files_and_hashes_via_download() {
        let mut conn = server();

        let plain = scan_remote(&mut conn, "/remote", &excludes(&["*.tmp"]), false, &mut None).unwrap();
        assert_eq!(plain, vec![ReportEntry::scan("a.txt", None, Some(5))]);

        let hashed = scan_remote(&mut conn, "/remote", &excludes(&["*.tmp"]), true, &mut None).unwrap();
        assert_eq!(hashed[0].remote_md5.as_deref(), Some(HELLO_MD5));
    }

    #[test]
    fn scan_remote_reports_the_file_that_failed_to_hash() {
        let mut conn = server();
        conn.files.clear();

        let err = scan_remote(&mut conn, "/remote", &excludes(&["*.tmp"]), true, &mut None).unwrap_err();

        assert!(err.to_string().contains("a.txt"), "{err}");
    }

    #[test]
    fn load_local_side_from_csv_applies_excludes_and_returns_known_md5() {
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("local.csv");
        std::fs::write(
            &csv,
            "path,status,local_size,remote_size,local_md5,remote_md5\na.txt,Scan,5,,abc,\nskip.tmp,Scan,1,,def,\n",
        )
        .unwrap();

        let (entries, known) =
            load_local_side(&LocalSource::Csv(csv), &excludes(&["*.tmp"]), false, &mut None).unwrap();

        assert_eq!(entries, vec![LocalEntry { relative_path: "a.txt".to_string(), size: 5 }]);
        assert_eq!(known.get("a.txt").map(String::as_str), Some("abc"));
    }

    #[test]
    fn load_local_side_fails_for_missing_csv() {
        let source = LocalSource::Csv("/nonexistent/local.csv".into());

        assert!(load_local_side(&source, &ExcludeSet::default(), false, &mut None).is_err());
    }

    #[test]
    fn resolve_hashes_marks_hash_mismatch_using_live_sources() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"HELLO").unwrap(); // same size as remote "hello"
        let mut conn = server();
        let mut entries = vec![DiffEntry::new("a.txt", DiffStatus::Match, Some(5), Some(5))];
        let empty = HashMap::new();

        resolve_hashes(
            Some(&mut conn),
            Some("/remote"),
            &LocalSource::Live(dir.path().to_path_buf()),
            (&empty, &empty),
            &mut entries,
            &mut None,
        )
        .unwrap();

        assert_eq!(entries[0].status, DiffStatus::HashMismatch);
    }

    fn remote_entries() -> Vec<RemoteEntry> {
        vec![
            RemoteEntry { relative_path: "a.txt".into(), size: 5 },
            RemoteEntry { relative_path: "sub/b.txt".into(), size: 5 },
        ]
    }

    #[test]
    fn download_writes_files_and_creates_subdirectories() {
        let dest = tempfile::tempdir().unwrap();
        let mut conn = MockFtpConnection { default_content: Some(b"hello".to_vec()), ..server() };

        let counts = download_remote_files(&mut conn, "/remote", &remote_entries(), dest.path(), &mut None).unwrap();

        assert_eq!(counts, (2, 0));
        assert_eq!(std::fs::read(dest.path().join("a.txt")).unwrap(), b"hello");
        assert_eq!(std::fs::read(dest.path().join("sub/b.txt")).unwrap(), b"hello");
        assert!(!dest.path().join(".a.txt.ftpdiff-part").exists());
    }

    #[test]
    fn download_skips_files_with_same_size_and_replaces_others() {
        let dest = tempfile::tempdir().unwrap();
        std::fs::write(dest.path().join("a.txt"), b"HELLO").unwrap(); // same size: kept
        std::fs::create_dir(dest.path().join("sub")).unwrap();
        std::fs::write(dest.path().join("sub/b.txt"), b"old").unwrap(); // different size: replaced
        let mut conn = MockFtpConnection { default_content: Some(b"hello".to_vec()), ..server() };

        let counts = download_remote_files(&mut conn, "/remote", &remote_entries(), dest.path(), &mut None).unwrap();

        assert_eq!(counts, (1, 1));
        assert_eq!(std::fs::read(dest.path().join("a.txt")).unwrap(), b"HELLO");
        assert_eq!(std::fs::read(dest.path().join("sub/b.txt")).unwrap(), b"hello");
    }

    #[test]
    fn download_refuses_paths_escaping_the_destination() {
        let dest = tempfile::tempdir().unwrap();
        let entries = vec![RemoteEntry { relative_path: "../evil.txt".into(), size: 1 }];

        let err = download_remote_files(&mut server(), "/remote", &entries, dest.path(), &mut None).unwrap_err();

        assert!(err.to_string().contains("evil.txt"), "{err}");
    }

    #[test]
    fn download_error_names_the_file_and_leaves_no_partial_file() {
        let dest = tempfile::tempdir().unwrap();
        let mut conn = server();
        conn.files.clear();

        let err = download_remote_files(&mut conn, "/remote", &remote_entries(), dest.path(), &mut None).unwrap_err();

        assert!(err.to_string().contains("a.txt"), "{err}");
        assert!(!dest.path().join("a.txt").exists());
        assert!(!dest.path().join(".a.txt.ftpdiff-part").exists());
    }

    #[test]
    fn exit_code_is_ok_only_when_everything_matches() {
        let matching = vec![DiffEntry::new("a", DiffStatus::Match, Some(1), Some(1))];
        let differing = vec![matching[0].clone(), DiffEntry::new("b", DiffStatus::LocalOnly, Some(1), None)];

        assert_eq!(exit_code_for(&matching), EXIT_OK);
        assert_eq!(exit_code_for(&differing), EXIT_FAILURES);
        assert_eq!(exit_code_for(&[]), EXIT_OK);
    }
}
