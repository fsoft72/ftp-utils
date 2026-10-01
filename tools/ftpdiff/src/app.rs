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

/// Downloads remote files (binary mode, streamed) into one directory as the
/// remote walk lists them, recreating subdirectories. A file already there
/// with the same size is skipped. Each file is written to a temporary
/// `.<name>.ftpdiff-part` file and renamed on success, so a failed transfer
/// never leaves a truncated file under its final name.
struct Downloader<'a> {
    dest: &'a Path,
    remote_root: &'a str,
    verbose: bool,
    downloaded: usize,
    skipped: usize,
}

impl Downloader<'_> {
    /// Fetches one listed file, or skips it if it is already up to date.
    fn fetch<C: FtpConnection>(&mut self, conn: &mut C, entry: &RemoteEntry) -> Result<(), CliError> {
        validate_relative_path(&entry.relative_path)
            .map_err(|e| CliError(format!("refusing to download {}: {e}", entry.relative_path)))?;

        let target = self.dest.join(&entry.relative_path);
        if fs::metadata(&target).is_ok_and(|m| m.is_file() && m.len() == entry.size) {
            self.skipped += 1;
            return Ok(());
        }

        if self.verbose {
            eprintln!("Downloading {}", entry.relative_path);
        }
        download_one(conn, &remote::join_remote(self.remote_root, &entry.relative_path), &target)
            .map_err(|e| CliError(format!("failed to download {}: {e}", entry.relative_path)))?;
        self.downloaded += 1;
        Ok(())
    }
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

