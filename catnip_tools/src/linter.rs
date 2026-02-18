use crate::config::{FormatConfig, LintConfig};
use crate::errors::find_errors;
use crate::formatter::format_code;
use serde::Serialize;
use std::collections::HashSet;
use tree_sitter::Node;

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
            check_semantic(tree.root_node(), source, &source_lines, &mut diagnostics);
        }
    }

    // Phase 4: Improvement suggestions
    if config.check_semantic {
        if let Some(ref tree) = tree {
            check_improvements(tree.root_node(), source, &source_lines, &mut diagnostics);
        }
    }

    diagnostics.sort_by(|a, b| a.line.cmp(&b.line).then(a.column.cmp(&b.column)));

    Ok(diagnostics)
}

// --- Phase 1: Syntax ---

fn parse_silent(source: &str) -> Option<tree_sitter::Tree> {
    let language = crate::get_language();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    parser.parse(source, None)
}

fn parse_and_check_syntax(
    source: &str,
    source_lines: &[&str],
) -> Result<tree_sitter::Tree, Vec<Diagnostic>> {
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
            "Parse failed",
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
    if let Some(at_pos) = msg.find("at line ") {
        let rest = &msg[at_pos + 8..];
        let parts: Vec<&str> = rest.splitn(2, ", column ").collect();
        if parts.len() == 2 {
            let line = parts[0].parse::<usize>().unwrap_or(1);
            let col = parts[1]
                .trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse::<usize>()
                .unwrap_or(1);
            return (line, col);
        }
    }
    if let Some(at_pos) = msg.find("line ") {
        let rest = &msg[at_pos + 5..];
        let parts: Vec<&str> = rest.splitn(2, ", column ").collect();
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
                        "W200",
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
                    "W202",
                    &format!(
                        "Expected {} lines, got {}",
                        formatted_lines.len(),
                        source_lines.len()
                    ),
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
                "W201",
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

// --- Phase 3: Semantic (CST walk) ---

struct DefInfo {
    name: String,
    line: usize,
    column: usize,
}

struct ScopeTracker {
    scopes: Vec<HashSet<String>>,
    definitions: Vec<DefInfo>,
    used: HashSet<String>,
}

impl ScopeTracker {
    fn new() -> Self {
        Self {
            scopes: vec![HashSet::new()],
            definitions: Vec::new(),
            used: HashSet::new(),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }

    fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    fn define(&mut self, name: &str, line: usize, column: usize) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string());
        }
        self.definitions.push(DefInfo {
            name: name.to_string(),
            line,
            column,
        });
    }

    fn define_builtin(&mut self, name: &str) {
        if let Some(scope) = self.scopes.first_mut() {
            scope.insert(name.to_string());
        }
    }

    fn use_name(&mut self, name: &str) {
        self.used.insert(name.to_string());
    }

    fn is_defined(&self, name: &str) -> bool {
        for scope in self.scopes.iter().rev() {
            if scope.contains(name) {
                return true;
            }
        }
        false
    }
}

const BUILTINS: &[&str] = &[
    "print",
    "len",
    "range",
    "str",
    "int",
    "float",
    "bool",
    "list",
    "dict",
    "tuple",
    "set",
    "abs",
    "min",
    "max",
    "sum",
    "sorted",
    "reversed",
    "enumerate",
    "zip",
    "map",
    "filter",
    "any",
    "all",
    "type",
    "isinstance",
    "hasattr",
    "getattr",
    "setattr",
    "None",
    "True",
    "False",
    "_",
    "pragma",
    "round",
    "input",
    "open",
    "breakpoint",
    "repr",
    "ord",
    "chr",
    "hex",
    "bin",
    "oct",
    "id",
    "hash",
    "callable",
    "super",
    "property",
    "staticmethod",
    "classmethod",
    "slice",
    "frozenset",
    "bytes",
    "bytearray",
    "memoryview",
    "complex",
    "divmod",
    "pow",
    "format",
    "vars",
    "dir",
    "globals",
    "locals",
    "exec",
    "eval",
    "compile",
    "__import__",
];

