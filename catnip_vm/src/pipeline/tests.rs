//! Tests for the pure VM pipeline (parse → compile → execute).

use super::*;

#[test]
fn test_eval_int() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("42").unwrap().as_int(), Some(42));
}

#[test]
fn test_eval_float() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("3.14").unwrap();
    assert!((r.as_float().unwrap() - 3.14).abs() < 1e-10);
}

#[test]
fn test_eval_arithmetic() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("2 + 3 * 4").unwrap().as_int(), Some(14));
}

#[test]
fn test_eval_string() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(r#""hello""#).unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("hello"));
    r.decref();
}

#[test]
fn test_eval_bool() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("true").unwrap().as_bool(), Some(true));
    assert_eq!(p.execute("false").unwrap().as_bool(), Some(false));
}

#[test]
fn test_eval_none() {
    let mut p = PurePipeline::new().unwrap();
    assert!(p.execute("nil").unwrap().is_nil());
}

#[test]
fn test_eval_comparison() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("3 < 5").unwrap().as_bool(), Some(true));
    assert_eq!(p.execute("3 > 5").unwrap().as_bool(), Some(false));
    assert_eq!(p.execute("5 == 5").unwrap().as_bool(), Some(true));
}

#[test]
fn test_eval_negation() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("-42").unwrap().as_int(), Some(-42));
}

#[test]
fn test_eval_not() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("not true").unwrap().as_bool(), Some(false));
}

#[test]
fn test_eval_lambda() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("f = (x) => { x * 2 }; f(21)").unwrap().as_int(), Some(42));
}

// bind_args must read vararg_idx and collect excess positional args into a
// list, matching the PyO3 VM. Before the fix the variadic slot bound a
// single arg and the rest were silently dropped.
#[test]
fn test_eval_vararg_collects_excess() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("f = (x, *rest) => { rest }; f(1, 2, 3, 4)").unwrap();
    let list = unsafe { r.as_native_list_ref() }.expect("vararg slot should hold a list");
    assert_eq!(
        list.as_slice_cloned()
            .iter()
            .filter_map(|v| v.as_int())
            .collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
    r.decref();
}

#[test]
fn test_eval_vararg_only() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(
        p.execute("f = (*args) => { len(args) }; f(1, 2, 3, 4, 5)")
            .unwrap()
            .as_int(),
        Some(5)
    );
}

#[test]
fn test_eval_vararg_empty() {
    let mut p = PurePipeline::new().unwrap();
    // No excess args -> the variadic slot is an empty list, not nil.
    assert_eq!(
        p.execute("f = (x, *rest) => { len(rest) }; f(1)").unwrap().as_int(),
        Some(0)
    );
}

#[test]
fn test_eval_string_default_repeated() {
    let mut p = PurePipeline::new().unwrap();
    // A heap default must be incref'd on bind: without it each call
    // over-released the shared constant (use-after-free under reuse).
    let r = p
        .execute(r#"f = (x, y="default_witness") => { y }; [f(1), f(1), f(1), f(1)]"#)
        .unwrap();
    let list = unsafe { r.as_native_list_ref() }.expect("result list");
    for v in list.as_slice_cloned() {
        assert_eq!(unsafe { v.as_native_str_ref() }, Some("default_witness"));
    }
    r.decref();
}

#[test]
fn test_eval_fn_def_and_call() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(
        p.execute("double = (x) => { x * 2 }; double(21)").unwrap().as_int(),
        Some(42)
    );
}

// Reachable call error paths through the real compiler (CallMethod not-found
// and a non-function Call). Their args are popped (owned) before the dispatch
// fails; releasing them must not corrupt VM state, so repeated failures keep
// erroring cleanly and a follow-up call still succeeds.
#[test]
fn test_eval_call_errors_release_cleanly() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("struct P { x }\np = P(\"hi\")").unwrap();
    for _ in 0..50 {
        assert!(p.execute(r#"p.nope("a", "b")"#).is_err(), "unknown method should error");
        assert!(
            p.execute(r#"nf = 42; nf("a")"#).is_err(),
            "non-function call should error"
        );
    }
    assert_eq!(p.execute("1 + 1").unwrap().as_int(), Some(2));
}

#[test]
fn test_persistence_variables() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("x = 42").unwrap();
    assert_eq!(p.execute("x + 8").unwrap().as_int(), Some(50));
}

#[test]
fn test_persistence_functions() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("double = (n) => { n * 2 }").unwrap();
    assert_eq!(p.execute("double(21)").unwrap().as_int(), Some(42));
}

#[test]
fn test_closure() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(
        p.execute("x = 5; add_x = (y) => { x + y }; add_x(3)").unwrap().as_int(),
        Some(8)
    );
}

// A doubly-nested closure must see an outer parameter that only the innermost
// closure references. The intermediate closure does not name `v` directly, so a
// capture filtered by the child's referenced names would drop it and the inner
// `v` would fall through to the global homonym (yielding 100, not 7).
#[test]
fn test_closure_double_nested_captures_outer_param() {
    let mut p = PurePipeline::new().unwrap();
    let src = "v = 100; f = (v) => { g = () => { h = () => { v }; h() }; g() }; f(7)";
    assert_eq!(p.execute(src).unwrap().as_int(), Some(7));
}

// Same transitive capture, but the outer binding is a non-global local with no
// homonym: a broken chain raises NameError instead of resolving to 42.
#[test]
fn test_closure_double_nested_non_global() {
    let mut p = PurePipeline::new().unwrap();
    let src = "outer = () => { w = 42; g = () => { h = () => { w }; h() }; g() }; outer()";
    assert_eq!(p.execute(src).unwrap().as_int(), Some(42));
}

// A closure mutating a module global must be visible when the module reads the
// name back. The module keeps a local slot aliasing the global (for fast
// LoadLocal), so the slot is re-synced from globals on return to the module
// frame -- otherwise the read sees a stale slot (yielding 0 instead of 2).
#[test]
fn test_closure_mutates_module_global() {
    let mut p = PurePipeline::new().unwrap();
    let src = "counter = 0; inc = () => { counter = counter + 1 }; inc(); inc(); counter";
    assert_eq!(p.execute(src).unwrap().as_int(), Some(2));
}

// The slot re-sync must be refcount-correct for heap globals: a closure that
// reassigns a module list, read back at module level, must yield the new value
// (no stale slot) without leaking the old list or double-freeing the new one.
#[test]
fn test_closure_reassigns_module_heap_global() {
    // Settled 2026-07-04 (read-based rule): a write-only global name inside a
    // function creates a LOCAL -- the module binding is untouched. A
    // write-through requires reading the name in the function.
    let mut p = PurePipeline::new().unwrap();
    let src = "data = [1]; swap = () => { data = [2, 3] }; swap(); len(data)";
    assert_eq!(p.execute(src).unwrap().as_int(), Some(1));
    let mut p2 = PurePipeline::new().unwrap();
    let src2 = "data = [1]; grow = () => { data = data + [2, 3] }; grow(); len(data)";
    assert_eq!(p2.execute(src2).unwrap().as_int(), Some(3));
}

// Reading a global back INSIDE the closure right after writing it: the store
// used to register a local slot while emitting StoreScope only, so the later
// read compiled to LoadLocal on a slot never filled (INVALID canary in debug,
// None in release). The write-through store must not create a slot; the read
// stays LoadScope.
#[test]
fn test_closure_reads_global_back_after_write() {
    let mut p = PurePipeline::new().unwrap();
    let src = "n = 0; bump = () => { n = n + 1; n }; bump()";
    assert_eq!(p.execute(src).unwrap().as_int(), Some(1));
}

// Same shape used inside an expression, plus the module-level read afterwards.
#[test]
fn test_closure_write_then_read_in_expression() {
    let mut p = PurePipeline::new().unwrap();
    let src = "n = 0; bump = () => { n = n + 1; n }; t = bump() + 1; t + n";
    // bump() returns 1, t = 2, n = 1 at module level.
    assert_eq!(p.execute(src).unwrap().as_int(), Some(3));
}

// Repeated re-sync of a heap global from many returns to the module frame: each
// iteration drops the previous list and aliases the new one. A double-free would
// crash and a leak-driven imbalance would surface here.
#[test]
fn test_closure_module_global_resync_stress() {
    let mut p = PurePipeline::new().unwrap();
    let src = "acc = []; push1 = () => { acc = acc + [1] }; i = 0; while i < 50 { push1(); i = i + 1 }; len(acc)";
    assert_eq!(p.execute(src).unwrap().as_int(), Some(50));
}

// Characterization (parity with catnip_rs): the module-slot re-sync runs on the
// normal Return path only, not on exception unwinding. When a callee mutates a
// global then raises, the global itself is updated (a closure reading it sees 1)
// but the module's local slot stays stale until the next normal return, so the
// module reads 0. Both VMs behave identically here; pin it so a future change to
// the unwind path is noticed and aligned across the two VMs.
#[test]
fn test_global_mutation_before_raise_leaves_module_slot_stale() {
    let mut p = PurePipeline::new().unwrap();
    let src =
        "counter = 0; f = () => { counter = counter + 1; raise \"x\" }; try { f() } except { _ => { 0 } }; counter";
    assert_eq!(p.execute(src).unwrap().as_int(), Some(0));
}

#[test]
fn test_syntax_error() {
    let mut p = PurePipeline::new().unwrap();
    assert!(p.execute("if {").is_err());
}

#[test]
fn test_reset() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("x = 42").unwrap();
    p.reset();
    assert!(p.execute("x").is_err());
}

#[test]
fn test_list_literal() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("list(1, 2, 3)").unwrap();
    assert!(r.is_native_list());
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 3);
    r.decref();
}

#[test]
fn test_set_global() {
    let mut p = PurePipeline::new().unwrap();
    p.set_global("x", crate::value::Value::from_int(42));
    assert_eq!(p.execute("x + 1").unwrap().as_int(), Some(43));
}

#[test]
fn test_parse_to_sexp() {
    let mut p = PurePipeline::new().unwrap();
    let sexp = p.parse_to_sexp("x = 2 + 3").unwrap();
    assert!(sexp.starts_with("(source_file"));
    assert!(sexp.contains("identifier"));
    assert!(sexp.contains("additive"));
}

#[test]
fn test_parse_to_ir() {
    let mut p = PurePipeline::new().unwrap();
    let ir = p.parse_to_ir("2 + 3", false).unwrap();
    match ir {
        IR::Program(items) => assert_eq!(items.len(), 1),
        _ => panic!("expected Program"),
    }
}

#[test]
fn test_parse_to_ir_semantic() {
    let mut p = PurePipeline::new().unwrap();
    let ir = p.parse_to_ir("2 + 3", true).unwrap();
    match ir {
        IR::Program(_) => {}
        _ => panic!("expected Program"),
    }
}

// --- Control flow ---

#[test]
fn test_if_else() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("if true { 1 } else { 2 }").unwrap().as_int(), Some(1));
    assert_eq!(p.execute("if false { 1 } else { 2 }").unwrap().as_int(), Some(2));
}

#[test]
fn test_if_elif_else() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("x = 2; if x == 1 { 10 } elif x == 2 { 20 } else { 30 }")
        .unwrap();
    assert_eq!(r.as_int(), Some(20));
}

#[test]
fn test_while_loop() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("x = 0; while x < 5 { x = x + 1 }; x").unwrap();
    assert_eq!(r.as_int(), Some(5));
}

#[test]
fn test_for_range() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("s = 0; for i in range(5) { s = s + i }; s").unwrap();
    assert_eq!(r.as_int(), Some(10));
}

#[test]
fn test_for_list() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("s = 0; for x in list(1, 2, 3) { s = s + x }; s").unwrap();
    assert_eq!(r.as_int(), Some(6));
}

#[test]
fn test_break_in_while() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("x = 0; while true { x = x + 1; if x == 3 { break } }; x")
        .unwrap();
    assert_eq!(r.as_int(), Some(3));
}

// --- Strings ---

#[test]
fn test_string_concat() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(r#""hello" ++ " world""#).unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("hello world"));
    r.decref();
}

#[test]
fn test_fstring() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(r#"x = 42; f"value={x}""#).unwrap();
    assert!(r.is_native_str());
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("value=42"));
    r.decref();
}

#[test]
fn test_string_methods() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(r#""hello".upper()"#).unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("HELLO"));
    r.decref();
}

// --- Collections ---

#[test]
fn test_tuple_literal() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("tuple(1, 2, 3)").unwrap();
    assert!(r.is_native_tuple());
    let t = unsafe { r.as_native_tuple_ref().unwrap() };
    assert_eq!(t.len(), 3);
    r.decref();
}

#[test]
fn test_dict_literal() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("dict(a=1, b=2)").unwrap();
    assert!(r.is_native_dict());
    r.decref();
}

#[test]
fn test_list_append() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("xs = list(1, 2); xs.append(3); xs").unwrap();
    assert!(r.is_native_list());
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 3);
    r.decref();
}

#[test]
fn test_list_getitem() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("xs = list(10, 20, 30); xs(1)").unwrap().as_int(), Some(20));
}

#[test]
fn test_in_operator() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("2 in list(1, 2, 3)").unwrap().as_bool(), Some(true));
    assert_eq!(p.execute("5 in list(1, 2, 3)").unwrap().as_bool(), Some(false));
}

// --- Pattern matching ---

#[test]
fn test_match_literal() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("match 2 { 1 => { 10 } 2 => { 20 } _ => { 30 } }").unwrap();
    assert_eq!(r.as_int(), Some(20));
}

// --- Arithmetic edge cases ---

#[test]
fn test_floor_div() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("7 // 2").unwrap().as_int(), Some(3));
    assert_eq!(p.execute("-7 // 2").unwrap().as_int(), Some(-4));
}

#[test]
fn test_modulo() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("7 % 3").unwrap().as_int(), Some(1));
    assert_eq!(p.execute("-7 % 3").unwrap().as_int(), Some(2));
}

#[test]
fn test_power() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("2 ** 10").unwrap().as_int(), Some(1024));
}

#[test]
fn test_bitwise() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("6 & 3").unwrap().as_int(), Some(2));
    assert_eq!(p.execute("6 | 3").unwrap().as_int(), Some(7));
    assert_eq!(p.execute("6 ^ 3").unwrap().as_int(), Some(5));
}

// --- Functions advanced ---

#[test]
fn test_default_params() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("f = (x, y=10) => { x + y }; f(5)").unwrap();
    assert_eq!(r.as_int(), Some(15));
}

#[test]
fn test_recursive_function() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("fact = (n) => { if n <= 1 { 1 } else { n * fact(n - 1) } }; fact(5)")
        .unwrap();
    assert_eq!(r.as_int(), Some(120));
}

#[test]
fn test_higher_order_function() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("apply = (f, x) => { f(x) }; double = (x) => { x * 2 }; apply(double, 21)")
        .unwrap();
    assert_eq!(r.as_int(), Some(42));
}

// --- Builtins ---

#[test]
fn test_builtin_len() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute(r#"len("hello")"#).unwrap().as_int(), Some(5));
    assert_eq!(p.execute("len(list(1, 2, 3))").unwrap().as_int(), Some(3));
}

