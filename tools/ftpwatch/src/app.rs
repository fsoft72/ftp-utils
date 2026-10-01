//! ftpwatch's orchestration: `init` (download + baseline) and `check`
//! (listing-only scan, comparison, log, new snapshot). The work is done in
//! functions generic over `FtpConnection` so it can be tested without a
//! server.

use std::path::{Path, PathBuf};

use ftp_utils_core::connection::{self, ConnectionError, RemoteParams};
use ftp_utils_core::download::{DownloadError, Downloader};
use ftp_utils_core::exclude::ExcludeSet;
use ftp_utils_core::exit::{EXIT_FAILURES, EXIT_OK};
use ftp_utils_core::ftp_client::SuppaFtpConnection;
use ftp_utils_core::remote::{self, RemoteEntry};
use ftp_utils_core::{FtpConnection, FtpConnectionError};

use crate::cli::{Cli, Command, CommonArgs};
use crate::compare::{compare_snapshots, Comparison};
use crate::config::{self, SiteSettings};
use crate::log::format_log;
use crate::store::{self, SiteDir, StoreError};
use crate::timefmt::{format_stamp, now_unix};

/// Environment variable holding the FTP password.
const PASSWORD_ENV_VAR: &str = "FTPWATCH_PASSWORD";

ftp_utils_core::message_error! {
    /// A fatal ftpwatch error: printed once as `Error: <message>`, exit code 2.
    pub CliError
}

impl From<ConnectionError> for CliError {
    /// Converts the error into its message.
    fn from(e: ConnectionError) -> Self {
        CliError(e.to_string())
    }
}

impl From<DownloadError> for CliError {
    /// Converts the error into its message.
    fn from(e: DownloadError) -> Self {
        CliError(e.to_string())
    }
}

impl From<FtpConnectionError> for CliError {
    /// Converts the error into its message.
    fn from(e: FtpConnectionError) -> Self {
        CliError(e.to_string())
    }
}

impl From<StoreError> for CliError {
    /// Converts the error into its message.
    fn from(e: StoreError) -> Self {
        CliError(e.to_string())
    }
}

/// Runs the requested command and returns the process exit code:
/// `EXIT_OK` for `init` and for a `check` without changes, `EXIT_FAILURES`
/// for a `check` that found changes.
pub fn run(cli: &Cli) -> Result<i32, CliError> {
    match &cli.command {
        Command::Init(args) => run_init(&args.common, args.force),
        Command::Check(args) => run_check(&args.common),
    }
}

/// Connects to the server (asking for the password if needed).
fn connect(params: &RemoteParams, common: &CommonArgs) -> Result<SuppaFtpConnection, CliError> {
    let password = connection::read_password(common.password.as_deref(), PASSWORD_ENV_VAR)?;
    if common.verbose {
        eprintln!("Connecting to {}:{} as {}...", params.host, params.port, params.user);
    }
    SuppaFtpConnection::connect_params(params, &password)
        .map_err(|e| CliError(format!("failed to connect to {}:{}: {e}", params.host, params.port)))
}

/// Runs the `init` command: loads config, connects, downloads, records the baseline.
fn run_init(common: &CommonArgs, force: bool) -> Result<i32, CliError> {
    let site = SiteDir::new(common.site_dir.clone());
    let settings = config::load(&site.config_path())?;
    ensure_can_init(&site, force)?;

    let mut conn = connect(&settings.remote, common)?;
    let count = init(&mut conn, &site, &settings, now_unix(), common.verbose)?;
    conn.close();

    println!("Downloaded {count} files to {} and recorded the baseline snapshot.", site.files_dir().display());
    Ok(EXIT_OK)
}

/// Runs the `check` command: scans, logs, records a snapshot and picks the exit code.
fn run_check(common: &CommonArgs) -> Result<i32, CliError> {
    let site = SiteDir::new(common.site_dir.clone());
    let settings = config::load(&site.config_path())?;
    let site_name = _site_name(site.root());

    // Verify the baseline before connecting, so a missing one never reaches the password prompt.
    _latest_snapshot_or_error(&site)?;

    let now = now_unix();
    let mut conn = connect(&settings.remote, common)?;
    let comparison = check(&mut conn, &site, &settings, &site_name, now)?;
    conn.close();

    println!(
        "{} changes found ({} unchanged). Log: {}",
        comparison.changes.len(),
        comparison.unchanged,
        site.log_path(&format_stamp(now)).display()
    );
    Ok(exit_code_for(&comparison))
}

