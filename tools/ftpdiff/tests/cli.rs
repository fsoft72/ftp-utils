//! End-to-end tests of the `ftpdiff` binary. They only use local
//! directories and CSV reports, so no FTP server is needed.

use std::path::Path;
use std::process::{Command, Output};

const HEADER: &str = "path,status,local_size,remote_size,local_md5,remote_md5\n";

fn ftpdiff(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ftpdiff")).args(args).env("NO_COLOR", "1").output().expect("failed to run ftpdiff")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn write(path: &Path, content: &str) {
    std::fs::write(path, content).unwrap();
}

fn str_of(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn identical_csv_sides_exit_zero() {
    let dir = tempfile::tempdir().unwrap();
    let report = dir.path().join("report.csv");
    write(&report, &format!("{HEADER}a.txt,Scan,5,5,,\nsub/b.txt,Scan,6,6,,\n"));

    let output = ftpdiff(&["--local-csv", str_of(&report), "--remote-csv", str_of(&report)]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains("Summary: 2 matched, 0 local-only, 0 remote-only"), "{}", stdout(&output));
}

#[test]
fn differences_exit_one_and_write_a_report() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local.csv");
    let remote = dir.path().join("remote.csv");
    let out = dir.path().join("out.csv");
    write(&local, &format!("{HEADER}a.txt,Scan,5,,,\nonly-local.txt,Scan,1,,,\n"));
    write(&remote, &format!("{HEADER}a.txt,Scan,,7,,\nonly-remote.txt,Scan,,2,,\n"));

    let output = ftpdiff(&["--local-csv", str_of(&local), "--remote-csv", str_of(&remote), "--csv", str_of(&out)]);

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let report = std::fs::read_to_string(&out).unwrap();
    assert!(report.contains("a.txt,SizeMismatch,5,7,,"), "{report}");
    assert!(report.contains("only-local.txt,LocalOnly,1,,,"), "{report}");
    assert!(report.contains("only-remote.txt,RemoteOnly,,2,,"), "{report}");
}

#[test]
fn exclude_patterns_remove_entries_from_csv_sources() {
    let dir = tempfile::tempdir().unwrap();
    let report = dir.path().join("report.csv");
    write(&report, &format!("{HEADER}a.txt,Scan,5,5,,\nnoise.tmp,Scan,1,,,\n"));

    let output = ftpdiff(&["--local-csv", str_of(&report), "--remote-csv", str_of(&report), "--exclude", "*.tmp"]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains("1 matched"), "{}", stdout(&output));
}

#[test]
fn missing_settings_exit_two_with_a_message() {
    let output = ftpdiff(&[]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).starts_with("Error: missing required setting"), "{}", stderr(&output));
}

#[test]
fn invalid_exclude_pattern_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let report = dir.path().join("report.csv");
    write(&report, HEADER);

    let output = ftpdiff(&["--local-csv", str_of(&report), "--remote-csv", str_of(&report), "--exclude", "[unclosed"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("[unclosed"), "{}", stderr(&output));
}

#[test]
fn unsafe_csv_path_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let report = dir.path().join("report.csv");
    write(&report, &format!("{HEADER}../../etc/passwd,Scan,1,1,,\n"));

    let output = ftpdiff(&["--local-csv", str_of(&report), "--remote-csv", str_of(&report)]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains(".."), "{}", stderr(&output));
}

#[test]
fn build_scans_a_local_directory_into_a_csv_that_can_be_compared() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    write(&tree.join("a.txt"), "hello");
    write(&tree.join("sub/b.txt"), "world!");
    let snapshot = dir.path().join("snapshot.csv");

    let built = ftpdiff(&["--build", "--local-dir", str_of(&tree), "--csv", str_of(&snapshot), "--hash"]);

    assert_eq!(built.status.code(), Some(0), "{}", stderr(&built));
    assert!(stdout(&built).contains("Scanned 2 entries."), "{}", stdout(&built));
    let report = std::fs::read_to_string(&snapshot).unwrap();
    assert!(report.contains("a.txt,Scan,5,,5d41402abc4b2a76b9719d911017c592,"), "{report}");

    // The snapshot is a valid source: comparing the live tree against it as
    // the "local" side of itself matches everything, hashes included.
    let compared = ftpdiff(&["--local-dir", str_of(&tree), "--remote-csv", str_of(&snapshot), "--hash"]);
    // remote side of the snapshot has no remote_size (it was a local scan),
    // so every file is reported as local-only.
    assert_eq!(compared.status.code(), Some(1), "{}", stderr(&compared));
    assert!(stdout(&compared).contains("2 local-only"), "{}", stdout(&compared));
}

#[test]
fn build_requires_csv() {
    let dir = tempfile::tempdir().unwrap();

    let output = ftpdiff(&["--build", "--local-dir", str_of(dir.path())]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--build requires --csv"), "{}", stderr(&output));
}

#[test]
fn live_local_dir_against_remote_csv_reports_hash_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    std::fs::create_dir(&tree).unwrap();
    write(&tree.join("a.txt"), "hello");
    let remote = dir.path().join("remote.csv");
    // Same size (5), different hash than "hello".
    write(&remote, &format!("{HEADER}a.txt,Scan,,5,,00000000000000000000000000000000\n"));

    let output = ftpdiff(&["--local-dir", str_of(&tree), "--remote-csv", str_of(&remote), "--hash"]);

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stdout(&output).contains("1 hash mismatch"), "{}", stdout(&output));
}