#[test]
fn test_builtin_abs() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("abs(-42)").unwrap().as_int(), Some(42));
}

#[test]
fn test_builtin_type() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("typeof(42)").unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("int"));
    r.decref();
}

// --- Multi-statement ---

#[test]
fn test_multi_statement_returns_last() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("1; 2; 3").unwrap().as_int(), Some(3));
}

// --- and/or ---

#[test]
fn test_and_or() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("true and false").unwrap().as_bool(), Some(false));
    assert_eq!(p.execute("false or true").unwrap().as_bool(), Some(true));
}

// --- Gap tests ---

#[test]
fn test_continue_in_for() {
    let mut p = PurePipeline::new().unwrap();
    // Use list iterator (not for_range) to test continue with ForIter
    let r = p
        .execute("s = 0; for i in list(0, 1, 2) { if i == 1 { continue }; s = s + i }; s")
        .unwrap();
    assert_eq!(r.as_int(), Some(2)); // 0+2
}

#[test]
fn test_continue_in_for_range() {
    let mut p = PurePipeline::new().unwrap();
    // Use list-based for loop (not optimized range) - continue works there
    let r = p
        .execute("s = 0; for i in list(0, 1, 2) { if i == 1 { continue }; s = s + i }; s")
        .unwrap();
    assert_eq!(r.as_int(), Some(2)); // 0+2
}

#[test]
fn test_continue_in_range_loop() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("s = 0; for i in range(6) { if i % 2 == 0 { continue }; s = s + i }; s")
        .unwrap();
    assert_eq!(r.as_int(), Some(9)); // 1+3+5
}

#[test]
fn test_continue_in_while() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("s = 0; i = 0; while i < 6 { i = i + 1; if i % 2 == 0 { continue }; s = s + i }; s")
        .unwrap();
    assert_eq!(r.as_int(), Some(9)); // 1+3+5
}

#[test]
fn test_nested_closures() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("make_adder = (x) => { (y) => { x + y } }; add5 = make_adder(5); add5(3)")
        .unwrap();
    assert_eq!(r.as_int(), Some(8));
}

#[test]
fn test_unpack_assignment() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("(a, b) = tuple(1, 2); a + b").unwrap();
    assert_eq!(r.as_int(), Some(3));
}

#[test]
fn test_match_variable_binding() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("match 42 { x => { x * 2 } }").unwrap();
    assert_eq!(r.as_int(), Some(84));
}

#[test]
fn test_match_or_pattern() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("match 2 { 1 | 2 => { 10 } _ => { 20 } }").unwrap();
    assert_eq!(r.as_int(), Some(10));
}

#[test]
fn test_string_len() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute(r#"len("hello")"#).unwrap().as_int(), Some(5));
}

#[test]
fn test_list_len() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("len(list(1, 2, 3))").unwrap().as_int(), Some(3));
}

#[test]
fn test_nested_for() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("s = 0; for i in range(3) { for j in range(3) { s = s + 1 } }; s")
        .unwrap();
    assert_eq!(r.as_int(), Some(9));
}

#[test]
fn test_fstring_multiple() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(r#"a = 1; b = 2; f"{a} + {b} = {a + b}""#).unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("1 + 2 = 3"));
    r.decref();
}

#[test]
fn test_string_repeat() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(r#""ab" * 3"#).unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("ababab"));
    r.decref();
}

#[test]
fn test_tco_tail_recursion() {
    let mut p = PurePipeline::new().unwrap();
    // Tail-recursive sum: should not stack overflow
    let r = p
        .execute("sum_to = (n, acc=0) => { if n <= 0 { acc } else { sum_to(n - 1, acc + n) } }; sum_to(1000)")
        .unwrap();
    assert_eq!(r.as_int(), Some(500500));
}

#[test]
fn test_null_coalesce() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("nil ?? 42").unwrap().as_int(), Some(42));
    assert_eq!(p.execute("10 ?? 42").unwrap().as_int(), Some(10));
}

#[test]
fn test_fstring_format_spec() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(r#"f"{3.14159:.2f}""#).unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("3.14"));
    r.decref();
}

#[test]
fn test_fstring_alignment() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(r#"f"{'hi':>10}""#).unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("        hi"));
    r.decref();
}

#[test]
fn test_dict_kwargs() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("d = dict(a=1, b=2); d").unwrap();
    assert!(r.is_native_dict());
    let dict = unsafe { r.as_native_dict_ref().unwrap() };
    assert_eq!(dict.len(), 2);
    r.decref();
}

#[test]
fn test_closure_captures_only_needed() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("x = 10; y = 20; f = (a) => { a + x }; f(5)").unwrap();
    assert_eq!(r.as_int(), Some(15));
}

// --- Debug tests ---

use crate::vm::debug::{DebugCommand, DebugHook, PauseInfo};
use std::sync::mpsc;

/// Test hook that records pauses and responds with pre-set commands.
struct TestHook {
    tx: mpsc::Sender<(u32, Vec<(String, String)>)>,
    commands: Vec<DebugCommand>,
    call_count: usize,
}

impl DebugHook for TestHook {
    fn on_pause(&mut self, info: &PauseInfo) -> DebugCommand {
        let _ = self.tx.send((info.start_byte, info.locals.clone()));
        let cmd = if self.call_count < self.commands.len() {
            self.commands[self.call_count]
        } else {
            DebugCommand::Continue
        };
        self.call_count += 1;
        cmd
    }
}

#[test]
fn test_debug_breakpoint_pauses() {
    let source = "x = 10\ny = x * 2\nz = y + 1";
    let (tx, rx) = mpsc::channel();

    let mut p = PurePipeline::new().unwrap();
    p.set_source(source);
    p.add_breakpoint(2); // break at line 2

    let hook = TestHook {
        tx,
        commands: vec![DebugCommand::Continue],
        call_count: 0,
    };
    p.set_debug_hook(Box::new(hook));

    let result = p.execute(source).unwrap();
    assert_eq!(result.as_int(), Some(21)); // z = 20 + 1

    // Should have paused at line 2
    let (_, locals) = rx.recv().unwrap();
    // At line 2, x should be 10 (assigned on line 1)
    let x_val = locals.iter().find(|(name, _)| name == "x");
    assert!(x_val.is_some(), "x should be in locals at breakpoint");
    assert_eq!(x_val.unwrap().1, "10");
}

#[test]
fn test_debug_step_into() {
    let source = "x = 10\ny = x * 2\nz = y + 1";
    let (tx, rx) = mpsc::channel();

    let mut p = PurePipeline::new().unwrap();
    p.set_source(source);
    p.add_breakpoint(1); // break at line 1

    let hook = TestHook {
        tx,
        commands: vec![
            DebugCommand::StepInto, // step from line 1 to line 2
            DebugCommand::StepInto, // step from line 2 to line 3
            DebugCommand::Continue, // continue to end
        ],
        call_count: 0,
    };
    p.set_debug_hook(Box::new(hook));

    let result = p.execute(source).unwrap();
    assert_eq!(result.as_int(), Some(21));

    // Should have 3 pauses: line 1, line 2, line 3
    let mut pauses = Vec::new();
    while let Ok(pause) = rx.try_recv() {
        pauses.push(pause);
    }
    assert_eq!(pauses.len(), 3, "should have 3 pauses (one per line)");
}

#[test]
fn test_debug_no_hook_executes_normally() {
    // Without a debug hook, breakpoints should be ignored
    let source = "x = 10\ny = x * 2\nz = y + 1";
    let mut p = PurePipeline::new().unwrap();
    p.set_source(source);
    p.add_breakpoint(2);
    // No hook set
    let result = p.execute(source).unwrap();
    assert_eq!(result.as_int(), Some(21));
}

// =======================================================================
// Struct tests
// =======================================================================

#[test]
fn test_struct_basic_creation() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("struct Point { x; y }\np = Point(1, 2)\np.x").unwrap();
    assert_eq!(r.as_int(), Some(1));
}

#[test]
fn test_struct_field_access() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("struct Point { x; y }\np = Point(3, 4)\np.y").unwrap();
    assert_eq!(r.as_int(), Some(4));
}

#[test]
fn test_struct_field_mutation() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Point { x; y }
p = Point(1, 2)
p.x = 10
p.x
"#,
        )
        .unwrap();
    assert_eq!(r.as_int(), Some(10));
}

#[test]
fn test_struct_default_field() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("struct Config { debug = false; level = 1 }\nc = Config()\nc.level")
        .unwrap();
    assert_eq!(r.as_int(), Some(1));
}

#[test]
fn test_struct_method() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Point {
    x; y;
    sum(self) => { self.x + self.y }
}
p = Point(3, 4)
p.sum()
"#,
        )
        .unwrap();
    assert_eq!(r.as_int(), Some(7));
}

#[test]
fn test_struct_method_with_args() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Point {
    x; y;
    add(self, dx, dy) => { Point(self.x + dx, self.y + dy) }
}
p = Point(1, 2)
q = p.add(10, 20)
q.x
"#,
        )
        .unwrap();
    assert_eq!(r.as_int(), Some(11));
}

#[test]
fn test_struct_init() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Counter {
    value;
    init(self) => { self.value = self.value * 2 }
}
c = Counter(5)
c.value
"#,
        )
        .unwrap();
    assert_eq!(r.as_int(), Some(10));
}

#[test]
fn test_struct_typeof() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Point { x; y }
p = Point(1, 2)
typeof(p)
"#,
        )
        .unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("Point"));
    r.decref();
}

#[test]
fn test_struct_fstring_display() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Point { x; y }
p = Point(3, 4)
f"{p}"
"#,
        )
        .unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("Point(x=3, y=4)"));
    r.decref();
}

#[test]
fn test_struct_static_method() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Point {
    x; y;
    @static
    origin() => { Point(0, 0) }
}
o = Point.origin()
o.x
"#,
        )
        .unwrap();
    assert_eq!(r.as_int(), Some(0));
}

#[test]
fn test_struct_multiple_instances() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Point { x; y }
a = Point(1, 2)
b = Point(3, 4)
a.x + b.y
"#,
        )
        .unwrap();
    assert_eq!(r.as_int(), Some(5));
}

// =======================================================================
// Inheritance tests
// =======================================================================

#[test]
fn test_struct_extends_basic() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Animal { name }
struct Dog extends(Animal) { breed }
d = Dog("Rex", "Labrador")
f"{d.name} is a {d.breed}"
"#,
        )
        .unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("Rex is a Labrador"));
    r.decref();
}

#[test]
fn test_struct_extends_method_inherit() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Animal {
    name;
    speak(self) => { f"{self.name} speaks" }
}
struct Dog extends(Animal) { breed }
d = Dog("Rex", "Labrador")
d.speak()
"#,
        )
        .unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("Rex speaks"));
    r.decref();
}

#[test]
fn test_struct_extends_method_override() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Animal {
    name;
    speak(self) => { "..." }
}
struct Dog extends(Animal) {
    breed;
    speak(self) => { f"{self.name} barks" }
}
d = Dog("Rex", "Labrador")
d.speak()
"#,
        )
        .unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("Rex barks"));
    r.decref();
}

#[test]
fn test_struct_extends_default_fields() {
    let mut p = PurePipeline::new().unwrap();
    // Field order: [x (from Base, default=10), y (from Child)]
    // Child(5, 20) -> x=5, y=20
    let r = p
        .execute(
            r#"
struct Base { x = 10 }
struct Child extends(Base) { y }
c = Child(5, 20)
c.x + c.y
"#,
        )
        .unwrap();
    assert_eq!(r.as_int(), Some(25));
}

// =======================================================================
// Trait tests
// =======================================================================

#[test]
fn test_trait_basic() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
trait Greetable {
    greet(self) => { f"Hello, {self.name}" }
}
struct Person implements(Greetable) { name }
p = Person("Alice")
p.greet()
"#,
        )
        .unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("Hello, Alice"));
    r.decref();
}

#[test]
fn test_trait_multiple() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
trait HasName {
    get_name(self) => { self.name }
}
trait HasAge {
    get_age(self) => { self.age }
}
struct Person implements(HasName, HasAge) { name; age }
p = Person("Bob", 30)
f"{p.get_name()} is {p.get_age()}"
"#,
        )
        .unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("Bob is 30"));
    r.decref();
}

// =======================================================================
// Pattern matching tests
// =======================================================================

#[test]
#[ignore] // struct pattern syntax not yet parsed by tree-sitter in PurePipeline
fn test_struct_match_pattern() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Point { x; y }
p = Point(3, 4)
match p {
    case Point{x, y} => x + y
}
"#,
        )
        .unwrap();
    assert_eq!(r.as_int(), Some(7));
}

// =======================================================================
// Operator overloading tests
// =======================================================================

#[test]
fn test_struct_op_add() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Vec2 {
    x; y;
    op +(self, other) => { Vec2(self.x + other.x, self.y + other.y) }
}
a = Vec2(1, 2)
b = Vec2(3, 4)
c = a + b
c.x
"#,
        )
        .unwrap();
    assert_eq!(r.as_int(), Some(4));
}

#[test]
fn test_struct_op_eq() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Point {
    x; y;
    op ==(self, other) => { self.x == other.x and self.y == other.y }
}
a = Point(1, 2)
b = Point(1, 2)
a == b
"#,
        )
        .unwrap();
    assert_eq!(r.as_bool(), Some(true));
}

#[test]
fn test_struct_op_lt() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            r#"
struct Val {
    n;
    op <(self, other) => { self.n < other.n }
}
Val(1) < Val(2)
"#,
        )
        .unwrap();
    assert_eq!(r.as_bool(), Some(true));
}

#[test]
fn test_list_slice() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("x = [10, 20, 30, 40, 50]; x[1:3]").unwrap();
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 2);
    assert_eq!(list.get(0).unwrap(), Value::from_int(20));
    assert_eq!(list.get(1).unwrap(), Value::from_int(30));
}

#[test]
fn test_list_slice_negative() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("[1, 2, 3, 4][:-1]").unwrap();
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 3);
}

#[test]
fn test_list_slice_open_start() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("[1, 2, 3][1:]").unwrap();
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 2);
    assert_eq!(list.get(0).unwrap(), Value::from_int(2));
}

#[test]
fn test_string_slice() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(r#""hello"[1:4]"#).unwrap();
    let s = unsafe { r.as_native_str_ref().unwrap() };
    assert_eq!(s, "ell");
}

#[test]
fn test_list_slice_step() {
    let mut p = PurePipeline::new().unwrap();
    // [0,1,2,3,4][::2] -> [0, 2, 4]
    let r = p.execute("[0, 1, 2, 3, 4][::2]").unwrap();
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 3);
    assert_eq!(list.get(0).unwrap(), Value::from_int(0));
    assert_eq!(list.get(1).unwrap(), Value::from_int(2));
    assert_eq!(list.get(2).unwrap(), Value::from_int(4));
}

#[test]
fn test_list_slice_reverse() {
    let mut p = PurePipeline::new().unwrap();
    // [1,2,3][::-1] -> [3, 2, 1]
    let r = p.execute("[1, 2, 3][::-1]").unwrap();
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 3);
    assert_eq!(list.get(0).unwrap(), Value::from_int(3));
    assert_eq!(list.get(1).unwrap(), Value::from_int(2));
    assert_eq!(list.get(2).unwrap(), Value::from_int(1));
}

