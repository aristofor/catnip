// FILE: catnip_rs/tests/regression_chained_calls.rs
/// Tests de régression pour les appels chaînés.
///
/// Bug historique: f(a)(b) causait ambiguïté avec unpacking tuple assignments
/// Cause: Tree-sitter traitait newlines comme whitespace, ne pouvait pas distinguer:
///   - `x = 1; (a, b) = tuple(2, 3)` (deux statements)
///   - `x = 1(a, b)` (appel chaîné)
/// Solution: External scanner pour newlines significatifs (scanner.c)
/// Status: ✅ FIXÉ - Appels chaînés et unpacking fonctionnent simultanément
///
/// Date: 2026-01-31
mod common;
use common::{assert_output, assert_output_contains};

#[test]
fn test_chained_calls_with_intermediate_assignment() {
    // Appels chaînés avec assignation intermédiaire (workaround)
    let code = "make_adder = (x) => { (y) => { x + y } }; g = make_adder(5); g(3)";
    assert_output(code, "8");
}

#[test]
fn test_chained_calls_with_assignation() {
    // Appels chaînés directs et avec assignation intermédiaire
    let code_chained = "make_adder = (x) => { (y) => { x + y } }; make_adder(10)(7)";
    let code_assigned = "make_adder = (x) => { (y) => { x + y } }; g = make_adder(10); g(7)";

    assert_output(code_chained, "17"); // Appels chaînés directs
    assert_output(code_assigned, "17"); // Avec assignation intermédiaire
}

#[test]
fn test_chained_calls_different_values() {
    // Vérifier que les closures capturent correctement les bonnes valeurs
    let code = r#"
        make_adder = (x) => { (y) => { x + y } };
        result1 = make_adder(5)(3);
        result2 = make_adder(10)(7);
        result3 = make_adder(100)(200);
        print(result1);
        print(result2);
        print(result3)
    "#;

    // On vérifie que chaque appel capture la bonne valeur de x
    assert_output_contains(code, "8");
    assert_output_contains(code, "17");
    assert_output_contains(code, "300");
}

#[test]
fn test_chained_calls_arithmetic() {
    // Appels chaînés avec expressions arithmétiques
    assert_output("f = (a) => { (b) => { a * b } }; f(5)(3)", "15");
    assert_output("f = (a) => { (b) => { a - b } }; f(10)(3)", "7");
    assert_output("f = (a) => { (b) => { a // b } }; f(20)(4)", "5");
}

#[test]
fn test_chained_calls_multiple_params() {
    // Appels chaînés avec plusieurs paramètres
    assert_output("f = (a, b) => { (c) => { a + b + c } }; f(1, 2)(3)", "6");
}

#[test]
fn test_chained_calls_nested_in_expression() {
    // Appels chaînés comme partie d'une expression plus large
    assert_output("f = (a) => { (b) => { a + b } }; result = f(5)(3) * 2; result", "16");
}

#[test]
fn test_chained_calls_as_function_arg() {
    // Appels chaînés passés comme argument
    assert_output(
        "f = (a) => { (b) => { a + b } }; g = (x) => { x * 2 }; g(f(5)(3))",
        "16",
    );
}

#[test]
fn test_chained_calls_3_levels() {
    // 3 niveaux d'appels chaînés: f(a)(b)(c)
    assert_output("f = (a) => { (b) => { (c) => { a + b + c } } }; f(1)(2)(3)", "6");
}

#[test]
fn test_chained_calls_returns_value() {
    // Appels chaînés qui retournent une valeur utilisable
    assert_output("f = (a) => { (b) => { a + b } }; x = f(10)(5); x * 2", "30");
}
