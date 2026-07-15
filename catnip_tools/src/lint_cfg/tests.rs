//! Tests for CFG-based lints.

use super::*;

fn parse(source: &str) -> tree_sitter::Tree {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&catnip_grammar::get_language()).unwrap();
    parser.parse(source, None).unwrap()
}

fn lint_deep(source: &str) -> Vec<Diagnostic> {
    let tree = parse(source);
    let config = LintConfig::default();
    check_deep(tree.root_node(), source, &config)
}

fn codes(diags: &[Diagnostic]) -> Vec<&str> {
    diags.iter().map(|d| d.code.as_str()).collect()
}

fn names(diags: &[Diagnostic]) -> Vec<String> {
    diags
        .iter()
        .map(|d| {
            // Extract variable name from "'name' may be uninitialized"
            d.message.split('\'').nth(1).unwrap_or("").to_string()
        })
        .collect()
}

// -- W310 positive cases --

#[test]
fn test_w310_if_without_else() {
    let diags = lint_deep("if cond {\n    x = 1\n}\nprint(x)");
    assert_eq!(codes(&diags), vec!["W310"]);
    assert_eq!(names(&diags), vec!["x"]);
}

#[test]
fn test_w310_elif_missing_branch() {
    let diags = lint_deep("if a {\n    x = 1\n} elif b {\n    x = 2\n}\nprint(x)");
    assert_eq!(codes(&diags), vec!["W310"]);
}

// -- W310 negative cases --

#[test]
fn test_w310_if_else_both_define() {
    let diags = lint_deep("if cond {\n    x = 1\n} else {\n    x = 2\n}\nprint(x)");
    assert!(diags.is_empty(), "expected no W310, got: {:?}", diags);
}

#[test]
fn test_w310_defined_before_if() {
    let diags = lint_deep("x = 0\nif cond {\n    x = 1\n}\nprint(x)");
    assert!(diags.is_empty(), "expected no W310, got: {:?}", diags);
}

#[test]
fn test_w310_union_nullary_variants_no_false_positive() {
    // Nullary union variants (`a; b; c`) are declarations, not variable reads.
    let diags = lint_deep("union V {\n    excellent; solide; faible\n}");
    assert!(diags.is_empty(), "expected no W310 on union variants, got: {:?}", diags);
}

#[test]
fn test_w310_enum_variants_no_false_positive() {
    let diags = lint_deep("enum Color {\n    red; green; blue\n}");
    assert!(diags.is_empty(), "expected no W310 on enum variants, got: {:?}", diags);
}

#[test]
fn test_w310_union_method_body_analyzed() {
    // A union method body is a real scope: use-before-def must still fire,
    // just like a struct method (the union/enum stmt itself is opaque).
    let src = "union U {\n    a; b\n\n    m(self) => {\n        if self {\n            y = 1\n        }\n        print(y)\n    }\n}";
    let diags = lint_deep(src);
    assert_eq!(codes(&diags), vec!["W310"]);
    assert_eq!(names(&diags), vec!["y"]);
}

#[test]
fn test_nested_lambda_in_union_method_captures_param() {
    // Regression (adversarial review): a lambda nested in a union method must
    // see the method's params as parent-visible captures, exactly like inside
    // a struct method -- mutating a captured param must not fire W310/W312.
    let union_src =
        "union U {\n    a; b\n\n    m(self, k) => {\n        step = () => { k = k + 1 }\n        step()\n    }\n}";
    let struct_src =
        "struct S {\n    a; b\n\n    m(self, k) => {\n        step = () => { k = k + 1 }\n        step()\n    }\n}";
    let union_diags = lint_deep(union_src);
    assert!(
        union_diags.is_empty(),
        "no false W310/W312 expected in union method, got: {:?}",
        union_diags
    );
    assert_eq!(
        codes(&union_diags),
        codes(&lint_deep(struct_src)),
        "union method must lint like struct method"
    );
}

#[test]
fn test_w310_no_warning_for_undefined() {
    // Variable never defined anywhere -> E200 territory, not W310.
    let diags = lint_deep("print(x)");
    assert!(diags.is_empty());
}

#[test]
fn test_w310_no_warning_for_builtins() {
    let diags = lint_deep("if cond {\n    x = 1\n}\nprint(len(x))");
    // Only x should trigger, not len or print.
    let ns = names(&diags);
    assert_eq!(ns, vec!["x"]);
}

#[test]
fn test_w310_while_loop() {
    let diags = lint_deep("while cond {\n    x = 1\n}\nprint(x)");
    // x is defined inside loop body which may not execute.
    assert_eq!(codes(&diags), vec!["W310"]);
}

#[test]
fn test_w310_for_loop_variable_safe() {
    // For loop variable is defined in the header, but the loop may not execute.
    // However, reads of the iterable are fine.
    let diags = lint_deep("items = [1, 2, 3]\nfor x in items {\n    print(x)\n}");
    assert!(diags.is_empty(), "expected no W310, got: {:?}", diags);
}