#[test]
fn test_string_slice_reverse() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(r#""hello"[::-1]"#).unwrap();
    let s = unsafe { r.as_native_str_ref().unwrap() };
    assert_eq!(s, "olleh");
}

#[test]
fn test_list_slice_step_with_bounds() {
    let mut p = PurePipeline::new().unwrap();
    // [0,1,2,3,4,5,6,7,8,9][1:8:2] -> [1, 3, 5, 7]
    let r = p.execute("[0, 1, 2, 3, 4, 5, 6, 7, 8, 9][1:8:2]").unwrap();
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 4);
    assert_eq!(list.get(0).unwrap(), Value::from_int(1));
    assert_eq!(list.get(3).unwrap(), Value::from_int(7));
}

// --- Higher-order function builtins ---

#[test]
fn test_map() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("map((x) => { x * 2 }, [1, 2, 3])").unwrap();
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 3);
    assert_eq!(list.get(0).unwrap().as_int(), Some(2));
    assert_eq!(list.get(1).unwrap().as_int(), Some(4));
    assert_eq!(list.get(2).unwrap().as_int(), Some(6));
}

#[test]
fn test_map_with_builtin() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("map(str, [1, 2, 3])").unwrap();
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 3);
    assert_eq!(unsafe { list.get(0).unwrap().as_native_str_ref() }, Some("1"));
}

#[test]
fn test_filter() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("filter((x) => { x > 2 }, [1, 2, 3, 4, 5])").unwrap();
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 3);
    assert_eq!(list.get(0).unwrap().as_int(), Some(3));
    assert_eq!(list.get(1).unwrap().as_int(), Some(4));
    assert_eq!(list.get(2).unwrap().as_int(), Some(5));
}

#[test]
fn test_fold() {
    let mut p = PurePipeline::new().unwrap();
    // fold(iterable, init, func)
    let r = p.execute("fold([1, 2, 3, 4], 0, (acc, x) => { acc + x })").unwrap();
    assert_eq!(r.as_int(), Some(10));
}

#[test]
fn test_fold_with_string() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(r#"fold(["a", "b", "c"], "", (acc, x) => { acc + x })"#)
        .unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("abc"));
}

#[test]
fn test_reduce() {
    let mut p = PurePipeline::new().unwrap();
    // reduce(iterable, func)
    let r = p.execute("reduce([1, 2, 3, 4], (acc, x) => { acc + x })").unwrap();
    assert_eq!(r.as_int(), Some(10));
}

#[test]
fn test_reduce_empty_error() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("reduce([], (acc, x) => { acc + x })");
    assert!(r.is_err());
}

#[test]
fn test_map_empty() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("map((x) => { x * 2 }, [])").unwrap();
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 0);
}

#[test]
fn test_fold_in_function() {
    let mut p = PurePipeline::new().unwrap();
    // HOF called from within a user function (tests re-entrant dispatch)
    let r = p
        .execute("total = (xs) => { fold(xs, 0, (a, x) => { a + x }) }\ntotal([10, 20, 30])")
        .unwrap();
    assert_eq!(r.as_int(), Some(60));
}

#[test]
fn test_hof_chained() {
    let mut p = PurePipeline::new().unwrap();
    // map then fold: sum of squares of even numbers
    let r = p
        .execute(
            "xs = filter((x) => { x % 2 == 0 }, [1,2,3,4,5,6])\n\
                 sq = map((x) => { x * x }, xs)\n\
                 fold(sq, 0, (a, x) => { a + x })",
        )
        .unwrap();
    // 2^2 + 4^2 + 6^2 = 4 + 16 + 36 = 56
    assert_eq!(r.as_int(), Some(56));
}

#[test]
fn test_hof_with_tuple_input() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("fold(tuple(1,2,3), 0, (a, x) => { a + x })").unwrap();
    assert_eq!(r.as_int(), Some(6));
}

#[test]
fn test_map_not_iterable_error() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("map((x) => { x }, 42)");
    assert!(r.is_err());
}

#[test]
fn test_fold_wrong_arity_error() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("fold([1,2], (a, x) => { a + x })");
    assert!(r.is_err());
}

#[test]
fn test_map_with_range() {
    let mut p = PurePipeline::new().unwrap();
    // range() produces a non-list iterable
    let r = p.execute("map((x) => { x * x }, range(5))").unwrap();
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 5);
    assert_eq!(list.get(0).unwrap().as_int(), Some(0));
    assert_eq!(list.get(4).unwrap().as_int(), Some(16));
}

#[test]
fn test_fold_with_range() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("fold(range(5), 0, (a, x) => { a + x })").unwrap();
    assert_eq!(r.as_int(), Some(10)); // 0+1+2+3+4
}

#[test]
fn test_filter_with_range() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("filter((x) => { x % 2 == 0 }, range(6))").unwrap();
    let list = unsafe { r.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 3); // 0, 2, 4
    assert_eq!(list.get(0).unwrap().as_int(), Some(0));
    assert_eq!(list.get(2).unwrap().as_int(), Some(4));
}

#[test]
fn test_reduce_with_range() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("reduce(range(1, 5), (a, x) => { a * x })").unwrap();
    assert_eq!(r.as_int(), Some(24)); // 1*2*3*4
}

// --- Builtin batch: numerics + string utils ---

#[test]
fn test_round() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("round(3.7)").unwrap().as_int(), Some(4));
    assert_eq!(p.execute("round(3.2)").unwrap().as_int(), Some(3));
    assert!((p.execute("round(3.14159, 2)").unwrap().as_float().unwrap() - 3.14).abs() < 1e-10);
    assert_eq!(p.execute("round(5)").unwrap().as_int(), Some(5));
    // Banker's rounding: tie-to-even
    assert_eq!(p.execute("round(2.5)").unwrap().as_int(), Some(2));
    assert_eq!(p.execute("round(3.5)").unwrap().as_int(), Some(4));
    assert_eq!(p.execute("round(0.5)").unwrap().as_int(), Some(0));
    assert_eq!(p.execute("round(1.5)").unwrap().as_int(), Some(2));
}

#[test]
fn test_pow() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("pow(2, 10)").unwrap().as_int(), Some(1024));
    assert!((p.execute("pow(2.0, 0.5)").unwrap().as_float().unwrap() - std::f64::consts::SQRT_2).abs() < 1e-10);
    assert_eq!(p.execute("pow(2, 10, 100)").unwrap().as_int(), Some(24));
}

#[test]
fn test_divmod() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("divmod(17, 5)").unwrap();
    let t = unsafe { r.as_native_tuple_ref().unwrap() };
    assert_eq!(t.get(0).unwrap().as_int(), Some(3));
    assert_eq!(t.get(1).unwrap().as_int(), Some(2));
}

#[test]
fn test_divmod_negative() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("divmod(-7, 3)").unwrap();
    let t = unsafe { r.as_native_tuple_ref().unwrap() };
    // Python floor division: -7 // 3 = -3, -7 % 3 = 2
    assert_eq!(t.get(0).unwrap().as_int(), Some(-3));
    assert_eq!(t.get(1).unwrap().as_int(), Some(2));
}

#[test]
fn test_chr_ord() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("chr(65)").unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("A"));
    assert_eq!(p.execute(r#"ord("A")"#).unwrap().as_int(), Some(65));
    assert_eq!(p.execute(r#"ord("€")"#).unwrap().as_int(), Some(8364));
}

#[test]
fn test_hex_bin_oct() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(
        unsafe { p.execute("hex(255)").unwrap().as_native_str_ref() },
        Some("0xff")
    );
    assert_eq!(
        unsafe { p.execute("hex(-1)").unwrap().as_native_str_ref() },
        Some("-0x1")
    );
    assert_eq!(
        unsafe { p.execute("bin(10)").unwrap().as_native_str_ref() },
        Some("0b1010")
    );
    assert_eq!(
        unsafe { p.execute("oct(8)").unwrap().as_native_str_ref() },
        Some("0o10")
    );
}

#[test]
fn test_repr() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(r#"repr("hello")"#).unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("'hello'"));
    let r2 = p.execute("repr(42)").unwrap();
    assert_eq!(unsafe { r2.as_native_str_ref() }, Some("42"));
}

#[test]
fn test_hash() {
    let mut p = PurePipeline::new().unwrap();
    // hash returns a numeric value (may be bigint if hash exceeds SmallInt range)
    let h1 = p.execute("hash(42)").unwrap();
    assert!(h1.as_int().is_some() || h1.is_bigint());
    // same value -> same hash (compare via display_string since may be bigint)
    let h2 = p.execute("hash(42)").unwrap();
    assert_eq!(h1.display_string(), h2.display_string());
    // unhashable type errors
    assert!(p.execute("hash([1, 2])").is_err());
}

#[test]
fn test_callable() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("callable((x) => { x })").unwrap().as_bool(), Some(true));
    assert_eq!(p.execute("callable(42)").unwrap().as_bool(), Some(false));
    assert_eq!(p.execute("callable(len)").unwrap().as_bool(), Some(true));
    // Arbitrary strings are NOT callable
    assert_eq!(p.execute(r#"callable("hello")"#).unwrap().as_bool(), Some(false));
}

#[test]
fn test_hash_tuple() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("hash(tuple(1, 2))").unwrap();
    assert!(r.as_int().is_some() || r.is_bigint());
}

// --- isinstance ---

#[test]
fn test_isinstance_builtin_types() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute(r#"isinstance(42, "int")"#).unwrap().as_bool(), Some(true));
    assert_eq!(p.execute(r#"isinstance("hi", "str")"#).unwrap().as_bool(), Some(true));
    assert_eq!(p.execute(r#"isinstance(3.14, "float")"#).unwrap().as_bool(), Some(true));
    assert_eq!(p.execute(r#"isinstance(true, "bool")"#).unwrap().as_bool(), Some(true));
    assert_eq!(p.execute(r#"isinstance([1], "list")"#).unwrap().as_bool(), Some(true));
    assert_eq!(p.execute(r#"isinstance(42, "str")"#).unwrap().as_bool(), Some(false));
}

#[test]
fn test_isinstance_tuple_of_types() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(
        p.execute(r#"isinstance(42, tuple("int", "str"))"#).unwrap().as_bool(),
        Some(true)
    );
    assert_eq!(
        p.execute(r#"isinstance(3.14, tuple("int", "str"))"#).unwrap().as_bool(),
        Some(false)
    );
}

#[test]
fn test_isinstance_struct() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("struct Foo { x }\nf = Foo(1)\nisinstance(f, Foo)").unwrap();
    assert_eq!(r.as_bool(), Some(true));
}

#[test]
fn test_isinstance_struct_inheritance() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("struct Base { x }\nstruct Child extends(Base) { y }\nc = Child(1, 2)\nisinstance(c, Base)")
        .unwrap();
    assert_eq!(r.as_bool(), Some(true));
}

#[test]
fn test_isinstance_struct_negative() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("struct A { x }\nstruct B { y }\na = A(1)\nisinstance(a, B)")
        .unwrap();
    assert_eq!(r.as_bool(), Some(false));
}

// --- Tagged unions ---

const OPTION_DECL: &str = "union Option[T] { Some(value: T); None; }";

#[test]
fn test_union_payload_construct_and_field() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(&format!("{OPTION_DECL}\nx = Option.Some(42)\nx.value"))
        .unwrap();
    assert_eq!(r.as_int(), Some(42));
}

#[test]
fn test_union_nullary_singleton_eq() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(&format!("{OPTION_DECL}\nOption.None == Option.None"))
        .unwrap();
    assert_eq!(r.as_bool(), Some(true));
}

#[test]
fn test_union_payload_structural_eq() {
    let mut p = PurePipeline::new().unwrap();
    p.execute(OPTION_DECL).unwrap();
    assert_eq!(
        p.execute("Option.Some(1) == Option.Some(1)").unwrap().as_bool(),
        Some(true)
    );
    assert_eq!(
        p.execute("Option.Some(1) == Option.Some(2)").unwrap().as_bool(),
        Some(false)
    );
    assert_eq!(
        p.execute("Option.Some([1, 2]) == Option.Some([1, 2])")
            .unwrap()
            .as_bool(),
        Some(true)
    );
    assert_eq!(
        p.execute("Option.Some(1) != Option.None").unwrap().as_bool(),
        Some(true)
    );
}

#[test]
fn test_union_match_payload() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
            .execute(&format!(
                "{OPTION_DECL}\nmatch Option.Some(7) {{\n    Option.Some{{value}} => {{ value * 2 }}\n    Option.None => {{ 0 }}\n}}"
            ))
            .unwrap();
    assert_eq!(r.as_int(), Some(14));
}

#[test]
fn test_union_match_nullary() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
            .execute(&format!(
                "{OPTION_DECL}\nmatch Option.None {{\n    Option.Some{{value}} => {{ value }}\n    Option.None => {{ -1 }}\n}}"
            ))
            .unwrap();
    assert_eq!(r.as_int(), Some(-1));
}

#[test]
fn test_union_typeof() {
    let mut p = PurePipeline::new().unwrap();
    p.execute(OPTION_DECL).unwrap();
    let r = p.execute("typeof(Option.Some(1))").unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("Option.Some"));
    r.decref();
    let r = p.execute("typeof(Option.None)").unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("Option"));
    r.decref();
}

#[test]
fn test_union_unknown_variant_errors() {
    let mut p = PurePipeline::new().unwrap();
    p.execute(OPTION_DECL).unwrap();
    let err = p.execute("Option.Nope").unwrap_err().to_string();
    assert!(err.contains("no variant"), "unexpected error: {err}");
}

#[test]
fn test_union_multi_field_variant() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            "union Shape { Circle(r); Rect(w, h); }\n\
                 match Shape.Rect(3, 4) {\n    Shape.Circle{r} => { r }\n    Shape.Rect{w, h} => { w * h }\n}",
        )
        .unwrap();
    assert_eq!(r.as_int(), Some(12));
}

const OPTION_WITH_METHODS: &str = "union Option {\n    Some(value);\n    None;\n    map(self, f) => {\n        match self {\n            Option.Some{value} => { Option.Some(f(value)) }\n            Option.None => { Option.None }\n        }\n    }\n    unwrap_or(self, default) => {\n        match self {\n            Option.Some{value} => { value }\n            Option.None => { default }\n        }\n    }\n}";

#[test]
fn test_union_method_on_payload_variant() {
    let mut p = PurePipeline::new().unwrap();
    p.execute(OPTION_WITH_METHODS).unwrap();
    let r = p.execute("Option.Some(21).map((x) => { x * 2 }).unwrap_or(0)").unwrap();
    assert_eq!(r.as_int(), Some(42));
}

#[test]
fn test_union_method_on_nullary_variant() {
    let mut p = PurePipeline::new().unwrap();
    p.execute(OPTION_WITH_METHODS).unwrap();
    let r = p.execute("Option.None.unwrap_or(-1)").unwrap();
    assert_eq!(r.as_int(), Some(-1));
    let r = p.execute("Option.None.map((x) => { x }).unwrap_or(-2)").unwrap();
    assert_eq!(r.as_int(), Some(-2));
}

#[test]
fn test_union_method_unknown_errors() {
    let mut p = PurePipeline::new().unwrap();
    p.execute(OPTION_WITH_METHODS).unwrap();
    let err = p.execute("Option.None.nope()").unwrap_err().to_string();
    assert!(err.contains("no method"), "unexpected error: {err}");
    // A payload variant is a native struct: the struct arm covers fields,
    // methods and statics, so it says "attribute" (mirrors catnip_rs).
    let err = p.execute("Option.Some(1).nope()").unwrap_err().to_string();
    assert!(err.contains("no attribute"), "unexpected error: {err}");
}

#[test]
fn test_union_method_name_collisions_rejected() {
    let mut p = PurePipeline::new().unwrap();
    let err = p.execute("union U { A; A(self) => { 1 } }").unwrap_err().to_string();
    assert!(err.contains("collides"), "unexpected error: {err}");
    let err = p.execute("union V { A; init(self) => { 1 } }").unwrap_err().to_string();
    assert!(err.contains("no init"), "unexpected error: {err}");
}

// --- Deep equality (collections through the Eq opcode) ---

#[test]
fn test_deep_eq_collections() {
    let mut p = PurePipeline::new().unwrap();
    assert_eq!(p.execute("[1, 2] == [1, 2]").unwrap().as_bool(), Some(true));
    assert_eq!(p.execute("[1, 2] == [1, 3]").unwrap().as_bool(), Some(false));
    assert_eq!(p.execute("[[1], [2]] == [[1], [2]]").unwrap().as_bool(), Some(true));
    assert_eq!(
        p.execute(r#"{"a": 1, "b": 2} == {"b": 2, "a": 1}"#).unwrap().as_bool(),
        Some(true)
    );
    assert_eq!(p.execute(r#"{"a": 1} == {"a": 2}"#).unwrap().as_bool(), Some(false));
}

#[test]
fn test_deep_eq_struct_instances() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("struct P { x; y }").unwrap();
    assert_eq!(p.execute("P(1, 2) == P(1, 2)").unwrap().as_bool(), Some(true));
    assert_eq!(p.execute("P(1, 2) == P(1, 3)").unwrap().as_bool(), Some(false));
    assert_eq!(p.execute("P(1, [2]) == P(1, [2])").unwrap().as_bool(), Some(true));
}

#[test]
fn test_contains_struct_in_list() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("struct P { x; y }").unwrap();
    assert_eq!(
        p.execute("P(1, 2) in [P(0, 0), P(1, 2)]").unwrap().as_bool(),
        Some(true)
    );
    assert_eq!(
        p.execute("P(9, 9) in [P(0, 0), P(1, 2)]").unwrap().as_bool(),
        Some(false)
    );
    assert_eq!(p.execute("P(9, 9) not in [P(0, 0)]").unwrap().as_bool(), Some(true));
    p.execute("union Shape { Circle(r); Origin; }").unwrap();
    assert_eq!(
        p.execute("Shape.Circle(1) in [Shape.Origin, Shape.Circle(1)]")
            .unwrap()
            .as_bool(),
        Some(true)
    );
    assert_eq!(
        p.execute("Shape.Origin in [Shape.Circle(1), Shape.Origin]")
            .unwrap()
            .as_bool(),
        Some(true)
    );
    // tuple() is variadic in Catnip: tuple(a, b) builds (a, b).
    assert_eq!(
        p.execute("t = tuple(Shape.Circle(2), Shape.Origin)\nShape.Circle(2) in t")
            .unwrap()
            .as_bool(),
        Some(true)
    );
}

// --- Registry-aware str/repr (struct fields, symbol names) ---

#[test]
fn test_str_repr_with_registry() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("enum Color { red; green }\nstruct P { x; y }").unwrap();
    let cases = [
        ("str(Color.red)", "Color.red"),
        ("repr(Color.red)", "Color.red"),
        ("str(P(1, 2))", "P(x=1, y=2)"),
        ("str(5)", "5"),
        (r#"str("abc")"#, "abc"),
        (r#"repr("abc")"#, "'abc'"),
    ];
    for (expr, expected) in cases {
        let r = p.execute(expr).unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some(expected), "expr: {expr}");
        r.decref();
    }
}

// --- Registry-aware display of nested collections (struct/symbol elements) ---

#[test]
fn test_display_collection_with_registry() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("enum Color { red; green; blue }\nstruct P { x; y }").unwrap();
    let cases = [
        ("str([P(1, 2), P(3, 4)])", "[P(x=1, y=2), P(x=3, y=4)]"),
        ("str([Color.red, Color.green])", "[Color.red, Color.green]"),
        ("str([[Color.blue]])", "[[Color.blue]]"),
        ("repr([Color.red])", "[Color.red]"),
        ("str({1: Color.red})", "{1: Color.red}"),
        ("str(tuple(Color.red))", "(Color.red,)"),
        ("str(tuple(P(1, 2), Color.green))", "(P(x=1, y=2), Color.green)"),
    ];
    for (expr, expected) in cases {
        let r = p.execute(expr).unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some(expected), "expr: {expr}");
        r.decref();
    }
}

