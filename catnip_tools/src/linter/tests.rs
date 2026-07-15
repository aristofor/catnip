//! Tests for the linter (syntax, style, semantic diagnostics).

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
    tracker.push_scope(false);
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

fn e200_errors(source: &str) -> Vec<Diagnostic> {
    let tree = parse_silent(source).unwrap();
    let lines: Vec<&str> = source.lines().collect();
    let mut diags = Vec::new();
    check_semantic(tree.root_node(), source, &lines, true, &mut diags);
    diags.into_iter().filter(|d| d.code == "E200").collect()
}

#[test]
fn test_semantic_union_body_not_name_checked() {
    // Union type name, variants, `self`, and method-internal refs live in the
    // union's own namespace -- like struct/trait bodies, they are not walked.
    let source = "union K {\n    a; b\n\n    label(self): str => {\n        match self { K.a => { \"a\" } _ => { \"b\" } }\n    }\n}\nx = K.a";
    let errors = e200_errors(source);
    assert!(errors.is_empty(), "Unexpected E200 on union: {:?}", errors);
}

#[test]
fn test_semantic_enum_body_not_name_checked() {
    let source = "enum Color { red; green; blue }\nc = Color.red";
    let errors = e200_errors(source);
    assert!(errors.is_empty(), "Unexpected E200 on enum: {:?}", errors);
}

#[test]
fn test_semantic_named_recursion_resolves() {
    // A named function is visible inside its own body (named recursion).
    let source = "f = (n) => { if n <= 0 { 0 } else { f(n - 1) } }\nf(3)";
    let errors = e200_errors(source);
    assert!(errors.is_empty(), "Unexpected E200 on recursion: {:?}", errors);
}

#[test]
fn test_semantic_named_recursion_still_flags_other_undefined() {
    // The recursion fix must not suppress genuinely undefined names.
    let source = "f = (n) => { f(n) + zzz }\nf(1)";
    let errors = e200_errors(source);
    assert_eq!(errors.len(), 1, "Expected only 'zzz': {:?}", errors);
    assert!(errors[0].message.contains("'zzz'"));
}

#[test]
fn test_semantic_pragma_qualified_attr_not_name_checked() {
    // `ND.process`: the attr is a pragma value, not a name to resolve.
    let source = "pragma(\"nd_mode\", ND.process)\nxs = list(1, 2, 3)\nxs.[print(_)]";
    let errors = e200_errors(source);
    assert!(errors.is_empty(), "Unexpected E200 on pragma: {:?}", errors);
}

