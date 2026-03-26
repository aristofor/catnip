// FILE: catnip_tools/src/errors.rs
use crate::suggest::{format_suggestion, suggest_similar};
use catnip_grammar::symbols;
use tree_sitter::Node;

pub const UNEXPECTED_TOKEN_PREFIX: &str = "Unexpected token";
pub const SYNTAX_ERROR_PREFIX: &str = "Syntax error";
pub const EXPECTED_PREFIX: &str = "Expected";
pub const AT_LINE_FRAGMENT: &str = "at line ";
pub const COLUMN_FRAGMENT: &str = ", column ";

/// Cross-language keyword mappings (common mistakes from other languages).
const CROSS_LANG_ALIASES: &[(&str, &str)] = &[
    ("class", "struct"),
    ("switch", "match"),
    ("def", "fn"),
    ("function", "fn"),
    ("func", "fn"),
    ("elif", "elif"),
    ("elseif", "elif"),
    ("else if", "elif"),
    ("None", "nil"),
    ("null", "nil"),
    ("undefined", "nil"),
    ("True", "true"),
    ("False", "false"),
    ("let", "="),
    ("var", "="),
    ("const", "="),
];

/// Try to suggest a keyword for an unexpected token.
fn suggest_keyword(token: &str) -> Option<String> {
    // First check cross-language aliases (exact match)
    let token_trimmed = token.trim();
    for &(alias, replacement) in CROSS_LANG_ALIASES {
        if token_trimmed == alias {
            if replacement == "fn" {
                return Some("Catnip uses `(args) => { }` for function definitions".to_string());
            }
            if replacement == "=" {
                return Some(format!(
                    "Catnip uses `name = value` for variable binding (no `{token_trimmed}` keyword)"
                ));
            }
            return Some(format!("Did you mean '{replacement}'?"));
        }
    }

    // Then try fuzzy matching against grammar keywords
    let keyword_candidates: Vec<&str> = symbols::keywords().iter().map(String::as_str).collect();
    let suggestions = suggest_similar(token_trimmed, &keyword_candidates, 1, 0.6);
    format_suggestion(&suggestions)
}

fn format_location(line: usize, col: usize) -> String {
    format!("{AT_LINE_FRAGMENT}{line}{COLUMN_FRAGMENT}{col}")
}

fn format_unexpected_token(snippet: &str, line: usize, col: usize) -> String {
    format!("{UNEXPECTED_TOKEN_PREFIX} '{snippet}' {}", format_location(line, col))
}

fn format_syntax_error(line: usize, col: usize) -> String {
    format!("{SYNTAX_ERROR_PREFIX} {}", format_location(line, col))
}

fn format_expected(expected: &str, line: usize, col: usize) -> String {
    format!("{EXPECTED_PREFIX} {expected} {}", format_location(line, col))
}

/// Find syntax errors in the parse tree
pub fn find_errors(node: Node, source: &str) -> Option<String> {
    if node.kind() == "ERROR" {
        let text = &source[node.byte_range()];
        let (line, col) = (node.start_position().row + 1, node.start_position().column + 1);

        if !text.trim().is_empty() {
            let snippet = if text.len() > 20 { &text[..20] } else { text };
            // Extract the first word for keyword matching
            let first_word = snippet.split_whitespace().next().unwrap_or(snippet);
            let hint = suggest_keyword(first_word);
            return match hint {
                Some(h) => Some(format!("{}. {}", format_unexpected_token(snippet, line, col), h)),
                None => Some(format_unexpected_token(snippet, line, col)),
            };
        } else {
            return Some(format_syntax_error(line, col));
        }
    }

    if node.is_missing() {
        let (line, col) = (node.start_position().row + 1, node.start_position().column + 1);
        let expected = node.kind().replace('_', " ");
        return Some(format_expected(&expected, line, col));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(error) = find_errors(child, source) {
            return Some(error);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suggest_keyword_class() {
        let hint = suggest_keyword("class");
        assert_eq!(hint, Some("Did you mean 'struct'?".to_string()));
    }

    #[test]
    fn test_suggest_keyword_switch() {
        let hint = suggest_keyword("switch");
        assert_eq!(hint, Some("Did you mean 'match'?".to_string()));
    }

    #[test]
    fn test_suggest_keyword_def() {
        let hint = suggest_keyword("def");
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("(args) => { }"));
    }

    #[test]
    fn test_suggest_keyword_typo() {
        let hint = suggest_keyword("whle");
        assert_eq!(hint, Some("Did you mean 'while'?".to_string()));
    }

    #[test]
    fn test_suggest_keyword_no_match() {
        let hint = suggest_keyword("xyzzy");
        assert!(hint.is_none());
    }
}