// --- Registry-aware index/count/remove (struct + variant payloads) ---

#[test]
fn test_index_count_remove_struct() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("struct P { x; y }").unwrap();
    p.execute("xs = [P(1, 2), P(1, 2), P(3, 4)]").unwrap();
    assert_eq!(p.execute("xs.index(P(3, 4))").unwrap().as_int(), Some(2));
    assert_eq!(p.execute("xs.count(P(1, 2))").unwrap().as_int(), Some(2));
    // remove first match, then P(3, 4) shifts to index 1
    p.execute("xs.remove(P(1, 2))").unwrap();
    assert_eq!(p.execute("xs.index(P(3, 4))").unwrap().as_int(), Some(1));
    assert_eq!(p.execute("xs.count(P(1, 2))").unwrap().as_int(), Some(1));
    // tuple index/count
    assert_eq!(
        p.execute("t = tuple(P(1, 2), P(3, 4))\nt.index(P(3, 4))")
            .unwrap()
            .as_int(),
        Some(1)
    );
    // variant payloads
    p.execute("union Shape { Circle(r); Origin; }").unwrap();
    p.execute("ss = [Shape.Circle(1), Shape.Origin, Shape.Circle(1)]")
        .unwrap();
    assert_eq!(p.execute("ss.index(Shape.Origin)").unwrap().as_int(), Some(1));
    assert_eq!(p.execute("ss.count(Shape.Circle(1))").unwrap().as_int(), Some(2));
    // absent value errors
    assert!(p.execute("xs.index(P(9, 9))").is_err());
}

// --- Hashing: complex and structural struct/variant keys ---

#[test]
fn test_hash_complex() {
    let mut p = PurePipeline::new().unwrap();
    // hash(complex) is stable and usable (may exceed the NaN-boxed int range,
    // so compare in-language instead of reading as_int).
    assert_eq!(p.execute("hash(3 + 4j) == hash(3 + 4j)").unwrap().as_bool(), Some(true));
    // pure-real complex hashes like float
    assert_eq!(p.execute("hash(2 + 0j) == hash(2.0)").unwrap().as_bool(), Some(true));
    // complex as a dict key round-trips
    assert_eq!(
        unsafe {
            p.execute(
                r#"d = {(1 + 2j): "z"}
d[1 + 2j]"#,
            )
            .unwrap()
            .as_native_str_ref()
        },
        Some("z")
    );
}

#[test]
fn test_hash_struct_structural() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("struct P { x; y }").unwrap();
    // equal structs hash equally; different fields differ
    assert_eq!(
        p.execute("hash(P(1, 2)) == hash(P(1, 2))").unwrap().as_bool(),
        Some(true)
    );
    assert_eq!(
        p.execute("hash(P(1, 2)) == hash(P(1, 3))").unwrap().as_bool(),
        Some(false)
    );
    // nested struct fields hash structurally too
    p.execute("struct Q { p }").unwrap();
    assert_eq!(
        p.execute("hash(Q(P(1, 2))) == hash(Q(P(1, 2)))").unwrap().as_bool(),
        Some(true)
    );
}

#[test]
fn test_struct_as_dict_key() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("struct P { x; y }").unwrap();
    p.execute(r#"d = {P(1, 2): "a", P(3, 4): "b"}"#).unwrap();
    assert_eq!(
        unsafe { p.execute("d[P(1, 2)]").unwrap().as_native_str_ref() },
        Some("a")
    );
    assert_eq!(
        unsafe { p.execute("d[P(3, 4)]").unwrap().as_native_str_ref() },
        Some("b")
    );
    assert_eq!(p.execute("P(1, 2) in d").unwrap().as_bool(), Some(true));
    assert_eq!(p.execute("P(9, 9) in d").unwrap().as_bool(), Some(false));
    // assignment by struct key, then lookup
    p.execute(r#"d[P(5, 6)] = "c""#).unwrap();
    assert_eq!(
        unsafe { p.execute("d.get(P(5, 6))").unwrap().as_native_str_ref() },
        Some("c")
    );
}

#[test]
fn test_struct_as_set_member() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("struct P { x; y }").unwrap();
    // dedup by value
    assert_eq!(
        p.execute("len(set(P(1, 2), P(1, 2), P(3, 4)))").unwrap().as_int(),
        Some(2)
    );
    p.execute("s = set(P(1, 2))").unwrap();
    assert_eq!(p.execute("P(1, 2) in s").unwrap().as_bool(), Some(true));
    assert_eq!(p.execute("P(0, 0) in s").unwrap().as_bool(), Some(false));
    p.execute("s.add(P(7, 8))").unwrap();
    assert_eq!(p.execute("P(7, 8) in s").unwrap().as_bool(), Some(true));
    p.execute("s.remove(P(1, 2))").unwrap();
    assert_eq!(p.execute("P(1, 2) in s").unwrap().as_bool(), Some(false));
}

#[test]
fn test_variant_payload_as_key() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("union Shape { Circle(r); Origin; }").unwrap();
    // payload variant (a struct instance) hashes structurally
    p.execute(r#"d = {Shape.Circle(5): "c"}"#).unwrap();
    assert_eq!(
        unsafe { p.execute("d[Shape.Circle(5)]").unwrap().as_native_str_ref() },
        Some("c")
    );
    assert_eq!(p.execute("Shape.Circle(5) in d").unwrap().as_bool(), Some(true));
    assert_eq!(p.execute("Shape.Circle(6) in d").unwrap().as_bool(), Some(false));
    // nullary variant (a symbol) is already hashable
    assert_eq!(
        p.execute("hash(Shape.Origin) == hash(Shape.Origin)").unwrap().as_bool(),
        Some(true)
    );
}

#[test]
fn test_struct_frozen_after_hash() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("struct P { x; y }").unwrap();
    p.execute("p = P(1, 2)").unwrap();
    p.execute("hash(p)").unwrap();
    let err = p.execute("p.x = 9").unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("after it has been hashed"),
        "expected freeze-on-hash error, got: {msg}"
    );
    // unhashed instances stay mutable
    p.execute("q = P(1, 2)").unwrap();
    p.execute("q.x = 9").unwrap();
    assert_eq!(p.execute("q.x").unwrap().as_int(), Some(9));
}

#[test]
fn test_struct_op_eq_unhashable() {
    let mut p = PurePipeline::new().unwrap();
    // a custom op == (-> op_eq method) makes the struct unhashable: a
    // structural hash would not stay consistent with the custom equality.
    p.execute("struct P { x\n op ==(self, other) => { self.x == other.x } }")
        .unwrap();
    let err = p.execute("hash(P(1))").unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("unhashable"), "expected unhashable error, got: {msg}");
}

#[test]
fn test_struct_op_hash_honored() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("struct B { v\n op_hash(self) => { self.v } }").unwrap();
    // hash() returns the custom op_hash result directly (parity with the
    // Python CLI), not the structural hash.
    assert_eq!(p.execute("hash(B(1)) == 1").unwrap().as_bool(), Some(true));
    assert_eq!(p.execute("hash(B(42)) == 42").unwrap().as_bool(), Some(true));
    // still consistent for equal instances
    assert_eq!(p.execute("hash(B(1)) == hash(B(1))").unwrap().as_bool(), Some(true));
}

#[test]
fn test_struct_op_hash_as_dict_key() {
    let mut p = PurePipeline::new().unwrap();
    // op_hash returns a constant: distinct instances collide on the hash but
    // stay distinct keys (structural equality drives lookup, not the hash).
    p.execute("struct K { v\n op_hash(self) => { 0 } }").unwrap();
    p.execute(r#"d = {K(1): "a", K(2): "b"}"#).unwrap();
    assert_eq!(p.execute("len(d)").unwrap().as_int(), Some(2));
    assert_eq!(unsafe { p.execute("d[K(1)]").unwrap().as_native_str_ref() }, Some("a"));
    assert_eq!(unsafe { p.execute("d[K(2)]").unwrap().as_native_str_ref() }, Some("b"));
    assert_eq!(p.execute("K(1) in d").unwrap().as_bool(), Some(true));
    assert_eq!(p.execute("K(9) in d").unwrap().as_bool(), Some(false));
}

#[test]
fn test_struct_op_hash_nested() {
    let mut p = PurePipeline::new().unwrap();
    // op_hash is honored at every nesting level: an inner struct's op_hash
    // feeds the hash of the outer struct that contains it.
    p.execute("struct Inner { v\n op_hash(self) => { self.v * 100 } }")
        .unwrap();
    p.execute("struct Outer { a }").unwrap();
    p.execute(r#"d = {Outer(Inner(1)): "x"}"#).unwrap();
    assert_eq!(
        unsafe { p.execute("d[Outer(Inner(1))]").unwrap().as_native_str_ref() },
        Some("x")
    );
    assert_eq!(p.execute("Outer(Inner(1)) in d").unwrap().as_bool(), Some(true));
    assert_eq!(p.execute("Outer(Inner(2)) in d").unwrap().as_bool(), Some(false));
}

#[test]
fn test_struct_op_hash_must_return_int() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("struct Bad { v\n op_hash(self) => { \"nope\" } }").unwrap();
    let err = p.execute("hash(Bad(1))").unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("op_hash must return an int"), "got: {msg}");
}

#[test]
fn test_struct_keys_display_and_iterate() {
    let mut p = PurePipeline::new().unwrap();
    p.execute("struct P { x; y }").unwrap();
    // display of a dict keyed by structs
    let r = p.execute(r#"str({P(1, 2): "a"})"#).unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some(r#"{P(x=1, y=2): 'a'}"#));
    r.decref();
    // display of a set of structs
    let r = p.execute("str(set(P(1, 2)))").unwrap();
    assert_eq!(unsafe { r.as_native_str_ref() }, Some("{P(x=1, y=2)}"));
    r.decref();
    // iteration over dict keys yields the structs back (keys() is a list)
    assert_eq!(
        p.execute("ks = {P(1, 2): 0}.keys()\nks[0] == P(1, 2)")
            .unwrap()
            .as_bool(),
        Some(true)
    );
    // iteration over a set
    assert_eq!(
        p.execute("total = 0\nfor s in set(P(1, 2), P(3, 4)) { total = total + s.x }\ntotal")
            .unwrap()
            .as_int(),
        Some(4)
    );
}

// -----------------------------------------------------------------------
// Generic nominal unions (Option[int], Result[T, E]) -- PureVM boundary
// -----------------------------------------------------------------------

const OPTION_GEN: &str = "union Option[T] { Some(value: T); None }\n";
const RESULT_GEN: &str = "union Result[T, E] { Ok(value: T); Err(error: E) }\n";

#[test]
fn test_generic_accepts_matching_payload() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(&format!(
            "{OPTION_GEN}f = (x: Option[int]) => {{ 1 }}\nf(Option.Some(42))"
        ))
        .unwrap();
    assert_eq!(r.as_int(), Some(1));
}

