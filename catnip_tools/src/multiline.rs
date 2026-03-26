// FILE: catnip_tools/src/multiline.rs
//! Multiline detection and preprocessing for interactive input.
//!
//! Pure logic: no I/O, no PyO3. Shared by REPL, debug console, and PyO3 shims.
//! Operator/keyword sets come from `catnip_grammar::symbols` (extracted from
//! grammar.json) so they stay in sync with the parser.

/// Check whether the accumulated text needs more lines.
///
/// Returns true if brackets are unbalanced or the last token is a
/// continuation operator/keyword.
pub fn should_continue_multiline(text: &str) -> bool {
    let stripped = text.trim_end();

    let brace_count = text.matches('{').count() as i32 - text.matches('}').count() as i32;
    let paren_count = text.matches('(').count() as i32 - text.matches(')').count() as i32;
    let bracket_count = text.matches('[').count() as i32 - text.matches(']').count() as i32;

    if brace_count > 0 || paren_count > 0 || bracket_count > 0 {
        return true;
    }

    if has_continuation_op(stripped) {
        return true;
    }

    if let Some(last_word) = stripped.split_whitespace().last() {
        if catnip_grammar::symbols::is_block_starter_keyword(last_word) {
            return true;
        }
    }

    false
}

/// Check whether a single line ends with a continuation operator.
pub fn has_continuation_op(line: &str) -> bool {
    let stripped = line.trim_end();
    stripped.ends_with(',')
        || catnip_grammar::symbols::operators()
            .iter()
            .any(|op| stripped.ends_with(op.as_str()))
}

/// Join continuation lines that end with an operator or start with a postfix operator.
///
/// Lines ending with a continuation operator are merged with the next
/// line(s) into a single logical line. Lines starting with a postfix
/// operator (`.` for broadcast, field access, method calls) are merged
/// with the previous line.
pub fn preprocess_multiline(code: &str) -> String {
    let lines: Vec<&str> = code.split('\n').collect();
    let n_lines = lines.len();

    if n_lines == 1 {
        return code.to_string();
    }

    // Pass 1: join lines ending with continuation operator
    let mut pass1 = Vec::new();
    let mut i = 0;

    while i < n_lines {
        let line = lines[i];
        let stripped = line.trim_end();

        if has_continuation_op(stripped) {
            let mut accumulated = vec![stripped.to_string()];
            let mut j = i + 1;

            while j < n_lines {
                let next_line = lines[j].trim_start();
                accumulated.push(next_line.to_string());

                if has_continuation_op(next_line) {
                    j += 1;
                } else {
                    j += 1;
                    break;
                }
            }

            pass1.push(accumulated.join(" "));
            i = j;
        } else {
            pass1.push(line.to_string());
            i += 1;
        }
    }

    // Pass 2: join lines starting with postfix operator (. for broadcast/method/field)
    let mut result: Vec<String> = Vec::new();
    for line in pass1 {
        if starts_with_postfix_op(&line) && !result.is_empty() {
            let prev = result.pop().unwrap();
            result.push(format!("{}{}", prev.trim_end(), line.trim_start()));
        } else {
            result.push(line);
        }
    }

    result.join("\n")
}

/// Check whether a line starts with a postfix operator (continuation from previous line).
///
/// Matches `.identifier`, `.[broadcast]`, `.(call)` but not `.5` (float literal).
fn starts_with_postfix_op(line: &str) -> bool {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('.') || trimmed.len() < 2 {
        return false;
    }
    let next = trimmed.as_bytes()[1];
    next == b'[' || next == b'(' || next.is_ascii_alphabetic() || next == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_line_no_continuation() {
        assert!(!should_continue_multiline("x = 42"));
    }

    #[test]
    fn test_trailing_operator() {
        assert!(should_continue_multiline("x +"));
        assert!(should_continue_multiline("x ="));
        assert!(should_continue_multiline("a,"));
    }

    #[test]
    fn test_unbalanced_braces() {
        assert!(should_continue_multiline("if x {"));
        assert!(!should_continue_multiline("if x { 1 }"));
    }

    #[test]
    fn test_unbalanced_parens() {
        assert!(should_continue_multiline("f(x,"));
        assert!(!should_continue_multiline("f(x)"));
    }

    #[test]
    fn test_continuation_keyword() {
        assert!(should_continue_multiline("} else"));
        assert!(should_continue_multiline("x = if"));
    }

    #[test]
    fn test_has_continuation_op() {
        assert!(has_continuation_op("x +"));
        assert!(has_continuation_op("a,"));
        assert!(!has_continuation_op("x + 1"));
    }

    #[test]
    fn test_preprocess_single_line() {
        assert_eq!(preprocess_multiline("x = 42"), "x = 42");
    }

    #[test]
    fn test_preprocess_joins_continuation() {
        assert_eq!(preprocess_multiline("x +\n1"), "x + 1");
    }

    #[test]
    fn test_preprocess_chain() {
        assert_eq!(preprocess_multiline("a +\nb +\nc"), "a + b + c");
    }

    #[test]
    fn test_preprocess_mixed() {
        let input = "x = 1\ny +\nz\nw = 2";
        let expected = "x = 1\ny + z\nw = 2";
        assert_eq!(preprocess_multiline(input), expected);
    }

    #[test]
    fn test_preprocess_postfix_broadcast() {
        let input = "numbers\n.[if > 0]";
        assert_eq!(preprocess_multiline(input), "numbers.[if > 0]");
    }

    #[test]
    fn test_preprocess_postfix_chain() {
        let input = "list(1, 2, 3)\n.[~>(x) => { x * 2 }]\n.[if > 3]";
        assert_eq!(
            preprocess_multiline(input),
            "list(1, 2, 3).[~>(x) => { x * 2 }].[if > 3]"
        );
    }

    #[test]
    fn test_preprocess_postfix_method() {
        let input = "obj\n.method()\n.field";
        assert_eq!(preprocess_multiline(input), "obj.method().field");
    }

    #[test]
    fn test_preprocess_postfix_not_float() {
        // .5 is a float literal, not a postfix op
        let input = "x = 1\n.5";
        assert_eq!(preprocess_multiline(input), "x = 1\n.5");
    }

    #[test]
    fn test_preprocess_postfix_with_assignment() {
        let input = "catnip = list(\"data\", \"rules\", \"noise\")\n.[if != \"noise\"]";
        assert_eq!(
            preprocess_multiline(input),
            "catnip = list(\"data\", \"rules\", \"noise\").[if != \"noise\"]"
        );
    }

    #[test]
    fn test_starts_with_postfix_op() {
        assert!(starts_with_postfix_op(".[if > 0]"));
        assert!(starts_with_postfix_op(".method()"));
        assert!(starts_with_postfix_op(".field"));
        assert!(starts_with_postfix_op("  .[broadcast]"));
        assert!(starts_with_postfix_op("._private"));
        assert!(starts_with_postfix_op(".(call)"));
        assert!(!starts_with_postfix_op(".5"));
        assert!(!starts_with_postfix_op("."));
        assert!(!starts_with_postfix_op("x.method()"));
        assert!(!starts_with_postfix_op("normal line"));
    }

    #[test]
    fn test_pasted_multiline_block() {
        let text = "# comment\nnumbers = list(10, 20, 35, 40, 55, 60)\nresult = None\n\nfor num in numbers {\n    if num > 50 {\n        result = num\n        break\n    }\n}\n\nresult";
        assert!(!should_continue_multiline(text));
    }
}