#[test]
fn test_w310_except_binding_no_false_positive() {
    // The except binding variable should be tracked as a def.
    let diags = lint_deep("try {\n    x = 1\n} except {\n    e: Error => {\n        print(e)\n    }\n}");
    // `e` is bound by the except clause -- no W310 for it.
    let ns = names(&diags);
    assert!(
        !ns.contains(&"e".to_string()),
        "except binding 'e' should not trigger W310, got: {:?}",
        ns
    );
}

#[test]
fn test_w310_except_binding_after_handler() {
    // Reading the except binding after the try/except block.
    // `e` is only defined on the except path, so W310 is correct here.
    let diags = lint_deep("try {\n    x = 1\n} except {\n    e: Error => {\n        y = 1\n    }\n}\nprint(e)");
    let ns = names(&diags);
    assert!(
        ns.contains(&"e".to_string()),
        "except binding outside handler should trigger W310, got: {:?}",
        ns
    );
}

// -- G4.1: try/except tests --

#[test]
fn test_w310_try_var_read_in_except() {
    // Variable defined in try only -- except may run before the def.
    let diags = lint_deep("try {\n    x = risky()\n} except {\n    _: Error => {\n        print(x)\n    }\n}");
    let ns = names(&diags);
    assert!(
        ns.contains(&"x".to_string()),
        "x defined in try should be possibly uninitialized in except, got: {:?}",
        ns
    );
}

#[test]
fn test_w310_try_except_both_define() {
    // Variable defined in both try and except -- safe after the block.
    let diags = lint_deep("try {\n    x = 1\n} except {\n    _: Error => {\n        x = 2\n    }\n}\nprint(x)");
    let ns = names(&diags);
    assert!(
        !ns.contains(&"x".to_string()),
        "x defined in both try and except should be safe, got: {:?}",
        ns
    );
}

#[test]
#[ignore = "finally false negative: modeling exception path requires block duplication"]
fn test_w310_try_finally_var_from_try() {
    // Variable defined in try, read in finally -- may not be defined.
    let diags = lint_deep("try {\n    x = 1\n} finally {\n    print(x)\n}");
    let ns = names(&diags);
    assert!(
        ns.contains(&"x".to_string()),
        "x defined in try should be possibly uninitialized in finally, got: {:?}",
        ns
    );
}

#[test]
fn test_w310_try_finally_no_false_positive_after() {
    // Reaching code after try/finally means try completed -- no false W310.
    let diags = lint_deep("try {\n    x = 1\n} finally {\n    cleanup()\n}\nprint(x)");
    let ns = names(&diags);
    assert!(
        !ns.contains(&"x".to_string()),
        "x defined in try should be safe after try/finally, got: {:?}",
        ns
    );
}

#[test]
fn test_w310_multiple_except_clauses() {
    // Variable defined in some except clauses but not all.
    let diags = lint_deep(
        "try {\n    risky()\n} except {\n    _: TypeError => {\n        x = 1\n    }\n    _: ValueError => {\n        y = 2\n    }\n}\nprint(x)",
    );
    let ns = names(&diags);
    assert!(
        ns.contains(&"x".to_string()),
        "x defined only in one except clause should trigger W310, got: {:?}",
        ns
    );
}

// -- G4.2: break/continue tests --

#[test]
fn test_w310_break_only_path() {
    // Variable defined only on the break path.
    let diags = lint_deep("while True {\n    if cond {\n        x = 1\n        break\n    }\n}\nprint(x)");
    let ns = names(&diags);
    assert!(
        ns.contains(&"x".to_string()),
        "x defined only in break path should trigger W310, got: {:?}",
        ns
    );
}

#[test]
#[ignore = "dead code after continue: def never registered, needs W311 or dead-def collection"]
fn test_w310_after_continue_unreachable() {
    // Variable defined after continue -- never reached.
    let diags = lint_deep("while cond {\n    continue\n    x = 1\n}\nprint(x)");
    let ns = names(&diags);
    assert!(
        ns.contains(&"x".to_string()),
        "x after continue is unreachable, should trigger W310, got: {:?}",
        ns
    );
}

// -- G4.3: match tests --

#[test]
fn test_w310_match_all_cases_define() {
    // All cases (including wildcard) define x -- safe.
    let diags = lint_deep("val = 1\nmatch val {\n    1 => { x = 1 }\n    _ => { x = 2 }\n}\nprint(x)");
    let ns = names(&diags);
    assert!(
        !ns.contains(&"x".to_string()),
        "x defined in all match cases should be safe, got: {:?}",
        ns
    );
}

#[test]
fn test_w310_match_partial_cases() {
    // x defined in one case only, no wildcard.
    let diags = lint_deep("match val {\n    1 => { x = 1 }\n    2 => { y = 2 }\n}\nprint(x)");
    let ns = names(&diags);
    assert!(
        ns.contains(&"x".to_string()),
        "x defined in only one match case should trigger W310, got: {:?}",
        ns
    );
}

#[test]
fn test_w310_match_guarded_wildcard_not_exhaustive() {
    // A guarded wildcard is not exhaustive -- the guard may fail.
    let diags = lint_deep("val = 1\nmatch val {\n    1 => { x = 1 }\n    _ if cond => { x = 2 }\n}\nprint(x)");
    let ns = names(&diags);
    assert!(
        ns.contains(&"x".to_string()),
        "guarded wildcard should not make match exhaustive, got: {:?}",
        ns
    );
}

