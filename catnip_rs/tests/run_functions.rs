// FILE: catnip_rs/tests/run_functions.rs
//! Integration tests for catnip binary - Functions and lambdas.

mod common;
use common::{assert_error, assert_output};

#[test]
fn test_lambda_simple() {
    // Lambda assignment and call
    assert_output("f = (x) => { x * 2 }; f(5)", "10");
    // Immediate lambda call
    let code = "add_one = (x) => { x + 1 }; add_one(10)";
    assert_output(code, "11");
}

#[test]
fn test_lambda_no_params() {
    assert_output("f = () => { 42 }; f()", "42");
}

#[test]
fn test_lambda_multiple_params() {
    assert_output("f = (a, b) => { a + b }; f(3, 7)", "10");
    assert_output("mul = (x, y) => { x * y }; mul(4, 5)", "20");
}

#[test]
fn test_lambda_default_params() {
    assert_output("f = (x, y=10) => { x + y }; f(5)", "15");
    assert_output("f = (x, y=10) => { x + y }; f(5, 3)", "8");
}

#[test]
fn test_named_function() {
    let code = r#"
double = (x) => { x * 2 };
result = double(21);
result
"#;
    assert_output(code, "42");
}

#[test]
fn test_closure_captures_variable() {
    let code = r#"
x = 10;
f = (y) => { x + y };
f(5)
"#;
    assert_output(code, "15");
}

#[test]
fn test_nested_functions() {
    let code = r#"
outer = (x) => {
    inner = (y) => { x + y };
    inner(5)
};
outer(10)
"#;
    assert_output(code, "15");
}

#[test]
fn test_recursion_factorial() {
    let code = r#"
factorial = (n) => {
    if n <= 1 { 1 } else { n * factorial(n - 1) }
};
factorial(5)
"#;
    assert_output(code, "120");
}

#[test]
fn test_recursion_fibonacci() {
    let code = r#"
fib = (n) => {
    if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
};
fib(10)
"#;
    assert_output(code, "55");
}

#[test]
fn test_tco_factorial() {
    let code = r#"
pragma("tco", True);
factorial = (n, acc=1) => {
    if n <= 1 { acc } else { factorial(n - 1, acc * n) }
};
factorial(10)
"#;
    assert_output(code, "3628800");
}

#[test]
fn test_tco_sum() {
    let code = r#"
pragma("tco", True);
sum_range = (n, acc=0) => {
    if n <= 0 { acc } else { sum_range(n - 1, acc + n) }
};
sum_range(100)
"#;
    assert_output(code, "5050");
}

#[test]
fn test_higher_order_function() {
    let code = r#"
apply = (f, x) => { f(x) };
double = (n) => { n * 2 };
apply(double, 21)
"#;
    assert_output(code, "42");
}

#[test]
fn test_function_returns_function() {
    let code = r#"
make_adder = (x) => {
    (y) => { x + y }
};
add5 = make_adder(5);
add5(10)
"#;
    assert_output(code, "15");
}

#[test]
fn test_multiple_returns() {
    let code = r#"
sign = (x) => {
    if x > 0 { return 1 };
    if x < 0 { return -1 };
    0
};
list(sign(10), sign(-5), sign(0))
"#;
    assert_output(code, "[1, -1, 0]");
}

// --- fold / reduce ---

#[test]
fn test_fold_sum() {
    assert_output("fold(list(1,2,3), 0, (acc,x)=>{acc+x})", "6");
}

#[test]
fn test_fold_empty() {
    assert_output("fold(list(), 0, (acc,x)=>{acc+x})", "0");
}

#[test]
fn test_fold_string_concat() {
    assert_output(r#"fold(list("a","b","c"), "", (acc,x)=>{acc+x})"#, "abc");
}

#[test]
fn test_fold_product() {
    assert_output("fold(list(1,2,3,4), 1, (acc,x)=>{acc*x})", "24");
}

#[test]
fn test_fold_with_broadcast() {
    assert_output("fold(list(1,2,3).[* 10], 0, (acc,x)=>{acc+x})", "60");
}

#[test]
fn test_reduce_sum() {
    assert_output("reduce(list(1,2,3), (acc,x)=>{acc+x})", "6");
}

#[test]
fn test_reduce_single() {
    assert_output("reduce(list(42), (acc,x)=>{acc+x})", "42");
}

#[test]
fn test_reduce_empty_error() {
    assert_error("reduce(list(), (acc,x)=>{acc+x})");
}
