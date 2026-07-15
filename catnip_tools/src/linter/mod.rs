// FILE: catnip_tools/src/linter.rs
use crate::config::{FormatConfig, LintConfig};
use crate::errors::{AT_LINE_FRAGMENT, COLUMN_FRAGMENT, find_errors};
use crate::formatter::format_code;
use catnip_grammar::node_kinds as NK;
use catnip_grammar::symbols;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use tree_sitter::Node;

const PARSE_FAILED_MESSAGE: &str = "Parse failed";

mod improvements;
mod semantic;
use improvements::*;
use semantic::*;

// --- Types ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Severity {
    Error = 0,
    Warning = 1,
    Info = 2,
    Hint = 3,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub severity: Severity,
    pub line: usize,
    pub column: usize,
    pub end_line: Option<usize>,
    pub end_column: Option<usize>,
    pub source_line: Option<String>,
    pub suggestion: Option<String>,
}

// --- Main entry point ---

pub fn lint_code(source: &str, config: &LintConfig) -> Result<Vec<Diagnostic>, String> {
    let source_lines: Vec<&str> = source.lines().collect();
    let mut diagnostics = Vec::new();

    // Phase 1: Syntax
    let tree = if config.check_syntax {
        match parse_and_check_syntax(source, &source_lines) {
            Ok(tree) => Some(tree),
            Err(diags) => {
                return Ok(diags);
            }
        }
    } else {
        parse_silent(source)
    };

    // Phase 2: Style
    if config.check_style {
        check_style(source, &source_lines, &mut diagnostics);
    }

    // Phase 3: Semantic
    if config.check_semantic {
        if let Some(ref tree) = tree {
            check_semantic(
                tree.root_node(),
                source,
                &source_lines,
                config.check_names,
                &mut diagnostics,
            );
        }
    }

    // Phase 4: Improvement suggestions
    if config.check_semantic {
        if let Some(ref tree) = tree {
            check_improvements(tree.root_node(), source, &source_lines, config, &mut diagnostics);
        }
    }

    // Phase 5: Deep analysis (CFG-based)
    if config.check_ir {
        if let Some(ref tree) = tree {
            let deep_diags = crate::lint_cfg::check_deep(tree.root_node(), source, config);
            diagnostics.extend(deep_diags);
        }
    }

    // Filter noqa-suppressed diagnostics using tree-sitter comment nodes
    if let Some(ref tree) = tree {
        let noqa = collect_noqa_directives(tree.root_node(), source);
        diagnostics.retain(|d| match noqa.get(&d.line) {
            Some(NoqaDirective::All) => false,
            Some(NoqaDirective::Codes(codes)) => !codes.iter().any(|c| c == &d.code),
            None => true,
        });
    }

    // Filter globally disabled codes (config-level on/off, applied like noqa)
    if !config.disabled_codes.is_empty() {
        diagnostics.retain(|d| !config.disabled_codes.contains(&d.code));
    }

    diagnostics.sort_by(|a, b| a.line.cmp(&b.line).then(a.column.cmp(&b.column)));

    Ok(diagnostics)
}

// --- noqa suppression ---

/// Collect noqa suppressions: line -> set of suppressed codes (empty = suppress all).
/// Public API for callers that append diagnostics after lint_code().
pub fn collect_noqa(source: &str) -> HashMap<usize, HashSet<String>> {
    let tree = match parse_silent(source) {
        Some(t) => t,
        None => return HashMap::new(),
    };
    let directives = collect_noqa_directives(tree.root_node(), source);
    let mut result = HashMap::new();
    for (line, directive) in directives {
        match directive {
            NoqaDirective::All => {
                result.insert(line, HashSet::new()); // empty = all
            }
            NoqaDirective::Codes(codes) => {
                result.insert(line, codes.into_iter().collect());
            }
        }
    }
    result
}

enum NoqaDirective {
    All,
    Codes(Vec<String>),
}

