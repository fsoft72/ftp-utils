//! End-to-end tests of the `ftpops` binary for operations that never touch
//! the network (`--dry-run` and `delete --on local`).

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const HEADER: &str = "path,status,local_size,remote_size,local_md5,remote_md5\n";

/// Runs `ftpops delete --on local --filter local-only ...` with dummy
/// connection settings (required by the CLI, unused for local deletes).
fn ftpops(args: &[&str], stdin: Option<&str>, local_dir: &Path) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ftpops"))
        .args(args)
        .args(["--host", "unused.invalid", "--user", "u", "--remote-dir", "/r", "--local-dir"])
        .arg(local_dir)
        .env("NO_COLOR", "1")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run ftpops");

    if let Some(input) = stdin {
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    }
    child.wait_with_output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// A temp dir holding `a.txt` and `b.txt` plus a CSV marking both `LocalOnly`.
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "a").unwrap();
    std::fs::write(dir.path().join("b.txt"), "b").unwrap();
    let csv = dir.path().join("report.csv");
    std::fs::write(&csv, format!("{HEADER}a.txt,LocalOnly,1,,,\nb.txt,LocalOnly,1,,,\nkeep.txt,Match,1,1,,\n")).unwrap();
    (dir, csv)
}

const DELETE_LOCAL: [&str; 6] = ["delete", "--on", "local", "--filter", "local-only", "--csv"];

fn delete_args(csv: &Path) -> Vec<&str> {
    let mut args = DELETE_LOCAL.to_vec();
    args.push(csv.to_str().unwrap());
    args
}

#[test]
fn dry_run_lists_files_and_deletes_nothing() {
    let (dir, csv) = fixture();
    let mut args = delete_args(&csv);
    args.push("--dry-run");

    let output = ftpops(&args, None, dir.path());

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains("Would delete: a.txt"), "{}", stdout(&output));
    assert!(stdout(&output).contains("Would delete 2 file(s)."), "{}", stdout(&output));
    assert!(dir.path().join("a.txt").exists() && dir.path().join("b.txt").exists());
}

#[test]
fn delete_without_confirmation_aborts_and_deletes_nothing() {
    let (dir, csv) = fixture();

    let output = ftpops(&delete_args(&csv), None, dir.path()); // stdin is closed

    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(stderr(&output).contains("aborted, nothing was deleted"), "{}", stderr(&output));
    assert!(dir.path().join("a.txt").exists() && dir.path().join("b.txt").exists());
}

#[test]
fn answering_no_aborts() {
    let (dir, csv) = fixture();

    let output = ftpops(&delete_args(&csv), Some("n\n"), dir.path());

    assert_eq!(output.status.code(), Some(2));
    assert!(dir.path().join("a.txt").exists());
}

#[test]
fn answering_yes_deletes_only_the_filtered_rows() {
    let (dir, csv) = fixture();
    std::fs::write(dir.path().join("keep.txt"), "k").unwrap();

    let output = ftpops(&delete_args(&csv), Some("y\n"), dir.path());

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains("Summary: 2 succeeded, 0 skipped, 0 failed"), "{}", stdout(&output));
    assert!(!dir.path().join("a.txt").exists() && !dir.path().join("b.txt").exists());
    assert!(dir.path().join("keep.txt").exists());
}

#[test]
fn yes_flag_skips_the_prompt() {
    let (dir, csv) = fixture();
    let mut args = delete_args(&csv);
    args.push("--yes");

    let output = ftpops(&args, None, dir.path());

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!dir.path().join("a.txt").exists());
}

#[test]
fn failed_operations_exit_one() {
    let (dir, csv) = fixture();
    std::fs::remove_file(dir.path().join("a.txt")).unwrap();
    let mut args = delete_args(&csv);
    args.push("--yes");

    let output = ftpops(&args, None, dir.path());

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stdout(&output).contains("1 succeeded, 0 skipped, 1 failed"), "{}", stdout(&output));
}

#[test]
fn csv_with_path_traversal_is_rejected_before_anything_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let victim_dir = dir.path().join("victim");
    let work_dir = dir.path().join("work");
    std::fs::create_dir_all(&victim_dir).unwrap();
    std::fs::create_dir_all(&work_dir).unwrap();
    std::fs::write(victim_dir.join("precious.txt"), "keep me").unwrap();
    std::fs::write(work_dir.join("a.txt"), "a").unwrap();
    let csv = dir.path().join("report.csv");
    std::fs::write(&csv, format!("{HEADER}a.txt,LocalOnly,1,,,\n../victim/precious.txt,LocalOnly,1,,,\n")).unwrap();
    let mut args = delete_args(&csv);
    args.push("--yes");

    let output = ftpops(&args, None, &work_dir);

    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(stderr(&output).contains(".."), "{}", stderr(&output));
    assert!(victim_dir.join("precious.txt").exists());
    assert!(work_dir.join("a.txt").exists(), "nothing at all may be deleted when the CSV is invalid");
}

#[test]
fn incompatible_filter_and_direction_exit_two() {
    let (dir, csv) = fixture();

    let output = ftpops(
        &["delete", "--on", "local", "--filter", "remote-only", "--csv", csv.to_str().unwrap(), "--yes"],
        None,
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("remote-only"), "{}", stderr(&output));
    assert!(dir.path().join("a.txt").exists());
}

#[test]
fn copy_dry_run_previews_without_connecting() {
    let (dir, csv) = fixture();
    let csv = csv.to_str().unwrap();

    let output = ftpops(
        &["copy", "--to", "remote", "--filter", "local-only", "--csv", csv, "--dry-run"],
        None,
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains("Would copy 2 file(s)."), "{}", stdout(&output));
}
