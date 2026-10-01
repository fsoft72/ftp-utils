//! End-to-end tests of the `ftpwatch` binary that need no FTP server:
//! argument and configuration errors, which fail before any connection.

use std::process::{Command, Output};

/// Runs the `ftpwatch` binary with the given arguments.
fn ftpwatch(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ftpwatch")).args(args).output().expect("failed to run ftpwatch")
}

/// Returns the captured stderr as text.
fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn help_lists_both_commands() {
    let output = ftpwatch(&["--help"]);

    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(text.contains("init") && text.contains("check"), "{text}");
}

#[test]
fn a_site_without_config_json_exits_two() {
    let dir = tempfile::tempdir().unwrap();

    for command in ["init", "check"] {
        let output = ftpwatch(&[command, dir.path().to_str().unwrap()]);

        assert_eq!(output.status.code(), Some(2), "{command}");
        assert!(stderr(&output).starts_with("Error: cannot read config file"), "{}", stderr(&output));
    }
}

#[test]
fn init_refuses_an_existing_baseline_before_connecting() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.json"), r#"{"host":"127.0.0.1","user":"u","remote_dir":"/"}"#).unwrap();
    std::fs::create_dir(dir.path().join("snapshots")).unwrap();
    std::fs::write(dir.path().join("snapshots/2026-10-01_030000.csv"), "path,status\n").unwrap();

    let output = ftpwatch(&["init", dir.path().to_str().unwrap()]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--force"), "{}", stderr(&output));
}

#[test]
fn check_without_a_baseline_exits_two_before_connecting() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.json"), r#"{"host":"127.0.0.1","user":"u","remote_dir":"/"}"#).unwrap();

    let output = ftpwatch(&["check", dir.path().to_str().unwrap()]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("ftpwatch init"), "{}", stderr(&output));
}