/// Walk tree-sitter COMMENT nodes to find `# noqa` directives.
/// Only matches real comments, not `"# noqa"` inside strings.
fn collect_noqa_directives(root: Node, source: &str) -> HashMap<usize, NoqaDirective> {
    let mut map = HashMap::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == NK::COMMENT {
            if let Some(directive) = parse_noqa_comment(&source[node.byte_range()]) {
                map.insert(node.start_position().row + 1, directive);
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    map
}

/// Parse a comment node's text for a noqa directive.
///
/// Supported forms:
/// - `# noqa` -- suppress all
/// - `# noqa: W200` -- suppress W200 only
/// - `# noqa: W200, I100` -- suppress multiple codes
/// - `# noqa: W200 -- reason` -- suppress W200 (trailing text ignored)
/// - `# noqa -- reason` -- suppress all
///
/// Rejects non-directives like `# noqa123`.
fn parse_noqa_comment(comment: &str) -> Option<NoqaDirective> {
    let text = comment.strip_prefix('#')?.trim_start();
    if !text.starts_with("noqa") {
        return None;
    }
    let rest = &text[4..]; // skip "noqa"
    if rest.is_empty() {
        return Some(NoqaDirective::All);
    }
    // "noqa" must be followed by a word boundary, not "noqa123"
    if rest.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let rest = rest.trim_start();
    if let Some(codes_str) = rest.strip_prefix(':') {
        // "# noqa: W200, I100 -- reason"
        // Take first word of each comma-separated chunk
        let codes: Vec<String> = codes_str
            .split(',')
            .filter_map(|chunk| {
                let word = chunk.split_whitespace().next()?;
                if word.is_empty() || word.starts_with('-') {
                    None
                } else {
                    Some(word.to_string())
                }
            })
            .collect();
        if codes.is_empty() {
            Some(NoqaDirective::All)
        } else {
            Some(NoqaDirective::Codes(codes))
        }
    } else {
        Some(NoqaDirective::All)
    }
}

// --- Phase 1: Syntax ---

fn parse_silent(source: &str) -> Option<tree_sitter::Tree> {
    let language = crate::get_language();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    parser.parse(source, None)
}

fn parse_and_check_syntax(source: &str, source_lines: &[&str]) -> Result<tree_sitter::Tree, Vec<Diagnostic>> {
    let language = crate::get_language();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).map_err(|e| {
        vec![make_diagnostic(
            "E100",
            &e.to_string(),
            Severity::Error,
            1,
            1,
            None,
            None,
        )]
    })?;

    let tree = parser.parse(source, None).ok_or_else(|| {
        vec![make_diagnostic(
            "E100",
            PARSE_FAILED_MESSAGE,
            Severity::Error,
            1,
            1,
            None,
            None,
        )]
    })?;

    if let Some(error_msg) = find_errors(tree.root_node(), source) {
        let (line, col) = extract_line_col(&error_msg);
        let src_line = if line > 0 && line <= source_lines.len() {
            Some(source_lines[line - 1].to_string())
        } else {
            None
        };
        return Err(vec![make_diagnostic(
            "E100",
            &error_msg,
            Severity::Error,
            line,
            col,
            src_line,
            None,
        )]);
    }

    Ok(tree)
}

fn extract_line_col(msg: &str) -> (usize, usize) {
    if let Some(at_pos) = msg.find(AT_LINE_FRAGMENT) {
        let rest = &msg[at_pos + AT_LINE_FRAGMENT.len()..];
        let parts: Vec<&str> = rest.splitn(2, COLUMN_FRAGMENT).collect();
        if parts.len() == 2 {
            let line = parts[0].parse::<usize>().unwrap_or(1);
            let col = parts[1]
                .trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse::<usize>()
                .unwrap_or(1);
            return (line, col);
        }
    }
    (1, 1)
}

// --- Phase 2: Style ---

fn check_style(source: &str, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    // Compare with formatted code
    let format_config = FormatConfig::default();
    if let Ok(formatted) = format_code(source, &format_config) {
        if formatted != source {
            let formatted_lines: Vec<&str> = formatted.lines().collect();
            for (i, (orig, fmt)) in source_lines.iter().zip(formatted_lines.iter()).enumerate() {
                if orig != fmt {
                    diagnostics.push(make_diagnostic(
                        "W100",
                        "Line differs from formatted version",
                        Severity::Warning,
                        i + 1,
                        1,
                        Some(orig.to_string()),
                        Some(fmt.to_string()),
                    ));
                }
            }

            if source_lines.len() != formatted_lines.len() {
                diagnostics.push(make_diagnostic(
                    "W102",
                    &format!("Expected {} lines, got {}", formatted_lines.len(), source_lines.len()),
                    Severity::Info,
                    source_lines.len(),
                    1,
                    None,
                    None,
                ));
            }
        }
    }

    // Trailing whitespace
    for (i, line) in source_lines.iter().enumerate() {
        let trimmed = line.trim_end();
        if trimmed.len() < line.len() {
            diagnostics.push(make_diagnostic(
                "W101",
                "Trailing whitespace",
                Severity::Warning,
                i + 1,
                trimmed.len() + 1,
                Some(line.to_string()),
                Some(trimmed.to_string()),
            ));
        }
    }
}

// --- Helpers ---

fn node_text(node: Node, source: &str) -> String {
    node.utf8_text(source.as_bytes()).unwrap_or("").to_string()
}

fn make_diagnostic(
    code: &str,
    message: &str,
    severity: Severity,
    line: usize,
    column: usize,
    source_line: Option<String>,
    suggestion: Option<String>,
) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        message: message.to_string(),
        severity,
        line,
        column,
        end_line: None,
        end_column: None,
        source_line,
        suggestion,
    }
}

#[cfg(test)]
mod tests;
