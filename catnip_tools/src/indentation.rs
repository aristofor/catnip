// FILE: catnip_tools/src/indentation.rs
//! Auto-indentation logic for interactive input.
//!
//! Computes the expected indent level based on bracket nesting,
//! skipping strings and comments. Shared by REPL and any future editor.

/// Compute the number of spaces for the next line after `text`.
///
/// Scans the full text, tracking `{}` and `()[]` depth while
/// skipping string literals and `#` comments. Returns
/// `(brace_level + paren_depth) * indent_size`.
pub fn compute_next_indent(text: &str, indent_size: usize) -> usize {
    let mut brace_level: i32 = 0;
    let mut paren_depth: i32 = 0;

    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // Triple-quoted strings
        if (b == b'"' || b == b'\'') && i + 2 < len && bytes[i + 1] == b && bytes[i + 2] == b {
            let quote = b;
            i += 3;
            while i + 2 < len {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote && bytes[i + 1] == quote && bytes[i + 2] == quote {
                    i += 3;
                    break;
                }
                i += 1;
            }
            // If we ran out of text inside a triple-quote, skip remaining
            if i + 2 >= len && !(i >= 3 && bytes[i - 1] == quote && bytes[i - 2] == quote && bytes[i - 3] == quote) {
                break;
            }
            continue;
        }

        // Single-quoted strings
        if b == b'"' || b == b'\'' {
            let quote = b;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        // Comments: skip until newline
        if b == b'#' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        match b {
            b'{' => brace_level += 1,
            b'}' => brace_level -= 1,
            b'(' | b'[' => paren_depth += 1,
            b')' | b']' => paren_depth -= 1,
            _ => {}
        }

        i += 1;
    }

    let level = brace_level.max(0) + paren_depth.max(0);
    (level as usize) * indent_size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_indent() {
        assert_eq!(compute_next_indent("x = 42", 4), 0);
    }

    #[test]
    fn test_open_brace() {
        assert_eq!(compute_next_indent("if True {", 4), 4);
    }

    #[test]
    fn test_nested_braces() {
        assert_eq!(compute_next_indent("if True {\n  if False {", 4), 8);
    }

    #[test]
    fn test_closed_brace() {
        assert_eq!(compute_next_indent("if True {\n  x\n}", 4), 0);
    }

    #[test]
    fn test_open_paren() {
        assert_eq!(compute_next_indent("f(1,", 4), 4);
    }

    #[test]
    fn test_open_bracket() {
        assert_eq!(compute_next_indent("x = [1, 2,", 4), 4);
    }

    #[test]
    fn test_mixed_brace_and_paren() {
        assert_eq!(compute_next_indent("if f(", 4), 4);
        assert_eq!(compute_next_indent("if f() {", 4), 4);
    }

    #[test]
    fn test_string_ignored() {
        assert_eq!(compute_next_indent("x = \"{\"", 4), 0);
        assert_eq!(compute_next_indent("x = '('", 4), 0);
    }

    #[test]
    fn test_triple_quote_ignored() {
        assert_eq!(compute_next_indent("x = \"\"\"{\"\"\"", 4), 0);
    }

    #[test]
    fn test_comment_ignored() {
        assert_eq!(compute_next_indent("x = 1 # {", 4), 0);
    }

    #[test]
    fn test_escaped_quote_in_string() {
        assert_eq!(compute_next_indent("x = \"\\\"{}\"", 4), 0);
    }

    #[test]
    fn test_empty_input() {
        assert_eq!(compute_next_indent("", 4), 0);
    }

    #[test]
    fn test_indent_size_2() {
        assert_eq!(compute_next_indent("if True {", 2), 2);
    }

    #[test]
    fn test_balanced_then_open() {
        assert_eq!(compute_next_indent("f(x)\nif True {", 4), 4);
    }

    #[test]
    fn test_unclosed_triple_quote() {
        // Inside a triple-quoted string, brackets don't count
        assert_eq!(compute_next_indent("x = \"\"\"hello {", 4), 0);
    }
}
