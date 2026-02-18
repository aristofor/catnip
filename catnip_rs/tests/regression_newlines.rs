// FILE: catnip_rs/tests/regression_newlines.rs
/// Tests de régression pour les newlines significatifs.
///
/// Bug historique: Newlines causaient ambiguïté entre statements et arguments multilignes
/// Problème: Tree-sitter devait distinguer entre:
///   - Newlines comme séparateurs de statements (significatives)
///   - Newlines comme whitespace dans arguments/listes (non significatives)
/// Solution: External scanner + newlines dans extras
/// Status: ✅ FIXÉ
///
/// Date: 2026-01-31
mod common;
use common::assert_output;

#[test]
fn test_newline_separates_statements() {
    // Newlines séparent les statements (pas besoin de semicolon)
    let code = "x = 1
y = 2
x + y";
    assert_output(code, "3");
}

#[test]
fn test_semicolon_separates_statements() {
    // Semicolons séparent aussi les statements
    let code = "x = 1; y = 2; x + y";
    assert_output(code, "3");
}

#[test]
fn test_mixed_separators() {
    // Mix newlines et semicolons
    let code = "x = 1; y = 2
z = 3
x + y + z";
    assert_output(code, "6");
}

#[test]
fn test_semicolon_then_newline() {
    // Semicolon suivi de newline (deux séparateurs consécutifs)
    let code = "x = { 42 };
y = 1;
x + y";
    assert_output(code, "43");
}

#[test]
fn test_multiple_newlines() {
    // Plusieurs newlines consécutives (OK)
    let code = "x = 1


y = 2
x + y";
    assert_output(code, "3");
}

#[test]
fn test_newline_in_arguments() {
    // Newlines dans arguments de fonction (ignorées comme whitespace)
    let code = "result = max(10,
    20,
    30,
    5)
result";
    assert_output(code, "30");
}

#[test]
fn test_newline_in_list() {
    // Newlines dans list literal (ignorées)
    let code = "x = list(1,
    2,
    3)
len(x)";
    assert_output(code, "3");
}

#[test]
fn test_newline_in_tuple() {
    // Newlines dans tuple literal (ignorées)
    let code = "t = tuple(1,
    2,
    3)
len(t)";
    assert_output(code, "3");
}

#[test]
fn test_newline_in_dict() {
    // Newlines dans dict literal (ignorées)
    let code = "d = dict((\"a\", 1),
    (\"b\", 2),
    (\"c\", 3))
len(d)";
    assert_output(code, "3");
}

#[test]
fn test_newline_after_opening_brace() {
    // Newline après { dans block
    let code = "x = {
    42
}
x";
    assert_output(code, "42");
}

#[test]
fn test_newline_in_lambda() {
    // Newlines dans lambda multilignes
    let code = "f = (x) => {
    y = x * 2
    y + 1
}
f(5)";
    assert_output(code, "11");
}

#[test]
fn test_newline_in_if() {
    // Newlines dans if expression
    let code = "x = if (True) {
    42
} else {
    0
}
x";
    assert_output(code, "42");
}

#[test]
fn test_newline_in_for_loop() {
    // Newlines dans for loop avec unpacking
    let code = "sum = 0
for (i, j) in list(tuple(1, 2), tuple(3, 4)) {
    sum = sum + i + j
}
sum";
    assert_output(code, "10");
}

#[test]
fn test_newline_in_while_loop() {
    // Newlines dans while loop
    let code = "x = 0
while (x < 5) {
    x = x + 1
}
x";
    assert_output(code, "5");
}

#[test]
fn test_unpacking_with_newline() {
    // Unpacking sur ligne séparée (pas d'ambiguïté avec appels chaînés)
    let code = "x = 1
(a, b) = tuple(2, 3)
a";
    assert_output(code, "2");
}

#[test]
fn test_chained_calls_no_newline() {
    // Appels chaînés sans newline (doit marcher)
    let code = "make_adder = (x) => { (y) => { x + y } }; make_adder(5)(3)";
    assert_output(code, "8");
}

#[test]
fn test_chained_calls_with_newline_between_statements() {
    // Appels chaînés avec newline entre statements (pas entre les appels)
    let code = "make_adder = (x) => { (y) => { x + y } }
make_adder(5)(3)";
    assert_output(code, "8");
}

#[test]
fn test_deeply_nested_multiline() {
    // Nesting profond avec newlines à tous les niveaux
    let code = "f = (a) => {
    g = (b) => {
        h = (c) => {
            a + b + c
        }
        h
    }
    g
}
result = f(1)(2)(3)
result";
    assert_output(code, "6");
}

#[test]
fn test_trailing_newlines() {
    // Trailing newlines après dernier statement
    let code = "x = 42

";
    assert_output(code, "42");
}

#[test]
fn test_leading_newlines() {
    // Leading newlines avant premier statement
    let code = "

x = 42
x";
    assert_output(code, "42");
}

#[test]
fn test_empty_lines_in_block() {
    // Lignes vides dans un block
    let code = "x = {
    a = 1

    b = 2

    a + b
}
x";
    assert_output(code, "3");
}

#[test]
fn test_newline_after_comma_in_lambda_params() {
    // Newlines dans paramètres de lambda (ignorées)
    let code = "f = (a,
    b,
    c) => { a + b + c }
f(1, 2, 3)";
    assert_output(code, "6");
}

#[test]
fn test_complex_mixed_case() {
    // Cas complexe : mix de tout
    let code = "make_adder = (x) => { (y) => { x + y } }
values = list(1,
    2,
    3)
sum = 0; for v in values {
    sum = sum + v
}
adder = make_adder(sum)
result = adder(10)
result";
    assert_output(code, "16"); // sum=6, adder(10)=16
}

#[test]
fn test_newline_in_nested_arguments() {
    // Newlines dans arguments imbriqués
    let code = "result = max(
    min(10, 20),
    min(
        30,
        40
    ),
    5
)
result";
    assert_output(code, "30"); // max(10, 30, 5) = 30
}

#[test]
fn test_expression_continuation() {
    // Expression qui continue sur ligne suivante via parenthèses
    let code = "x = (1 +
    2 +
    3)
x";
    assert_output(code, "6");
}