#[test]
fn test_w310_match_with_wildcard_defines() {
    // Wildcard case defines x but specific case doesn't.
    let diags = lint_deep("match val {\n    1 => { y = 1 }\n    _ => { x = 2 }\n}\nprint(x)");
    let ns = names(&diags);
    assert!(
        ns.contains(&"x".to_string()),
        "x not defined in all match cases should trigger W310, got: {:?}",
        ns
    );
}

// -- G2.3: bare variable pattern as catch-all --

#[test]
fn test_w310_match_bare_var_catchall_exhaustive() {
    // A bare variable pattern (`n =>`) is an irrefutable catch-all, just
    // like `_`. The match is exhaustive, so x is defined on every path.
    let diags = lint_deep("match val {\n    1 => { x = 1 }\n    n => { x = 2 }\n}\nprint(x)");
    let ns = names(&diags);
    assert!(
        !ns.contains(&"x".to_string()),
        "bare var pattern should make match exhaustive (no W310), got: {:?}",
        ns
    );
}

#[test]
fn test_w310_match_guarded_bare_var_not_exhaustive() {
    // A guarded bare variable pattern is not exhaustive -- the guard may fail.
    let diags = lint_deep("match val {\n    1 => { x = 1 }\n    n if n > 0 => { x = 2 }\n}\nprint(x)");
    let ns = names(&diags);
    assert!(
        ns.contains(&"x".to_string()),
        "guarded bare var should not make match exhaustive, got: {:?}",
        ns
    );
}

#[test]
fn test_w310_match_or_pattern_var_catchall() {
    // An or-pattern with a bare-var alternative (`1 | n`) is a catch-all.
    let diags = lint_deep("match val {\n    1 | n => { x = 1 }\n    2 => { x = 2 }\n}\nprint(x)");
    let ns = names(&diags);
    assert!(
        !ns.contains(&"x".to_string()),
        "or-pattern with bare-var alternative should be exhaustive, got: {:?}",
        ns
    );
}

// -- G1.3: intra-block ordering --

#[test]
fn test_w310_read_before_def_same_block() {
    // Read before def in the same block should trigger W310.
    let diags = lint_deep("print(x)\nx = 1");
    let ns = names(&diags);
    assert!(
        ns.contains(&"x".to_string()),
        "read before def in same block should trigger W310, got: {:?}",
        ns
    );
}

#[test]
fn test_w310_def_before_read_same_block() {
    // Def before read in the same block should NOT trigger W310.
    let diags = lint_deep("x = 1\nprint(x)");
    assert!(diags.is_empty(), "def before read should be safe, got: {:?}", diags);
}

// -- G4.4: nested control flow --

#[test]
fn test_w310_if_inside_while() {
    let diags = lint_deep("while cond {\n    if flag {\n        x = 1\n    }\n}\nprint(x)");
    let ns = names(&diags);
    assert!(
        ns.contains(&"x".to_string()),
        "x defined only in if-inside-while should trigger W310, got: {:?}",
        ns
    );
}

#[test]
fn test_w310_try_inside_if() {
    // x defined in try inside one if branch only.
    let diags = lint_deep(
        "if cond {\n    try {\n        x = 1\n    } except {\n        _: Error => { x = 2 }\n    }\n}\nprint(x)",
    );
    let ns = names(&diags);
    assert!(
        ns.contains(&"x".to_string()),
        "x defined inside if-without-else should trigger W310, got: {:?}",
        ns
    );
}

#[test]
fn test_w310_match_inside_for() {
    let diags = lint_deep(
        "items = [1]\nfor i in items {\n    match i {\n        1 => { x = 1 }\n        _ => { x = 2 }\n    }\n}\nprint(x)",
    );
    let ns = names(&diags);
    assert!(
        ns.contains(&"x".to_string()),
        "x defined only inside for body should trigger W310 (loop may not execute), got: {:?}",
        ns
    );
}

#[test]
fn test_w310_return_in_nested_if() {
    // All branches of outer if either define x or return.
    let diags = lint_deep(
        "if a {\n    if b {\n        return 0\n    } else {\n        x = 1\n    }\n} else {\n    x = 2\n}\nprint(x)",
    );
    let ns = names(&diags);
    assert!(
        !ns.contains(&"x".to_string()),
        "x defined on all non-returning paths should be safe, got: {:?}",
        ns
    );
}

// -- G4.5: edge cases --

#[test]
fn test_w310_empty_source() {
    let diags = lint_deep("");
    assert!(diags.is_empty());
}

#[test]
fn test_w310_single_statement() {
    let diags = lint_deep("x = 1");
    assert!(diags.is_empty());
}

#[test]
fn test_w310_duplicate_read_single_alert() {
    // Variable read in two branches -- only one W310 alert expected.
    let diags = lint_deep("if cond {\n    x = 1\n}\nprint(x)\nprint(x)");
    let ns = names(&diags);
    assert_eq!(ns.len(), 1, "should only report x once, got: {:?}", ns);
}

#[test]
fn test_w310_destructuring_assignment() {
    // Destructuring defines both variables.
    let diags = lint_deep("(a, b) = (1, 2)\nprint(a)\nprint(b)");
    assert!(diags.is_empty(), "destructured vars should be safe, got: {:?}", diags);
}

