// FILE: catnip_rs/tests/standalone_cli.rs
//! Integration tests for catnip-standalone binary - CLI options.

mod common;
use common::standalone_binary;
use std::process::Command;

#[test]
fn test_version_flag() {
    let output = Command::new(standalone_binary())
        .arg("--version")
        .output()
        .expect("Failed to run --version");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("catnip"),
        "Version output should mention 'catnip'"
    );
}

#[test]
fn test_help_flag() {
    let output = Command::new(standalone_binary())
        .arg("--help")
        .output()
        .expect("Failed to run --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage") || stdout.contains("USAGE"));
    assert!(stdout.contains("command") || stdout.contains("-c"));
}

#[test]
fn test_verbose_flag() {
    let output = Command::new(standalone_binary())
        .arg("-c")
        .arg("2 + 3")
        .arg("-v")
        .output()
        .expect("Failed to run with -v");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Verbose mode should show result
    assert!(stdout.contains("5"));
}

#[test]
fn test_stdin_mode() {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(standalone_binary())
        .arg("--stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to spawn stdin mode");

    {
        let stdin = child.stdin.as_mut().expect("Failed to open stdin");
        stdin
            .write_all(b"21 * 2")
            .expect("Failed to write to stdin");
    }

    let output = child.wait_with_output().expect("Failed to read output");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(stdout, "42");
}

#[test]
fn test_multiple_statements() {
    let code = "x = 10; y = 20; x + y";
    let output = Command::new(standalone_binary())
        .arg("-c")
        .arg(code)
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(stdout, "30");
}

#[test]
fn test_multiline_code() {
    let code = r#"
x = 10
y = 20
x + y
"#;
    let output = Command::new(standalone_binary())
        .arg("-c")
        .arg(code)
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(stdout, "30");
}

#[test]
fn test_info_subcommand() {
    let output = Command::new(standalone_binary())
        .arg("info")
        .output()
        .expect("Failed to run info subcommand");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Info should show runtime information
    assert!(
        stdout.contains("Rust") || stdout.contains("Python") || stdout.contains("VM"),
        "Info output should show runtime details"
    );
}