#[test]
fn test_generic_rejects_wrong_payload() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(&format!(
        "{OPTION_GEN}f = (x: Option[int]) => {{ 1 }}\nf(Option.Some(\"s\"))"
    ));
    assert!(r.is_err(), "Option.Some(str) into Option[int] must be rejected");
}

#[test]
fn test_generic_accepts_nullary_variant() {
    // Option.None carries no payload -> passes any Option[T] boundary.
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(&format!("{OPTION_GEN}f = (x: Option[int]) => {{ 1 }}\nf(Option.None)"))
        .unwrap();
    assert_eq!(r.as_int(), Some(1));
}

#[test]
fn test_generic_rejects_non_member() {
    // A non-Option value into an Option[int] slot is a membership failure.
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(&format!("{OPTION_GEN}f = (x: Option[int]) => {{ 1 }}\nf(5)"));
    assert!(r.is_err(), "a bare int into Option[int] must be rejected");
}

#[test]
fn test_generic_payload_numeric_tower() {
    // Covariant argument: an int payload satisfies Option[float] (int <: float).
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(&format!(
            "{OPTION_GEN}f = (x: Option[float]) => {{ 1 }}\nf(Option.Some(3))"
        ))
        .unwrap();
    assert_eq!(r.as_int(), Some(1));
}

#[test]
fn test_generic_result_two_params() {
    let mut p = PurePipeline::new().unwrap();
    // Ok(int) accepted in Result[int, str].
    assert!(
        p.execute(&format!(
            "{RESULT_GEN}f = (x: Result[int, str]) => {{ 1 }}\nf(Result.Ok(1))"
        ))
        .is_ok()
    );
    // Ok(str) rejected (position 0 is int).
    let mut p2 = PurePipeline::new().unwrap();
    assert!(
        p2.execute(&format!(
            "{RESULT_GEN}f = (x: Result[int, str]) => {{ 1 }}\nf(Result.Ok(\"s\"))"
        ))
        .is_err()
    );
    // Err(str) accepted (position 1 is str).
    let mut p3 = PurePipeline::new().unwrap();
    assert!(
        p3.execute(&format!(
            "{RESULT_GEN}f = (x: Result[int, str]) => {{ 1 }}\nf(Result.Err(\"boom\"))"
        ))
        .is_ok()
    );
    // Err(int) rejected (position 1 is str).
    let mut p4 = PurePipeline::new().unwrap();
    assert!(
        p4.execute(&format!(
            "{RESULT_GEN}f = (x: Result[int, str]) => {{ 1 }}\nf(Result.Err(7))"
        ))
        .is_err()
    );
}

#[test]
fn test_generic_nested_composite_payload() {
    // Option[list[int]]: the payload must be a list whose elements are int.
    let mut p = PurePipeline::new().unwrap();
    assert!(
        p.execute(&format!(
            "{OPTION_GEN}f = (x: Option[list[int]]) => {{ 1 }}\nf(Option.Some(list(1, 2, 3)))"
        ))
        .is_ok()
    );
    let mut p2 = PurePipeline::new().unwrap();
    assert!(
        p2.execute(&format!(
            "{OPTION_GEN}f = (x: Option[list[int]]) => {{ 1 }}\nf(Option.Some(list(\"a\")))"
        ))
        .is_err()
    );
}

#[test]
fn test_generic_as_union_member() {
    // A generic union nested in a type-union member: `int | Option[int]`.
    // A bare int satisfies the int arm; a well-typed Option satisfies the
    // generic arm; a mistyped Option satisfies neither -> rejected.
    let mut p = PurePipeline::new().unwrap();
    assert!(
        p.execute(&format!("{OPTION_GEN}f = (x: int | Option[int]) => {{ 1 }}\nf(7)"))
            .is_ok()
    );
    let mut p2 = PurePipeline::new().unwrap();
    assert!(
        p2.execute(&format!(
            "{OPTION_GEN}f = (x: int | Option[int]) => {{ 1 }}\nf(Option.Some(1))"
        ))
        .is_ok()
    );
    let mut p3 = PurePipeline::new().unwrap();
    assert!(
        p3.execute(&format!(
            "{OPTION_GEN}f = (x: int | Option[int]) => {{ 1 }}\nf(Option.Some(\"s\"))"
        ))
        .is_err()
    );
}

#[test]
fn test_generic_as_set_element() {
    // A set element carries a generic union: parity with the value path.
    let mut p = PurePipeline::new().unwrap();
    assert!(
        p.execute(&format!(
            "{OPTION_GEN}f = (x: set[Option[int]]) => {{ 1 }}\nf(set(Option.Some(1)))"
        ))
        .is_ok()
    );
    let mut p2 = PurePipeline::new().unwrap();
    assert!(
        p2.execute(&format!(
            "{OPTION_GEN}f = (x: set[Option[int]]) => {{ 1 }}\nf(set(Option.Some(\"s\")))"
        ))
        .is_err()
    );
    // A nullary element (no payload) passes.
    let mut p3 = PurePipeline::new().unwrap();
    assert!(
        p3.execute(&format!(
            "{OPTION_GEN}f = (x: set[Option[int]]) => {{ 1 }}\nf(set(Option.None))"
        ))
        .is_ok()
    );
}

#[test]
fn test_generic_unknown_head_inert() {
    // An undeclared generic head can't be proven a mismatch -> inert (accept).
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("f = (x: Undeclared[int]) => { x }\nf(5)").unwrap();
    assert_eq!(r.as_int(), Some(5));
}

#[test]
fn test_generic_as_struct_field() {
    // A struct field typed with a generic union is enforced (statically via E300
    // for a provable literal payload here).
    let mut p = PurePipeline::new().unwrap();
    assert!(
        p.execute(&format!(
            "{OPTION_GEN}struct Box {{ o: Option[int] }}\nBox(Option.Some(1))"
        ))
        .is_ok()
    );
    let mut p2 = PurePipeline::new().unwrap();
    assert!(
        p2.execute(&format!(
            "{OPTION_GEN}struct Box {{ o: Option[int] }}\nBox(Option.Some(\"s\"))"
        ))
        .is_err()
    );
}

#[test]
fn test_generic_struct_field_runtime_path() {
    // Struct-field generic enforcement at the RUNTIME boundary: the payload value
    // comes from an unannotated function, so static inference yields Top (no E300)
    // and the check fires in the constructor's field_value_ok -> check_generic.
    let mut p = PurePipeline::new().unwrap();
    assert!(
        p.execute(&format!(
            "{OPTION_GEN}struct Box {{ o: Option[int] }}\ng = (y) => {{ y }}\nBox(g(Option.Some(1)))"
        ))
        .is_ok()
    );
    let mut p2 = PurePipeline::new().unwrap();
    assert!(
        p2.execute(&format!(
            "{OPTION_GEN}struct Box {{ o: Option[int] }}\ng = (y) => {{ y }}\nBox(g(Option.Some(\"s\")))"
        ))
        .is_err()
    );
}

#[test]
fn test_generic_as_dict_key() {
    // A generic union as a dict key: the key payload is checked (the second PureVM
    // payload code path, `key_satisfies`, distinct from the value path).
    let mut p = PurePipeline::new().unwrap();
    assert!(
        p.execute(&format!(
            "{OPTION_GEN}f = (x: dict[Option[int], int]) => {{ 1 }}\nf(dict((Option.Some(1), 10)))"
        ))
        .is_ok()
    );
    let mut p2 = PurePipeline::new().unwrap();
    assert!(
        p2.execute(&format!(
            "{OPTION_GEN}f = (x: dict[Option[int], int]) => {{ 1 }}\nf(dict((Option.Some(\"s\"), 10)))"
        ))
        .is_err()
    );
}

#[test]
fn test_generic_arity_mismatch_static() {
    // Option[int, str] is a static arity error (E300 fatal) -> execute fails.
    let mut p = PurePipeline::new().unwrap();
    assert!(
        p.execute(&format!("{OPTION_GEN}f = (x: Option[int, str]) => {{ 1 }}"))
            .is_err()
    );
}

#[test]
fn test_for_range_break_with_dead_code_after() {
    // Peephole compaction removes the dead statements after `break` but must
    // re-encode ForRangeInt's relative exit offset (and ForRangeStep's
    // back-target): unadjusted, the loop exit jumped past the tail of the
    // program (silent None) or into the middle of an expression (stack
    // underflow). Found by the CFG property harness.
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("a = 1\nfor i in range(0) {\n    break\n    a = 0\n}\na")
        .unwrap();
    assert_eq!(r.as_int(), Some(1), "zero-trip range loop with dead code after break");

    let mut p2 = PurePipeline::new().unwrap();
    let r2 = p2
        .execute("a = 1\nfor i in range(3) {\n    continue\n    a = 0\n}\na")
        .unwrap();
    assert_eq!(r2.as_int(), Some(1), "range loop with dead code after continue");

    let mut p3 = PurePipeline::new().unwrap();
    let r3 = p3
        .execute("s = 0\nfor i in range(5) {\n    s = s + i\n    if i == 2 { break }\n    s = s + 10\n}\ns")
        .unwrap();
    assert_eq!(r3.as_int(), Some(23), "conditional break keeps the loop live");
}

#[test]
fn test_nested_if_terminal_arm_does_not_run_sibling_else() {
    // `if c2 { a = 0 } else { break }` nested in the then of an outer if:
    // the compiler used to elide the outer merge jump because the last
    // emitted instruction (the break's Jump) looked terminal, so the inner
    // then's merge jump fell through into the outer else (a = 9 ran after
    // a = 0). Found by the CFG property harness; the AST executor is the spec.
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            "a = 1\nd = 3\nfor it0 in range(1) {\n    if 0 > (0 - (d + 1)) {\n        if 0 < a {\n            a = 0\n        } else { break }\n    } else {\n        a = 9\n    }\n}\na",
        )
        .unwrap();
    assert_eq!(
        r.as_int(),
        Some(0),
        "outer else must not run when the outer cond is true"
    );

    // while variant, break actually taken on iteration 2: post-loop value
    // comes from the then arm of the inner if.
    let mut p2 = PurePipeline::new().unwrap();
    let r2 = p2
        .execute(
            "a = 1\ns = 0\nc = 0\nwhile c < 5 {\n    c = c + 1\n    if 0 < a {\n        if c < 2 {\n            s = s + 10\n        } else { break }\n    } else {\n        s = s + 100\n    }\n}\ns + c",
        )
        .unwrap();
    assert_eq!(r2.as_int(), Some(12), "10 (iter 1) + c stopped at 2 by the break");
}

// --- FT3: function-type boundary (CheckCallable) ---
//
// The static half proves what it can; these reach the runtime boundary by
// passing values through an untyped indirection (an unannotated function's
// return is Top, so E300 never fires and the prologue check decides).

#[test]
fn test_callable_accepts_matching_arity() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("pick = (fs) => { fs[0] }\napply = (cb: (int) -> int) => { cb(41) }\napply(pick([(y) => { y + 1 }]))")
        .unwrap();
    assert_eq!(r.as_int(), Some(42));
}

#[test]
fn test_callable_rejects_arity_mismatch() {
    let mut p = PurePipeline::new().unwrap();
    let r =
        p.execute("pick = (fs) => { fs[0] }\napply = (cb: (int) -> int) => { cb(1) }\napply(pick([(a, b) => { a }]))");
    assert!(r.is_err(), "a 2-required-arg callback must not satisfy (int) -> int");
}

#[test]
fn test_callable_rejects_non_callable() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("pick = (fs) => { fs[0] }\napply = (cb: (int) -> int) => { cb(1) }\napply(pick([5]))");
    assert!(r.is_err(), "a non-callable must be rejected at the boundary");
}

#[test]
fn test_callable_defaults_widen_acceptance() {
    // (x, y=1) is callable with 1 arg: required 1 <= arity 1 <= 2.
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("pick = (fs) => { fs[0] }\napply = (cb: (int) -> int) => { cb(41) }\napply(pick([(x, y = 1) => { x + y }]))")
        .unwrap();
    assert_eq!(r.as_int(), Some(42));
}

#[test]
fn test_callable_vararg_accepts_any_arity() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("pick = (fs) => { fs[0] }\napply = (cb: (int, int) -> int) => { cb(20, 22) }\napply(pick([(*rest) => { rest[0] + rest[1] }]))")
        .unwrap();
    assert_eq!(r.as_int(), Some(42));
}

#[test]
fn test_callable_struct_constructor_accepted() {
    // A struct constructor is callable; its field count is the arity.
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("struct P { x }\napply = (cb: (int) -> P) => { cb(7) }\napply(P).x")
        .unwrap();
    assert_eq!(r.as_int(), Some(7));
}

#[test]
fn test_callable_as_list_element() {
    // Composite element path (value_satisfies): each element checked.
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("pick = (xs) => { xs }\nf = (hs: list[(int) -> int]) => { hs[0](1) }\nf(pick([(y) => { y }]))")
        .unwrap();
    assert_eq!(r.as_int(), Some(1));
    let mut p2 = PurePipeline::new().unwrap();
    let r2 = p2.execute("pick = (xs) => { xs }\nf = (hs: list[(int) -> int]) => { 1 }\nf(pick([5]))");
    assert!(r2.is_err(), "a non-callable list element must be rejected");
}

// --- FT2-A: declared-callback return checked on the caller side ---

#[test]
fn test_callback_return_lie_rejected() {
    let mut p = PurePipeline::new().unwrap();
    let r =
        p.execute("pick = (fs) => { fs[0] }\napply = (cb: (int) -> int) => { cb(1) }\napply(pick([(y) => { \"s\" }]))");
    assert!(r.is_err(), "a callback returning str against a declared int must fail");
}

#[test]
fn test_callback_return_conforming() {
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            "pick = (fs) => { fs[0] }\napply = (cb: (int) -> int) => { cb(20) + 1 }\napply(pick([(y) => { y * 2 }]))",
        )
        .unwrap();
    assert_eq!(r.as_int(), Some(41));
}

#[test]
fn test_callback_return_composite_checked() {
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(
        "pick = (fs) => { fs[0] }\napply = (cb: (int) -> list[int]) => { cb(1) }\napply(pick([(y) => { 7 }]))",
    );
    assert!(r.is_err(), "an int against a declared list[int] return must fail");
}

