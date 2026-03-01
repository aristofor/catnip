// FILE: catnip_rs/tests/standalone_basic.rs
//! Integration tests for catnip-standalone binary - Basic operations.

mod common;
use common::{assert_error, assert_output};

#[test]
fn test_literals() {
    assert_output("42", "42");
    assert_output("3.14", "3.14");
    assert_output("True", "True");
    assert_output("False", "False");
    // None doesn't print anything (same as Python)
    assert_output(r#""hello""#, "hello");
}

#[test]
fn test_arithmetic() {
    assert_output("2 + 3", "5");
    assert_output("10 - 7", "3");
    assert_output("4 * 5", "20");
    assert_output("15 / 3", "5.0");
    assert_output("17 % 5", "2");
    assert_output("2 ** 8", "256");
}

#[test]
fn test_arithmetic_precedence() {
    assert_output("2 + 3 * 4", "14");
    assert_output("(2 + 3) * 4", "20");
    assert_output("10 - 2 * 3", "4");
    assert_output("2 ** 3 ** 2", "512"); // Right associative
}

#[test]
fn test_comparison() {
    assert_output("5 > 3", "True");
    assert_output("5 < 3", "False");
    assert_output("5 >= 5", "True");
    assert_output("5 <= 4", "False");
    assert_output("5 == 5", "True");
    assert_output("5 != 3", "True");
}

#[test]
fn test_logical_operators() {
    assert_output("True and True", "True");
    assert_output("True and False", "False");
    assert_output("True or False", "True");
    assert_output("False or False", "False");
    assert_output("not True", "False");
    assert_output("not False", "True");
}

#[test]
fn test_bitwise_operators() {
    assert_output("5 & 3", "1"); // 101 & 011 = 001
    assert_output("5 | 3", "7"); // 101 | 011 = 111
    assert_output("5 ^ 3", "6"); // 101 ^ 011 = 110
    assert_output("~5", "-6"); // ~101 = -110
    assert_output("8 << 2", "32"); // 1000 << 2 = 100000
    assert_output("8 >> 2", "2"); // 1000 >> 2 = 10
}

#[test]
fn test_variables() {
    assert_output("x = 10; x", "10");
    assert_output("x = 5; y = 3; x + y", "8");
    assert_output("a = 10; a = a + 5; a", "15");
}

#[test]
fn test_lists() {
    assert_output("list(1, 2, 3)", "[1, 2, 3]");
    assert_output("len(list(1, 2, 3))", "3");
    assert_output("list(1, 2, 3)[0]", "1");
    assert_output("list(1, 2, 3)[1]", "2");
    assert_output("list(1, 2, 3)[-1]", "3");
}

#[test]
fn test_tuples() {
    assert_output("tuple(1, 2, 3)", "(1, 2, 3)");
    assert_output("tuple(1, 2)[0]", "1");
    assert_output("tuple(5)", "(5,)");
}

#[test]
fn test_dicts() {
    // Dict creation (Python interop)
    assert_output("d = dict(); d", "{}");
    assert_output("d = dict(); len(d)", "0");
}

#[test]
fn test_blocks() {
    assert_output("{ 42 }", "42");
    assert_output("{ 1; 2; 3 }", "3");
    assert_output("{ a = 10; b = 20; a + b }", "30");
}

#[test]
fn test_comments() {
    assert_output("42 # this is a comment", "42");
    assert_output("# comment\n42", "42");
}

#[test]
fn test_syntax_error() {
    assert_error("2 +");
    assert_error("(");
    assert_error("x =");
}

#[test]
fn test_undefined_variable() {
    assert_error("undefined_var");
}
