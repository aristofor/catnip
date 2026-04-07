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
                let word = chunk.trim().split_whitespace().next()?;
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

// --- Phase 3: Semantic (CST walk) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefKind {
    Local,
    Param,
    VariadicParam,
    ForVar,
    MatchVar,
    WithVar,
    ExceptVar,
}

#[derive(Debug, Clone)]
struct DefInfo {
    name: String,
    kind: DefKind,
    line: usize,
    column: usize,
    scope_depth: usize,
}

#[derive(Debug, Default)]
struct ScopeFrame {
    names: HashSet<String>,
    definitions: Vec<DefInfo>,
    used: HashSet<String>,
}

struct ScopeTracker {
    scopes: Vec<ScopeFrame>,
    check_names: bool,
}

impl ScopeTracker {
    fn new(check_names: bool) -> Self {
        Self {
            scopes: vec![ScopeFrame::default()],
            check_names,
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(ScopeFrame::default());
    }

    fn pop_scope(&mut self, diagnostics: &mut Vec<Diagnostic>, source_lines: &[&str]) {
        if self.scopes.len() > 1 {
            if let Some(frame) = self.scopes.pop() {
                self.emit_unused_diagnostics(frame, diagnostics, source_lines);
            }
        }
    }

    fn define(&mut self, name: &str, line: usize, column: usize, kind: DefKind) {
        let depth = self.scopes.len() - 1;
        if let Some(frame) = self.scopes.last_mut() {
            frame.names.insert(name.to_string());
            frame.definitions.push(DefInfo {
                name: name.to_string(),
                kind,
                line,
                column,
                scope_depth: depth,
            });
        }
    }

    fn define_local(&mut self, name: &str, line: usize, column: usize) {
        self.define(name, line, column, DefKind::Local);
    }

    fn define_builtin(&mut self, name: &str) {
        if let Some(scope) = self.scopes.first_mut() {
            scope.names.insert(name.to_string());
        }
    }

    fn is_defined_in_current_scope(&self, name: &str) -> bool {
        self.scopes.last().is_some_and(|scope| scope.names.contains(name))
    }

    fn is_defined_in_parent_scope(&self, name: &str) -> bool {
        if self.scopes.len() <= 1 {
            return false;
        }
        self.scopes[..self.scopes.len() - 1]
            .iter()
            .rev()
            .any(|scope| scope.names.contains(name))
    }

    fn use_name(&mut self, name: &str) {
        for depth in (0..self.scopes.len()).rev() {
            if self.scopes[depth].names.contains(name) {
                self.scopes[depth].used.insert(name.to_string());
                return;
            }
        }
    }

    fn is_defined(&self, name: &str) -> bool {
        for scope in self.scopes.iter().rev() {
            if scope.names.contains(name) {
                return true;
            }
        }
        false
    }

    fn emit_unused_diagnostics(&self, frame: ScopeFrame, diagnostics: &mut Vec<Diagnostic>, source_lines: &[&str]) {
        for def in frame.definitions {
            if def.scope_depth == 0 || def.name.starts_with('_') {
                continue;
            }
            if frame.used.contains(&def.name) {
                continue;
            }

            let code = match def.kind {
                DefKind::Param | DefKind::VariadicParam if def.name != "self" => "W201",
                DefKind::Param | DefKind::VariadicParam => continue,
                _ => "W200",
            };

            let message = match def.kind {
                DefKind::Param | DefKind::VariadicParam => {
                    format!("Parameter '{}' is never used", def.name)
                }
                _ => format!("Variable '{}' is defined but never used", def.name),
            };

            diagnostics.push(make_diagnostic(
                code,
                &message,
                Severity::Warning,
                def.line,
                def.column,
                source_lines.get(def.line.saturating_sub(1)).map(|s| s.to_string()),
                None,
            ));
        }
    }
}

// GENERATED FROM catnip/context.py - do not edit manually.
// Run: python catnip_tools/gen_builtins.py
// @generated-builtins-start
const BUILTINS: &[&str] = &[
    "ArithmeticError",
    "AttributeError",
    "Exception",
    "False",
    "IndexError",
    "KeyError",
    "LookupError",
    "META",
    "MemoryError",
    "ND",
    "NameError",
    "None",
    "RUNTIME",
    "RuntimeError",
    "True",
    "TypeError",
    "ValueError",
    "ZeroDivisionError",
    "_",
    "__import__",
    "_cache",
    "abs",
    "abstract",
    "all",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "cached",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "debug",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "fold",
    "format",
    "freeze",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "hex",
    "id",
    "import",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "jit",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "pragma",
    "print",
    "property",
    "pure",
    "range",
    "reduce",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "static",
    "staticmethod",
    "str",
    "sum",
    "super",
    "thaw",
    "tuple",
    "typeof",
    "vars",
    "zip",
];
// @generated-builtins-end

fn check_semantic(
    root: Node,
    source: &str,
    source_lines: &[&str],
    check_names: bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut tracker = ScopeTracker::new(check_names);

    for keyword in symbols::keywords() {
        tracker.define_builtin(keyword);
    }

    for &name in BUILTINS {
        tracker.define_builtin(name);
    }

    walk_node(root, source, &mut tracker, diagnostics, source_lines);
    check_dead_code_after_return(root, source_lines, diagnostics);
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
        NK::SOURCE_FILE | NK::BLOCK => {
            walk_children(node, source, tracker, diagnostics, source_lines);
        }

        NK::ASSIGNMENT => {
            walk_assignment(node, source, tracker, diagnostics, source_lines);
        }

        NK::LAMBDA_EXPR => {
            walk_lambda(node, source, tracker, diagnostics, source_lines);
        }

        NK::FOR_STMT => {
            walk_for(node, source, tracker, diagnostics, source_lines);
        }

        NK::MATCH_EXPR => {
            walk_match(node, source, tracker, diagnostics, source_lines);
        }

        NK::STRUCT_STMT | NK::TRAIT_STMT => {
            // Define the struct/trait name, but don't walk body children:
            // fields, methods, and `self` live in the struct's own namespace
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source);
                let line = name_node.start_position().row + 1;
                let col = name_node.start_position().column + 1;
                tracker.define_local(&name, line, col);
            }
            // Walk implements/extends clauses (they reference outer names)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    NK::STRUCT_IMPLEMENTS | NK::STRUCT_EXTENDS | NK::TRAIT_EXTENDS => {
                        walk_children(child, source, tracker, diagnostics, source_lines);
                    }
                    _ => {}
                }
            }
        }

        NK::IF_EXPR
        | NK::ELIF_CLAUSE
        | NK::ELSE_CLAUSE
        | NK::WHILE_STMT
        | NK::TRY_STMT
        | NK::EXCEPT_BLOCK
        | NK::FINALLY_CLAUSE
        | NK::RAISE_STMT => {
            walk_children(node, source, tracker, diagnostics, source_lines);
        }

        NK::WITH_STMT => {
            walk_with_stmt(node, source, tracker, diagnostics, source_lines);
        }

        NK::EXCEPT_CLAUSE => {
            walk_except_clause(node, source, tracker, diagnostics, source_lines);
        }

        NK::CHAINED => {
            walk_chained(node, source, tracker, diagnostics, source_lines);
        }

        NK::CALL => {
            define_selective_import(node, source, tracker);
            walk_children(node, source, tracker, diagnostics, source_lines);
        }

        NK::KWARG | NK::DICT_KWARG => {
            // Only walk the value, not the key (key is a parameter name, not a reference)
            if let Some(val) = node.child_by_field_name("value") {
                walk_node(val, source, tracker, diagnostics, source_lines);
            }
        }

        NK::IDENTIFIER => {
            let name = node_text(node, source);
            check_reference(&name, node, tracker, diagnostics, source_lines);
        }

        NK::DECORATOR => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == NK::IDENTIFIER {
                    let name = node_text(child, source);
                    tracker.use_name(&name);
                }
            }
        }

        NK::RETURN_STMT => {
            walk_children(node, source, tracker, diagnostics, source_lines);
        }

        NK::PATTERN_VAR
        | NK::PATTERN_LITERAL
        | NK::PATTERN_WILDCARD
        | NK::PATTERN_OR
        | NK::PATTERN_TUPLE
        | NK::PATTERN_STAR
        | NK::PATTERN_STRUCT => {}

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

