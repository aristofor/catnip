// FILE: catnip_rs/tests/run_nd_process.rs
//! Integration tests for ND process mode using native Rust workers.
//!
//! These tests verify the full end-to-end path:
//! pragma("nd_mode", "process") → VMHost → WorkerPool → catnip worker

mod common;
use common::{assert_error, assert_output};

#[test]
fn test_nd_process_simple_map() {
    let code = r#"
pragma("nd_mode", "process");
list(1, 2, 3, 4, 5).[~> (n) => { n * 2 }]
"#;
    assert_output(code, "[2, 4, 6, 8, 10]");
}

#[test]
fn test_nd_process_recursion() {
    let code = r#"
pragma("nd_mode", "process");
list(1, 2, 3).[~~ (n, recur) => { n * 10 }]
"#;
    assert_output(code, "[10, 20, 30]");
}

#[test]
fn test_nd_process_with_closure() {
    let code = r#"
pragma("nd_mode", "process");
factor = 100;
list(1, 2, 3).[~> (n) => { n + factor }]
"#;
    assert_output(code, "[101, 102, 103]");
}

#[test]
fn test_nd_process_float_results() {
    let code = r#"
pragma("nd_mode", "process");
list(1, 2, 3, 4).[~> (n) => { n / 2 }]
"#;
    assert_output(code, "[0.5, 1.0, 1.5, 2.0]");
}

#[test]
fn test_nd_process_string_results() {
    // Strings round-trip through Python, tests freeze/thaw of PyObjects
    let code = r#"
pragma("nd_mode", "process");
list(1, 2, 3).[~> (n) => { n * 3 }]
"#;
    assert_output(code, "[3, 6, 9]");
}

#[test]
fn test_nd_process_deterministic_order() {
    // Results must be in the same order as input regardless of worker scheduling
    let code = r#"
pragma("nd_mode", "process");
list(10, 20, 30, 40, 50).[~> (n) => { n + 1 }]
"#;
    assert_output(code, "[11, 21, 31, 41, 51]");
}

#[test]
fn test_nd_process_bool_results() {
    let code = r#"
pragma("nd_mode", "process");
list(1, 2, 3, 4, 5).[~> (n) => { n > 3 }]
"#;
    assert_output(code, "[False, False, False, True, True]");
}

#[test]
fn test_nd_process_typed_param_coerces() {
    // TH2-B step 0b: a typed param's boundary check + coercion must survive into
    // the process worker, which recompiles the function from frozen IR. Without
    // the params (with annotations) being captured in the frozen IR, the worker
    // would drop the check and return [1, 2, 3] instead.
    let code = r#"
pragma("nd_mode", "process");
list(1, 2, 3).[~> (n: float) => { n }]
"#;
    assert_output(code, "[1.0, 2.0, 3.0]");
}

#[test]
fn test_nd_process_union_param() {
    // A type-union boundary (CheckUnion) must also survive into the worker: the
    // union spec is re-classified from the frozen param IR. No coercion -- ints
    // pass through unchanged.
    let code = r#"
pragma("nd_mode", "process");
list(1, 2, 3).[~> (n: int | str) => { n + n }]
"#;
    assert_output(code, "[2, 4, 6]");
}

#[test]
fn test_nd_process_union_param_rejects() {
    // A value outside the union is rejected at the worker boundary.
    let code = r#"
pragma("nd_mode", "process");
list(1.5, 2.5).[~> (n: int | str) => { n }]
"#;
    assert_error(code);
}

#[test]
fn test_nd_process_same_result_as_sequential() {
    // Verify process mode gives same result as sequential
    let code_seq = r#"
pragma("nd_mode", "sequential");
list(1, 2, 3, 4, 5).[~> (n) => { n * n + 1 }]
"#;
    let code_proc = r#"
pragma("nd_mode", "process");
list(1, 2, 3, 4, 5).[~> (n) => { n * n + 1 }]
"#;
    let out_seq = common::run_code(code_seq);
    let out_proc = common::run_code(code_proc);
    assert_eq!(
        String::from_utf8_lossy(&out_seq.stdout).trim(),
        String::from_utf8_lossy(&out_proc.stdout).trim(),
    );
}