/// Walks the remote tree; when `download_dir` is set, each file is
/// downloaded right when the walk lists it, and a summary line is printed.
fn walk_and_download<C: FtpConnection>(
    conn: &mut C,
    remote_root: &str,
    exclude: &ExcludeSet,
    download_dir: Option<&Path>,
    verbose: bool,
    progress: &mut Progress,
) -> Result<Vec<RemoteEntry>, CliError> {
    let Some(dest) = download_dir else {
        return Ok(remote::walk_remote(conn, remote_root, exclude, progress.as_deref_mut())?);
    };

    let mut downloader = Downloader { dest, remote_root, verbose, downloaded: 0, skipped: 0 };
    let entries = remote::walk_remote_with(conn, remote_root, exclude, progress.as_deref_mut(), |conn, entry| {
        downloader.fetch(conn, entry)
    })?;

    println!(
        "Downloaded {} files to {} ({} skipped, same size).",
        downloader.downloaded,
        dest.display(),
        downloader.skipped
    );
    Ok(entries)
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
            let entries = walk_and_download(
                &mut conn,
                &params.remote_dir,
                &effective.exclude,
                effective.download_dir.as_deref(),
                effective.verbose,
                &mut progress,
            )?;
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
/// `hash` is set and downloading each file into `download_dir` (if given)
/// during the scan.
fn scan_remote<C: FtpConnection>(
    conn: &mut C,
    remote_dir: &str,
    exclude: &ExcludeSet,
    hash: bool,
    download_dir: Option<&Path>,
    verbose: bool,
    progress: &mut Progress,
) -> Result<Vec<ReportEntry>, CliError> {
    let mut entries = Vec::new();
    for item in walk_and_download(conn, remote_dir, exclude, download_dir, verbose, progress)? {
        let mut entry = ReportEntry::scan(item.relative_path, None, Some(item.size)).with_remote_mtime(item.modified);
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
            let entries = scan_remote(
                &mut conn,
                &params.remote_dir,
                &build.exclude,
                build.hash,
                build.download_dir.as_deref(),
                build.verbose,
                &mut progress,
            )?;
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
                RawRemoteEntry { name: "a.txt".into(), is_dir: false, size: 5, modified: None },
                RawRemoteEntry { name: "skip.tmp".into(), is_dir: false, size: 1, modified: None },
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

        let plain = scan_remote(&mut conn, "/remote", &excludes(&["*.tmp"]), false, None, false, &mut None).unwrap();
        assert_eq!(plain, vec![ReportEntry::scan("a.txt", None, Some(5))]);

        let hashed = scan_remote(&mut conn, "/remote", &excludes(&["*.tmp"]), true, None, false, &mut None).unwrap();
        assert_eq!(hashed[0].remote_md5.as_deref(), Some(HELLO_MD5));
    }

    #[test]
    fn scan_remote_reports_the_file_that_failed_to_hash() {
        let mut conn = server();
        conn.files.clear();

        let err = scan_remote(&mut conn, "/remote", &excludes(&["*.tmp"]), true, None, false, &mut None).unwrap_err();

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

    /// Walks `/remote` on `conn`, downloading into `dest`.
    fn download_walk(conn: &mut MockFtpConnection, dest: &Path) -> Result<Vec<RemoteEntry>, CliError> {
        walk_and_download(conn, "/remote", &excludes(&["*.tmp"]), Some(dest), false, &mut None)
    }

    #[test]
    fn download_happens_during_the_walk_and_honours_excludes() {
        let dest = tempfile::tempdir().unwrap();
        let mut conn = MockFtpConnection { default_content: Some(b"hello".to_vec()), ..server() };

        let entries = download_walk(&mut conn, dest.path()).unwrap();

        assert_eq!(entries, vec![RemoteEntry { relative_path: "a.txt".into(), size: 5, modified: None }]);
        assert_eq!(std::fs::read(dest.path().join("a.txt")).unwrap(), b"hello");
        assert!(!dest.path().join("skip.tmp").exists());
        assert!(!dest.path().join(".a.txt.ftpdiff-part").exists());
    }

    #[test]
    fn download_creates_subdirectories() {
        let dest = tempfile::tempdir().unwrap();
        let mut conn = server();
        conn.listings.get_mut("/remote").unwrap().push(RawRemoteEntry {
            name: "sub".into(),
            is_dir: true,
            size: 0,
            modified: None,
        });
        conn.listings.insert(
            "/remote/sub".into(),
            vec![RawRemoteEntry { name: "b.txt".into(), is_dir: false, size: 5, modified: None }],
        );
        conn.files.insert("/remote/sub/b.txt".into(), b"hello".to_vec());

        download_walk(&mut conn, dest.path()).unwrap();

        assert_eq!(std::fs::read(dest.path().join("sub/b.txt")).unwrap(), b"hello");
    }

    #[test]
    fn download_skips_same_size_and_replaces_different_size() {
        let dest = tempfile::tempdir().unwrap();
        std::fs::write(dest.path().join("a.txt"), b"HELLO").unwrap(); // same size: kept

        download_walk(&mut server(), dest.path()).unwrap();
        assert_eq!(std::fs::read(dest.path().join("a.txt")).unwrap(), b"HELLO");

        std::fs::write(dest.path().join("a.txt"), b"old").unwrap(); // different size: replaced
        download_walk(&mut server(), dest.path()).unwrap();
        assert_eq!(std::fs::read(dest.path().join("a.txt")).unwrap(), b"hello");
    }

    #[test]
    fn download_refuses_paths_escaping_the_destination() {
        let dest = tempfile::tempdir().unwrap();
        let mut downloader =
            Downloader { dest: dest.path(), remote_root: "/remote", verbose: false, downloaded: 0, skipped: 0 };
        let entry = RemoteEntry { relative_path: "../evil.txt".into(), size: 1, modified: None };

        let err = downloader.fetch(&mut server(), &entry).unwrap_err();

        assert!(err.to_string().contains("evil.txt"), "{err}");
    }

    #[test]
    fn download_error_names_the_file_and_leaves_no_partial_file() {
        let dest = tempfile::tempdir().unwrap();
        let mut conn = server();
        conn.files.clear();

        let err = download_walk(&mut conn, dest.path()).unwrap_err();

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