// -- G5.1: W311 unreachable code --

#[test]
fn test_w311_not_after_direct_return() {
    // Direct return is intra-block dead code (W300 territory), not W311.
    let diags = lint_deep("return 1\nprint(42)");
    let w311: Vec<_> = diags.iter().filter(|d| d.code == "W311").collect();
    assert!(
        w311.is_empty(),
        "direct return should not trigger W311 (W300 handles it): {:?}",
        w311
    );
}

#[test]
fn test_w311_after_all_branches_return() {
    let diags = lint_deep("if cond {\n    return 1\n} else {\n    return 2\n}\nx = 3");
    let w311: Vec<_> = diags.iter().filter(|d| d.code == "W311").collect();
    assert_eq!(w311.len(), 1, "expected one W311, got: {:?}", w311);
    assert_eq!(w311[0].line, 6);
}

#[test]
fn test_w311_no_false_positive_with_else() {
    let diags = lint_deep("if cond {\n    return 1\n}\nx = 3");
    let w311: Vec<_> = diags.iter().filter(|d| d.code == "W311").collect();
    assert!(
        w311.is_empty(),
        "only one branch returns, code is reachable: {:?}",
        w311
    );
}

#[test]
fn test_w311_not_after_direct_raise() {
    // Direct raise is intra-block dead code (W300 territory), not W311.
    let diags = lint_deep("raise Error()\nx = 1");
    let w311: Vec<_> = diags.iter().filter(|d| d.code == "W311").collect();
    assert!(w311.is_empty(), "direct raise should not trigger W311: {:?}", w311);
}

#[test]
fn test_w311_match_all_cases_return() {
    let diags = lint_deep("match val {\n    1 => { return 1 }\n    _ => { return 2 }\n}\nprint(42)");
    let w311: Vec<_> = diags.iter().filter(|d| d.code == "W311").collect();
    assert_eq!(w311.len(), 1, "all match cases return, code after is dead: {:?}", w311);
}

#[test]
fn test_w311_match_bare_var_catchall_returns() {
    // Bare var catch-all makes the match exhaustive; all cases return, so the
    // code after is unreachable (W311).
    let diags = lint_deep("match val {\n    1 => { return 1 }\n    n => { return 2 }\n}\nprint(42)");
    let w311: Vec<_> = diags.iter().filter(|d| d.code == "W311").collect();
    assert_eq!(
        w311.len(),
        1,
        "bare var catch-all with all-returning cases: code after is dead: {:?}",
        w311
    );
}

#[test]
fn test_w311_through_finally() {
    // All try paths return, finally runs but deferred return makes code after unreachable.
    let diags = lint_deep("try {\n    return 1\n} finally {\n    cleanup()\n}\nprint(42)");
    let w311: Vec<_> = diags.iter().filter(|d| d.code == "W311").collect();
    assert_eq!(
        w311.len(),
        1,
        "code after try/finally with return should be dead: {:?}",
        w311
    );
}

// -- G5.3: W313 redundant else after terminating branch --

fn w313(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags.iter().filter(|d| d.code == "W313").collect()
}

#[test]
fn test_w313_else_after_return() {
    let diags =
        lint_deep("f = () => {\n    if cond {\n        return 1\n    } else {\n        x = 2\n        do(x)\n    }\n}");
    let hits = w313(&diags);
    assert_eq!(hits.len(), 1, "expected one W313, got: {:?}", hits);
    assert_eq!(hits[0].severity, Severity::Hint);
}

#[test]
fn test_w313_else_after_raise() {
    let diags = lint_deep("if cond {\n    raise Error()\n} else {\n    do_stuff()\n}");
    let hits = w313(&diags);
    assert_eq!(hits.len(), 1, "raise should trigger W313 on else: {:?}", hits);
}

#[test]
fn test_w313_both_branches_return_no_hint() {
    // Both branches terminate -- no W313 (symmetric, can't simplify by extraction).
    let diags = lint_deep("f = () => {\n    if cond {\n        return 1\n    } else {\n        return 2\n    }\n}");
    let hits = w313(&diags);
    assert!(hits.is_empty(), "both-return should not trigger W313: {:?}", hits);
}

#[test]
fn test_w313_elif_after_return() {
    // if X returns, elif Y has follow-up code -> redundant elif.
    let diags = lint_deep("f = () => {\n    if a {\n        return 1\n    } elif b {\n        x = 2\n    }\n}");
    let hits = w313(&diags);
    assert_eq!(hits.len(), 1, "elif after return should trigger W313: {:?}", hits);
}

#[test]
fn test_w313_no_else_no_hint() {
    // No else branch -- nothing redundant.
    let diags = lint_deep("f = () => {\n    if cond {\n        return 1\n    }\n    x = 2\n}");
    let hits = w313(&diags);
    assert!(hits.is_empty(), "if without else shouldn't trigger W313: {:?}", hits);
}

#[test]
fn test_w313_then_does_not_terminate() {
    // Then doesn't terminate -- else is not redundant.
    let diags = lint_deep("if cond {\n    x = 1\n} else {\n    x = 2\n}");
    let hits = w313(&diags);
    assert!(hits.is_empty(), "no terminating branch -> no W313: {:?}", hits);
}