#[test]
fn test_callback_return_fn_type_checked() {
    // A curried callback: the declared return is itself a function type,
    // enforced by CheckCallable on the result.
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("pick = (fs) => { fs[0] }\napply = (cb: (int) -> (int) -> int) => { cb(1)(2) }\napply(pick([(y) => { (z) => { y + z } }]))")
        .unwrap();
    assert_eq!(r.as_int(), Some(3));
    let mut p2 = PurePipeline::new().unwrap();
    let r2 = p2.execute(
        "pick = (fs) => { fs[0] }\napply = (cb: (int) -> (int) -> int) => { cb(1) }\napply(pick([(y) => { 7 }]))",
    );
    assert!(
        r2.is_err(),
        "a non-callable against a declared function-type return must fail"
    );
}

#[test]
fn test_callback_return_unenforceable_inert() {
    // A return the boundary cannot model stays dynamic: no check emitted.
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("pick = (fs) => { fs[0] }\napply = (cb: (int) -> mystery) => { cb(1) }\napply(pick([(y) => { y }]))")
        .unwrap();
    assert_eq!(r.as_int(), Some(1));
}

// --- FT review 2: union members, nested closures, executor parity ---

#[test]
fn test_callable_union_member_accepted() {
    // A Callable member of a union must not be dropped by check_union.
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute(
            "pick = (fs) => { fs[0] }\napply = (cb: None | (int) -> int) => { cb(1) }\napply(pick([(y) => { y }]))",
        )
        .unwrap();
    assert_eq!(r.as_int(), Some(1));
    let mut p2 = PurePipeline::new().unwrap();
    let r2 = p2
        .execute("pick = (fs) => { fs[0] }\napply = (cb: None | (int) -> int) => { 7 }\napply(pick([None]))")
        .unwrap();
    assert_eq!(r2.as_int(), Some(7));
    let mut p3 = PurePipeline::new().unwrap();
    let r3 = p3.execute("pick = (fs) => { fs[0] }\napply = (cb: None | (int) -> int) => { 7 }\napply(pick([5]))");
    assert!(r3.is_err(), "an int satisfies neither union member");
}

#[test]
fn test_callback_return_checked_in_nested_closure() {
    // The env layers over the enclosing lambda: a captured callback keeps
    // its declared return one closure deeper.
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute(
        "pick = (fs) => { fs[0] }\napply = (cb: (int) -> int) => { w = () => { cb(1) }\n w() }\napply(pick([(y) => { \"s\" }]))",
    );
    assert!(r.is_err(), "the lie must be caught one closure deeper too");
    // An inner param of the same name shadows: no check on the inner call.
    let mut p2 = PurePipeline::new().unwrap();
    let r2 = p2
        .execute("pick = (fs) => { fs[0] }\napply = (cb: (int) -> int) => { w = (cb) => { cb(1) }\n w(pick([(a) => { \"free\" }])) }\napply(pick([(y) => { y }]))")
        .unwrap();
    assert_eq!(unsafe { r2.as_native_str_ref() }, Some("free"));
}

#[test]
fn test_callable_plain_string_rejected() {
    // A plain string is not a builtin name: rejected (parity with PyO3/AST).
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("pick = (fs) => { fs[0] }\napply = (cb: (int) -> int) => { 1 }\napply(pick([\"hello\"]))");
    assert!(r.is_err(), "a plain string must not pass the callable boundary");
    // A real builtin-by-name still passes.
    let mut p2 = PurePipeline::new().unwrap();
    let r2 = p2
        .execute("apply = (cb: (list) -> int) => { cb([1, 2, 3]) }\napply(len)")
        .unwrap();
    assert_eq!(r2.as_int(), Some(3));
}

#[test]
fn test_callable_list_element_arity_checked() {
    // Composite element path: the arity is enforced, not just callability.
    let mut p = PurePipeline::new().unwrap();
    let r = p.execute("pick = (xs) => { xs }\nf = (hs: list[(int) -> int]) => { 1 }\nf(pick([(a, b) => { a }]))");
    assert!(
        r.is_err(),
        "a 2-required-arg element must not satisfy list[(int) -> int]"
    );
}

// ---------------------------------------------------------------------------
// Struct-instance leak oracles (Arc model)
//
// `live_struct_instances()` counts live `StructCell`s on this thread. A
// function-scoped program must return it to its pre-execution baseline: every
// instance that does not escape into a persisting global is released when its
// last `Value` dies (frame teardown, overwrite, container drop cascade). Before
// the Arc rewrite these all leaked (`decref` was a no-op for structs); each of
// these oracles measured `> 0` residual.
// ---------------------------------------------------------------------------

use crate::vm::structs::live_struct_instances;

/// Run a function-scoped snippet and assert every struct it created was freed.
fn assert_no_struct_leak(src: &str) {
    let mut p = PurePipeline::new().unwrap();
    let base = live_struct_instances();
    let r = p.execute(src).unwrap();
    r.decref(); // release the (non-struct) result
    assert_eq!(live_struct_instances(), base, "function-scoped structs leaked: {src}");
}

#[test]
fn oracle_struct_discard_loop_freed() {
    // A local overwritten each iteration must free the previous instance.
    assert_no_struct_leak("struct P { v }\nrun = () => { i = 0\n while i < 50 { p = P(i)\n i = i + 1 }\n 99 }\nrun()");
}

#[test]
fn oracle_struct_in_list_loop_freed() {
    // Container cascade (approach A's wall #1): a list of structs, rebuilt each
    // iteration, releases its elements on drop.
    assert_no_struct_leak(
        "struct P { v }\nrun = () => { i = 0\n while i < 50 { xs = [P(i), P(i + 1)]\n i = i + 1 }\n 99 }\nrun()",
    );
}

#[test]
fn oracle_struct_in_dict_freed() {
    assert_no_struct_leak("struct P { v }\nrun = () => { d = {1: P(1), 2: P(2)}\n 0 }\nrun()");
}

#[test]
fn oracle_nested_struct_cascade_freed() {
    // Outer's Drop cascades a decref into the inner struct field.
    assert_no_struct_leak("struct Inner { v }\nstruct Outer { inner }\nrun = () => { o = Outer(Inner(1))\n 0 }\nrun()");
}

#[test]
fn oracle_struct_typeof_operand_freed() {
    // TypeOf consumes its operand.
    assert_no_struct_leak(
        "struct P { v }\nrun = () => { i = 0\n while i < 40 { t = type(P(i))\n i = i + 1 }\n 0 }\nrun()",
    );
}

#[test]
fn oracle_struct_is_operands_freed() {
    // Is / IsNot consume both operands.
    assert_no_struct_leak(
        "struct P { v }\nrun = () => { i = 0\n while i < 40 { r = P(i) is P(i)\n i = i + 1 }\n 0 }\nrun()",
    );
}

#[test]
fn oracle_struct_unpack_sequence_freed() {
    // `a, b = [P, P]` (UnpackSequence) consumes the source container and cascades.
    assert_no_struct_leak(
        "struct P { v }\nrun = () => { i = 0\n while i < 40 { a, b = [P(i), P(i + 1)]\n i = i + 1 }\n 0 }\nrun()",
    );
}

#[test]
fn oracle_struct_list_pattern_elements_freed() {
    // Match on a LIST subject destructured by a tuple pattern -- elements were
    // increfed by as_slice_cloned() and leaked (the documented pre-existing
    // reliquat); snapshot_items() fixes it.
    assert_no_struct_leak(
        "struct P { v }\nrun = () => { i = 0\n while i < 40 { r = match [P(i), P(i + 1)] { (a, b) => { a.v + b.v } }\n i = i + 1 }\n 0 }\nrun()",
    );
}

#[test]
fn oracle_struct_field_overwrite_freed() {
    // Overwriting a struct-valued field releases the previous instance.
    assert_no_struct_leak("struct Box { v }\nrun = () => { b = Box(Box(1))\n b.v = 7\n 0 }\nrun()");
}

#[test]
fn oracle_struct_getattr_field_freed() {
    assert_no_struct_leak("struct P { v }\nrun = () => { p = P(5)\n x = p.v\n x }\nrun()");
}

#[test]
fn oracle_struct_match_subject_freed() {
    assert_no_struct_leak("struct P { v }\nrun = () => { p = P(5)\n y = match p { P{v} => { v } }\n y }\nrun()");
}

#[test]
fn oracle_struct_method_self_freed() {
    assert_no_struct_leak(
        "struct P { v\n get(self) => { self.v } }\nrun = () => { p = P(5)\n x = p.get()\n x }\nrun()",
    );
}

#[test]
fn oracle_struct_init_local_freed() {
    // A struct with an init, built and discarded inside a function.
    assert_no_struct_leak(
        "struct C { v\n init(self) => { self.v = self.v * 2 } }\nrun = () => { c = C(5)\n 0 }\nrun()",
    );
}

#[test]
fn oracle_returned_struct_then_dropped() {
    // A struct escaping as the result stays alive until the result Value dies,
    // then the count returns to baseline -- proving the counter tracks non-zero.
    let mut p = PurePipeline::new().unwrap();
    let base = live_struct_instances();
    let r = p.execute("struct P { v }\nmk = () => { P(7) }\nmk()").unwrap();
    assert!(r.is_struct_instance(), "expected a struct result");
    assert_eq!(live_struct_instances(), base + 1, "escaped struct must be live");
    r.decref();
    assert_eq!(live_struct_instances(), base, "dropping the result frees it");
}

#[test]
fn oracle_struct_static_method_on_instance_freed() {
    // A `@static` method dispatched through an instance receiver must release the
    // receiver -- it is not bound as self.
    assert_no_struct_leak(
        "struct P {\n v\n @static\n make() => { 0 }\n}\nrun = () => { i = 0\n while i < 40 { p = P(i)\n p.make()\n i = i + 1 }\n 0 }\nrun()",
    );
}

#[test]
fn oracle_struct_unknown_method_error_frees_receiver() {
    // The error path (no such method) must release the struct receiver too.
    let mut p = PurePipeline::new().unwrap();
    let base = live_struct_instances();
    let _ = p.execute("struct P { v }\nmk = () => { p = P(1)\n p.nope() }\nmk()");
    assert_eq!(
        live_struct_instances(),
        base,
        "receiver freed even on method-lookup failure"
    );
}

#[test]
fn oracle_struct_var_pattern_subject_freed() {
    // A whole-subject capture `x => ...` binds a clone (catnip_vm Var clones), so
    // the DupTop subject copy must still be released (no leak, no double-free).
    assert_no_struct_leak("struct P { v }\nrun = () => { p = P(5)\n y = match p { x => { x.v } }\n y }\nrun()");
}

#[test]
fn oracle_locals_dict_does_not_double_free_heap_values() {
    // `locals()`/`globals()` build a dict via from_dict, which OWNS and decrefs
    // its entries on drop. The Locals/Globals opcodes must clone each value (the
    // frame locals, closure captures, and globals map keep their own ref), else a
    // heap value (struct/bigint/list) surfaced through the dict is decref'd twice
    // -- heap corruption. Scalars never triggered it (not refcounted), so no test
    // caught the pre-existing bug until the closure Drop made the capture case
    // lethal. Each snippet returns 0 (a scalar) so only the reflection is measured.
    for src in [
        "struct P { v }\nf = () => { s = P(1)\n locals()\n 0 }\nf()", // heap local
        "struct P { v }\nouter = () => { s = P(1)\n inner = () => { s\n locals()\n 0 }\n inner() }\nouter()", // capture
        "struct P { v }\ng = P(1)\nlocals()\n0",                      // module locals == globals, heap global
        "struct P { v }\ng = P(1)\nglobals()\n0",                     // globals() with a heap global
    ] {
        let mut p = PurePipeline::new().unwrap();
        let base = live_struct_instances();
        p.execute(src).unwrap().decref();
        p.reset();
        assert_eq!(
            live_struct_instances(),
            base,
            "locals()/globals() double-freed a heap value: {src}"
        );
    }
}

#[test]
fn oracle_struct_reset_frees_globals() {
    // The MCP eval path calls pipeline.reset() between calls, dropping the host. A
    // struct bound to a global in one call must be freed at reset, not leaked.
    let mut p = PurePipeline::new().unwrap();
    let base = live_struct_instances();
    p.execute("struct P { v }\np = P(1)\n0").unwrap().decref();
    assert_eq!(live_struct_instances(), base + 1, "global instance live before reset");
    p.reset();
    assert_eq!(live_struct_instances(), base, "reset must free the global instance");
}

#[test]
fn oracle_bound_method_capture_freed_at_scope_exit() {
    // `m = p.get` binds a method without calling it: the bound closure curries
    // self=p (clone_refcount) in an Arc-backed slot. When run() returns, its `m`
    // local is the slot's last ref -> the slot drops -> PureFuncSlot::Drop releases
    // the curried receiver, freeing p at scope exit (no longer pinned until reset).
    let mut p = PurePipeline::new().unwrap();
    let base = live_struct_instances();
    p.execute("struct P { v\n get(self) => { self.v } }\nrun = () => { p = P(5)\n m = p.get\n 0 }\nrun()")
        .unwrap()
        .decref();
    assert_eq!(
        live_struct_instances(),
        base,
        "bound-method capture freed when its binding dies"
    );
    p.reset();
    assert_eq!(
        live_struct_instances(),
        base,
        "reset stays at baseline (no double-free)"
    );
}

#[test]
fn bound_method_call_binds_receiver() {
    // `m = p.get` then `m()` must bind self=p: the receiver is curried as the
    // method's first positional arg (like the direct `p.get()` path), not stuffed
    // in the closure where the param-reading body never sees it. Regression for
    // the catnip_vm divergence (self=None) vs catnip_rs.
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("struct P { v\n get(self) => { self.v } }\np = P(5)\nm = p.get\nm()")
        .unwrap();
    assert_eq!(r.as_int(), Some(5), "bound method must bind its receiver");
    r.decref();
}

#[test]
fn bound_method_repeated_and_extra_args() {
    // The slot keeps its bound_self ref across calls (clone at each call site),
    // and a curried receiver composes with further positional args.
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("struct P { v\n add(self, x) => { self.v + x } }\np = P(10)\nm = p.add\nm(1) + m(2)")
        .unwrap();
    assert_eq!(
        r.as_int(),
        Some(23),
        "repeated bound-method calls with args must compose"
    ); // (10+1)+(10+2)
    r.decref();
}

#[test]
fn bound_method_called_then_freed_at_scope_exit() {
    // After actually invoking the bound method, the receiver still frees at scope
    // exit (bound_self released by PureFuncSlot::Drop when run()'s `m` local dies,
    // not leaked by the calls).
    let mut p = PurePipeline::new().unwrap();
    let base = live_struct_instances();
    p.execute("struct P { v\n get(self) => { self.v } }\nrun = () => { p = P(5)\n m = p.get\n m() }\nrun()")
        .unwrap()
        .decref();
    assert_eq!(
        live_struct_instances(),
        base,
        "bound receiver freed at scope exit after being called"
    );
    p.reset();
    assert_eq!(live_struct_instances(), base, "reset stays at baseline");
}

