// FILE: catnip_rs/tests/common/mod.rs
//! Common utilities for standalone binary integration tests.

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Command, Output};

/// Path to the standalone binary
pub fn standalone_binary() -> PathBuf {
    // Dans un workspace Cargo, le binaire est dans target/ du workspace root
    // CARGO_MANIFEST_DIR pointe vers catnip_rs/, on remonte d'un niveau
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // Remonter au workspace root
    path.push("target");

    // Détecter le mode (release ou debug) selon PROFILE
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    path.push(profile);
    path.push("catnip-standalone");
    path
}

/// Execute code with the standalone binary
pub fn run_code(code: &str) -> Output {
    Command::new(standalone_binary())
        .arg("-c")
        .arg(code)
        .output()
        .expect("Failed to execute catnip-standalone")
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
        let path = standalone_binary();
        assert!(
            path.exists(),
            "Standalone binary not found at {:?}. Run 'cargo build --release --bin catnip-standalone'",
            path
        );
    }
}
