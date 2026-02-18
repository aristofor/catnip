// FILE: catnip_rs/tests/regression_vmfunction.rs
//! Tests de régression pour bugs critiques VMFunction/Lambda
//!
//! Chaque test documente un bug historique et vérifie qu'il ne revient pas.

mod common;
use common::assert_output;

/// Régression: VMFunction retournait la string 'xx' au lieu du résultat int
///
/// Bug: Le transformer générait `Identifier("x")` pour les paramètres de lambda
/// dans le corps de la fonction. Lors de la conversion IRPure → Op Python,
/// `Identifier` était converti en PyString au lieu de Ref, ce qui générait
/// LoadConst au lieu de LoadLocal dans le bytecode.
///
/// Fix: Traitement spécial dans convert.rs pour OpLambda - les paramètres
/// restent des strings, mais le corps convertit Identifier → Ref.
///
/// Ce test DOIT échouer si le bug revient.
#[test]
fn test_lambda_param_not_string_literal() {
    // Lambda simple avec paramètre utilisé dans le corps
    assert_output("double = (x) => { x * 2 }; double(5)", "10");

    // Lambda avec multiple paramètres
    assert_output("add = (a, b) => { a + b }; add(3, 7)", "10");

    // Lambda avec paramètre utilisé plusieurs fois
    assert_output("square = (n) => { n * n }; square(4)", "16");

    // Closure capturant une variable ET utilisant un paramètre
    assert_output("x = 5; add_x = (y) => { x + y }; add_x(3)", "8");
}

/// Régression: Vérifier que les closures capturent correctement
#[test]
fn test_closure_captures_outer_variable() {
    // Closure simple
    assert_output("x = 10; f = () => { x }; f()", "10");

    // Closure avec paramètre et capture
    assert_output(
        "multiplier = 3; times = (n) => { n * multiplier }; times(4)",
        "12",
    );

    // Closure nested
    assert_output(
        "outer = 5; f = () => { inner = 3; g = () => { outer + inner }; g() }; f()",
        "8",
    );
}

/// Régression: Lambdas imbriquées
#[test]
fn test_nested_lambdas() {
    // Lambda retournant une lambda
    assert_output(
        "make_adder = (x) => { (y) => { x + y } }; add5 = make_adder(5); add5(3)",
        "8",
    );

    // Lambda avec lambda comme paramètre (higher-order)
    assert_output(
        "apply = (f, x) => { f(x) }; double = (n) => { n * 2 }; apply(double, 5)",
        "10",
    );
}

/// Régression: Lambdas avec valeurs par défaut
#[test]
fn test_lambda_default_params() {
    assert_output("f = (x = 10) => { x * 2 }; f()", "20");
    assert_output("f = (x = 10) => { x * 2 }; f(5)", "10");
}

/// Régression: Lambdas sans paramètres
#[test]
fn test_lambda_no_params() {
    assert_output("get_42 = () => { 42 }; get_42()", "42");
}

/// Régression: Récursion avec lambdas (TCO)
#[test]
fn test_lambda_recursion() {
    // Factorial avec lambda assignée
    assert_output(
        "fact = (n) => { if n <= 1 { 1 } else { n * fact(n - 1) } }; fact(5)",
        "120",
    );
}