fn first_named_child(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    let child = node.children(&mut cursor).find(|child| child.is_named());
    child
}

fn statement_payload(statement: Node) -> Option<Node> {
    if statement.kind() == NK::STATEMENT {
        return first_named_child(statement);
    }
    Some(statement)
}

fn check_dead_code_after_return(root: Node, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == NK::BLOCK {
            check_block_dead_code_after_return(current, source_lines, diagnostics);
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn check_block_dead_code_after_return(block: Node, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let mut return_seen = false;
    let mut return_line = None;
    let mut cursor = block.walk();
    for child in block.children(&mut cursor) {
        if child.kind() != NK::STATEMENT {
            continue;
        }
        let Some(payload) = statement_payload(child) else {
            continue;
        };
        if return_seen {
            if Some(payload.start_position().row + 1) == return_line {
                continue;
            }
            let line = payload.start_position().row + 1;
            let col = payload.start_position().column + 1;
            diagnostics.push(make_diagnostic(
                "W300",
                "Unreachable code after return",
                Severity::Warning,
                line,
                col,
                source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                None,
            ));
            continue;
        }

        if payload.kind() == NK::RETURN_STMT {
            return_seen = true;
            return_line = Some(payload.start_position().row + 1);
        }
    }
}

#[derive(Debug)]
struct BindingInfo {
    name: String,
    line: usize,
    column: usize,
}

fn collect_lvalue_bindings(node: Node, source: &str, bindings: &mut Vec<BindingInfo>) {
    match node.kind() {
        NK::IDENTIFIER => {
            bindings.push(BindingInfo {
                name: node_text(node, source),
                line: node.start_position().row + 1,
                column: node.start_position().column + 1,
            });
        }
        NK::LVALUE | NK::UNPACK_TARGET | NK::UNPACK_TUPLE | NK::UNPACK_SEQUENCE | NK::UNPACK_ITEMS => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_lvalue_bindings(child, source, bindings);
            }
        }
        NK::VARIADIC_PARAM => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == NK::IDENTIFIER {
                    bindings.push(BindingInfo {
                        name: node_text(child, source),
                        line: child.start_position().row + 1,
                        column: child.start_position().column + 1,
                    });
                }
            }
        }
        _ => {}
    }
}