fn check_semantic(
    root: Node,
    source: &str,
    source_lines: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut tracker = ScopeTracker::new();

    for &name in BUILTINS {
        tracker.define_builtin(name);
    }

    walk_node(root, source, &mut tracker, diagnostics, source_lines);

    let builtin_set: HashSet<&str> = BUILTINS.iter().copied().collect();
    for def in &tracker.definitions {
        if builtin_set.contains(def.name.as_str()) {
            continue;
        }
        if def.name.starts_with('_') {
            continue;
        }
        if !tracker.used.contains(&def.name) {
            diagnostics.push(make_diagnostic(
                "W310",
                &format!("Variable '{}' is defined but never used", def.name),
                Severity::Warning,
                def.line,
                def.column,
                source_lines
                    .get(def.line.saturating_sub(1))
                    .map(|s| s.to_string()),
                None,
            ));
        }
    }
}

fn walk_node(
    node: Node,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    let kind = node.kind();

    match kind {
        "source_file" | "block" => {
            walk_children(node, source, tracker, diagnostics, source_lines);
        }

        "assignment" => {
            walk_assignment(node, source, tracker, diagnostics, source_lines);
        }

        "lambda_expr" => {
            walk_lambda(node, source, tracker, diagnostics, source_lines);
        }

        "for_stmt" => {
            walk_for(node, source, tracker, diagnostics, source_lines);
        }

        "match_expr" => {
            walk_match(node, source, tracker, diagnostics, source_lines);
        }

        "if_expr" | "elif_clause" | "else_clause" | "while_stmt" => {
            walk_children(node, source, tracker, diagnostics, source_lines);
        }

        "chained" => {
            walk_chained(node, source, tracker, diagnostics, source_lines);
        }

        "call" => {
            walk_children(node, source, tracker, diagnostics, source_lines);
        }

        "identifier" => {
            let name = node_text(node, source);
            check_reference(&name, node, tracker, diagnostics, source_lines);
        }

        "decorator" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = node_text(child, source);
                    tracker.use_name(&name);
                }
            }
        }

        "return_stmt" => {
            walk_children(node, source, tracker, diagnostics, source_lines);
        }

        "pattern_var" | "pattern_literal" | "pattern_wildcard" | "pattern_or" | "pattern_tuple"
        | "pattern_star" => {}

        _ => {
            walk_children(node, source, tracker, diagnostics, source_lines);
        }
    }
}

fn walk_children(
    node: Node,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, source, tracker, diagnostics, source_lines);
    }
}

fn walk_assignment(
    node: Node,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();

    let mut lvalue_nodes = Vec::new();
    let mut rvalue_node = None;
    let mut decorator_nodes = Vec::new();

    for child in &children {
        match child.kind() {
            "decorator" => decorator_nodes.push(*child),
            "lvalue" | "unpack_target" | "identifier" | "unpack_tuple" | "unpack_sequence"
            | "setattr" => {
                if let Some(prev) = rvalue_node.take() {
                    lvalue_nodes.push(prev);
                }
                rvalue_node = Some(*child);
            }
            "=" => {
                if let Some(prev) = rvalue_node.take() {
                    lvalue_nodes.push(prev);
                }
            }
            _ => {
                if let Some(prev) = rvalue_node.take() {
                    lvalue_nodes.push(prev);
                }
                rvalue_node = Some(*child);
            }
        }
    }

    for dec in &decorator_nodes {
        let mut c = dec.walk();
        for child in dec.children(&mut c) {
            if child.kind() == "identifier" {
                tracker.use_name(&node_text(child, source));
            }
        }
    }

    for lv in &lvalue_nodes {
        define_lvalue(*lv, source, tracker);
    }

    if let Some(rv) = rvalue_node {
        walk_node(rv, source, tracker, diagnostics, source_lines);
    }
}

fn define_lvalue(node: Node, source: &str, tracker: &mut ScopeTracker) {
    let kind = node.kind();
    match kind {
        "identifier" => {
            let name = node_text(node, source);
            let line = node.start_position().row + 1;
            let col = node.start_position().column + 1;
            tracker.define(&name, line, col);
        }
        "lvalue" | "unpack_target" | "unpack_tuple" | "unpack_sequence" | "unpack_items" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                define_lvalue(child, source, tracker);
            }
        }
        "setattr" => {}
        "variadic_param" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = node_text(child, source);
                    let line = child.start_position().row + 1;
                    let col = child.start_position().column + 1;
                    tracker.define(&name, line, col);
                }
            }
        }
        _ => {}
    }
}

fn walk_lambda(
    node: Node,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    tracker.push_scope();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "lambda_params" => {
                define_lambda_params(child, source, tracker, diagnostics, source_lines);
            }
            "block" => {
                walk_node(child, source, tracker, diagnostics, source_lines);
            }
            _ => {}
        }
    }

    tracker.pop_scope();
}