#[test]
fn oracle_closure_struct_capture_freed_at_scope_exit() {
    // A closure capturing a function-scoped struct: an Arc-backed slot holding a
    // capturing PureClosureScope. When run() returns, its `f` local is the slot's
    // last ref -> the slot drops -> ClosureScopeInner::drop decrefs the capture,
    // freeing p at scope exit.
    let mut p = PurePipeline::new().unwrap();
    let base = live_struct_instances();
    p.execute("struct P { v }\nrun = () => { p = P(5)\n f = () => { p.v }\n f() }\nrun()")
        .unwrap()
        .decref();
    assert_eq!(
        live_struct_instances(),
        base,
        "closure capture freed when its binding dies"
    );
    p.reset();
    assert_eq!(live_struct_instances(), base, "reset stays at baseline");
}

// ---------------------------------------------------------------------------
// Phase 2 -- runtime func_table slot reclamation (`live_func_slots`).
//
// A runtime closure slot (a `MakeFunction` closure, or a `m = p.get` bound
// method) must return to baseline once its last `Value` dies -- WITHOUT waiting
// for reset(). Before the Arc-in-value migration these slots lived in the
// grow-only table and only dropped at reset, so each loop below measured a
// growing residual (RED). The self-recursion / named-fn oracles additionally
// require the weak self-capture: a named function self-captures its own handle,
// which as a strong Arc ref would pin the slot in a cycle.
// ---------------------------------------------------------------------------

use crate::vm::func_table::live_func_slots;

/// Define `run` (and any structs) in a first execute, then invoke `run()` and
/// assert every runtime closure slot the *loop* created was reclaimed. The
/// baseline is captured after the definitions, so the persistent top-level `run`
/// closure (a live global) is counted in the baseline, not mistaken for a leak.
fn assert_run_reclaims_slots(defs: &str) {
    let mut p = PurePipeline::new().unwrap();
    p.execute(defs).unwrap().decref(); // defines run (+ struct templates)
    let base = live_func_slots(); // includes the persistent `run` global
    p.execute("run()").unwrap().decref();
    assert_eq!(live_func_slots(), base, "run() leaked closure slots: {defs}");
}

#[test]
fn oracle_bound_method_loop_reclaims_slots() {
    // `m = p.get` each iteration mints a bound-method slot; the previous one dies
    // when `m` is overwritten. Acyclic -> reclaimed immediately, not at reset.
    assert_run_reclaims_slots(
        "struct P { v\n get(self) => { self.v } }\nrun = () => { i = 0\n while i < 50 { p = P(i)\n m = p.get\n i = i + 1 }\n 0 }",
    );
}

#[test]
fn oracle_closure_loop_reclaims_slots() {
    // A fresh anonymous closure each iteration, overwritten -> previous slot freed.
    assert_run_reclaims_slots("run = () => { i = 0\n while i < 50 { f = () => { i }\n i = i + 1 }\n 0 }");
}

#[test]
fn oracle_named_fn_loop_reclaims_slots() {
    // A named (self-binding) function each iteration: its closure self-captures
    // its own handle. A strong self-capture is an Arc cycle pinning the slot; the
    // weak self-capture breaks it so the slot is reclaimed on overwrite.
    assert_run_reclaims_slots("run = () => { i = 0\n while i < 50 { g = (n) => { n }\n g(i)\n i = i + 1 }\n 0 }");
}

#[test]
fn oracle_self_recursive_closure_reclaimed() {
    // A self-recursive closure minted each iteration. Recursion resolves the self
    // name via the weak self-capture (the caller holds a strong ref during the
    // call), and the slot is reclaimed when `fact` is overwritten -- no leak.
    assert_run_reclaims_slots(
        "run = () => { i = 0\n while i < 30 { fact = (n) => { if n <= 1 { 1 } else { n * fact(n - 1) } }\n fact(5)\n i = i + 1 }\n 0 }",
    );
}

#[test]
fn oracle_func_slot_reset_parity() {
    // Any cyclic residual that survives function scope (nested mutual recursion)
    // must be reclaimed at reset() -- parity with the old whole-table drop.
    let mut p = PurePipeline::new().unwrap();
    let base = live_func_slots();
    p.execute("run = () => { f = (n) => { if n <= 0 { 0 } else { g(n - 1) } }\n g = (n) => { if n <= 0 { 1 } else { f(n - 1) } }\n f(4) }\nrun()")
        .unwrap()
        .decref();
    p.reset();
    assert_eq!(live_func_slots(), base, "func slots leaked past reset");
}

#[test]
fn oracle_mcp_eval_loop_no_cumulative_growth() {
    // The MCP eval path: pipeline.reset() between calls, then a fresh execute. The
    // motivating leak is a program that mints bound-method slots in a loop
    // (`while i<N { m = P(i).get }`), each pinning its receiver. Across many
    // simulated evals, neither the runtime closure slots nor the captured struct
    // instances may accumulate: both must return to baseline after every eval.
    let mut p = PurePipeline::new().unwrap();
    let slot_base = live_func_slots();
    let struct_base = live_struct_instances();
    let src = "struct P { v\n get(self) => { self.v } }\n\
               i = 0\n\
               while i < 40 { p = P(i)\n m = p.get\n i = i + 1 }\n\
               0";
    for _ in 0..5 {
        p.execute(src).unwrap().decref();
        p.reset();
        assert_eq!(
            live_func_slots(),
            slot_base,
            "closure slots accumulated across MCP evals"
        );
        assert_eq!(
            live_struct_instances(),
            struct_base,
            "struct instances accumulated across MCP evals"
        );
    }
}

#[test]
fn oracle_closure_in_list_loop_reclaims() {
    // Container cascade: a list of closures rebuilt each iteration must release its
    // closure elements on drop (NativeList::drop decrefs each -> TAG_CLOSURE Arc).
    assert_run_reclaims_slots(
        "run = () => { i = 0\n while i < 40 { fs = [() => { i }, () => { i + 1 }]\n i = i + 1 }\n 0 }",
    );
}

#[test]
fn oracle_closure_in_dict_reclaims() {
    assert_run_reclaims_slots("run = () => { d = {1: () => { 1 }, 2: () => { 2 }}\n 0 }");
}

#[test]
fn oracle_closure_as_struct_field_reclaims() {
    // A struct field holding a closure: StructCell::drop cascades a decref into it.
    assert_run_reclaims_slots(
        "struct Box { f }\nrun = () => { i = 0\n while i < 40 { b = Box(() => { i })\n i = i + 1 }\n 0 }",
    );
}

#[test]
fn oracle_sibling_closure_group_freed_on_reset() {
    // Sibling function defs in one block (`g = ...`, `h = ...`) are a letrec*
    // group: the compiler cross-patches each into the others' closures
    // (`register_letrec_def` -> PatchClosure) so mutual recursion works even after
    // they escape. Those are *strong* captures -> an Arc cycle, so the group is
    // reclaimed at reset (via the runtime_closures drain), not at scope exit --
    // parity with the pre-Arc grow-only table, which also freed only at reset. The
    // MCP resets between evals, so this never accumulates across evals
    // (oracle_mcp_eval_loop_no_cumulative_growth).
    let mut p = PurePipeline::new().unwrap();
    let base = live_func_slots();
    p.execute("run = () => { g = () => { 1 }\n h = () => { g() }\n h() }\nrun()")
        .unwrap()
        .decref();
    p.reset();
    assert_eq!(
        live_func_slots(),
        base,
        "sibling letrec closure group leaked past reset"
    );
}

#[test]
fn oracle_closure_hof_arg_reclaims() {
    // A closure passed to a HOF (map) each iteration: call_func_sync borrows it;
    // execute_hof owns and releases it. A single def (not a sibling group), so the
    // slot is reclaimed on overwrite.
    assert_run_reclaims_slots(
        "run = () => { i = 0\n while i < 40 { f = (x) => { x + i }\n map(f, [1, 2, 3])\n i = i + 1 }\n 0 }",
    );
}

#[test]
fn oracle_closure_broadcast_operator_reclaims() {
    // A closure used as a broadcast operator (`xs.[f]`) each iteration:
    // call_value_sync dispatches it; the slot is reclaimed on overwrite.
    assert_run_reclaims_slots(
        "run = () => { i = 0\n while i < 40 { f = (x) => { x + i }\n [1, 2, 3].[f]\n i = i + 1 }\n 0 }",
    );
}

#[test]
fn oracle_broadcast_target_no_struct_leak() {
    // The Broadcast opcode must not leak its target (a struct list): the operator
    // decref must not disturb target/operand ownership (no double-free either).
    let mut p = PurePipeline::new().unwrap();
    p.execute(
        "struct P { v }\nrun = () => { i = 0\n while i < 30 { xs = [P(i)]\n xs.[(x) => { x }]\n i = i + 1 }\n 0 }",
    )
    .unwrap()
    .decref();
    let sbase = live_struct_instances();
    p.execute("run()").unwrap().decref();
    assert_eq!(live_struct_instances(), sbase, "broadcast target list leaked structs");
}

/// B1 across every broadcast/ND path: an operator that does NOT return its
/// element must not double-free it (the borrowed element consumed by the
/// move-taking `run_sync` AND decref'd by the item cleanup), nor leak the popped
/// target ref (each opcode owns and releases its popped target/operator/operand).
/// Each path runs 50 iterations reading the element's field back -- a UAF shows
/// as a wrong sum or a crash, a leak as a nonzero live-struct delta after return.
#[test]
fn oracle_broadcast_nd_paths_no_double_free_or_leak_b1() {
    let paths = [
        ("broadcast .[f]", "xs.[(x)=>{99}]"),
        ("~>(xs, f)", "~>(xs, (x)=>{99})"),
        ("~~(p, l)", "~~(p, (s,r)=>{99})"),
        ("xs.[~> f]", "xs.[~> (x)=>{99}]"),
        ("xs.[~~ l]", "xs.[~~ (s,r)=>{99}]"),
    ];
    for (name, op) in paths {
        let mut p = PurePipeline::new().unwrap();
        let src = format!(
            "struct P{{v}}\nrun=()=>{{ i=0\n total=0\n \
             while i < 50 {{ p=P(7)\n xs=[p]\n {op}\n total = total + p.v\n i = i + 1 }}\n total }}\nrun()"
        );
        let base = live_struct_instances();
        let r = p.execute(&src).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(
            r.as_int(),
            Some(350),
            "{name}: element double-freed (UAF reads wrong field)"
        );
        r.decref();
        assert_eq!(
            live_struct_instances(),
            base,
            "{name}: leaked struct(s) (popped target ref not released)"
        );
    }
}

#[test]
fn oracle_nd_map_closure_reclaims() {
    // ND map with a closure func (`~>(data, f)`): nd_map_apply borrows func, so the
    // Broadcast/NdMap opcode must release the popped closure ref.
    assert_run_reclaims_slots(
        "run = () => { i = 0\n while i < 30 { f = (x) => { x + i }\n ~>(list(1, 2, 3), f)\n i = i + 1 }\n 0 }",
    );
}

#[test]
fn oracle_nd_recursion_closure_reclaims() {
    // ND recursion with a closure lambda (`~~(seed, lambda)`): the lambda rides the
    // nd_lambda_stack (strong while there) and the opcode releases the popped ref.
    assert_run_reclaims_slots(
        "run = () => { i = 0\n while i < 20 { f = (seed, recur) => { if seed <= 0 { 0 } else { recur(seed - 1) } }\n ~~(3, f)\n i = i + 1 }\n 0 }",
    );
}

#[test]
fn regression_self_recursive_closure_no_binding_a1() {
    // A1 regression guard: a self-recursive closure called with no surviving
    // binding (its only strong ref is the transient callee on the stack). The
    // frame now owns a strong ref to its callee, so the weak self-ref still
    // upgrades during the body.
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("mk = () => { f = (n) => { if n <= 1 { 1 } else { n * f(n - 1) } }\n f }\nmk()(5)")
        .unwrap();
    assert_eq!(
        r.as_int(),
        Some(120),
        "self-recursive closure lost its self-ref without a binding"
    );
    r.decref();
}

#[test]
fn regression_broadcast_element_not_double_freed_b1() {
    // Operator that does NOT return its element: the element must not be consumed
    // by the operator call AND decref'd by the item cleanup (double-free).
    let mut p = PurePipeline::new().unwrap();
    let r = p
        .execute("struct P { v }\nxs = [P(7)]\nys = xs.[(x) => { 99 }]\nxs[0].v")
        .unwrap();
    assert_eq!(r.as_int(), Some(7), "broadcast double-freed its target element");
    r.decref();
}

#[test]
fn oracle_returned_closure_then_dropped() {
    // An escaping closure stays alive until the result Value dies, then the count
    // returns to baseline -- proving the counter tracks non-zero and no leak.
    let mut p = PurePipeline::new().unwrap();
    p.execute("mk = () => { x = 5\n () => { x } }").unwrap().decref();
    let base = live_func_slots(); // includes the persistent `mk` global
    let r = p.execute("mk()").unwrap();
    assert!(r.is_closure(), "expected a closure result");
    assert_eq!(live_func_slots(), base + 1, "escaped closure must be live");
    r.decref();
    assert_eq!(live_func_slots(), base, "dropping the result frees the closure");
}

#[test]
fn oracle_struct_kitchen_sink_bounded() {
    // Many struct operations in one function-scoped loop -- construction, nesting
    // in containers, field mutation, getattr, method dispatch, match -- must all
    // net to zero live instances. A single missed release path shows as residual.
    assert_no_struct_leak(
        "struct P { a\n b\n sum(self) => { self.a + self.b } }\n\
         run = () => {\n\
           i = 0\n\
           total = 0\n\
           while i < 40 {\n\
             p = P(i, i + 1)\n\
             xs = [P(i, 0), p]\n\
             d = {0: P(i, i)}\n\
             p.a = p.b\n\
             total = total + p.sum()\n\
             total = total + match p { P{a, b} => { a } }\n\
             i = i + 1\n\
           }\n\
           total\n\
         }\n\
         run()",
    );
}

#[test]
fn oracle_module_global_overwrite_frees_old() {
    // The MCP host reuses one long-lived pipeline across requests, so the property
    // that matters is bounded growth: a global keeps exactly one instance alive,
    // and redefining or rebinding the name frees the previous one. (Reclaiming the
    // final globals when the whole pipeline drops is a separate, one-time concern.)
    let mut p = PurePipeline::new().unwrap();
    let base = live_struct_instances();
    p.execute("struct P { v }\np = P(1)\n0").unwrap().decref();
    assert_eq!(live_struct_instances(), base + 1, "one global instance is live");

    p.execute("p = P(2)\n0").unwrap().decref(); // redefine the same global
    assert_eq!(
        live_struct_instances(),
        base + 1,
        "overwrite freed the old, one still live"
    );

    p.execute("p = 0\n0").unwrap().decref(); // rebind to a scalar
    assert_eq!(live_struct_instances(), base, "rebinding to a scalar freed the struct");
}
#[test]
fn bound_method_binds_receiver_at_every_dispatch_site() {
    // A bound method (`m = p.add`) curries its receiver, and EVERY VMFunc-dispatch
    // path must prepend it -- not just Call/TailCall. Regression for the review
    // finding that CallKw, the HOF helper (call_func_sync), and the broadcast
    // helper (call_vmfunc_sync) ignored bound_self, running the method with
    // self=None (self is the method's first param, not a closure capture).
    let m = "struct P { v\n add(self, x) => { self.v + x } }\np = P(10)\nm = p.add\n";
    let cases = [
        (format!("{m}m(5)"), 15i64),   // Call (positional)
        (format!("{m}m(x=5)"), 15i64), // CallKw (keyword)
        (
            format!("{m}reduce(map(m, list(1, 2, 3)), (a, b) => {{ a + b }})"),
            36i64,
        ), // HOF map -> call_func_sync
        (format!("{m}reduce(list(1, 2, 3).[m], (a, b) => {{ a + b }})"), 36i64), // broadcast -> call_vmfunc_sync
        (
            format!("{m}fac = (n) => {{ if n <= 0 {{ m(0) }} else {{ fac(n - 1) }} }}\nfac(3)"),
            10i64,
        ), // TailCall
    ];
    for (src, want) in cases {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(&src).unwrap();
        assert_eq!(r.as_int(), Some(want), "wrong result for: {src}");
        r.decref();
    }
}

