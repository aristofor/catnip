// FILE: catnip_rs/src/repl/mod.rs
//! REPL text utilities optimized in Rust.
//!
//! Contains performance-critical functions for:
//! - Multiline continuation detection
//! - Code preprocessing
//! - Command parsing
//!
//! The full TUI REPL lives in the `catnip_repl` crate.

use pyo3::prelude::*;

/// Continuation operators
const CONTINUATION_OPS: &[&str] = &[
    // Arithmetic (sorted by length, longest first)
    "**", "//", "+", "-", "*", "/", "%", // Bitwise
    "<<", ">>", "&", "|", "^", // Comparison
    "==", "!=", "<=", ">=", "<", ">", // Other
    ",", "=",
];

/// Continuation keywords
const CONTINUATION_KEYWORDS: &[&str] = &["if", "elif", "else", "while", "for", "match"];

/// Check if input requires multiline continuation.
///
/// Detects unclosed delimiters and continuation operators.
#[pyfunction]
#[pyo3(name = "should_continue_multiline")]
pub fn should_continue_multiline(text: &str) -> bool {
    let stripped = text.trim_end();

    // Count delimiters
    let brace_count = text.matches('{').count() as i32 - text.matches('}').count() as i32;
    let paren_count = text.matches('(').count() as i32 - text.matches(')').count() as i32;
    let bracket_count = text.matches('[').count() as i32 - text.matches(']').count() as i32;

    // Continue if any delimiters are still open
    if brace_count > 0 || paren_count > 0 || bracket_count > 0 {
        return true;
    }

    // Check for operators (multi-char operators first - already sorted)
    for op in CONTINUATION_OPS {
        if stripped.ends_with(op) {
            return true;
        }
    }

    // Check for continuation keywords
    if let Some(last_word) = stripped.split_whitespace().last() {
        if CONTINUATION_KEYWORDS.contains(&last_word) {
            return true;
        }
    }

    false
}

/// Check if line ends with continuation operator (internal helper).
fn has_continuation_op(line: &str) -> bool {
    let stripped = line.trim_end();
    CONTINUATION_OPS.iter().any(|op| stripped.ends_with(op))
}

/// Preprocess multiline code to handle implicit continuations.
///
/// Joins lines ending with an operator or comma with the following lines,
/// preserving spaces for indentation.
#[pyfunction]
#[pyo3(name = "preprocess_multiline")]
pub fn preprocess_multiline(code: &str) -> String {
    let lines: Vec<&str> = code.split('\n').collect();
    let n_lines = lines.len();

    if n_lines == 1 {
        return code.to_string();
    }

    let mut processed = Vec::new();
    let mut i = 0;

    while i < n_lines {
        let line = lines[i];
        let stripped = line.trim_end();

        // If the line has a continuation, accumulate all following lines
        if has_continuation_op(stripped) {
            let mut accumulated = vec![stripped.to_string()];
            let mut j = i + 1;

            // Continue accumulating while we have continuations
            while j < n_lines {
                let next_line = lines[j].trim_start();
                accumulated.push(next_line.to_string());

                // If this line also has a continuation, keep going
                if has_continuation_op(next_line) {
                    j += 1;
                } else {
                    // Last line of the continuation
                    j += 1;
                    break;
                }
            }

            // Join all accumulated lines
            processed.push(accumulated.join(" "));
            i = j; // Skip all processed lines
        } else {
            processed.push(line.to_string());
            i += 1;
        }
    }

    processed.join("\n")
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
