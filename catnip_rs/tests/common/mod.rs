// FILE: catnip_rs/tests/common/mod.rs
//! Common utilities for catnip binary integration tests.

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Command, Output};

/// Path to the catnip binary
pub fn run_binary() -> PathBuf {
    // Use the binary built by cargo test (handles --target-dir correctly)
    let mut path = std::env::current_exe().expect("Failed to get current exe path");
    // current_exe = target/<dir>/deps/test_binary-hash
    // binary     = target/<dir>/catnip
    path.pop(); // remove test binary name
    path.pop(); // remove "deps"
    path.push("catnip");
    path
}

/// Execute code with the catnip binary
pub fn run_code(code: &str) -> Output {
    Command::new(run_binary())
        .arg("-c")
        .arg(code)
        .output()
        .expect("Failed to execute catnip")
}

/// Execute code and expect success
pub fn assert_output(code: &str, expected: &str) {
    let output = run_code(code);
    assert!(
        output.status.success(),
        "Command failed with stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(stdout, expected, "Output mismatch for code: {}", code);
}

/// Execute code and expect failure
pub fn assert_error(code: &str) {
    let output = run_code(code);
    assert!(
        !output.status.success(),
        "Command should have failed but succeeded with output: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// Execute code and verify output contains expected substring
pub fn assert_output_contains(code: &str, expected: &str) {
    let output = run_code(code);
    assert!(
        output.status.success(),
        "Command failed with stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(expected),
        "Output does not contain '{}'. Full output: {}",
        expected,
        stdout
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_exists() {
        let path = run_binary();
        assert!(
            path.exists(),
            "catnip binary not found at {:?}. Run 'cargo build --release --bin catnip'",
            path
        );
    }
}
