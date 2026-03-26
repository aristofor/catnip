// FILE: catnip_rs/src/repl/mod.rs
//! REPL text utilities - PyO3 wrappers around `catnip_tools::multiline`.
//!
//! The full TUI REPL lives in the `catnip_repl` crate.

use std::time::SystemTime;

use pyo3::prelude::*;

/// Pick a random exit message for the given reason.
///
/// `reason`: `"ok"`, `"abort"`, or `"weird"`.
#[pyfunction]
#[pyo3(name = "repl_exit_message")]
pub fn repl_exit_message(reason: &str) -> &'static str {
    use catnip_core::constants;

    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);

    let msgs = match reason {
        "abort" => constants::REPL_EXIT_ABORT,
        "weird" => constants::REPL_EXIT_RARE,
        _ => constants::REPL_EXIT_OK,
    };
    msgs[nanos % msgs.len()]
}

/// Check if input requires multiline continuation.
///
/// Detects unclosed delimiters and continuation operators.
#[pyfunction]
#[pyo3(name = "should_continue_multiline")]
pub fn should_continue_multiline(text: &str) -> bool {
    catnip_tools::multiline::should_continue_multiline(text)
}

/// Preprocess multiline code to handle implicit continuations.
///
/// Joins lines ending with an operator or comma with the following lines,
/// preserving spaces for indentation.
#[pyfunction]
#[pyo3(name = "preprocess_multiline")]
pub fn preprocess_multiline(code: &str) -> String {
    catnip_tools::multiline::preprocess_multiline(code)
}

/// Parse a REPL command (starting with slash).
///
/// Returns (command_name, args) or (None, None) if not a command.
#[pyfunction]
#[pyo3(name = "parse_repl_command")]
pub fn parse_repl_command(command: &str) -> (Option<String>, Option<String>) {
    if !command.starts_with('/') {
        return (None, None);
    }

    // Remove slash prefix
    let cmd_rest = command[1..].trim();

    // Parse command and args
    let mut parts = cmd_rest.splitn(2, char::is_whitespace);

    let cmd_name = match parts.next() {
        Some(name) if !name.is_empty() => name.to_lowercase(),
        _ => return (None, None),
    };

    // Skip whitespace and get args
    let cmd_args = parts
        .next()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    (Some(cmd_name), cmd_args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_continue_multiline_delimiters() {
        assert!(should_continue_multiline("(1 + 2"));
        assert!(should_continue_multiline("[1, 2, 3"));
        assert!(should_continue_multiline("{a: 1"));
        assert!(!should_continue_multiline("(1 + 2)"));
    }

    #[test]
    fn test_should_continue_multiline_operators() {
        assert!(should_continue_multiline("1 +"));
        assert!(should_continue_multiline("x ="));
        assert!(should_continue_multiline("a,"));
        assert!(!should_continue_multiline("1"));
    }

    #[test]
    fn test_should_continue_multiline_keywords() {
        assert!(should_continue_multiline("if"));
        assert!(should_continue_multiline("x = 1 if"));
        assert!(!should_continue_multiline("result"));
    }

    #[test]
    fn test_preprocess_multiline_simple() {
        let code = "1 +\n2";
        assert_eq!(preprocess_multiline(code), "1 + 2");
    }

    #[test]
    fn test_preprocess_multiline_multiple() {
        let code = "1 +\n2 *\n3";
        assert_eq!(preprocess_multiline(code), "1 + 2 * 3");
    }

    #[test]
    fn test_preprocess_multiline_no_continuation() {
        let code = "1 + 2\n3 + 4";
        assert_eq!(preprocess_multiline(code), "1 + 2\n3 + 4");
    }

    #[test]
    fn test_parse_repl_command_valid() {
        let (cmd, args) = parse_repl_command("/help");
        assert_eq!(cmd, Some("help".to_string()));
        assert_eq!(args, None);

        let (cmd, args) = parse_repl_command("/load file.cat");
        assert_eq!(cmd, Some("load".to_string()));
        assert_eq!(args, Some("file.cat".to_string()));
    }

    #[test]
    fn test_parse_repl_command_invalid() {
        let (cmd, args) = parse_repl_command("not a command");
        assert_eq!(cmd, None);
        assert_eq!(args, None);
    }
}
