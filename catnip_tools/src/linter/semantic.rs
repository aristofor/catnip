// FILE: catnip_tools/src/linter/semantic.rs
use super::*;

// --- Phase 3: Semantic (CST walk) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DefKind {
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
    /// True for a real function/lambda boundary, false for a control-flow
    /// block (`for`/`match`/`except`). An assignment to a name defined above a
    /// block boundary but within the same function is a write-through, not a
    /// shadowing new binding.
    is_function: bool,
}

/// Where an assignment target resolves relative to function boundaries.
enum AssignTarget {
    /// Not defined in any enclosing scope -- a fresh local.
    NotDefined,
    /// Defined higher up but within the current function (only block scopes
    /// crossed) -- a write-through, never a shadow.
    SameFunction,
    /// Defined beyond a function boundary (a captured/closed-over name).
    AcrossFunction,
}

pub(crate) struct ScopeTracker {
    scopes: Vec<ScopeFrame>,
    check_names: bool,
}

impl ScopeTracker {
    pub(crate) fn new(check_names: bool) -> Self {
        Self {
            // The module top-level is the root function boundary.
            scopes: vec![ScopeFrame {
                is_function: true,
                ..Default::default()
            }],
            check_names,
        }
    }

    pub(crate) fn push_scope(&mut self, is_function: bool) {
        self.scopes.push(ScopeFrame {
            is_function,
            ..Default::default()
        });
    }

    pub(crate) fn pop_scope(&mut self, diagnostics: &mut Vec<Diagnostic>, source_lines: &[&str]) {
        if self.scopes.len() > 1 {
            if let Some(frame) = self.scopes.pop() {
                self.emit_unused_diagnostics(frame, diagnostics, source_lines);
            }
        }
    }

    pub(crate) fn define(&mut self, name: &str, line: usize, column: usize, kind: DefKind) {
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

    /// Classify an assignment target (already known not to be in the current
    /// scope). Walks scopes inner-to-outer; a name found before crossing any
    /// function boundary is a write-through within the same function, one found
    /// after crossing is a captured name subject to the reading rule. The
    /// `contains` check runs before marking the crossing for a given scope, so
    /// the current function's own params/locals count as same-function.
    fn resolve_assignment(&self, name: &str) -> AssignTarget {
        let mut crossed_function = false;
        for scope in self.scopes.iter().rev() {
            if scope.names.contains(name) {
                return if crossed_function {
                    AssignTarget::AcrossFunction
                } else {
                    AssignTarget::SameFunction
                };
            }
            if scope.is_function {
                crossed_function = true;
            }
        }
        AssignTarget::NotDefined
    }

    fn use_name(&mut self, name: &str) {
        for depth in (0..self.scopes.len()).rev() {
            if self.scopes[depth].names.contains(name) {
                self.scopes[depth].used.insert(name.to_string());
                return;
            }
        }
    }

    pub(crate) fn is_defined(&self, name: &str) -> bool {
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

pub(crate) fn check_semantic(
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

        NK::STRUCT_STMT | NK::TRAIT_STMT | NK::UNION_STMT | NK::ENUM_STMT => {
            // Define the type name, but don't walk body children: fields,
            // variants, methods, and `self` live in the type's own namespace
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

        NK::PRAGMA_QUALIFIED => {
            // `ND.process`: the namespace is a name reference, but the attr is
            // a pragma value (like a getattr attribute), not a name to resolve.
            if let Some(ns) = node.child_by_field_name("namespace") {
                walk_node(ns, source, tracker, diagnostics, source_lines);
            }
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
        // Named functions are visible inside their own body (Catnip supports
        // named recursion: `f = (n) => { f(n - 1) }`). Bind the name before
        // walking a lambda RHS so the recursive reference resolves.
        if rv.kind() == NK::LAMBDA_EXPR {
            resolve_assignment_bindings(&bindings, Some(rv), source, tracker, diagnostics, source_lines);
            walk_node(rv, source, tracker, diagnostics, source_lines);
        } else {
            walk_node(rv, source, tracker, diagnostics, source_lines);
            resolve_assignment_bindings(&bindings, Some(rv), source, tracker, diagnostics, source_lines);
        }
    } else {
        resolve_assignment_bindings(&bindings, None, source, tracker, diagnostics, source_lines);
    }
}

/// Resolve assignment bindings against the current scope: emit W203 keyword
/// warnings, then define locals / detect W204 shadows per the write-through
/// reading rule. `rvalue` is the RHS node (used to check whether a
/// cross-function write mutates a captured name); `None` for bare bindings.
fn resolve_assignment_bindings(
    bindings: &[BindingInfo],
    rvalue: Option<Node>,
    source: &str,
    tracker: &mut ScopeTracker,
    diagnostics: &mut Vec<Diagnostic>,
    source_lines: &[&str],
) {
    for binding in bindings {
        check_keyword_name(&binding.name, binding.line, binding.column, source_lines, diagnostics);

        if tracker.is_defined_in_current_scope(&binding.name) {
            continue;
        }

        match tracker.resolve_assignment(&binding.name) {
            // Write-through to a block-scoped parent of the same function
            // (e.g. `acc = True` inside a `for`). Not a new binding, not a
            // shadow -- the outer def stays and later reads mark it used.
            AssignTarget::SameFunction => {}
            // Beyond a function boundary (a captured name). Reading rule: a
            // write-through only if the function also reads the name;
            // otherwise a fresh local that shadows the captured one.
            AssignTarget::AcrossFunction => {
                let mutates_capture =
                    rvalue.is_some_and(|rv| subtree_reads_identifier_skip_nested_lambdas(rv, &binding.name, source));
                if !mutates_capture {
                    emit_shadow_w204(&binding.name, binding.line, binding.column, source_lines, diagnostics);
                    tracker.define_local(&binding.name, binding.line, binding.column);
                }
            }
            AssignTarget::NotDefined => {
                tracker.define_local(&binding.name, binding.line, binding.column);
            }
        }
    }
}

/// Emit W204 (shadows an outer-scope variable), unless the name is a
/// `_`-prefixed throwaway.
fn emit_shadow_w204(name: &str, line: usize, col: usize, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    if name == "_" || name.starts_with('_') {
        return;
    }
    diagnostics.push(make_diagnostic(
        "W204",
        &format!("'{}' shadows variable from outer scope", name),
        Severity::Warning,
        line,
        col,
        source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
        None,
    ));
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
    tracker.push_scope(true);

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
    tracker.push_scope(false);

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
    tracker.push_scope(false);

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
    tracker.push_scope(false);

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