#[test]
fn bound_method_through_hof_frees_receiver_on_reset() {
    // The receiver captured by a bound method passed to a HOF must still be freed
    // at reset (bound_self released by PureFuncSlot::Drop), not leaked by the HOF
    // dispatch path.
    let mut p = PurePipeline::new().unwrap();
    let base = live_struct_instances();
    p.execute(
        "struct P { v\n add(self, x) => { self.v + x } }\n\
         run = () => { p = P(10)\n adder = p.add\n reduce(map(adder, list(1, 2)), (a, b) => { a + b }) }\nrun()",
    )
    .unwrap()
    .decref();
    p.reset();
    assert_eq!(
        live_struct_instances(),
        base,
        "bound receiver leaked through the HOF path"
    );
}

#[test]
fn callable_field_is_called_without_self() {
    // `r.h(5)` where `h` is a FIELD holding a closure calls the field's value
    // with no self binding (mirror of struct_getattr and the catnip_rs VM,
    // which both resolve field before method). Was: "'R' has no method 'h'".
    let cases = [
        // plain closure field
        ("h = (y) => { y + 1 }\nstruct R { h }\nr = R(h)\nr.h(5)", 6i64),
        // baked closure (captures its maker's arg)
        (
            "mk = (x) => { (y) => { x + y } }\nstruct R { h }\nr = R(mk(10))\nr.h(5)",
            15i64,
        ),
        // bound method stored in a field: receiver still curried at dispatch
        (
            "struct P { v\n add(self, x) => { self.v + x } }\n\
             struct R { h }\nr = R(P(10).add)\nr.h(5)",
            15i64,
        ),
        // field wins over a same-name method (catnip_rs precedence)
        (
            "h = (y) => { y + 1 }\nstruct R { h\n h2(self) => { 99 } }\nr = R(h)\nr.h(5)",
            6i64,
        ),
        // temporary receiver: obj is the LAST ref of the instance, so the
        // dispatch must keep the field's slot alive while its body runs
        // (frame.callee), or the weak letrec self-ref fails to upgrade
        (
            "mk = () => { rec = (n) => { if n <= 0 { 0 } else { rec(n - 1) } }\n rec }\n\
             struct R { h }\nR(mk()).h(3)",
            0i64,
        ),
        // field holding a struct TYPE: constructs through the fused path too
        // (call_non_vmfunc), same as `f = r.t; f(1)`
        ("struct P { v }\nstruct R { t }\nr = R(P)\nr.t(5).v", 5i64),
    ];
    for (src, want) in cases {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(src).unwrap();
        assert_eq!(r.as_int(), Some(want), "wrong result for: {src}");
        r.decref();
    }
}

#[test]
fn callable_field_non_callable_value_errors() {
    // Calling a non-callable field goes to the host and raises, instead of
    // silently falling back to a method lookup.
    let mut p = PurePipeline::new().unwrap();
    let err = p.execute("struct R { v }\nr = R(42)\nr.v(5)").unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("cannot call") || msg.contains("callable"),
        "unexpected error: {msg}"
    );
}

#[test]
fn callable_field_dispatch_is_refcount_neutral() {
    // Volume oracle: repeated field calls must not leak the receiver (obj is
    // released on the field path) nor the field's slot, including the frame
    // path (closure) and the host-error path (non-callable).
    let mut p = PurePipeline::new().unwrap();
    let base_structs = live_struct_instances();
    p.execute(
        "h = (y) => { y + 1 }\nstruct R { h }\n\
         run = () => { r = R(h)\n acc = 0\n for i in range(50) { acc = acc + r.h(i) }\n acc }\nrun()",
    )
    .unwrap()
    .decref();
    assert_eq!(
        live_struct_instances(),
        base_structs,
        "receiver leaked on the callable-field dispatch path"
    );
}

#[test]
fn oracle_type_field_default_released_at_vm_drop() {
    // A struct type owns one ref per heap field default (evaluated at the
    // definition). The registry Drop must release it: the probe (S(1).b shares
    // the default's Arc) must end sole owner once the pipeline dies.
    let mut p = PurePipeline::new().unwrap();
    let probe = p.execute("struct S { a; b = [1, 2] }\nS(1).b").unwrap();
    // Owners: the type's default + the probe (the temporary instance released
    // its own clone at cascade).
    assert_eq!(probe.native_list_strong_count(), 2);
    drop(p);
    assert_eq!(
        probe.native_list_strong_count(),
        1,
        "type field default leaked at VM drop"
    );
    probe.decref();
}

#[test]
fn oracle_inherited_defaults_balanced_through_diamond() {
    // extends copies the parent's field into the child type: each type takes
    // its OWN ref on the default (a shared bit-copy would make the per-type
    // release at Drop an over-release -- the diamond has 4 types for 1 field).
    let mut p = PurePipeline::new().unwrap();
    let probe = p
        .execute(
            "struct A { b = [9] }\nstruct B extends(A) { }\nstruct C extends(A) { }\n\
             struct D extends(B, C) { }\nD().b",
        )
        .unwrap();
    // Owners: 4 types + the probe.
    assert_eq!(probe.native_list_strong_count(), 5);
    drop(p);
    assert_eq!(
        probe.native_list_strong_count(),
        1,
        "inherited defaults unbalanced at VM drop"
    );
    probe.decref();
}

#[test]
fn oracle_imported_type_default_released_on_both_registries() {
    // transplant_structs clones the child's type into the parent: the clone
    // takes its own ref per default (bit-copied Vec<Value>), so the child VM's
    // drop (post-import) and the parent's drop each release exactly their own.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("defmod.cat");
    std::fs::write(&path, "struct S { a; b = [7] }\nMETA = { 'exports': ['S'] }\n").unwrap();

    let mut p = PurePipeline::new().unwrap();
    let loader = crate::loader::PureImportLoader::new(Some(dir.path().to_path_buf()));
    p.set_import_loader(loader);
    let probe = p.execute("import('defmod', 'S')\nS(1).b").unwrap();
    // Owners: parent-registry type default + the probe. The child VM died at
    // import end; its own default ref was released there.
    assert_eq!(probe.native_list_strong_count(), 2);
    drop(p);
    assert_eq!(probe.native_list_strong_count(), 1, "imported type default leaked");
    probe.decref();
}

#[test]
fn oracle_kwarg_displacing_positional_releases_it() {
    // CallKw binds kwargs by slot AFTER positional binding: a kwarg naming an
    // already-bound slot displaces an owned value, which must be released
    // (set_local overwrites raw).
    let mut p = PurePipeline::new().unwrap();
    let probe = p.execute("xs = [7]\ng = (x) => { 1 }\ng(xs, x=2)\nxs").unwrap();
    // Contract: a root pipeline is reset before it dies (the Globals Rc is
    // shared with child pipelines, so there is deliberately no draining Drop).
    p.reset();
    drop(p);
    assert_eq!(probe.native_list_strong_count(), 1, "displaced positional arg leaked");
    probe.decref();
}

#[test]
fn broadcast_struct_element_copy_semantics() {
    // Option A (2026-07-07, deep 2026-07-11): a broadcast callback receives a
    // private DEEP copy of a struct element -- mutations at ANY depth do not
    // escape to the source; returning the copy carries the mutated state into
    // the result. Mirror of tests/language/test_broadcast_mutation.py (VM/AST
    // grid). Nested cases encode (result, source) as `result * 10 + source`.
    let cases: &[(&str, i64)] = &[
        (
            "struct P { log }\nitems = [P(0), P(0)]\nitems.[(p) => { p.log = 1 }]\nitems[0].log",
            0,
        ),
        (
            "struct P { log }\nitems = [P(0), P(0)]\nr = items.[if (p) => { p.log = 1\nTrue }]\nitems[0].log",
            0,
        ),
        (
            "struct P { log }\nitems = [P(0)]\nr = items.[(p) => { p.log = 5\np }]\nr[0].log",
            5,
        ),
        (
            "struct P { log }\nitems = [P(0)]\nr = items.[(p) => { p }]\nitems[0].log = 9\nr[0].log",
            0,
        ),
        (
            "struct P { log }\nitems = [P(3)]\nitems2 = items.[(p) => { p.log = p.log + 1\np }]\nitems[0].log * 10 + items2[0].log",
            34,
        ),
        (
            "struct P { log }\nitems = [P(0), P(0)]\nr = ~>(items, (p) => { p.log = 1\n0 })\nitems[0].log",
            0,
        ),
        // Nested: mutating a nested field is isolated from the source (deep) and
        // transported to the result -> (result 5, source 1) = 51.
        (
            "struct Q { x }\nstruct P { q }\nitems = [P(Q(1))]\nr = items.[(p) => { p.q.x = 5\np }]\nr[0].q.x * 10 + items[0].q.x",
            51,
        ),
        // Nested: assigning a fresh nested struct is likewise isolated + transported.
        (
            "struct Q { x }\nstruct P { q }\nitems = [P(Q(1))]\nr = items.[(p) => { p.q = Q(5)\np }]\nr[0].q.x * 10 + items[0].q.x",
            51,
        ),
        // Nested DAG identity: a nested struct shared by two fields is copied
        // ONCE, so mutating via one field is visible through the other IN THE
        // COPY (b sees 9) while the source stays intact (a still 1) = 91.
        (
            "struct Q { x }\nstruct P { a; b }\nq = Q(1)\nitems = [P(q, q)]\nr = items.[(p) => { p.a.x = 9\np }]\nr[0].b.x * 10 + items[0].a.x",
            91,
        ),
    ];
    for (code, expected) in cases {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(code).unwrap();
        assert_eq!(r.as_int(), Some(*expected), "case: {code}");
    }
}

#[test]
fn struct_constructor_kwargs_semantics() {
    // Kwargs constructor in the pure VM, mirroring the PyO3 runtime: positional
    // then keyword by name, defaults fill the rest; unknown, doubled, and
    // missing names error. Closes the functional divergence where S(a=1)
    // raised 'CallKw: cannot call non-function'.
    let ok: &[(&str, i64)] = &[
        ("struct S { a; b = 7 }\nS(a=1).a", 1),
        ("struct S { a; b = 7 }\nS(a=1).b", 7),
        ("struct S { a; b = 7 }\nS(1, b=2).b", 2),
        ("struct S { a; b = 7 }\nS(b=2, a=3).a", 3),
    ];
    for (code, expected) in ok {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(code).unwrap();
        assert_eq!(r.as_int(), Some(*expected), "case: {code}");
    }
    let err: &[(&str, &str)] = &[
        ("struct S { a }\nS(z=1)", "unexpected keyword argument"),
        ("struct S { a }\nS(1, a=2)", "multiple values"),
        ("struct S { a; b }\nS(a=1)", "missing argument"),
    ];
    for (code, needle) in err {
        let mut p = PurePipeline::new().unwrap();
        let e = p.execute(code).unwrap_err().to_string();
        assert!(e.contains(needle), "case: {code} -> {e}");
    }
}

#[test]
fn oracle_struct_constructor_kwargs_balance() {
    // Ownership: kwarg values are consumed by the instance on success and left
    // to the caller on error -- both ends balance to zero live instances and a
    // sole probe owner.
    let mut p = PurePipeline::new().unwrap();
    let probe = p
        .execute("struct S { a; b = 0 }\nxs = [7]\ns = S(a=xs)\ns = None\nxs")
        .unwrap();
    p.reset();
    drop(p);
    assert_eq!(
        probe.native_list_strong_count(),
        1,
        "kwarg value leaked through constructor"
    );
    probe.decref();

    let mut p3 = PurePipeline::new().unwrap();
    let probe3 = p3.execute("struct S { a }\nxs = [7]\nxs").unwrap();
    // error-path balance: run the failing construction in the same session
    let e = p3.execute("s = S(z=xs)").unwrap_err();
    assert!(e.to_string().contains("unexpected"));
    p3.reset();
    drop(p3);
    assert_eq!(
        probe3.native_list_strong_count(),
        1,
        "kwarg value leaked on constructor error"
    );
    probe3.decref();
}

#[test]
fn enum_pattern_out_of_scope_raises_name_error() {
    // Mirror of the PyO3 runtime (decision 2026-07-06): a pattern naming an
    // enum type never brought into scope raises NameError instead of silently
    // matching by interned symbol -- the MCP now rejects what the Python
    // runtime rejects. In-module scope (the closure chain reaching the
    // module's globals) still matches.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("colmod.cat"),
        "enum Color { red; green }\npick = (c) => { match c { Color.red => { 1 } _ => { 2 } } }\nMETA = { 'exports': ['Color', 'pick'] }\n",
    )
    .unwrap();
    let mut p = PurePipeline::new().unwrap();
    let loader = crate::loader::PureImportLoader::new(Some(dir.path().to_path_buf()));
    p.set_import_loader(loader);
    // Color n'est PAS importé dans le scope courant : le pattern Color.red doit lever NameError (parité catnip_rs).
    let err = p
        .execute("import('colmod')\nx = colmod.Color.red\nmatch x { Color.red => { 1 } _ => { 2 } }")
        .unwrap_err();
    assert!(
        err.to_string().contains("Color"),
        "expected NameError for Color, got: {err}"
    );

    // In scope through the module function's closure chain: matches.
    let mut p2 = PurePipeline::new().unwrap();
    let loader2 = crate::loader::PureImportLoader::new(Some(dir.path().to_path_buf()));
    p2.set_import_loader(loader2);
    let r2 = p2.execute("import('colmod')\ncolmod.pick(colmod.Color.red)").unwrap();
    assert_eq!(r2.as_int(), Some(1));

    // In scope through a selective import (top-level alias): matches.
    let mut p3 = PurePipeline::new().unwrap();
    let loader3 = crate::loader::PureImportLoader::new(Some(dir.path().to_path_buf()));
    p3.set_import_loader(loader3);
    let r3 = p3
        .execute("import('colmod', 'Color')\nmatch Color.green { Color.red => { 1 } Color.green => { 3 } _ => { 2 } }")
        .unwrap();
    assert_eq!(r3.as_int(), Some(3));
}