/// Exit code of a `check`: `EXIT_OK` without changes, `EXIT_FAILURES` otherwise.
fn exit_code_for(comparison: &Comparison) -> i32 {
    if comparison.changes.is_empty() {
        EXIT_OK
    } else {
        EXIT_FAILURES
    }
}

/// Derives the site name from the directory name, resolving the path first so
/// `.` still yields a name; falls back to the path's display string.
fn _site_name(root: &Path) -> String {
    let resolved = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    match resolved.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => resolved.display().to_string(),
    }
}

/// Fails if the site already has a snapshot, unless `force` is set, so a
/// second `init` cannot silently replace the baseline.
pub fn ensure_can_init(site: &SiteDir, force: bool) -> Result<(), CliError> {
    if force {
        return Ok(());
    }
    match site.latest_snapshot()? {
        None => Ok(()),
        Some(existing) => Err(CliError(format!(
            "site already initialized (found snapshot {}); use --force to start over",
            existing.display()
        ))),
    }
}

/// Walks the remote tree downloading every non-excluded file into the
/// site's `files/` directory, then writes the baseline snapshot. Returns
/// the number of files. No snapshot is written if anything fails.
pub fn init<C: FtpConnection>(
    conn: &mut C,
    site: &SiteDir,
    settings: &SiteSettings,
    now: i64,
    verbose: bool,
) -> Result<usize, CliError> {
    let files_dir = site.files_dir();
    let remote_root = settings.remote.remote_dir.as_str();
    let mut downloader = Downloader::new(&files_dir, remote_root, verbose);

    let entries = remote::walk_remote_with(conn, remote_root, &settings.exclude, None, |conn, entry| {
        downloader.fetch(conn, entry).map_err(CliError::from)
    })?;

    store::write_snapshot(&site.snapshot_path(&format_stamp(now)), &entries)?;
    Ok(entries.len())
}

/// Returns the latest snapshot path, or the "run `ftpwatch init` first" error.
fn _latest_snapshot_or_error(site: &SiteDir) -> Result<PathBuf, CliError> {
    site.latest_snapshot()?
        .ok_or_else(|| CliError(format!("no snapshot found for {}; run `ftpwatch init` first", site.root().display())))
}

/// Reads the latest snapshot without the entries the current excludes
/// would skip (so adding an exclude does not report those files deleted).
fn _load_previous(site: &SiteDir, exclude: &ExcludeSet) -> Result<(String, Vec<RemoteEntry>), CliError> {
    let path = _latest_snapshot_or_error(site)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let entries = store::read_snapshot(&path)?.into_iter().filter(|e| !exclude.is_excluded(&e.relative_path)).collect();
    Ok((name, entries))
}

