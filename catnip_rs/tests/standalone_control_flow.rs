// FILE: catnip_rs/tests/standalone_control_flow.rs
//! Integration tests for catnip-standalone binary - Control flow.

mod common;
use common::assert_output;

#[test]
fn test_if_true() {
    assert_output("if True { 42 }", "42");
    assert_output("if 5 > 3 { 100 }", "100");
}

#[test]
fn test_if_false() {
    // if False returns None but doesn't print (empty output)
    assert_output("x = if False { 42 }; x", "");
    assert_output("x = if 5 < 3 { 100 }; x", "");
}

#[test]
fn test_if_else() {
    assert_output("if True { 10 } else { 20 }", "10");
    assert_output("if False { 10 } else { 20 }", "20");
}

#[test]
fn test_if_elif_else() {
    assert_output("if False { 1 } elif True { 2 } else { 3 }", "2");
    assert_output("if False { 1 } elif False { 2 } else { 3 }", "3");
}

#[test]
fn test_while_loop() {
    assert_output("i = 0; while i < 5 { i = i + 1 }; i", "5");
    assert_output(
        "sum = 0; i = 1; while i <= 10 { sum = sum + i; i = i + 1 }; sum",
        "55",
    );
}

#[test]
fn test_for_loop() {
    assert_output(
        "sum = 0; for x in list(1, 2, 3) { sum = sum + x }; sum",
        "6",
    );
    assert_output("result = 0; for i in range(5) { result = i }; result", "4");
}

#[test]
fn test_break() {
    assert_output(
        "i = 0; while i < 10 { if i == 5 { break }; i = i + 1 }; i",
        "5",
    );
}

#[test]
fn test_continue() {
    assert_output(
        "sum = 0; i = 0; while i < 5 { i = i + 1; if i == 3 { continue }; sum = sum + i }; sum",
        "12", // 1 + 2 + 4 + 5 = 12 (skips 3)
    );
}

#[test]
fn test_nested_loops() {
    assert_output(
        "sum = 0; for i in range(3) { for j in range(3) { sum = sum + 1 } }; sum",
        "9",
    );
}

#[test]
fn test_match_literal() {
    let code = r#"
x = 2;
match x {
    1 => { "one" }
    2 => { "two" }
    3 => { "three" }
}
"#;
    assert_output(code, "two");
}

#[test]
fn test_match_variable() {
    let code = r#"
x = 42;
match x {
    n => { n * 2 }
}
"#;
    assert_output(code, "84");
}

#[test]
fn test_match_wildcard() {
    let code = r#"
x = 100;
match x {
    1 => { "one" }
    2 => { "two" }
    _ => { "other" }
}
"#;
    assert_output(code, "other");
}

#[test]
fn test_match_or_pattern() {
    let code = r#"
x = 2;
match x {
    1 | 2 | 3 => { "small" }
    _ => { "large" }
}
"#;
    assert_output(code, "small");
}

#[test]
fn test_match_guard() {
    let code = r#"
x = 15;
match x {
    n if n > 10 => { "big" }
    n if n > 5 => { "medium" }
    _ => { "small" }
}
"#;
    assert_output(code, "big");
}