#[test]
fn test_semantic_pragma_qualified_flags_undefined_namespace() {
    // The namespace of a pragma-qualified value is still a real reference.
    let source = "pragma(\"m\", Foo.bar)";
    let errors = e200_errors(source);
    assert_eq!(errors.len(), 1, "Expected only 'Foo': {:?}", errors);
    assert!(errors[0].message.contains("'Foo'"));
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

fn i103_present(source: &str) -> bool {
    let tree = parse_silent(source).unwrap();
    let lines: Vec<&str> = source.lines().collect();
    let mut diags = Vec::new();
    check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
    diags.iter().any(|d| d.code == "I103")
}

#[test]
fn test_i103_bare_var_is_catchall() {
    // A bare variable pattern is an irrefutable catch-all -- no I103.
    assert!(!i103_present("match v {\n    1 => { 1 }\n    n => { 2 }\n}"));
    assert!(!i103_present("match v {\n    n => { 1 }\n}"));
}

#[test]
fn test_i103_tuple_pattern_not_catchall() {
    // A tuple pattern is refutable (requires a tuple of that arity) even
    // though it binds variables -- I103 must fire.
    assert!(i103_present("match v {\n    (a, b) => { 1 }\n}"));
}

#[test]
fn test_i103_or_pattern_with_bare_var_is_catchall() {
    // An or-pattern with a bare-var alternative covers everything -- no I103.
    assert!(!i103_present("match v {\n    1 | n => { 1 }\n}"));
}

// --- W303: loop condition variable never modified ---

fn has_code(source: &str, code: &str) -> bool {
    let tree = parse_silent(source).unwrap();
    let lines: Vec<&str> = source.lines().collect();
    let mut diags = Vec::new();
    check_improvements(tree.root_node(), source, &lines, &LintConfig::default(), &mut diags);
    diags.iter().any(|d| d.code == code)
}

#[test]
fn test_w303_loop_var_never_modified() {
    // `running` is the whole condition and the body never touches it -> W303.
    assert!(has_code("while running {\n    x = 1\n}", "W303"));
    // Reading the variable in the body is still "never modified".
    assert!(has_code("while running {\n    x = running\n}", "W303"));
}

#[test]
fn test_w303_no_fire_when_reassigned() {
    assert!(!has_code("while running {\n    running = False\n}", "W303"));
}

#[test]
fn test_w303_no_fire_with_call() {
    // A call could mutate the variable through a captured closure.
    assert!(!has_code("while running {\n    tick()\n}", "W303"));
}

#[test]
fn test_w303_no_fire_with_exit() {
    // break/return/raise let the loop terminate.
    assert!(!has_code("while running {\n    break\n}", "W303"));
    assert!(!has_code("while running {\n    return 1\n}", "W303"));
}

#[test]
fn test_w303_no_fire_for_non_identifier_condition() {
    // `while True` is W302's job; comparisons are out of W303's narrow scope.
    assert!(!has_code("while True {\n    x = 1\n}", "W303"));
    assert!(!has_code("while i < n {\n    x = 1\n}", "W303"));
}

// --- W304: string-keyed subscript under ?? ---

#[test]
fn test_w304_subscript_under_coalesce() {
    assert!(has_code("d['k'] ?? 1", "W304"));
    // attribute chain before the index still fires (last member is the subscript)
    assert!(has_code("item.properties['eo:cloud_cover'] ?? 100", "W304"));
    // nested chain: the final ['b'] is the risky access
    assert!(has_code("d['a']['b'] ?? 1", "W304"));
    // fstring keys are string keys too
    assert!(has_code("d[f'k{i}'] ?? 1", "W304"));
}

#[test]
fn test_w304_no_fire_on_get() {
    assert!(!has_code("d.get('k') ?? 1", "W304"));
}

#[test]
fn test_w304_no_fire_out_of_narrow_scope() {
    // computed keys and integer indices could be list accesses; skipped
    assert!(!has_code("d[k] ?? 1", "W304"));
    assert!(!has_code("lst[0] ?? 1", "W304"));
    // multi-subscript is ND indexing, not a dict access
    assert!(!has_code("m['a', 'b'] ?? 1", "W304"));
    // subscript not in last position: the guard targets the method result
    assert!(!has_code("d['k'].strip() ?? 1", "W304"));
}

#[test]
fn test_w304_chain_operands() {
    // middle operand of a chain: `?? 1` does not guard it against KeyError
    assert!(has_code("a ?? d['k'] ?? 1", "W304"));
    // the final fallback of a chain is ordinary code
    assert!(!has_code("a ?? b ?? d['k']", "W304"));
}

#[test]
fn test_w304_comment_in_brackets() {
    // comments are extras; they must not shift the subscript count
    assert!(has_code("d[ # cache\n'k'] ?? 1", "W304"));
    assert!(has_code("d['k' # note\n] ?? 1", "W304"));
}

#[test]
fn test_w304_no_fire_on_right_side() {
    // with no enclosing chain, the right side is the final fallback
    assert!(!has_code("x ?? d['k']", "W304"));
}

// --- W401: side effects in broadcast ---

const ND_THREAD: &str = "pragma(\"nd_mode\", ND.thread)\n";

#[test]
fn test_w401_impure_builtin_in_parallel_broadcast() {
    // Under thread mode the broadcast order is unspecified -> side effects warn.
    assert!(has_code(&format!("{ND_THREAD}nums.[print(_)]"), "W401"));
    assert!(has_code(&format!("{ND_THREAD}data.[open(_)]"), "W401"));
    assert!(has_code("pragma(\"nd_mode\", ND.process)\nnums.[print(_)]", "W401"));
}

#[test]
fn test_w401_no_fire_in_sequential_default() {
    // Broadcast is sequential (ordered) by default -- side effects are
    // well-defined, so the idiomatic `data.[(x) => { print(x) }]` is fine.
    assert!(!has_code("nums.[print(_)]", "W401"));
    assert!(!has_code("nums.[(x) => { print(x) }]", "W401"));
    assert!(!has_code("pragma(\"nd_mode\", ND.sequential)\nnums.[print(_)]", "W401"));
}

#[test]
fn test_w401_no_fire_for_pure_or_user_calls() {
    // Even under thread mode: pure builtin, user function, pure arithmetic.
    assert!(!has_code(&format!("{ND_THREAD}nums.[len(_)]"), "W401"));
    assert!(!has_code(&format!("{ND_THREAD}nums.[f(_)]"), "W401"));
    assert!(!has_code(&format!("{ND_THREAD}nums.[* 2]"), "W401"));
}

#[test]
fn test_w401_no_fire_outside_broadcast() {
    // A plain impure call (no broadcast) is fine even under thread mode.
    assert!(!has_code(&format!("{ND_THREAD}print(nums)"), "W401"));
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

// --- disabled_codes (config-level on/off) ---

#[test]
fn test_disabled_code_suppresses_semantic() {
    let source = "y = x + 1";
    let config = LintConfig {
        check_style: false,
        check_names: true,
        disabled_codes: ["E200".to_string()].into_iter().collect(),
        ..Default::default()
    };
    let diags = lint_code(source, &config).unwrap();
    assert!(
        diags.iter().all(|d| d.code != "E200"),
        "E200 should be globally disabled, got: {:?}",
        diags
    );
}

#[test]
fn test_disabled_code_other_remains() {
    let source = "y = x + 1";
    let config = LintConfig {
        check_style: false,
        check_names: true,
        disabled_codes: ["W200".to_string()].into_iter().collect(),
        ..Default::default()
    };
    let diags = lint_code(source, &config).unwrap();
    let e200: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
    assert_eq!(e200.len(), 1, "Disabling W200 must not touch E200");
}

#[test]
fn test_disabled_empty_no_effect() {
    let source = "y = x + 1";
    let config = LintConfig {
        check_style: false,
        check_names: true,
        ..Default::default()
    };
    let diags = lint_code(source, &config).unwrap();
    let e200: Vec<_> = diags.iter().filter(|d| d.code == "E200").collect();
    assert_eq!(e200.len(), 1, "Empty disabled_codes leaves diagnostics intact");
}

#[test]
fn test_disabled_code_suppresses_style() {
    // Trailing whitespace triggers W101; disabling it removes only W101.
    let source = "x = 1 \n";
    let config = LintConfig {
        disabled_codes: ["W101".to_string()].into_iter().collect(),
        ..Default::default()
    };
    let diags = lint_code(source, &config).unwrap();
    assert!(
        diags.iter().all(|d| d.code != "W101"),
        "W101 should be globally disabled, got: {:?}",
        diags
    );
}

// -- Bug B: a write-through assignment inside a control-flow block mutates the
// enclosing variable; it is neither a shadow (W204) nor a dead binding (W200) --

fn semantic_diags(source: &str) -> Vec<Diagnostic> {
    let tree = parse_silent(source).unwrap();
    let lines: Vec<&str> = source.lines().collect();
    let mut diags = Vec::new();
    check_semantic(tree.root_node(), source, &lines, true, &mut diags);
    diags
}

#[test]
fn test_semantic_write_through_in_for_no_false_positive() {
    let diags = semantic_diags(
        "contains = (xs) => {\n    found = False\n    for x in xs {\n        if x {\n            found = True\n        }\n    }\n    found\n}",
    );
    let w: Vec<_> = diags.iter().filter(|d| d.code == "W204" || d.code == "W200").collect();
    assert!(w.is_empty(), "write-through in for must not fire W204/W200: {:?}", w);
}

#[test]
fn test_semantic_write_through_in_match_no_false_positive() {
    let diags = semantic_diags(
        "f = (v) => {\n    r = 0\n    match v {\n        1 => { r = 1 }\n        _ => { r = 2 }\n    }\n    r\n}",
    );
    let w: Vec<_> = diags.iter().filter(|d| d.code == "W204" || d.code == "W200").collect();
    assert!(w.is_empty(), "write-through in match must not fire W204/W200: {:?}", w);
}

#[test]
fn test_semantic_captured_write_without_read_still_shadows() {
    // True positive preserved: assigning a captured global without reading it
    // is a fresh local that shadows it (the reading rule) -> W204.
    let diags = semantic_diags("c = 0\nreset = () => { c = 0 }");
    let w204: Vec<_> = diags.iter().filter(|d| d.code == "W204").collect();
    assert_eq!(
        w204.len(),
        1,
        "captured write-without-read must still shadow: {:?}",
        diags
    );
    assert!(w204[0].message.contains("'c'"));
}

#[test]
fn test_semantic_unused_for_var_still_detected() {
    // True positive preserved: a loop variable never used -> W200.
    let diags = semantic_diags("f = () => {\n    for i in range(10) {\n        print(\"hi\")\n    }\n}");
    let w200: Vec<_> = diags.iter().filter(|d| d.code == "W200").collect();
    assert!(
        w200.iter().any(|d| d.message.contains("'i'")),
        "unused for-var must fire W200: {:?}",
        diags
    );
}