/// Scans the remote tree (listing only, never downloading), compares it
/// with the latest snapshot, writes the log and then today's snapshot.
/// Nothing is written if the scan fails or returns no files while the
/// previous snapshot had some, so a broken run cannot corrupt the history.
pub fn check<C: FtpConnection>(
    conn: &mut C,
    site: &SiteDir,
    settings: &SiteSettings,
    site_name: &str,
    now: i64,
) -> Result<Comparison, CliError> {
    let (previous_name, previous) = _load_previous(site, &settings.exclude)?;
    let current = remote::walk_remote(conn, &settings.remote.remote_dir, &settings.exclude, None)?;

    if current.is_empty() && !previous.is_empty() {
        return Err(CliError(format!(
            "the remote listing is empty but the previous snapshot has {} files; refusing to record every file as deleted",
            previous.len()
        )));
    }

    let comparison = compare_snapshots(&previous, &current);
    let stamp = format_stamp(now);
    store::write_log(&site.log_path(&stamp), &format_log(site_name, now, &previous_name, &comparison))?;
    store::write_snapshot(&site.snapshot_path(&stamp), &current)?;
    Ok(comparison)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use ftp_utils_core::testing::MockFtpConnection;
    use ftp_utils_core::RawRemoteEntry;

    const NOW: i64 = 1_790_000_000;
    const T0: i64 = 1_780_000_000;

    /// Builds site settings with the given exclude globs.
    fn settings(excludes: &[&str]) -> SiteSettings {
        let patterns: Vec<String> = excludes.iter().map(|s| s.to_string()).collect();
        SiteSettings {
            remote: RemoteParams {
                host: "h".into(),
                port: 21,
                user: "u".into(),
                remote_dir: "/site".into(),
                ftps: false,
                insecure_tls: false,
                timeout_secs: 30,
            },
            exclude: ExcludeSet::new(&patterns).unwrap(),
        }
    }

    /// Builds a raw remote file entry.
    fn file(name: &str, size: u64, modified: i64) -> RawRemoteEntry {
        RawRemoteEntry { name: name.into(), is_dir: false, size, modified: Some(modified) }
    }

    /// Builds a raw remote directory entry.
    fn dir(name: &str) -> RawRemoteEntry {
        RawRemoteEntry { name: name.into(), is_dir: true, size: 0, modified: None }
    }

    /// A strict mock: listing a directory not given here is an error, and
    /// there is no file content, so any download attempt fails.
    fn server(listings: Vec<(&str, Vec<RawRemoteEntry>)>) -> MockFtpConnection {
        MockFtpConnection::strict(listings.into_iter().map(|(k, v)| (k.to_string(), v)).collect::<HashMap<_, _>>())
    }

    /// Builds a snapshot entry.
    fn entry(path: &str, size: u64, modified: i64) -> RemoteEntry {
        RemoteEntry { relative_path: path.into(), size, modified: Some(modified) }
    }

    /// Site directory with a baseline snapshot named `2026-09-01_030000`.
    fn site_with_baseline(baseline: &[RemoteEntry]) -> (tempfile::TempDir, SiteDir) {
        let dir = tempfile::tempdir().unwrap();
        let site = SiteDir::new(dir.path().to_path_buf());
        store::write_snapshot(&site.snapshot_path("2026-09-01_030000"), baseline).unwrap();
        (dir, site)
    }

    /// Returns the content of the single log file.
    fn only_log(site: &SiteDir) -> String {
        let logs: Vec<_> = std::fs::read_dir(site.root().join("logs")).unwrap().flatten().collect();
        assert_eq!(logs.len(), 1);
        std::fs::read_to_string(logs[0].path()).unwrap()
    }

    /// Counts the files in the snapshots directory.
    fn snapshot_count(site: &SiteDir) -> usize {
        std::fs::read_dir(site.root().join("snapshots")).unwrap().count()
    }

    #[test]
    fn init_downloads_files_skips_excluded_dirs_and_writes_the_baseline() {
        let tmp = tempfile::tempdir().unwrap();
        let site = SiteDir::new(tmp.path().to_path_buf());
        // No listing for /site/wp-content/uploads: walking it would be an error.
        let mut conn = server(vec![
            ("/site", vec![file("index.php", 5, T0), dir("wp-content")]),
            ("/site/wp-content", vec![dir("uploads"), file("a.php", 5, T0 + 100)]),
        ]);
        conn.default_content = Some(b"hello".to_vec());

        let count = init(&mut conn, &site, &settings(&["wp-content/uploads/**"]), NOW, false).unwrap();

        assert_eq!(count, 2);
        assert_eq!(std::fs::read(site.files_dir().join("index.php")).unwrap(), b"hello");
        assert_eq!(std::fs::read(site.files_dir().join("wp-content/a.php")).unwrap(), b"hello");
        let snapshot = store::read_snapshot(&site.latest_snapshot().unwrap().unwrap()).unwrap();
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot.iter().all(|e| e.modified.is_some()));
    }

    #[test]
    fn init_failure_writes_no_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let site = SiteDir::new(dir.path().to_path_buf());
        let mut conn = server(vec![("/site", vec![file("index.php", 5, T0)])]); // no content: download fails

        assert!(init(&mut conn, &site, &settings(&[]), NOW, false).is_err());

        assert_eq!(site.latest_snapshot().unwrap(), None);
    }

    #[test]
    fn init_refuses_an_existing_baseline_unless_forced() {
        let (_dir, site) = site_with_baseline(&[]);

        let err = ensure_can_init(&site, false).unwrap_err();

        assert!(err.to_string().contains("--force"), "{err}");
        assert!(ensure_can_init(&site, true).is_ok());
        let fresh = tempfile::tempdir().unwrap();
        assert!(ensure_can_init(&SiteDir::new(fresh.path().to_path_buf()), false).is_ok());
    }

    #[test]
    fn check_without_changes_writes_a_clean_log_and_downloads_nothing() {
        let (_dir, site) = site_with_baseline(&[entry("index.php", 5, T0)]);
        let mut conn = server(vec![("/site", vec![file("index.php", 5, T0)])]); // no content: a download would fail

        let comparison = check(&mut conn, &site, &settings(&[]), "test.com", NOW).unwrap();

        assert!(comparison.changes.is_empty());
        assert!(only_log(&site).contains("Summary: no changes"));
        assert_eq!(snapshot_count(&site), 2);
        assert!(!site.files_dir().exists());
    }

    #[test]
    fn exit_code_is_ok_without_changes_and_failures_with_changes() {
        let (_dir, site) = site_with_baseline(&[entry("a.php", 5, T0)]);
        let same =
            check(&mut server(vec![("/site", vec![file("a.php", 5, T0)])]), &site, &settings(&[]), "s", NOW).unwrap();
        assert_eq!(exit_code_for(&same), EXIT_OK);

        let (_dir2, site2) = site_with_baseline(&[entry("a.php", 5, T0)]);
        let changed =
            check(&mut server(vec![("/site", vec![file("a.php", 6, T0)])]), &site2, &settings(&[]), "s", NOW).unwrap();
        assert_eq!(exit_code_for(&changed), EXIT_FAILURES);
    }

    #[test]
    fn site_name_uses_the_directory_name_and_resolves_dot() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("test.com");
        std::fs::create_dir(&sub).unwrap();
        assert_eq!(_site_name(&sub), "test.com");
        assert_eq!(_site_name(&sub.join("..").join("test.com")), "test.com");
        assert!(!_site_name(Path::new(".")).is_empty());
        assert_eq!(_site_name(Path::new("/")), "/");
    }

    #[test]
    fn check_reports_new_deleted_and_modified_files() {
        let (_dir, site) =
            site_with_baseline(&[entry("edit.php", 5, T0), entry("gone.php", 7, T0), entry("same.php", 9, T0)]);
        let mut conn = server(vec![(
            "/site",
            vec![file("edit.php", 6, T0), file("same.php", 9, T0), file("brand-new.php", 3, T0 + 500)],
        )]);

        let comparison = check(&mut conn, &site, &settings(&[]), "test.com", NOW).unwrap();

        assert_eq!(comparison.changes.len(), 3);
        let log = only_log(&site);
        assert!(log.contains("Summary: 1 new, 1 deleted, 1 modified, 1 unchanged"), "{log}");
        assert!(log.lines().any(|l| l.starts_with("NEW") && l.contains("brand-new.php")), "{log}");
        assert!(log.lines().any(|l| l.starts_with("DELETED") && l.contains("gone.php")), "{log}");
        assert!(log.lines().any(|l| l.starts_with("MODIFIED") && l.contains("edit.php")), "{log}");
        // The next run compares against today's snapshot.
        let latest = store::read_snapshot(&site.latest_snapshot().unwrap().unwrap()).unwrap();
        assert!(latest.iter().any(|e| e.relative_path == "brand-new.php"));
    }

    #[test]
    fn check_without_a_baseline_says_to_run_init() {
        let dir = tempfile::tempdir().unwrap();
        let site = SiteDir::new(dir.path().to_path_buf());

        let err = check(&mut server(vec![]), &site, &settings(&[]), "s", NOW).unwrap_err();

        assert!(err.to_string().contains("ftpwatch init"), "{err}");
    }

    #[test]
    fn check_that_fails_midway_writes_neither_log_nor_snapshot() {
        let (_dir, site) = site_with_baseline(&[entry("index.php", 5, T0)]);
        let mut conn = server(vec![("/site", vec![file("index.php", 5, T0), dir("sub")])]); // /site/sub has no listing

        assert!(check(&mut conn, &site, &settings(&[]), "s", NOW).is_err());

        assert_eq!(snapshot_count(&site), 1);
        assert!(!site.root().join("logs").exists());
    }

    #[test]
    fn check_refuses_an_empty_listing_when_the_baseline_has_files() {
        let (_dir, site) = site_with_baseline(&[entry("index.php", 5, T0)]);
        let mut conn = server(vec![("/site", vec![])]);

        let err = check(&mut conn, &site, &settings(&[]), "s", NOW).unwrap_err();

        assert!(err.to_string().contains("empty"), "{err}");
        assert_eq!(snapshot_count(&site), 1);
        assert!(!site.root().join("logs").exists());
    }

    #[test]
    fn newly_excluded_paths_are_not_reported_as_deleted() {
        let (_dir, site) = site_with_baseline(&[entry("index.php", 5, T0), entry("cache/x.tmp", 1, T0)]);
        let mut conn = server(vec![("/site", vec![file("index.php", 5, T0)])]);

        let comparison = check(&mut conn, &site, &settings(&["cache/**"]), "s", NOW).unwrap();

        assert!(comparison.changes.is_empty(), "{:?}", comparison.changes);
    }
}