fn define_lambda_params(
    node: Node,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "lambda_param" => {
                let mut pc = child.walk();
                let mut saw_name = false;
                for param_child in child.children(&mut pc) {
                    if param_child.kind() == "identifier" && !saw_name {
                        let name = node_text(param_child, source);
                        let line = param_child.start_position().row + 1;
                        let col = param_child.start_position().column + 1;
                        tracker.define(&name, line, col);
                        saw_name = true;
                    } else if saw_name && param_child.kind() != "=" {
                        walk_node(param_child, source, tracker, diagnostics, source_lines);
                    }
                }
            }
            "variadic_param" => {
                let mut pc = child.walk();
                for param_child in child.children(&mut pc) {
                    if param_child.kind() == "identifier" {
                        let name = node_text(param_child, source);
                        let line = param_child.start_position().row + 1;
                        let col = param_child.start_position().column + 1;
                        tracker.define(&name, line, col);
                    }
                }
            }
            _ => {}
        }
    }
}

fn walk_for(
    node: Node,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    tracker.push_scope();

    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();

    let mut defined_target = false;
    let mut past_in = false;

    for child in &children {
        match child.kind() {
            "unpack_target" | "identifier" if !defined_target => {
                define_lvalue(*child, source, tracker);
                defined_target = true;
            }
            "in" => {
                past_in = true;
            }
            "block" => {
                walk_node(*child, source, tracker, diagnostics, source_lines);
            }
            _ if past_in && child.kind() != "block" => {
                walk_node(*child, source, tracker, diagnostics, source_lines);
            }
            _ => {}
        }
    }

    tracker.pop_scope();
}

fn walk_match(
    node: Node,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "match_case" => {
                walk_match_case(child, source, tracker, diagnostics, source_lines);
            }
            "{" | "}" | "match" => {}
            _ => {
                walk_node(child, source, tracker, diagnostics, source_lines);
            }
        }
    }
}

fn walk_match_case(
    node: Node,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    tracker.push_scope();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "pattern" | "pattern_or" => {
                define_pattern_vars(child, source, tracker);
            }
            "block" => {
                walk_node(child, source, tracker, diagnostics, source_lines);
            }
            _ => {
                if child.kind() != "=>" && child.kind() != "if" {
                    walk_node(child, source, tracker, diagnostics, source_lines);
                }
            }
        }
    }

    tracker.pop_scope();
}

fn define_pattern_vars(node: Node, source: &str, tracker: &mut ScopeTracker) {
    let kind = node.kind();
    match kind {
        "pattern_var" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = node_text(child, source);
                    if name != "_" {
                        let line = child.start_position().row + 1;
                        let col = child.start_position().column + 1;
                        tracker.define(&name, line, col);
                    }
                }
            }
        }
        "pattern_wildcard" | "pattern_literal" => {}
        "pattern_star" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = node_text(child, source);
                    let line = child.start_position().row + 1;
                    let col = child.start_position().column + 1;
                    tracker.define(&name, line, col);
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                define_pattern_vars(child, source, tracker);
            }
        }
    }
}

fn walk_chained(
    node: Node,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "getattr" | "callattr" | "setattr" => {
                walk_member_args(child, source, tracker, diagnostics, source_lines);
            }
            "index" => {
                walk_children(child, source, tracker, diagnostics, source_lines);
            }
            _ => {
                walk_node(child, source, tracker, diagnostics, source_lines);
            }
        }
    }
}

fn walk_member_args(
    node: Node,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();

    for child in &children {
        match child.kind() {
            "identifier" => {}
            "arguments" => {
                walk_children(*child, source, tracker, diagnostics, source_lines);
            }
            _ => {}
        }
    }
}

fn check_reference(
    name: &str,
    node: Node,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    tracker.use_name(name);

    if !tracker.is_defined(name) {
        let line = node.start_position().row + 1;
        let col = node.start_position().column + 1;
        diagnostics.push(make_diagnostic(
            "E300",
            &format!("Name '{}' is not defined", name),
            Severity::Error,
            line,
            col,
            source_lines
                .get(line.saturating_sub(1))
                .map(|s| s.to_string()),
            None,
        ));
    }
}

// --- Phase 4: Improvement suggestions ---

fn check_improvements(
    root: Node,
    source: &str,
    source_lines: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    check_tco_opportunities(root, source, source_lines, diagnostics);
    check_redundant_boolean(root, source, source_lines, diagnostics);
    check_self_assignment(root, source, source_lines, diagnostics);
}