#[test]
fn test_w313_break_in_loop_triggers() {
    // break also terminates the branch w.r.t. the enclosing loop body.
    let diags = lint_deep("while cond {\n    if stop {\n        break\n    } else {\n        step()\n    }\n}");
    let hits = w313(&diags);
    assert_eq!(hits.len(), 1, "break should trigger W313 on else: {:?}", hits);
}

#[test]
fn test_w313_nested_if_all_branches_terminate() {
    // Inner if where all branches return -> outer then terminates.
    let diags = lint_deep(
        "f = () => {\n    if a {\n        if b {\n            return 1\n        } else {\n            return 2\n        }\n    } else {\n        x = 1\n    }\n}",
    );
    let hits = w313(&diags);
    assert_eq!(
        hits.len(),
        1,
        "nested if all-return should propagate termination: {:?}",
        hits
    );
}

#[test]
fn test_w313_only_else_terminates_no_hint() {
    // Only else terminates -- we don't propose flipping the condition (V1
    // restriction). No W313.
    let diags = lint_deep("if cond {\n    x = 1\n} else {\n    return 2\n}");
    let hits = w313(&diags);
    assert!(
        hits.is_empty(),
        "only-else terminates should not trigger W313: {:?}",
        hits
    );
}

// -- G5.2: W312 dead store (backward liveness) --

fn w312(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags.iter().filter(|d| d.code == "W312").collect()
}

#[test]
fn test_w312_overwrite_before_read() {
    // x = 1 is killed by x = 2 before being read.
    let diags = lint_deep("x = 1\nx = 2\nprint(x)");
    let hits = w312(&diags);
    assert_eq!(hits.len(), 1, "expected one W312 on first write: {:?}", hits);
    assert_eq!(hits[0].line, 1);
}

#[test]
fn test_w312_no_alert_when_read_between() {
    let diags = lint_deep("x = 1\nprint(x)\nx = 2\nprint(x)");
    let hits = w312(&diags);
    assert!(hits.is_empty(), "intermediate read keeps store live: {:?}", hits);
}

#[test]
fn test_w312_inter_branches_killed_after_merge() {
    // Both branches assign y, then y is unconditionally overwritten -- the
    // branch writes are dead stores.
    let diags = lint_deep("if cond {\n    y = 1\n} else {\n    y = 2\n}\ny = 3\nprint(y)");
    let hits = w312(&diags);
    let lines: Vec<usize> = hits.iter().map(|d| d.line).collect();
    assert!(
        lines.contains(&2) && lines.contains(&4),
        "expected W312 on both branch writes, got lines: {:?}",
        lines
    );
}

#[test]
fn test_w312_no_alert_when_branch_used_after_merge() {
    // Reading y after the if keeps both branch writes live.
    let diags = lint_deep("if cond {\n    y = 1\n} else {\n    y = 2\n}\nprint(y)");
    let hits = w312(&diags);
    assert!(hits.is_empty(), "merged branches are live: {:?}", hits);
}

#[test]
fn test_w312_no_alert_when_variable_never_read() {
    // x is never read anywhere -- W200 territory, not W312.
    let diags = lint_deep("x = 1");
    let hits = w312(&diags);
    assert!(hits.is_empty(), "never-read variable is W200, not W312: {:?}", hits);
}

#[test]
fn test_w312_for_var_binding_not_flagged() {
    // Loop variables are implicit bindings -- don't trigger W312 even if
    // the body reassigns them.
    let diags = lint_deep("for x in items {\n    x = transform(x)\n    print(x)\n}");
    let hits = w312(&diags);
    // x is read inside the body, the binding rewrites are user-driven on
    // a captured value; we don't want to chase that here.
    assert!(hits.is_empty(), "for-var binding shouldn't trigger W312: {:?}", hits);
}

#[test]
fn test_w312_dest_before_overwrite() {
    // Destructuring then immediate overwrite of one component.
    let diags = lint_deep("(a, b) = pair\na = 99\nprint(a)\nprint(b)");
    let hits = w312(&diags);
    let lines: Vec<usize> = hits.iter().map(|d| d.line).collect();
    assert!(
        lines.contains(&1),
        "destructured 'a' is killed by line-2 overwrite, got lines: {:?}",
        lines
    );
}

#[test]
fn test_w312_inside_function_body() {
    // W312 fires inside a lambda body.
    let diags = lint_deep("f = (z) => {\n    x = 1\n    x = z + 1\n    return x\n}");
    let hits = w312(&diags);
    let lines: Vec<usize> = hits.iter().map(|d| d.line).collect();
    assert!(
        lines.contains(&2),
        "first write inside lambda should be flagged: {:?}",
        lines
    );
}

// -- Lambda parameter seeding (regression: bodies must see their params) --

#[test]
fn test_lambda_param_read_no_w310() {
    // `z` is a parameter -- reading it inside the body must not trigger
    // W310 even though the body has no def for `z`.
    let diags = lint_deep("f = (z) => {\n    print(z)\n}");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(w310.is_empty(), "param read shouldn't trigger W310: {:?}", w310);
}

