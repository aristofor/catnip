// FILE: catnip_rs/tests/run_broadcast.rs
//! Integration tests for catnip binary - Broadcasting.

mod common;
use common::assert_output;

#[test]
fn test_broadcast_map_arithmetic() {
    assert_output("list(1, 2, 3).[* 2]", "[2, 4, 6]");
    assert_output("list(10, 20, 30).[+ 5]", "[15, 25, 35]");
    assert_output("list(5, 10, 15).[- 3]", "[2, 7, 12]");
    assert_output("list(2, 4, 6).[/ 2]", "[1.0, 2.0, 3.0]");
}

#[test]
fn test_broadcast_map_comparison() {
    assert_output("list(1, 5, 3, 8, 2).[> 4]", "[False, True, False, True, False]");
    assert_output("list(10, 20, 30).[== 20]", "[False, True, False]");
}

#[test]
fn test_broadcast_filter() {
    assert_output("list(1, 5, 3, 8, 2).[if > 4]", "[5, 8]");
    assert_output("list(10, 15, 20, 25).[if <= 20]", "[10, 15, 20]");
}

#[test]
fn test_broadcast_mask() {
    assert_output("list(10, 20, 30).[list(True, False, True)]", "[10, 30]");
}

#[test]
fn test_broadcast_nested() {
    assert_output("list(list(1, 2), list(3, 4)).[.[* 2]]", "[[2, 4], [6, 8]]");
}

#[test]
fn test_broadcast_function() {
    let code = r#"
double = (x) => { x * 2 };
list(1, 2, 3).[~> double]
"#;
    assert_output(code, "[2, 4, 6]");
}

#[test]
fn test_broadcast_chain() {
    assert_output("list(1, 2, 3, 4, 5).[* 2].[+ 1]", "[3, 5, 7, 9, 11]");
}

#[test]
fn test_broadcast_with_builtin() {
    assert_output("list(-1, 2, -3, 4).[~> abs]", "[1, 2, 3, 4]");
}

#[test]
fn test_broadcast_deep_nested() {
    assert_output("list(list(1, 2), list(3, 4)).[* 2]", "[[2, 4], [6, 8]]");
}

#[test]
fn test_broadcast_deep_mixed() {
    assert_output("list(1, list(2, 3)).[+ 10]", "[11, [12, 13]]");
}

#[test]
fn test_broadcast_deep_three_levels() {
    assert_output("list(list(list(1))).[* 3]", "[[[3]]]");
}