/// I100: Detect recursive calls not in tail position
fn check_tco_opportunities(
    root: Node,
    source: &str,
    source_lines: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    find_named_lambdas(root, source, source_lines, diagnostics);
}

fn find_named_lambdas(
    node: Node,
    source: &str,
    source_lines: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    if node.kind() == "assignment" {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();

        // Extract function name (may be inside lvalue/unpack_target) and lambda body
        let mut name: Option<String> = None;
        let mut lambda_node: Option<Node> = None;

        for child in &children {
            match child.kind() {
                "identifier" if name.is_none() => {
                    name = Some(node_text(*child, source));
                }
                "lvalue" | "unpack_target" if name.is_none() => {
                    // Dig into lvalue to find a simple identifier
                    if let Some(ident) = extract_single_identifier(*child, source) {
                        name = Some(ident);
                    }
                }
                "lambda_expr" => {
                    lambda_node = Some(*child);
                }
                _ => {}
            }
        }

        if let (Some(fn_name), Some(lambda)) = (name, lambda_node) {
            if let Some(body) = find_lambda_body(lambda) {
                find_non_tail_calls(&fn_name, body, lambda, source, source_lines, diagnostics);
            }
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_named_lambdas(child, source, source_lines, diagnostics);
    }
}

/// Extract a simple identifier from a potentially nested lvalue/unpack_target node
fn extract_single_identifier(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" => Some(node_text(node, source)),
        "lvalue" | "unpack_target" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(name) = extract_single_identifier(child, source) {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_lambda_body(lambda: Node) -> Option<Node> {
    let mut cursor = lambda.walk();
    for child in lambda.children(&mut cursor) {
        if child.kind() == "block" {
            return Some(child);
        }
    }
    None
}

/// Walk the lambda body looking for recursive calls, check tail position
fn find_non_tail_calls(
    fn_name: &str,
    body: Node,
    lambda: Node,
    source: &str,
    source_lines: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut stack = vec![body];
    while let Some(current) = stack.pop() {
        if current.kind() == "call" {
            // Check if callee is our function name
            let mut cursor = current.walk();
            let first_child = current.children(&mut cursor).next();
            if let Some(callee) = first_child {
                if callee.kind() == "identifier" && node_text(callee, source) == fn_name {
                    // Found a recursive call - check if it's in tail position
                    if !is_in_tail_position(current, lambda) {
                        let line = current.start_position().row + 1;
                        let col = current.start_position().column + 1;
                        diagnostics.push(make_diagnostic(
                            "I100",
                            &format!(
                                "Recursive call to '{}' is not in tail position — consider restructuring for TCO",
                                fn_name
                            ),
                            Severity::Hint,
                            line,
                            col,
                            source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                            None,
                        ));
                    }
                    continue; // Don't recurse into this call's children
                }
            }
        }

        // Don't descend into nested lambdas (different scope)
        if current.kind() == "lambda_expr" && current.id() != lambda.id() {
            continue;
        }

        // Recurse into children
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

/// Check if a node is in tail position relative to the lambda body.
/// Walks up the parent chain: block (last child), if/elif/else/match branches,
/// return_stmt are transparent. Anything else breaks tail position.
fn is_in_tail_position(node: Node, lambda: Node) -> bool {
    let mut current = node;

    loop {
        let parent = match current.parent() {
            Some(p) => p,
            None => return false,
        };

        // Reached the lambda node itself - we're in tail position
        if parent.id() == lambda.id() {
            return true;
        }

        match parent.kind() {
            "block" => {
                // Must be the last significant child of the block
                if !is_last_significant_child(current, parent) {
                    return false;
                }
                current = parent;
            }
            "statement" => {
                // Statement wraps an expression, transparent for tail position
                current = parent;
            }
            "if_expr" | "elif_clause" | "else_clause" | "match_case" | "match_expr" => {
                current = parent;
            }
            "return_stmt" => {
                current = parent;
            }
            // Any expression wrapping (arithmetic, comparison, etc.) breaks tail position
            _ => return false,
        }
    }
}

/// Check if `child` is the last named (non-anonymous) child of `parent`
fn is_last_significant_child(child: Node, parent: Node) -> bool {
    let mut cursor = parent.walk();
    let mut last_significant = None;
    for c in parent.children(&mut cursor) {
        // Skip punctuation and anonymous nodes
        if c.is_named() {
            last_significant = Some(c.id());
        }
    }
    last_significant == Some(child.id())
}

/// Check if a node is a boolean literal (true/false), handling `literal` wrapper
fn is_bool_literal(node: Node) -> Option<bool> {
    match node.kind() {
        "true" => Some(true),
        "false" => Some(false),
        "literal" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "true" {
                    return Some(true);
                }
                if child.kind() == "false" {
                    return Some(false);
                }
            }
            None
        }
        _ => None,
    }
}

/// I101: Detect redundant comparisons with boolean literals
fn check_redundant_boolean(
    root: Node,
    source: &str,
    source_lines: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == "comparison" {
            check_comparison_with_bool(current, source, source_lines, diagnostics);
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn check_comparison_with_bool(
    node: Node,
    source: &str,
    source_lines: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();

    // comparison: operand comp_op operand [comp_op operand ...]
    // We check simple cases: exactly 3 named children (left, op, right)
    let named: Vec<Node> = children.iter().copied().filter(|c| c.is_named()).collect();
    if named.len() != 3 {
        return;
    }

    let left = named[0];
    let op_node = named[1];
    let right = named[2];

    if op_node.kind() != "comp_op" {
        return;
    }

    let op = node_text(op_node, source);
    if op != "==" && op != "!=" {
        return;
    }

    let left_bool = is_bool_literal(left);
    let right_bool = is_bool_literal(right);

    let (is_bool, bool_val, other_text) = if let Some(val) = left_bool {
        (true, val, node_text(right, source))
    } else if let Some(val) = right_bool {
        (true, val, node_text(left, source))
    } else {
        (false, false, String::new())
    };

    if !is_bool {
        return;
    }

    let suggestion = match (op.as_str(), bool_val) {
        ("==", true) => other_text.clone(),             // x == True → x
        ("==", false) => format!("not {}", other_text), // x == False → not x
        ("!=", true) => format!("not {}", other_text),  // x != True → not x
        ("!=", false) => other_text.clone(),            // x != False → x
        _ => return,
    };

    let line = node.start_position().row + 1;
    let col = node.start_position().column + 1;
    diagnostics.push(make_diagnostic(
        "I101",
        "Redundant comparison with boolean literal",
        Severity::Hint,
        line,
        col,
        source_lines
            .get(line.saturating_sub(1))
            .map(|s| s.to_string()),
        Some(suggestion),
    ));
}

/// I102: Detect self-assignment (`x = x`)
fn check_self_assignment(
    root: Node,
    source: &str,
    source_lines: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == "assignment" {
            check_assignment_self(current, source, source_lines, diagnostics);
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn check_assignment_self(
    node: Node,
    source: &str,
    source_lines: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();

    // CST: assignment → lvalue "=" expression
    // For self-assignment: lvalue contains a single identifier, rvalue is the same identifier
    let mut lvalue_name: Option<String> = None;
    let mut rvalue_name: Option<String> = None;
    let mut eq_count = 0;
    let mut has_decorator = false;

    for child in &children {
        match child.kind() {
            "lvalue" | "unpack_target" => {
                if lvalue_name.is_none() {
                    lvalue_name = extract_single_identifier(*child, source);
                }
            }
            "identifier" => {
                // Could be lvalue (direct) or rvalue
                if eq_count == 0 && lvalue_name.is_none() {
                    lvalue_name = Some(node_text(*child, source));
                } else if eq_count > 0 {
                    rvalue_name = Some(node_text(*child, source));
                }
            }
            "=" => eq_count += 1,
            "decorator" => has_decorator = true,
            _ => {
                // Complex rvalue → not a simple self-assignment
                if eq_count > 0 {
                    return;
                }
            }
        }
    }

    if has_decorator || eq_count != 1 {
        return;
    }

    if let (Some(lhs), Some(rhs)) = (lvalue_name, rvalue_name) {
        if lhs == rhs {
            let line = node.start_position().row + 1;
            let col = node.start_position().column + 1;
            diagnostics.push(make_diagnostic(
                "I102",
                "Self-assignment has no effect",
                Severity::Hint,
                line,
                col,
                source_lines
                    .get(line.saturating_sub(1))
                    .map(|s| s.to_string()),
                Some("remove".to_string()),
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
mod tests {
    use super::*;

    #[test]
    fn test_extract_line_col() {
        assert_eq!(
            extract_line_col("Unexpected token 'x' at line 3, column 5"),
            (3, 5)
        );
        assert_eq!(
            extract_line_col("Expected expression at line 1, column 10"),
            (1, 10)
        );
        assert_eq!(extract_line_col("Some error"), (1, 1));
    }

    #[test]
    fn test_scope_tracker_basic() {
        let mut tracker = ScopeTracker::new();
        tracker.define("x", 1, 1);
        assert!(tracker.is_defined("x"));
        assert!(!tracker.is_defined("y"));
    }

    #[test]
    fn test_scope_tracker_nested() {
        let mut tracker = ScopeTracker::new();
        tracker.define("x", 1, 1);
        tracker.push_scope();
        tracker.define("y", 2, 1);
        assert!(tracker.is_defined("x"));
        assert!(tracker.is_defined("y"));
        tracker.pop_scope();
        assert!(tracker.is_defined("x"));
        assert!(!tracker.is_defined("y"));
    }

    #[test]
    fn test_semantic_simple() {
        let source = "x = 1\ny = x + 1";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, &mut diags);
        let errors: Vec<_> = diags.iter().filter(|d| d.code == "E300").collect();
        assert!(errors.is_empty(), "Unexpected E300: {:?}", errors);
    }

    #[test]
    fn test_semantic_undefined() {
        let source = "y = x + 1";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, &mut diags);
        let errors: Vec<_> = diags.iter().filter(|d| d.code == "E300").collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("'x'"));
    }

    #[test]
    fn test_semantic_unused() {
        let source = "x = 1";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, &mut diags);
        let warnings: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("'x'"));
    }

    #[test]
    fn test_semantic_underscore_ignored() {
        let source = "_x = 1";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, &mut diags);
        let warnings: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_semantic_builtins() {
        let source = "x = len(range(10))";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, &mut diags);
        let errors: Vec<_> = diags.iter().filter(|d| d.code == "E300").collect();
        assert!(errors.is_empty(), "Unexpected E300: {:?}", errors);
    }

    #[test]
    fn test_semantic_lambda_params() {
        let source = "f = (x, y) => { x + y }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, &mut diags);
        let errors: Vec<_> = diags.iter().filter(|d| d.code == "E300").collect();
        assert!(errors.is_empty(), "Unexpected E300: {:?}", errors);
    }

    #[test]
    fn test_semantic_for_loop() {
        let source = "for i in range(10) { print(i) }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, &mut diags);
        let errors: Vec<_> = diags.iter().filter(|d| d.code == "E300").collect();
        assert!(errors.is_empty(), "Unexpected E300: {:?}", errors);
    }

    #[test]
    fn test_lint_full_pipeline() {
        let config = LintConfig::default();
        let result = lint_code("x = 1\nprint(x)", &config).unwrap();
        // Should not have E300 errors
        let errors: Vec<_> = result.iter().filter(|d| d.code == "E300").collect();
        assert!(errors.is_empty());
    }

    // --- I100: TCO detection ---

    #[test]
    fn test_tco_detection() {
        let source = "fact = (n) => { if n <= 1 { 1 } else { n * fact(n - 1) } }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I100").collect();
        assert_eq!(hints.len(), 1, "Expected 1 I100 hint, got: {:?}", hints);
        assert!(hints[0].message.contains("fact"));
    }

    #[test]
    fn test_tco_tail_ok() {
        let source = "fact = (n, acc=1) => { if n <= 1 { acc } else { fact(n - 1, n * acc) } }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I100").collect();
        assert!(hints.is_empty(), "Unexpected I100: {:?}", hints);
    }

    #[test]
    fn test_tco_no_recursion() {
        let source = "f = (x) => { x + 1 }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I100").collect();
        assert!(hints.is_empty());
    }

    // --- I101: Redundant boolean ---

    #[test]
    fn test_redundant_boolean() {
        let source = "x = True\nif x == True { 1 }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I101").collect();
        assert_eq!(hints.len(), 1, "Expected 1 I101 hint, got: {:?}", hints);
        assert_eq!(hints[0].suggestion.as_deref(), Some("x"));
    }

    #[test]
    fn test_redundant_boolean_neq_false() {
        let source = "x = True\nif x != False { 1 }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I101").collect();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].suggestion.as_deref(), Some("x"));
    }

    #[test]
    fn test_no_redundant_comparison() {
        let source = "x = 1\nif x == 1 { 1 }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I101").collect();
        assert!(hints.is_empty());
    }

    // --- I102: Self-assignment ---

    #[test]
    fn test_self_assignment() {
        let source = "x = 1\nx = x";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I102").collect();
        assert_eq!(hints.len(), 1, "Expected 1 I102 hint, got: {:?}", hints);
    }

    #[test]
    fn test_different_assignment_no_warning() {
        let source = "x = 1\ny = x";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I102").collect();
        assert!(hints.is_empty());
    }
}