#[test]
fn test_lambda_param_partial_reassign_no_w310() {
    // `z` is reassigned in one branch of an if but still has the param
    // value on the other path. No W310.
    let diags = lint_deep("f = (z) => {\n    if cond {\n        z = 1\n    }\n    print(z)\n}");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(
        w310.is_empty(),
        "param partially reassigned should not trigger W310: {:?}",
        w310
    );
}

// -- Captured-variable mutation (regression: must not regress W310 nor W312) --

#[test]
fn test_capture_mutation_no_w310() {
    // `count` is bound in the parent scope; mutating it inside the lambda
    // must not trigger W310 even though the body has no def before the
    // RHS read.
    let diags = lint_deep("count = 0\ninc = () => {\n    count = count + 1\n}\nprint(count)");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(w310.is_empty(), "capture mutation shouldn't trigger W310: {:?}", w310);
}

#[test]
fn test_capture_mutation_no_w312() {
    // The write to `count` is observable from the parent scope -- not a
    // dead store from the linter's POV.
    let diags = lint_deep("count = 0\ninc = () => {\n    count = count + 1\n}\nprint(count)");
    let w312: Vec<_> = diags.iter().filter(|d| d.code == "W312").collect();
    assert!(w312.is_empty(), "capture write isn't a dead store: {:?}", w312);
}

#[test]
fn test_capture_pure_read_no_w310() {
    // Reading a captured variable inside a lambda (no write) is the most
    // common closure pattern.
    let diags = lint_deep("x = 10\nf = () => {\n    return x + 1\n}");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(w310.is_empty(), "capture read shouldn't trigger W310: {:?}", w310);
}

#[test]
fn test_nested_w310_still_fires_for_local() {
    // A truly partial def of a LOCAL var inside a lambda must still
    // trigger W310 -- capture detection is keyed on read-first events.
    let diags = lint_deep("f = () => {\n    if cond {\n        x = 1\n    }\n    print(x)\n}");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert_eq!(
        w310.len(),
        1,
        "partial def of a local should still fire W310: {:?}",
        w310
    );
}

#[test]
fn test_nested_read_before_def_fires_w310() {
    // Regression: a read-before-def inside a lambda whose name does NOT
    // exist in any enclosing scope must fire W310. The earlier heuristic
    // (read-first event = capture) silently suppressed this -- capture
    // classification must require an actual parent-scope binding.
    let diags = lint_deep("f = () => {\n    print(x)\n    x = 1\n}");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert_eq!(
        w310.len(),
        1,
        "read-before-local-def inside a lambda must fire W310: {:?}",
        w310
    );
    assert_eq!(w310[0].line, 2);
}

#[test]
fn test_nested_capture_visible_in_sibling_lambda_not_seeded() {
    // `x` is defined inside one lambda's body, NOT visible from a sibling
    // lambda's enclosing scope. The sibling reads `x` then writes it --
    // x is in defined_anywhere of the sibling's sub-CFG, so the
    // read-before-def must surface as W310 (not get swallowed by the
    // capture heuristic).
    let diags = lint_deep("a = () => { x = 1 }\nb = () => {\n    print(x)\n    x = 5\n}");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert_eq!(
        w310.len(),
        1,
        "x defined in sibling lambda must still trip W310 in `b`: {:?}",
        w310
    );
}

#[test]
fn test_nested_capture_from_outer_lambda_local() {
    // `local_x` is bound in the outer lambda's body; the inner lambda
    // legitimately captures it.
    let diags = lint_deep("outer = () => {\n    local_x = 1\n    inner = () => {\n        print(local_x)\n    }\n}");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(
        w310.is_empty(),
        "inner lambda reading outer's local should not trip W310: {:?}",
        w310
    );
}

#[test]
fn test_capture_read_before_outer_def_no_w310() {
    // G2.4 limitation guard: a captured read inside a lambda must NOT be
    // attributed to the lambda's *definition* point. Here `x` is defined
    // after the lambda but before the call, so the read at call time is
    // safe. A naive fix that injects the capture read where the lambda is
    // defined would fire a false positive here -- the worst failure mode.
    // Capture reads can only be located precisely at the call site, which
    // needs interprocedural analysis the CST linter deliberately omits.
    let diags = lint_deep("f = () => {\n    print(x)\n}\nx = 1\nf()");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(
        w310.is_empty(),
        "capture read must not be pinned to the lambda definition point: {:?}",
        w310
    );
}

// -- struct_method scope (regression: methods get their own CFG and params) --

#[test]
fn test_method_body_param_seeded() {
    // A method's `self` parameter must not trigger W310 inside the body.
    let diags = lint_deep("struct Point {\n    x; y;\n\n    sum(self) => {\n        return self.x + self.y\n    }\n}");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(w310.is_empty(), "method param `self` should be seeded: {:?}", w310);
}

#[test]
fn test_method_local_read_before_def_w310() {
    // A real read-before-local-def inside a method must still fire W310.
    let diags = lint_deep("struct S {\n    f(self) => {\n        print(x)\n        x = 1\n    }\n}");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert_eq!(
        w310.len(),
        1,
        "read-before-local-def inside a method must fire W310: {:?}",
        w310
    );
}