fn subtree_reads_identifier_skip_nested_lambdas(node: Node, name: &str, source: &str) -> bool {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.id() != node.id() && current.kind() == NK::LAMBDA_EXPR {
            continue;
        }
        if current.kind() == NK::IDENTIFIER && node_text(current, source) == name {
            return true;
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
    false
}

/// Detect `import("module", "Name")` and define `Name` in scope.
/// Handles both the positional form and multiple names.
fn define_selective_import(node: Node, source: &str, tracker: &mut ScopeTracker) {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();

    // First child must be identifier "import"
    match children.first() {
        Some(c) if c.kind() == NK::IDENTIFIER && node_text(*c, source) == "import" => {}
        _ => return,
    }

    // Find arguments > args node, then extract positional string args after the first
    for child in &children {
        if child.kind() != NK::ARGUMENTS {
            continue;
        }
        // arguments wraps an `args` node containing the actual positional args
        let mut arg_cursor = child.walk();
        for args_child in child.children(&mut arg_cursor) {
            if args_child.kind() != "args" {
                continue;
            }
            let mut inner_cursor = args_child.walk();
            let mut positional_index = 0;
            for arg in args_child.children(&mut inner_cursor) {
                if arg.kind() == NK::KWARG || arg.kind() == NK::DICT_KWARG {
                    continue;
                }
                if !arg.is_named() {
                    continue;
                }
                if positional_index > 0 {
                    // Second+ positional arg: extract string content as imported name
                    // Supports "name:alias" syntax (alias is the defined name)
                    let text = node_text(arg, source);
                    let raw = text.trim_matches('"').trim_matches('\'');
                    if !raw.is_empty() {
                        let defined_as = match raw.split_once(':') {
                            Some((_, alias)) if !alias.is_empty() => alias,
                            _ => raw,
                        };
                        tracker.define_builtin(defined_as);
                    }
                }
                positional_index += 1;
            }
        }
    }
}

/// Check if a node is a call to `import(... wild=True)`.
fn is_wild_import_call(node: Node, source: &str) -> bool {
    if node.kind() != NK::CALL {
        return false;
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();

    // First child must be identifier "import"
    let callee = match children.first() {
        Some(c) if c.kind() == NK::IDENTIFIER && node_text(*c, source) == "import" => c,
        _ => return false,
    };
    let _ = callee;

    // Look for kwarg wild=True anywhere inside arguments (may be nested in args_kwargs)
    fn has_wild_kwarg(node: Node, source: &str) -> bool {
        if node.kind() == NK::KWARG {
            let key = node.child_by_field_name("key");
            let val = node.child_by_field_name("value");
            if let (Some(k), Some(v)) = (key, val) {
                if node_text(k, source) == "wild" && node_text(v, source) == "True" {
                    return true;
                }
            }
        }
        let mut cursor = node.walk();
        let kids: Vec<Node> = node.children(&mut cursor).collect();
        kids.iter().any(|c| has_wild_kwarg(*c, source))
    }

    children
        .iter()
        .any(|c| c.kind() == NK::ARGUMENTS && has_wild_kwarg(*c, source))
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
            NK::DECORATOR => decorator_nodes.push(*child),
            NK::LVALUE | NK::UNPACK_TARGET | NK::IDENTIFIER | NK::UNPACK_TUPLE | NK::UNPACK_SEQUENCE | NK::SETATTR => {
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

    let mut bindings = Vec::new();
    for lv in &lvalue_nodes {
        collect_lvalue_bindings(*lv, source, &mut bindings);
    }

    // W202: assignment of wild import (returns None)
    if let Some(rv) = rvalue_node {
        if !lvalue_nodes.is_empty() && is_wild_import_call(rv, source) {
            let line = node.start_position().row + 1;
            let col = node.start_position().column + 1;
            diagnostics.push(make_diagnostic(
                "W202",
                "Wild import returns None; assignment is useless",
                Severity::Warning,
                line,
                col,
                source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                Some("Use import(\"...\", wild=True) without assignment".to_string()),
            ));
        }
    }

    for dec in &decorator_nodes {
        let mut c = dec.walk();
        for child in dec.children(&mut c) {
            if child.kind() == NK::IDENTIFIER {
                tracker.use_name(&node_text(child, source));
            }
        }
    }

    if let Some(rv) = rvalue_node {
        walk_node(rv, source, tracker, diagnostics, source_lines);

        for binding in bindings {
            check_keyword_name(&binding.name, binding.line, binding.column, source_lines, diagnostics);

            if tracker.is_defined_in_current_scope(&binding.name) {
                continue;
            }

            let shadows_parent = tracker.is_defined_in_parent_scope(&binding.name);
            let mutates_capture =
                shadows_parent && subtree_reads_identifier_skip_nested_lambdas(rv, &binding.name, source);

            if shadows_parent && !mutates_capture && binding.name != "_" && !binding.name.starts_with('_') {
                diagnostics.push(make_diagnostic(
                    "W204",
                    &format!("'{}' shadows variable from outer scope", binding.name),
                    Severity::Warning,
                    binding.line,
                    binding.column,
                    source_lines.get(binding.line.saturating_sub(1)).map(|s| s.to_string()),
                    None,
                ));
            }

            if !mutates_capture {
                tracker.define_local(&binding.name, binding.line, binding.column);
            }
        }
    } else {
        for binding in bindings {
            check_keyword_name(&binding.name, binding.line, binding.column, source_lines, diagnostics);

            if tracker.is_defined_in_current_scope(&binding.name) {
                continue;
            }
            if tracker.is_defined_in_parent_scope(&binding.name)
                && binding.name != "_"
                && !binding.name.starts_with('_')
            {
                diagnostics.push(make_diagnostic(
                    "W204",
                    &format!("'{}' shadows variable from outer scope", binding.name),
                    Severity::Warning,
                    binding.line,
                    binding.column,
                    source_lines.get(binding.line.saturating_sub(1)).map(|s| s.to_string()),
                    None,
                ));
            }
            tracker.define_local(&binding.name, binding.line, binding.column);
        }
    }
}

fn check_keyword_name(name: &str, line: usize, col: usize, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    if symbols::is_keyword(name) {
        diagnostics.push(make_diagnostic(
            "W203",
            &format!("'{}' is a language keyword, avoid using it as a name", name),
            Severity::Warning,
            line,
            col,
            source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
            None,
        ));
    }
}

fn define_lvalue(
    node: Node,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
    def_kind: DefKind,
) {
    let kind = node.kind();
    match kind {
        NK::IDENTIFIER => {
            let name = node_text(node, source);
            let line = node.start_position().row + 1;
            let col = node.start_position().column + 1;
            check_keyword_name(&name, line, col, source_lines, diagnostics);
            tracker.define(&name, line, col, def_kind);
        }
        NK::LVALUE | NK::UNPACK_TARGET | NK::UNPACK_TUPLE | NK::UNPACK_SEQUENCE | NK::UNPACK_ITEMS => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                define_lvalue(child, source, tracker, diagnostics, source_lines, def_kind);
            }
        }
        NK::SETATTR => {}
        NK::VARIADIC_PARAM => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == NK::IDENTIFIER {
                    let name = node_text(child, source);
                    let line = child.start_position().row + 1;
                    let col = child.start_position().column + 1;
                    check_keyword_name(&name, line, col, source_lines, diagnostics);
                    tracker.define(&name, line, col, DefKind::VariadicParam);
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
            NK::LAMBDA_PARAMS => {
                define_lambda_params(child, source, tracker, diagnostics, source_lines);
            }
            NK::BLOCK => {
                walk_node(child, source, tracker, diagnostics, source_lines);
            }
            _ => {}
        }
    }

    tracker.pop_scope(diagnostics, source_lines);
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
            NK::LAMBDA_PARAM => {
                let mut pc = child.walk();
                let mut saw_name = false;
                for param_child in child.children(&mut pc) {
                    if param_child.kind() == NK::IDENTIFIER && !saw_name {
                        let name = node_text(param_child, source);
                        let line = param_child.start_position().row + 1;
                        let col = param_child.start_position().column + 1;
                        check_keyword_name(&name, line, col, source_lines, diagnostics);
                        tracker.define(&name, line, col, DefKind::Param);
                        saw_name = true;
                    } else if saw_name && param_child.kind() != "=" {
                        walk_node(param_child, source, tracker, diagnostics, source_lines);
                    }
                }
            }
            NK::VARIADIC_PARAM => {
                let mut pc = child.walk();
                for param_child in child.children(&mut pc) {
                    if param_child.kind() == NK::IDENTIFIER {
                        let name = node_text(param_child, source);
                        let line = param_child.start_position().row + 1;
                        let col = param_child.start_position().column + 1;
                        check_keyword_name(&name, line, col, source_lines, diagnostics);
                        tracker.define(&name, line, col, DefKind::VariadicParam);
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
            NK::UNPACK_TARGET | NK::IDENTIFIER if !defined_target => {
                define_lvalue(*child, source, tracker, diagnostics, source_lines, DefKind::ForVar);
                defined_target = true;
            }
            "in" => {
                past_in = true;
            }
            NK::BLOCK => {
                walk_node(*child, source, tracker, diagnostics, source_lines);
            }
            _ if past_in && child.kind() != NK::BLOCK => {
                walk_node(*child, source, tracker, diagnostics, source_lines);
            }
            _ => {}
        }
    }

    tracker.pop_scope(diagnostics, source_lines);
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
            NK::MATCH_CASE => {
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
            NK::PATTERN | NK::PATTERN_OR => {
                define_pattern_vars(child, source, tracker);
            }
            NK::BLOCK => {
                walk_node(child, source, tracker, diagnostics, source_lines);
            }
            _ => {
                if child.kind() != "=>" && child.kind() != "if" {
                    walk_node(child, source, tracker, diagnostics, source_lines);
                }
            }
        }
    }

    tracker.pop_scope(diagnostics, source_lines);
}

/// Walk an except clause: `e: TypeError => { handler }`.
/// Binding is scoped to the handler block.
fn walk_except_clause(
    node: Node,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    tracker.push_scope();

    // Define binding if present
    if let Some(binding) = node.child_by_field_name("binding") {
        let name = node_text(binding, source);
        let line = binding.start_position().row + 1;
        let col = binding.start_position().column + 1;
        tracker.define(&name, line, col, DefKind::ExceptVar);
    }

    // Walk exception type references and handler block
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            NK::EXCEPT_PATTERN | NK::EXCEPT_TYPES => {
                // Type names are references (mark as read)
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == NK::IDENTIFIER {
                        walk_node(inner, source, tracker, diagnostics, source_lines);
                    }
                }
            }
            NK::BLOCK => {
                walk_node(child, source, tracker, diagnostics, source_lines);
            }
            _ => {}
        }
    }

    tracker.pop_scope(diagnostics, source_lines);
}

