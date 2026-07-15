// FILE: catnip_tools/src/linter/improvements.rs
use super::*;

// --- Phase 4: Improvement suggestions ---

pub(crate) fn check_improvements(
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
    check_loop_var_never_modified(root, source, source_lines, diagnostics);
    check_subscript_under_coalesce(root, source, source_lines, diagnostics);
    check_broadcast_side_effects(root, source, source_lines, diagnostics);
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
    NK::NULL_COALESCE,
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

/// W303: a `while <identifier>` loop whose condition variable is never
/// modified in the body. Deliberately narrow to stay false-positive-free:
/// the condition must be a single bare identifier (so `while True`, handled
/// by W302, and comparisons like `while i < n` are out of scope), and the
/// body must contain nothing that could change the variable or leave the
/// loop -- no function calls (a called closure could mutate a captured var),
/// no nested lambda, no `for`/`match`/`try` (whose targets could rebind it),
/// no rebinding assignment, and no `break`/`return`/`raise`. Under those
/// constraints the condition value is invariant, so the loop runs forever
/// (or never). Severity Info: the runtime truthiness is unknown.
fn check_loop_var_never_modified(root: Node, source: &str, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let bytes = source.as_bytes();
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == NK::WHILE_STMT {
            if let Some((name, body)) = while_single_ident_condition(current, bytes) {
                if !body_may_change_or_exit(body, name, bytes) {
                    let line = current.start_position().row + 1;
                    let col = current.start_position().column + 1;
                    diagnostics.push(make_diagnostic(
                        "W303",
                        &format!(
                            "Loop condition '{}' is never modified in the body; the loop cannot terminate normally",
                            name
                        ),
                        Severity::Info,
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

/// If `node` is a `while` whose condition is a single bare identifier, return
/// (identifier name, body block). `while_stmt` is `'while' expr block`.
fn while_single_ident_condition<'a>(node: Node<'a>, bytes: &'a [u8]) -> Option<(&'a str, Node<'a>)> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    let cond = children
        .iter()
        .copied()
        .find(|c| c.is_named() && c.kind() != NK::BLOCK)?;
    if cond.kind() != NK::IDENTIFIER {
        return None;
    }
    let body = children.iter().copied().find(|c| c.kind() == NK::BLOCK)?;
    Some((cond.utf8_text(bytes).unwrap_or(""), body))
}

/// True when the loop body could change `name` or exit the loop, making the
/// "never modified" conclusion unsafe. Conservative: any call, lambda,
/// `for`/`match`/`try`, exit statement, or assignment writing `name` counts.
fn body_may_change_or_exit(body: Node, name: &str, bytes: &[u8]) -> bool {
    let mut stack = vec![body];
    while let Some(cur) = stack.pop() {
        match cur.kind() {
            NK::CALL
            | NK::CALLATTR
            | NK::LAMBDA_EXPR
            | NK::FOR_STMT
            | NK::MATCH_EXPR
            | NK::TRY_STMT
            | NK::BREAK_STMT
            | NK::RETURN_STMT
            | NK::RAISE_STMT => return true,
            NK::ASSIGNMENT if assignment_writes_name(cur, name, bytes) => return true,
            _ => {}
        }
        let mut cursor = cur.walk();
        for child in cur.children(&mut cursor) {
            stack.push(child);
        }
    }
    false
}

/// True if `name` appears as an LHS target of `assignment`. LHS is everything
/// left of the last `=` token (`assignment = lvalue ('=' lvalue)* '=' expr`).
fn assignment_writes_name(node: Node, name: &str, bytes: &[u8]) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    let Some(last_eq) = children
        .iter()
        .rposition(|c| !c.is_named() && c.utf8_text(bytes).unwrap_or("") == "=")
    else {
        return false;
    };
    children[..last_eq].iter().any(|c| ident_in_subtree(*c, name, bytes))
}

/// True if any identifier in `node`'s subtree has text `name`.
fn ident_in_subtree(node: Node, name: &str, bytes: &[u8]) -> bool {
    if node.kind() == NK::IDENTIFIER {
        return node.utf8_text(bytes).unwrap_or("") == name;
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    children.into_iter().any(|c| ident_in_subtree(c, name, bytes))
}

/// W304: a string-keyed subscript as non-final operand of `??`. The operator
/// only coalesces None -- `d['k'] ?? default` still raises KeyError when the
/// key is absent, so it is not a "get with default". `d.get('k') ?? default`
/// covers both absence and null. Every operand except the chain's final
/// fallback is checked (left-assoc: the middle operand of `a ?? d['k'] ?? x`
/// sits as right child of a nested `??`). Scope is syntactic and deliberately
/// narrow: only an access whose final member is an index with a single
/// string/fstring literal subscript (the dict-access pattern); computed keys
/// and integer indices are out of scope. The heuristic is type-blind: a
/// container whose `__getitem__` tolerates missing keys (e.g. defaultdict)
/// is flagged too, and the suggested rewrite would skip its default_factory.
fn check_subscript_under_coalesce(root: Node, source: &str, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == NK::NULL_COALESCE {
            let mut cursor = current.walk();
            let operands: Vec<Node> = current.named_children(&mut cursor).filter(|n| !n.is_extra()).collect();
            // The right operand is the chain's final fallback unless this
            // `??` is nested as left operand of an enclosing one.
            let in_chain = current.parent().is_some_and(|p| p.kind() == NK::NULL_COALESCE);
            let checked = if in_chain {
                operands.len()
            } else {
                operands.len().min(1)
            };
            for operand in &operands[..checked] {
                if let Some(key) = string_subscript_key(*operand, source) {
                    let line = operand.start_position().row + 1;
                    let col = operand.start_position().column + 1;
                    diagnostics.push(make_diagnostic(
                        "W304",
                        &format!("'??' only coalesces None: [{key}] still raises KeyError when the key is missing"),
                        Severity::Warning,
                        line,
                        col,
                        source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                        Some(format!("use .get({key}) ?? default")),
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

/// If `operand` is a chained access whose final member is an index with a
/// single string/fstring literal subscript, return the key text. Comment
/// nodes are extras that may appear anywhere, so they are filtered out
/// before counting children.
fn string_subscript_key<'a>(operand: Node, source: &'a str) -> Option<&'a str> {
    if operand.kind() != NK::CHAINED {
        return None;
    }
    let mut cursor = operand.walk();
    let last = operand.named_children(&mut cursor).filter(|n| !n.is_extra()).last()?;
    if last.kind() != NK::INDEX {
        return None;
    }
    let mut cursor = last.walk();
    let mut subs = last.named_children(&mut cursor).filter(|n| !n.is_extra());
    let sub = subs.next()?;
    if subs.next().is_some() {
        return None; // multi-subscript = ND indexing, not a dict access
    }
    if sub.kind() != NK::LITERAL {
        return None;
    }
    let inner = sub.named_child(0)?;
    if inner.kind() != NK::STRING && inner.kind() != NK::FSTRING {
        return None;
    }
    sub.utf8_text(source.as_bytes()).ok()
}

/// Builtins with observable side effects (I/O, debugger). Broadcasting these
/// runs them once per element, in unspecified order under thread-mode ND
/// broadcast -- almost always a mistake. Kept to an unambiguous allowlist to
/// avoid false positives; user functions and unknown builtins are not flagged.
const BROADCAST_IMPURE_BUILTINS: &[&str] = &["print", "input", "open", "breakpoint"];

/// W401: a call to a known-impure builtin inside a broadcast (`.[ ... ]`),
/// but only when the file opts into a parallel ND mode. Broadcast is
/// **sequential by default** (ordered execution), so side effects are
/// well-defined; the order is only unspecified under
/// `pragma("nd_mode", ND.thread)` / `ND.process`, where per-element side
/// effects race and reorder. Gating on the pragma avoids flagging the common
/// (and idiomatic) sequential `data.[(x) => { print(x) }]` pattern.
fn check_broadcast_side_effects(root: Node, source: &str, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let bytes = source.as_bytes();
    if !file_enables_parallel_nd(root, bytes) {
        return;
    }
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.kind() == "broadcast" {
            report_impure_calls_in_broadcast(current, bytes, source_lines, diagnostics);
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

/// True when the file sets a non-sequential ND broadcast mode via
/// `pragma("nd_mode", ND.thread)` or `ND.process` (pragmas are file-scoped).
/// Under `ND.sequential` (the default) broadcast order is well-defined.
fn file_enables_parallel_nd(root: Node, bytes: &[u8]) -> bool {
    let mut stack = vec![root];
    while let Some(cur) = stack.pop() {
        if cur.kind() == "pragma_stmt" && pragma_sets_parallel_nd(cur, bytes) {
            return true;
        }
        let mut cursor = cur.walk();
        for child in cur.children(&mut cursor) {
            stack.push(child);
        }
    }
    false
}

/// True if `pragma_stmt` is `pragma("nd_mode", <thread|process>)`.
fn pragma_sets_parallel_nd(node: Node, bytes: &[u8]) -> bool {
    let mut cursor = node.walk();
    let args: Vec<Node> = node
        .children(&mut cursor)
        .filter(|c| c.kind() == "pragma_arg")
        .collect();
    if args.len() < 2 {
        return false;
    }
    if pragma_arg_value(args[0], bytes).as_deref() != Some("nd_mode") {
        return false;
    }
    matches!(
        pragma_arg_value(args[1], bytes).as_deref(),
        Some("thread") | Some("process")
    )
}

/// Extract a pragma argument's value: the unquoted string, or the trailing
/// name of a qualified value (`thread` from `ND.thread`), or a bare identifier.
fn pragma_arg_value(arg: Node, bytes: &[u8]) -> Option<String> {
    let mut cursor = arg.walk();
    let inner = arg.children(&mut cursor).find(|c| c.is_named())?;
    match inner.kind() {
        NK::STRING => Some(inner.utf8_text(bytes).ok()?.trim_matches(['"', '\'']).to_string()),
        NK::IDENTIFIER => Some(inner.utf8_text(bytes).ok()?.to_string()),
        "pragma_qualified" => {
            let mut c = inner.walk();
            let idents: Vec<Node> = inner.children(&mut c).filter(|n| n.kind() == NK::IDENTIFIER).collect();
            idents.last().and_then(|n| n.utf8_text(bytes).ok()).map(str::to_string)
        }
        _ => None,
    }
}

/// Scan a broadcast's operand for impure-builtin calls. Nested broadcasts are
/// skipped here -- the outer tree walk visits them, so each call is reported
/// once.
fn report_impure_calls_in_broadcast(
    broadcast: Node,
    bytes: &[u8],
    source_lines: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut stack = Vec::new();
    let mut cursor = broadcast.walk();
    for child in broadcast.children(&mut cursor) {
        stack.push(child);
    }
    while let Some(cur) = stack.pop() {
        if cur.kind() == "broadcast" {
            continue; // handled by the top-level walk
        }
        if cur.kind() == NK::CALL {
            if let Some(callee) = call_callee_name(cur, bytes) {
                if BROADCAST_IMPURE_BUILTINS.contains(&callee.0) {
                    let line = callee.1.start_position().row + 1;
                    let col = callee.1.start_position().column + 1;
                    diagnostics.push(make_diagnostic(
                        "W401",
                        &format!(
                            "Side effect in broadcast: '{}' is impure; it runs per element in unspecified order",
                            callee.0
                        ),
                        Severity::Warning,
                        line,
                        col,
                        source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                        None,
                    ));
                }
            }
        }
        let mut c = cur.walk();
        for child in cur.children(&mut c) {
            stack.push(child);
        }
    }
}

/// For a `call` node (`atom arguments`), return the callee name and node when
/// the callee is a bare identifier (a direct function call), else None.
fn call_callee_name<'a>(call: Node<'a>, bytes: &'a [u8]) -> Option<(&'a str, Node<'a>)> {
    let mut cursor = call.walk();
    let callee = call.children(&mut cursor).find(|c| c.is_named())?;
    if callee.kind() == NK::IDENTIFIER {
        Some((callee.utf8_text(bytes).unwrap_or(""), callee))
    } else {
        None
    }
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

pub(crate) fn check_nesting_depth(
    root: Node,
    source_lines: &[&str],
    max_depth: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
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

pub(crate) fn check_cyclomatic_complexity(
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

pub(crate) fn check_function_length(
    root: Node,
    source_lines: &[&str],
    max_length: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
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

pub(crate) fn check_too_many_parameters(
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

/// A pattern is an irrefutable catch-all when it is `_` (`pattern_wildcard`)
/// or a bare variable binding (`pattern_var`), or an or-pattern with such an
/// alternative. Container patterns (tuple, struct, enum) stay refutable even
/// when they bind variables -- `(a, b)` requires a 2-tuple -- so only the
/// `pattern`/`pattern_or` wrappers are transparent. Mirrors `catnip_core`'s
/// `is_catchall` (the semantic exhaustiveness source of truth).
fn pattern_is_catchall(node: Node) -> bool {
    match node.kind() {
        NK::PATTERN_WILDCARD | NK::PATTERN_VAR => true,
        NK::PATTERN | NK::PATTERN_OR => {
            let mut cursor = node.walk();
            let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
            children.into_iter().any(pattern_is_catchall)
        }
        _ => false,
    }
}