#[test]
fn test_nested_lambda_in_method_sees_self() {
    // A lambda nested inside a method body should see the method's params
    // (`self`, plus declared params) as captures, not as undefined reads.
    let diags = lint_deep(
        "struct S {\n    f(self, k) => {\n        g = () => {\n            return self.x + k\n        }\n    }\n}",
    );
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(
        w310.is_empty(),
        "lambda inside method should capture self and k from parent method scope: {:?}",
        w310
    );
}

// -- Attribute-name handling (regression: getattr/callattr identifiers
//    are not variable reads) --

#[test]
fn test_attribute_name_not_variable_read() {
    // `obj.x` reads `obj`, not a variable named `x`. With `x` defined
    // only after, an attribute access mustn't fabricate a W310 on `x`.
    let diags = lint_deep("obj = make()\nobj.x = 1\nx = 2\nprint(x)");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(
        w310.is_empty(),
        "attribute name `x` mustn't be tracked as a read of variable x: {:?}",
        w310
    );
}

#[test]
fn test_method_call_attribute_name_not_variable_read() {
    // `obj.x(arg)` -- `x` is a method name, not a variable read.
    let diags = lint_deep("obj = make()\nobj.x(arg)\nx = 2\nprint(x)");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    // arg is undefined → E200's job, not W310. x is defined-after but
    // not read before its def (the attribute access doesn't count).
    let names: Vec<&str> = w310
        .iter()
        .map(|d| d.message.split('\'').nth(1).unwrap_or(""))
        .collect();
    assert!(
        !names.contains(&"x"),
        "method name shouldn't be tracked as read of variable x: {:?}",
        w310
    );
}

#[test]
fn test_self_ref_detection_ignores_attribute_names() {
    // The shadow heuristic must NOT treat `self.x` in the RHS as a read
    // of `x`. In this method, the LHS `result` is locally bound; the
    // RHS `self.x + self.y` has no read of `result`. Therefore
    // `result = self.x + self.y` is NOT a self-referential write of
    // `result`. The prior read `print(result)` IS a real W310 (shadow
    // pattern).
    let diags = lint_deep(
        "result = 0\nstruct P {\n    x; y;\n    m(self) => {\n        print(result)\n        result = self.x + self.y\n    }\n}",
    );
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert_eq!(
        w310.len(),
        1,
        "shadow-style local write inside method must keep prior read flagged: {:?}",
        w310
    );
}

#[test]
fn test_local_shadow_with_prior_read_fires_w310() {
    // `x` is a module-level binding, but the lambda writes `x = 2`
    // without reading `x` in the RHS -- Python-style scoping makes `x`
    // a local for the entire lambda, so the prior `print(x)` is a real
    // read-before-local-def. Must fire W310 (the capture filter must
    // NOT swallow this).
    let diags = lint_deep("x = 0\nb = () => {\n    print(x)\n    x = 2\n}");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert_eq!(
        w310.len(),
        1,
        "shadow-style local write must keep prior read flagged: {:?}",
        w310
    );
    assert_eq!(w310[0].line, 3);
}

#[test]
fn test_capture_mutation_with_explicit_read_in_rhs_no_w310() {
    // `x = x + 1` IS self-referential -- the W310 capture filter must
    // still suppress the read warning here even though there's an
    // explicit write to x.
    let diags = lint_deep("x = 0\nb = () => {\n    x = x + 1\n}");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(
        w310.is_empty(),
        "self-referential write should keep capture classification: {:?}",
        w310
    );
}

#[test]
fn test_method_capture_mutation_no_w310_no_w312() {
    // `count` is bound at module level; a method that mutates the capture
    // must not trigger W310 (RHS read) or W312 (LHS write).
    let diags = lint_deep("count = 0\nstruct Counter {\n    bump(self) => {\n        count = count + 1\n    }\n}");
    let codes: Vec<&str> = diags
        .iter()
        .filter(|d| d.code == "W310" || d.code == "W312")
        .map(|d| d.code.as_str())
        .collect();
    assert!(
        codes.is_empty(),
        "method capturing module-level var shouldn't fire W310/W312: {:?}",
        codes
    );
}

#[test]
fn test_match_pattern_binding_not_capture() {
    // `x` is a pattern binding inside a lambda. The binding is sequenced
    // at the end of the PATTERN (not the case clause), so body reads
    // happen AFTER the binding -- so x is not misclassified as capture.
    // W310 isn't expected; W312 isn't expected either.
    let diags = lint_deep("f = (val) => {\n    match val {\n        x => { return x + 1 }\n    }\n}");
    let names: Vec<_> = diags.iter().filter(|d| d.code == "W310" || d.code == "W312").collect();
    assert!(
        names.is_empty(),
        "pattern binding should not surface W310/W312: {:?}",
        names
    );
}

#[test]
fn test_lambda_param_variadic_seeded() {
    let diags = lint_deep("f = (*args) => {\n    print(args)\n}");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(
        w310.is_empty(),
        "variadic param read shouldn't trigger W310: {:?}",
        w310
    );
}

#[test]
fn test_w312_self_overwrite_uses_rhs() {
    // x = x + 1 reads x in the RHS -- the previous write is live.
    let diags = lint_deep("x = 1\nx = x + 1\nprint(x)");
    let hits = w312(&diags);
    assert!(
        hits.is_empty(),
        "RHS-using overwrite keeps prior write live: {:?}",
        hits
    );
}