fn walk_with_stmt(
    node: Node,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    // Define binding names and walk value expressions
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "with_binding" {
            // Walk value expression first (may reference outer scope)
            if let Some(value) = child.child_by_field_name("value") {
                walk_node(value, source, tracker, diagnostics, source_lines);
            }
            // Define the binding name
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(name_node, source);
                let line = name_node.start_position().row + 1;
                let col = name_node.start_position().column + 1;
                tracker.define(&name, line, col, DefKind::WithVar);
            }
        } else if child.kind() == "block" {
            walk_node(child, source, tracker, diagnostics, source_lines);
        }
    }
}

fn define_pattern_vars(node: Node, source: &str, tracker: &mut ScopeTracker) {
    let kind = node.kind();
    match kind {
        NK::PATTERN_VAR => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == NK::IDENTIFIER {
                    let name = node_text(child, source);
                    if name != "_" {
                        let line = child.start_position().row + 1;
                        let col = child.start_position().column + 1;
                        tracker.define(&name, line, col, DefKind::MatchVar);
                    }
                }
            }
        }
        NK::PATTERN_WILDCARD | NK::PATTERN_LITERAL => {}
        NK::PATTERN_STRUCT => {
            // struct_name is a type reference, not a variable - only define fields
            let count = node.child_count() as u32;
            for i in 0..count {
                if let Some(child) = node.child(i) {
                    if child.kind() == NK::IDENTIFIER && node.field_name_for_child(i) == Some("fields") {
                        let name = node_text(child, source);
                        let line = child.start_position().row + 1;
                        let col = child.start_position().column + 1;
                        tracker.define(&name, line, col, DefKind::MatchVar);
                    }
                }
            }
        }
        NK::PATTERN_STAR => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == NK::IDENTIFIER {
                    let name = node_text(child, source);
                    let line = child.start_position().row + 1;
                    let col = child.start_position().column + 1;
                    tracker.define(&name, line, col, DefKind::MatchVar);
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
            NK::GETATTR | NK::CALLATTR | NK::SETATTR => {
                walk_member_args(child, source, tracker, diagnostics, source_lines);
            }
            NK::INDEX => {
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
            NK::IDENTIFIER => {}
            NK::ARGUMENTS => {
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

    if tracker.check_names && !tracker.is_defined(name) {
        let line = node.start_position().row + 1;
        let col = node.start_position().column + 1;
        diagnostics.push(make_diagnostic(
            "E200",
            &format!("Name '{}' is not defined", name),
            Severity::Error,
            line,
            col,
            source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
            None,
        ));
    }
}

// --- Phase 4: Improvement suggestions ---

fn check_improvements(
    root: Node,
    source: &str,
    source_lines: &[&str],
    config: &LintConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    check_tco_opportunities(root, source, source_lines, diagnostics);
    check_redundant_boolean(root, source, source_lines, diagnostics);
    check_self_assignment(root, source, source_lines, diagnostics);
    check_static_dead_branches(root, source_lines, diagnostics);
    check_detectable_infinite_loops(root, source_lines, diagnostics);
    if config.max_nesting_depth > 0 {
        check_nesting_depth(root, source_lines, config.max_nesting_depth, diagnostics);
    }
    if config.max_cyclomatic_complexity > 0 {
        check_cyclomatic_complexity(root, source_lines, config.max_cyclomatic_complexity, diagnostics);
    }
    if config.max_function_length > 0 {
        check_function_length(root, source_lines, config.max_function_length, diagnostics);
    }
    if config.max_parameters > 0 {
        check_too_many_parameters(root, source, source_lines, config.max_parameters, diagnostics);
    }
    check_match_without_wildcard(root, source_lines, diagnostics);
}

/// I100: Detect recursive calls not in tail position
fn check_tco_opportunities(root: Node, source: &str, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    find_named_lambdas(root, source, source_lines, diagnostics);
}

fn find_named_lambdas(node: Node, source: &str, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    if node.kind() == NK::ASSIGNMENT {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();

        // Extract function name (may be inside lvalue/unpack_target) and lambda body
        let mut name: Option<String> = None;
        let mut lambda_node: Option<Node> = None;

        for child in &children {
            match child.kind() {
                NK::IDENTIFIER if name.is_none() => {
                    name = Some(node_text(*child, source));
                }
                NK::LVALUE | NK::UNPACK_TARGET if name.is_none() => {
                    // Dig into lvalue to find a simple identifier
                    if let Some(ident) = extract_single_identifier(*child, source) {
                        name = Some(ident);
                    }
                }
                NK::LAMBDA_EXPR => {
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
        NK::IDENTIFIER => Some(node_text(node, source)),
        NK::LVALUE | NK::UNPACK_TARGET => {
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
    let body = lambda.children(&mut cursor).find(|child| child.kind() == NK::BLOCK);
    body
}

/// Branch-boundary node kinds: if/else/match/block/lambda and short-circuit
/// operators create exclusive execution paths. Recursive calls in different
/// branches are independent (not tree recursion).
const BRANCH_BOUNDARIES: &[&str] = &[
    NK::IF_EXPR,
    NK::ELIF_CLAUSE,
    NK::ELSE_CLAUSE,
    NK::MATCH_EXPR,
    NK::BLOCK,
    NK::LAMBDA_EXPR,
    // Short-circuit operators: only one side executes
    NK::BOOL_OR,
    NK::BOOL_AND,
    "null_coalesce", // ?? operator (no NK constant)
];

/// Check if a recursive call is part of a tree recursion expression, i.e.
/// a sibling in the same expression subtree is also a recursive call to the
/// same function. Example: `f(n-1) + f(n-2)` in an `additive` node.
/// Calls in exclusive branches (`if/else`) are NOT tree recursion.
fn is_tree_recursion_call(call_node: Node, fn_name: &str, source: &str) -> bool {
    let mut node = call_node;
    while let Some(parent) = node.parent() {
        // Stop at branch boundaries -- siblings beyond here are exclusive paths
        if BRANCH_BOUNDARIES.contains(&parent.kind()) {
            return false;
        }
        // Check if any sibling subtree of this parent also contains a recursive call
        let mut cursor = parent.walk();
        for child in parent.children(&mut cursor) {
            if child.id() == node.id() {
                continue;
            }
            if contains_recursive_call(child, fn_name, source) {
                return true;
            }
        }
        node = parent;
    }
    false
}

/// Check if a subtree contains a call to `fn_name`.
/// Stops at nested lambdas (different scope -- call may not run in this frame).
fn contains_recursive_call(node: Node, fn_name: &str, source: &str) -> bool {
    if node.kind() == NK::CALL {
        if let Some(callee) = node.child(0) {
            if callee.kind() == NK::IDENTIFIER && node_text(callee, source) == fn_name {
                return true;
            }
        }
    }
    // Don't descend into nested lambdas
    if node.kind() == NK::LAMBDA_EXPR {
        return false;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if contains_recursive_call(child, fn_name, source) {
            return true;
        }
    }
    false
}

/// Walk the lambda body looking for recursive calls, check tail position.
/// Calls that are part of tree recursion (e.g. `f(a) + f(b)`) are skipped
/// since they cannot be restructured for TCO. Calls in exclusive branches
/// (`if/else`) are still checked individually.
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
        if current.kind() == NK::CALL {
            // Check if callee is our function name
            let mut cursor = current.walk();
            let first_child = current.children(&mut cursor).next();
            if let Some(callee) = first_child {
                if callee.kind() == NK::IDENTIFIER && node_text(callee, source) == fn_name {
                    // Skip tree recursion (e.g. f(a) + f(b)) -- not a TCO candidate
                    if is_tree_recursion_call(current, fn_name, source) {
                        continue;
                    }
                    // Found a recursive call - check if it's in tail position
                    if !is_in_tail_position(current, lambda) {
                        let line = current.start_position().row + 1;
                        let col = current.start_position().column + 1;
                        diagnostics.push(make_diagnostic(
                            "I100",
                            &format!(
                                "Recursive call to '{}' is not in tail position - consider restructuring for TCO",
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
        if current.kind() == NK::LAMBDA_EXPR && current.id() != lambda.id() {
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
            NK::BLOCK => {
                // Must be the last significant child of the block
                if !is_last_significant_child(current, parent) {
                    return false;
                }
                current = parent;
            }
            NK::STATEMENT => {
                // Statement wraps an expression, transparent for tail position
                current = parent;
            }
            NK::IF_EXPR | NK::ELIF_CLAUSE | NK::ELSE_CLAUSE | NK::MATCH_CASE | NK::MATCH_EXPR => {
                current = parent;
            }
            NK::RETURN_STMT => {
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
        // Skip punctuation, anonymous nodes, and comments.
        if c.is_named() && c.kind() != NK::COMMENT {
            last_significant = Some(c.id());
        }
    }
    last_significant == Some(child.id())
}

/// Check if a node is a boolean literal (true/false), handling `literal` wrapper
fn is_bool_literal(node: Node) -> Option<bool> {
    match node.kind() {
        NK::TRUE => Some(true),
        NK::FALSE => Some(false),
        NK::LITERAL => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == NK::TRUE {
                    return Some(true);
                }
                if child.kind() == NK::FALSE {
                    return Some(false);
                }
            }
            None
        }
        _ => None,
    }
}

/// I101: Detect redundant comparisons with boolean literals
fn check_redundant_boolean(root: Node, source: &str, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == NK::COMPARISON {
            check_comparison_with_bool(current, source, source_lines, diagnostics);
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn check_comparison_with_bool(node: Node, source: &str, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
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

    if op_node.kind() != NK::COMP_OP {
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
        source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
        Some(suggestion),
    ));
}

/// I102: Detect self-assignment (`x = x`)
fn check_self_assignment(root: Node, source: &str, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == NK::ASSIGNMENT {
            check_assignment_self(current, source, source_lines, diagnostics);
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn check_assignment_self(node: Node, source: &str, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
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
            NK::LVALUE | NK::UNPACK_TARGET => {
                if lvalue_name.is_none() {
                    lvalue_name = extract_single_identifier(*child, source);
                }
            }
            NK::IDENTIFIER => {
                // Could be lvalue (direct) or rvalue
                if eq_count == 0 && lvalue_name.is_none() {
                    lvalue_name = Some(node_text(*child, source));
                } else if eq_count > 0 {
                    rvalue_name = Some(node_text(*child, source));
                }
            }
            "=" => eq_count += 1,
            NK::DECORATOR => has_decorator = true,
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
                source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                Some("remove".to_string()),
            ));
        }
    }
}

fn check_static_dead_branches(root: Node, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == NK::IF_EXPR {
            check_if_static_dead_branches(current, source_lines, diagnostics);
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn check_if_static_dead_branches(node: Node, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let Some(condition) = node.child_by_field_name("condition") else {
        return;
    };
    match is_bool_literal(condition) {
        Some(true) => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == NK::ELIF_CLAUSE || child.kind() == NK::ELSE_CLAUSE {
                    let line = child.start_position().row + 1;
                    let col = child.start_position().column + 1;
                    diagnostics.push(make_diagnostic(
                        "W301",
                        "Dead branch: previous condition is always True",
                        Severity::Warning,
                        line,
                        col,
                        source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                        None,
                    ));
                }
            }
        }
        Some(false) => {
            if let Some(consequence) = node.child_by_field_name("consequence") {
                let line = consequence.start_position().row + 1;
                let col = consequence.start_position().column + 1;
                diagnostics.push(make_diagnostic(
                    "W301",
                    "Dead branch: condition is always False",
                    Severity::Warning,
                    line,
                    col,
                    source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                    None,
                ));
            }
        }
        None => {}
    }
}

fn check_detectable_infinite_loops(root: Node, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == NK::WHILE_STMT && while_condition_is_true(current) && !loop_body_has_break(current) {
            let line = current.start_position().row + 1;
            let col = current.start_position().column + 1;
            diagnostics.push(make_diagnostic(
                "W302",
                "Loop condition is always True and body has no break",
                Severity::Warning,
                line,
                col,
                source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                None,
            ));
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn while_condition_is_true(node: Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() && child.kind() != NK::BLOCK && is_bool_literal(child) == Some(true) {
            return true;
        }
    }
    false
}

fn loop_body_has_break(loop_node: Node) -> bool {
    let mut cursor = loop_node.walk();
    let Some(body) = loop_node.children(&mut cursor).find(|child| child.kind() == NK::BLOCK) else {
        return false;
    };

    let mut stack = vec![body];
    while let Some(current) = stack.pop() {
        match current.kind() {
            NK::BREAK_STMT => return true,
            NK::LAMBDA_EXPR | NK::FOR_STMT | NK::WHILE_STMT => continue,
            _ => {}
        }

        let mut child_cursor = current.walk();
        for child in current.children(&mut child_cursor) {
            stack.push(child);
        }
    }

    false
}

fn check_nesting_depth(root: Node, source_lines: &[&str], max_depth: usize, diagnostics: &mut Vec<Diagnostic>) {
    walk_nesting_depth(root, 0, source_lines, max_depth, diagnostics);
}

fn walk_nesting_depth(
    node: Node,
    depth: usize,
    source_lines: &[&str],
    max_depth: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Reset depth at function boundaries -- nesting is per-function, not per-file
    if node.kind() == NK::LAMBDA_EXPR {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk_nesting_depth(child, 0, source_lines, max_depth, diagnostics);
        }
        return;
    }

    let is_control = matches!(
        node.kind(),
        NK::IF_EXPR | NK::WHILE_STMT | NK::FOR_STMT | NK::MATCH_EXPR | NK::TRY_STMT
    );
    let next_depth = if is_control { depth + 1 } else { depth };

    if is_control && next_depth > max_depth {
        let line = node.start_position().row + 1;
        let col = node.start_position().column + 1;
        diagnostics.push(make_diagnostic(
            "I200",
            &format!("Nesting depth {} exceeds threshold {}", next_depth, max_depth),
            Severity::Hint,
            line,
            col,
            source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
            None,
        ));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_nesting_depth(child, next_depth, source_lines, max_depth, diagnostics);
    }
}

fn check_cyclomatic_complexity(
    root: Node,
    source_lines: &[&str],
    max_complexity: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == NK::LAMBDA_EXPR {
            let complexity = compute_lambda_cyclomatic_complexity(current);
            if complexity > max_complexity {
                let line = current.start_position().row + 1;
                let col = current.start_position().column + 1;
                diagnostics.push(make_diagnostic(
                    "I201",
                    &format!(
                        "Function cyclomatic complexity is {}, threshold is {}",
                        complexity, max_complexity
                    ),
                    Severity::Hint,
                    line,
                    col,
                    source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                    None,
                ));
            }
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn compute_lambda_cyclomatic_complexity(lambda: Node) -> usize {
    let Some(body) = find_lambda_body(lambda) else {
        return 1;
    };

    let mut complexity = 1;
    let mut stack = vec![body];

    while let Some(current) = stack.pop() {
        if current.kind() == NK::LAMBDA_EXPR {
            continue;
        }

        match current.kind() {
            NK::IF_EXPR | NK::ELIF_CLAUSE | NK::WHILE_STMT | NK::FOR_STMT => complexity += 1,
            NK::MATCH_EXPR => complexity += count_match_cases(current).saturating_sub(1),
            NK::BOOL_AND | NK::BOOL_OR => complexity += 1,
            _ => {}
        }

        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }

    complexity
}

fn count_match_cases(node: Node) -> usize {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.kind() == NK::MATCH_CASE)
        .count()
}

fn check_function_length(root: Node, source_lines: &[&str], max_length: usize, diagnostics: &mut Vec<Diagnostic>) {
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == NK::LAMBDA_EXPR {
            if let Some(body) = find_lambda_body(current) {
                let statements = count_direct_statements(body);
                if statements > max_length {
                    let line = current.start_position().row + 1;
                    let col = current.start_position().column + 1;
                    diagnostics.push(make_diagnostic(
                        "I202",
                        &format!("Function has {} statements, threshold is {}", statements, max_length),
                        Severity::Hint,
                        line,
                        col,
                        source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                        None,
                    ));
                }
            }
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn count_direct_statements(block: Node) -> usize {
    let mut cursor = block.walk();
    block
        .children(&mut cursor)
        .filter(|child| child.kind() == NK::STATEMENT)
        .count()
}

fn check_too_many_parameters(
    root: Node,
    source: &str,
    source_lines: &[&str],
    max_params: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == NK::LAMBDA_EXPR {
            let count = count_lambda_parameters(current, source);
            if count > max_params {
                let line = current.start_position().row + 1;
                let col = current.start_position().column + 1;
                diagnostics.push(make_diagnostic(
                    "I203",
                    &format!("Function has {} parameters, threshold is {}", count, max_params),
                    Severity::Hint,
                    line,
                    col,
                    source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                    None,
                ));
            }
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn count_lambda_parameters(lambda: Node, source: &str) -> usize {
    let mut cursor = lambda.walk();
    for child in lambda.children(&mut cursor) {
        if child.kind() == NK::LAMBDA_PARAMS {
            let mut params_cursor = child.walk();
            return child
                .children(&mut params_cursor)
                .filter(|param| {
                    (param.kind() == NK::LAMBDA_PARAM || param.kind() == NK::VARIADIC_PARAM)
                        && node_text(*param, source) != "self"
                })
                .count();
        }
    }
    0
}

fn check_match_without_wildcard(root: Node, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == NK::MATCH_EXPR && !match_has_unconditional_catchall(current) {
            let line = current.start_position().row + 1;
            let col = current.start_position().column + 1;
            diagnostics.push(make_diagnostic(
                "I103",
                "Match has no wildcard branch; exhaustiveness depends on runtime values",
                Severity::Hint,
                line,
                col,
                source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                Some("_ => { ... }".to_string()),
            ));
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn match_has_unconditional_catchall(node: Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != NK::MATCH_CASE || child.child_by_field_name("guard").is_some() {
            continue;
        }

        let mut case_cursor = child.walk();
        for case_child in child.children(&mut case_cursor) {
            if (case_child.kind() == NK::PATTERN || case_child.kind() == NK::PATTERN_OR)
                && pattern_is_catchall(case_child)
            {
                return true;
            }
        }
    }
    false
}

/// A pattern is a catch-all if it contains a wildcard `_` or a bare variable binding.
fn pattern_is_catchall(node: Node) -> bool {
    if node.kind() == NK::PATTERN_WILDCARD || node.kind() == NK::PATTERN_VAR {
        return true;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if pattern_is_catchall(child) {
            return true;
        }
    }
    false
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
        assert_eq!(extract_line_col("Unexpected token 'x' at line 3, column 5"), (3, 5));
        assert_eq!(extract_line_col("Expected expression at line 1, column 10"), (1, 10));
        assert_eq!(extract_line_col("Some error"), (1, 1));
    }

    #[test]
    fn test_scope_tracker_basic() {
        let mut tracker = ScopeTracker::new(false);
        tracker.define("x", 1, 1, DefKind::Local);
        assert!(tracker.is_defined("x"));
        assert!(!tracker.is_defined("y"));
    }

    #[test]
    fn test_scope_tracker_nested() {
        let mut tracker = ScopeTracker::new(false);
        tracker.define("x", 1, 1, DefKind::Local);
        tracker.push_scope();
        tracker.define("y", 2, 1, DefKind::Local);
        assert!(tracker.is_defined("x"));
        assert!(tracker.is_defined("y"));
        tracker.pop_scope(&mut Vec::new(), &[]);
        assert!(tracker.is_defined("x"));
        assert!(!tracker.is_defined("y"));
    }

    #[test]
    fn test_semantic_simple() {
        let source = "x = 1\ny = x + 1";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, true, &mut diags);
        let errors: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert!(errors.is_empty(), "Unexpected E200: {:?}", errors);
    }

    #[test]
    fn test_semantic_undefined() {
        let source = "y = x + 1";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, true, &mut diags);
        let errors: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("'x'"));
    }

    #[test]
    fn test_semantic_unused_global_ignored() {
        // Global scope: symbols may be used externally (module API)
        let source = "x = 1";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, true, &mut diags);
        let warnings: Vec<_> = diags.iter().filter(|d| d.code == "W200").collect();
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_semantic_unused_local() {
        // Local scope: unused variable should warn
        let source = "f = () => { y = 1\n2 }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, true, &mut diags);
        let warnings: Vec<_> = diags.iter().filter(|d| d.code == "W200").collect();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("'y'"));
    }

    #[test]
    fn test_semantic_underscore_ignored() {
        let source = "_x = 1";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, true, &mut diags);
        let warnings: Vec<_> = diags.iter().filter(|d| d.code == "W200").collect();
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_semantic_builtins() {
        let source = "x = len(range(10))";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, true, &mut diags);
        let errors: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert!(errors.is_empty(), "Unexpected E200: {:?}", errors);
    }

    #[test]
    fn test_semantic_lambda_params() {
        let source = "f = (x, y) => { x + y }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, true, &mut diags);
        let errors: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert!(errors.is_empty(), "Unexpected E200: {:?}", errors);
    }

    #[test]
    fn test_semantic_for_loop() {
        let source = "for i in range(10) { len(range(i)) }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, true, &mut diags);
        let errors: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert!(errors.is_empty(), "Unexpected E200: {:?}", errors);
    }

    #[test]
    fn test_semantic_match_struct_pattern() {
        let source = "struct Point { x; y }\np = Point(1, 2)\nmatch p {\n    Point{x, y} => { x + y }\n}";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, true, &mut diags);
        let errors: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert!(errors.is_empty(), "Unexpected E200: {:?}", errors);
    }

    #[test]
    fn test_lint_full_pipeline() {
        let config = LintConfig::default();
        let result = lint_code("x = 1\nprint(x)", &config).unwrap();
        // Should not have E200 errors
        let errors: Vec<_> = result.iter().filter(|d| d.code == "E200").collect();
        assert!(errors.is_empty());
    }

    // --- I100: TCO detection ---

    #[test]
    fn test_tco_detection() {
        let source = "fact = (n) => { if n <= 1 { 1 } else { n * fact(n - 1) } }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
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
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I100").collect();
        assert!(hints.is_empty(), "Unexpected I100: {:?}", hints);
    }

    #[test]
    fn test_tco_tail_ok_with_trailing_comment_in_branch() {
        let source = "countdown = (n) => {\n    if n == 0 { \"Done\" } else {\n        print(n)\n        countdown(n - 1)  # tail call\n    }\n}";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I100").collect();
        assert!(hints.is_empty(), "Unexpected I100: {:?}", hints);
    }

    #[test]
    fn test_tco_tail_ok_with_trailing_comment_after_if_expr() {
        let source = "range_sum = (start, end, acc=0) => {\n    if start > end { acc }\n    else { range_sum(start + 1, end, acc + start) }  # tail call\n}";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I100").collect();
        assert!(hints.is_empty(), "Unexpected I100: {:?}", hints);
    }

    #[test]
    fn test_tco_tree_recursion_no_hint() {
        // Double recursion (divide & conquer) is not a TCO candidate
        let source = "hull_rec = (points, a, b) => {\n    if len(points) == 0 { list() }\n    else {\n        c = farthest(points, a, b)\n        hull_rec(left_of(points, a, c), a, c) + list(c) + hull_rec(left_of(points, c, b), c, b)\n    }\n}";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I100").collect();
        assert!(hints.is_empty(), "Tree recursion should not emit I100: {:?}", hints);
    }

    #[test]
    fn test_tco_fib_tree_recursion_no_hint() {
        // fib(n-1) + fib(n-2) is tree recursion, not a TCO candidate
        let source = "fib = (n) => {\n    if n <= 1 { n }\n    else { fib(n - 1) + fib(n - 2) }\n}";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I100").collect();
        assert!(hints.is_empty(), "Tree recursion should not emit I100: {:?}", hints);
    }

    #[test]
    fn test_tco_exclusive_branches_still_hint() {
        // Two recursive calls in exclusive if/else branches: each runs alone,
        // so the non-tail one is still a valid I100 candidate
        let source = "f = (n) => {\n    if n > 0 { f(n - 1) } else { 1 + f(n + 1) }\n}";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I100").collect();
        assert_eq!(
            hints.len(),
            1,
            "Non-tail call in else branch should emit I100: {:?}",
            hints
        );
    }

    #[test]
    fn test_tco_short_circuit_not_tree_recursion() {
        // f(n-1) or f(n-2): short-circuit means exclusive branches, not tree recursion
        let source = "f = (n) => {\n    if n == 0 { 0 } else { f(n - 1) or f(n - 2) }\n}";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I100").collect();
        // Both calls are non-tail (wrapped in bool_or), should emit I100
        assert!(
            !hints.is_empty(),
            "Short-circuit calls should still emit I100: {:?}",
            hints
        );
    }

    #[test]
    fn test_tco_nested_lambda_not_tree_recursion() {
        // f(n-1) next to a lambda containing f -- the lambda f is a different scope
        let source = "f = (n) => {\n    if n == 0 { 0 } else { helper(f(n - 1), () => { f(0) }) }\n}";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I100").collect();
        // f(n-1) is non-tail (inside helper() call), should emit I100
        // f(0) in the lambda is a different scope, should NOT make this tree recursion
        assert!(
            !hints.is_empty(),
            "Call next to nested lambda should still emit I100: {:?}",
            hints
        );
    }

    #[test]
    fn test_tco_no_recursion() {
        let source = "f = (x) => { x + 1 }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
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
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
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
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
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
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
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
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I102").collect();
        assert_eq!(hints.len(), 1, "Expected 1 I102 hint, got: {:?}", hints);
    }

    #[test]
    fn test_different_assignment_no_warning() {
        let source = "x = 1\ny = x";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
        let hints: Vec<_> = diags.iter().filter(|d| d.code == "I102").collect();
        assert!(hints.is_empty());
    }

    #[test]
    fn test_meta_builtin() {
        let source = "x = META.file";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, true, &mut diags);
        let errors: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert!(errors.is_empty(), "Unexpected E200: {:?}", errors);
    }

    #[test]
    fn test_selective_import_defines_name() {
        let source = "import(\"pathlib\", \"Path\")\nPath(\".\")";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_semantic(tree.root_node(), source, &lines, true, &mut diags);
        let errors: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert!(errors.is_empty(), "Unexpected E200: {:?}", errors);
    }

    // --- Custom threshold tests ---

    fn config_with_thresholds(nesting: usize, complexity: usize, length: usize, params: usize) -> LintConfig {
        LintConfig {
            max_nesting_depth: nesting,
            max_cyclomatic_complexity: complexity,
            max_function_length: length,
            max_parameters: params,
            ..Default::default()
        }
    }

    #[test]
    fn test_nesting_depth_custom_threshold() {
        // depth 2: if > for -- should trigger at threshold 1 but not at 5
        let source = "if True { for i in range(10) { i } }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();

        let mut diags = Vec::new();
        check_nesting_depth(tree.root_node(), &lines, 1, &mut diags);
        assert_eq!(
            diags.iter().filter(|d| d.code == "I200").count(),
            1,
            "depth 2 should exceed threshold 1"
        );

        let mut diags = Vec::new();
        check_nesting_depth(tree.root_node(), &lines, 5, &mut diags);
        assert!(
            diags.iter().filter(|d| d.code == "I200").count() == 0,
            "depth 2 should not exceed threshold 5"
        );
    }

    #[test]
    fn test_nesting_depth_disabled_when_zero() {
        let source = "if True { if True { if True { if True { if True { if True { 1 } } } } } }";
        let config = config_with_thresholds(0, 10, 30, 6);
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &config, &mut diags);
        assert!(
            diags.iter().filter(|d| d.code == "I200").count() == 0,
            "nesting check should be disabled when 0"
        );
    }

    #[test]
    fn test_cyclomatic_complexity_custom_threshold() {
        // 4 branches = complexity 5 (1 base + 4 if)
        let source = "f = (x) => { if x > 1 { 1 } elif x > 2 { 2 } elif x > 3 { 3 } elif x > 4 { 4 } else { 5 } }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();

        let mut diags = Vec::new();
        check_cyclomatic_complexity(tree.root_node(), &lines, 3, &mut diags);
        assert_eq!(
            diags.iter().filter(|d| d.code == "I201").count(),
            1,
            "complexity should exceed threshold 3"
        );

        let mut diags = Vec::new();
        check_cyclomatic_complexity(tree.root_node(), &lines, 10, &mut diags);
        assert!(
            diags.iter().filter(|d| d.code == "I201").count() == 0,
            "complexity should not exceed threshold 10"
        );
    }

    #[test]
    fn test_cyclomatic_complexity_disabled_when_zero() {
        let source = "f = (x) => { if x > 1 { 1 } elif x > 2 { 2 } elif x > 3 { 3 } elif x > 4 { 4 } else { 5 } }";
        let config = config_with_thresholds(5, 0, 30, 6);
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &config, &mut diags);
        assert!(
            diags.iter().filter(|d| d.code == "I201").count() == 0,
            "complexity check should be disabled when 0"
        );
    }

    #[test]
    fn test_function_length_custom_threshold() {
        // 4 statements
        let source = "f = () => {\na = 1\nb = 2\nc = 3\nd = 4\n}";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();

        let mut diags = Vec::new();
        check_function_length(tree.root_node(), &lines, 3, &mut diags);
        assert_eq!(
            diags.iter().filter(|d| d.code == "I202").count(),
            1,
            "4 statements should exceed threshold 3"
        );

        let mut diags = Vec::new();
        check_function_length(tree.root_node(), &lines, 30, &mut diags);
        assert!(
            diags.iter().filter(|d| d.code == "I202").count() == 0,
            "4 statements should not exceed threshold 30"
        );
    }

    #[test]
    fn test_function_length_disabled_when_zero() {
        let source = "f = () => {\na = 1\nb = 2\nc = 3\nd = 4\n}";
        let config = config_with_thresholds(5, 10, 0, 6);
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &config, &mut diags);
        assert!(
            diags.iter().filter(|d| d.code == "I202").count() == 0,
            "length check should be disabled when 0"
        );
    }

    #[test]
    fn test_too_many_parameters_custom_threshold() {
        let source = "f = (a, b, c) => { a + b + c }";
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();

        let mut diags = Vec::new();
        check_too_many_parameters(tree.root_node(), source, &lines, 2, &mut diags);
        assert_eq!(
            diags.iter().filter(|d| d.code == "I203").count(),
            1,
            "3 params should exceed threshold 2"
        );

        let mut diags = Vec::new();
        check_too_many_parameters(tree.root_node(), source, &lines, 6, &mut diags);
        assert!(
            diags.iter().filter(|d| d.code == "I203").count() == 0,
            "3 params should not exceed threshold 6"
        );
    }

    #[test]
    fn test_too_many_parameters_disabled_when_zero() {
        let source = "f = (a, b, c, d, e, f, g, h) => { a }";
        let config = config_with_thresholds(5, 10, 30, 0);
        let tree = parse_silent(source).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut diags = Vec::new();
        check_improvements(tree.root_node(), source, &lines, &config, &mut diags);
        assert!(
            diags.iter().filter(|d| d.code == "I203").count() == 0,
            "params check should be disabled when 0"
        );
    }

    // --- noqa suppression ---

    #[test]
    fn test_noqa_bare_suppresses_all() {
        let source = "y = x + 1 # noqa";
        let config = LintConfig {
            check_style: false,
            check_names: true,
            ..Default::default()
        };
        let diags = lint_code(source, &config).unwrap();
        assert!(diags.is_empty(), "Expected all suppressed, got: {:?}", diags);
    }

    #[test]
    fn test_noqa_specific_code() {
        let source = "y = x + 1 # noqa: E200";
        let config = LintConfig {
            check_style: false,
            check_names: true,
            ..Default::default()
        };
        let diags = lint_code(source, &config).unwrap();
        let e300: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert!(e300.is_empty(), "E200 should be suppressed");
    }

    #[test]
    fn test_noqa_wrong_code_not_suppressed() {
        let source = "y = x + 1 # noqa: W200";
        let config = LintConfig {
            check_style: false,
            check_names: true,
            ..Default::default()
        };
        let diags = lint_code(source, &config).unwrap();
        let e300: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert_eq!(e300.len(), 1, "E200 should NOT be suppressed by W200 noqa");
    }

    #[test]
    fn test_noqa_multiple_codes() {
        let source = "y = x + 1 # noqa: E200, W200";
        let config = LintConfig {
            check_style: false,
            check_names: true,
            ..Default::default()
        };
        let diags = lint_code(source, &config).unwrap();
        let e300: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert!(e300.is_empty(), "E200 should be suppressed");
    }

    #[test]
    fn test_noqa_does_not_affect_other_lines() {
        let source = "y = x + 1 # noqa\nz = w + 2";
        let config = LintConfig {
            check_style: false,
            check_names: true,
            ..Default::default()
        };
        let diags = lint_code(source, &config).unwrap();
        let e300: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert_eq!(e300.len(), 1, "Line 2 E200 should remain");
        assert_eq!(e300[0].line, 2);
    }

    #[test]
    fn test_noqa_in_string_does_not_suppress() {
        // "# noqa" inside a string must not suppress diagnostics on that line
        let source = "f = () => { y = \"# noqa\"; 1 }";
        let config = LintConfig {
            check_style: false,
            ..Default::default()
        };
        let diags = lint_code(source, &config).unwrap();
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W200").collect();
        assert_eq!(w310.len(), 1, "W200 should NOT be suppressed by # noqa in string");
    }

    #[test]
    fn test_noqa_code_with_trailing_reason() {
        let source = "y = x + 1 # noqa: E200 -- false positive";
        let config = LintConfig {
            check_style: false,
            check_names: true,
            ..Default::default()
        };
        let diags = lint_code(source, &config).unwrap();
        let e300: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert!(e300.is_empty(), "E200 should be suppressed even with trailing reason");
    }

    #[test]
    fn test_noqa_not_a_directive() {
        // "# noqa123" is not a valid noqa directive
        let source = "y = x + 1 # noqa123";
        let config = LintConfig {
            check_style: false,
            check_names: true,
            ..Default::default()
        };
        let diags = lint_code(source, &config).unwrap();
        let e300: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
        assert_eq!(e300.len(), 1, "noqa123 should not suppress anything");
    }
}