#[test]
fn test_w312_match_all_cases_overwritten() {
    let diags = lint_deep("match v {\n    1 => { y = 1 }\n    _ => { y = 2 }\n}\ny = 9\nprint(y)");
    let hits = w312(&diags);
    let lines: Vec<usize> = hits.iter().map(|d| d.line).collect();
    assert!(
        lines.contains(&2) && lines.contains(&3),
        "both match arms write y but it's overwritten -- expected W312 on both, got: {:?}",
        lines
    );
}

#[test]
fn test_w313_match_all_cases_terminate_in_branch() {
    // Then branch contains a match where all cases return + wildcard.
    let diags = lint_deep(
        "f = () => {\n    if cond {\n        match x {\n            1 => { return 1 }\n            _ => { return 2 }\n        }\n    } else {\n        y = 1\n    }\n}",
    );
    let hits = w313(&diags);
    assert_eq!(hits.len(), 1, "match-all-return should propagate: {:?}", hits);
}

// -- G2.1: control-flow expressions branch the CFG in value position --

#[test]
fn test_if_expr_in_rhs_conditional_write_w310() {
    // An `if`-expression in value position whose single branch writes `x`:
    // `x` is conditional, so the later read may be uninitialized. Before the
    // fix the nested write was flattened (seen as a read) and no W310 fired.
    let diags = lint_deep("cond = True\nr = (if cond { x = 1 })\nprint(x)");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert_eq!(
        w310.len(),
        1,
        "conditional write via if-expression must fire W310: {:?}",
        w310
    );
    assert_eq!(w310[0].line, 3);
}

#[test]
fn test_if_expr_in_rhs_with_else_no_w310() {
    // Both branches of the if-expression write `x`, so it is defined on every
    // path -- no W310. Guards against the fix over-reporting.
    let diags = lint_deep("cond = True\nr = (if cond { x = 1 } else { x = 2 })\nprint(x)");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(
        w310.is_empty(),
        "if-expression writing x on every branch must not fire W310: {:?}",
        w310
    );
}

#[test]
fn test_short_circuit_and_conditional_write_w310() {
    // The if-expression writes `x` on every internal branch, but the whole
    // RHS runs only when `cond` is truthy (short-circuit `and`). `x` is thus
    // conditional on the short-circuit -- the later read may be undefined.
    let diags = lint_deep("cond = True\nr = cond and (if cond { x = 1 } else { x = 2 })\nprint(x)");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert_eq!(
        w310.len(),
        1,
        "write reachable only through `and` RHS must fire W310: {:?}",
        w310
    );
    assert_eq!(w310[0].line, 3);
}

#[test]
fn test_plain_and_in_value_position_no_false_positive() {
    // A plain `a and b` in value position must not invent a W310: both
    // operands are defined, the short-circuit modeling only adds branching.
    let diags = lint_deep("a = 1\nb = 2\nr = a and b\nprint(r)");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(w310.is_empty(), "plain `a and b` must not fire W310: {:?}", w310);
}

// -- Bug A: a kwarg key is a parameter name, not a variable read --

#[test]
fn test_w310_kwarg_key_not_a_read() {
    // `dict(event=...)` -- `event` is a keyword name, not a read of a variable.
    let diags = lint_deep("event = dict(event=\"x\", date=\"y\")\nprint(event)");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(w310.is_empty(), "kwarg key must not fire W310, got: {:?}", w310);
}

#[test]
fn test_w310_kwarg_value_read_still_checked() {
    // A genuine variable read in a kwarg *value* is still analyzed.
    let diags = lint_deep("if c {\n    y = 1\n}\nfoo(x=y)");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert_eq!(w310.len(), 1, "kwarg value read must still fire W310: {:?}", diags);
    assert!(w310[0].message.contains("'y'"));
}

// -- Bug C: an assignment target inside a with / block-expression is a def --

#[test]
fn test_w310_block_expression_target_not_a_read() {
    // `x` assigned in a block-expression is a def, not a read -- even when the
    // same name is also defined later at the top level (the trigger condition).
    let diags = lint_deep("r = {\n    x = 10\n    x\n}\nx = 5\nprint(r, x)");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(w310.is_empty(), "block-expr target must not fire W310, got: {:?}", w310);
}

#[test]
fn test_w310_with_body_assignment_is_def() {
    // A `with` body assignment registers as a def, not a read.
    let diags = lint_deep("with d = get() {\n    arr = d.read()\n    print(arr)\n}\narr = 5\nprint(arr)");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert!(w310.is_empty(), "with body def must not fire W310, got: {:?}", w310);
}

#[test]
fn test_w310_with_body_still_detects_uninitialized() {
    // The `with` body is a real scope: a conditionally-uninitialized read
    // inside it must still fire W310 (not made opaque by the fix).
    let diags = lint_deep("with d = get() {\n    if c {\n        a = 1\n    }\n    print(a)\n}");
    let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
    assert_eq!(w310.len(), 1, "genuine uninit read in with must fire W310: {:?}", diags);
    assert!(w310[0].message.contains("'a'"));
}
