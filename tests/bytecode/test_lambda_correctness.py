# FILE: tests/bytecode/test_lambda_correctness.py
"""
Tests de correctness pour lambdas.

Valide que les lambdas s'exécutent correctement, ce qui prouve indirectement
que le bytecode est correct (paramètres → LoadLocal, pas LoadConst).

Ces tests DOIVENT échouer si le bug VMFunction revient (Identifier → LoadConst).
"""

import pytest

from catnip import Catnip


def execute(code: str):
    """Helper pour parser et exécuter du code."""
    c = Catnip()
    c.parse(code)
    return c.execute()


class TestLambdaParameterCorrectness:
    """
    Tests que les paramètres de lambda sont correctement utilisés.

    Régression: Si les paramètres utilisaient LoadConst au lieu de LoadLocal,
    ces tests échoueraient.
    """

    def test_lambda_single_param_returns_param(self):
        """Lambda retournant son paramètre."""
        result = execute("identity = (x) => { x }; identity(42)")
        assert result == 42, "Lambda should return its parameter, not a constant"

    def test_lambda_param_in_expression(self):
        """Paramètre utilisé dans une expression - TEST CRITIQUE."""
        result = execute("double = (x) => { x * 2 }; double(5)")
        assert result == 10, "Expected 10, got 'xx' if bug is present"

    def test_lambda_param_multiple_uses(self):
        """Paramètre utilisé plusieurs fois."""
        result = execute("square = (n) => { n * n }; square(7)")
        assert result == 49, "Parameter n should be loaded correctly twice"

    def test_lambda_multiple_params(self):
        """Lambda avec plusieurs paramètres."""
        result = execute("add = (a, b) => { a + b }; add(3, 4)")
        assert result == 7

    def test_lambda_param_vs_constant(self):
        """Distinguer paramètre et constante."""
        # Si x était LoadConst, on aurait toujours 10
        result1 = execute("add10 = (x) => { x + 10 }; add10(5)")
        assert result1 == 15, "Parameter x (5) + constant 10 should be 15"

        result2 = execute("add10 = (x) => { x + 10 }; add10(20)")
        assert result2 == 30, "Parameter x (20) + constant 10 should be 30"


class TestLambdaClosureCorrectness:
    """Tests closures (capture de variables externes)."""

    def test_closure_captures_outer_variable(self):
        """Variable externe capturée correctement."""
        result = execute("x = 100; f = () => { x }; f()")
        assert result == 100

    def test_closure_with_param_and_capture(self):
        """Closure avec paramètre ET variable capturée."""
        result = execute("multiplier = 10; times = (n) => { n * multiplier }; times(5)")
        assert result == 50, "Param n (5) * captured multiplier (10) = 50"

    def test_closure_param_shadows_outer(self):
        """Paramètre avec même nom que variable externe."""
        result = execute("x = 100; f = (x) => { x * 2 }; f(5)")
        assert result == 10, "Parameter x (5) should shadow outer x (100)"


class TestNestedLambdasCorrectness:
    """
    Tests lambdas imbriquées - bytecode et closures.

    NOTE: Tests d'appels chaînés complets dans catnip_rs/tests/regression_chained_calls.rs
          (26 tests Rust couvrent tous les cas d'appels chaînés)
    """

    def test_nested_lambda_closure(self):
        """Lambda imbriquée capture paramètre externe (test bytecode closure)."""
        result = execute("make_adder = (x) => { (y) => { x + y } }; add5 = make_adder(5); add5(3)")
        assert result == 8, "Closure doit capturer x=5 et ajouter y=3"


class TestLambdaEdgeCases:
    """Cas limites."""

    def test_lambda_no_params(self):
        """Lambda sans paramètres."""
        result = execute("constant = () => { 42 }; constant()")
        assert result == 42

    def test_lambda_with_defaults(self):
        """Lambda avec paramètres par défaut."""
        result1 = execute("f = (x = 10) => { x * 2 }; f()")
        assert result1 == 20

        result2 = execute("f = (x = 10) => { x * 2 }; f(5)")
        assert result2 == 10


class TestLambdaRegression:
    """
    Tests de régression spécifiques pour bugs historiques.

    Bug 1: Identifier("x") converti en PyString → LoadConst au lieu de LoadLocal
    Symptôme: Lambda retournait 'xx' au lieu de 10
    Fix: convert.rs traitement spécial OpLambda
    Status: ✅ FIXÉ

    Bug 2: Appels chaînés f(a)(b) causaient ambiguïté avec unpacking
    Symptôme: `x = 1; (a, b) = tuple(2, 3)` parsé comme `1(a, b)` (appel chaîné)
    Problème: Sans séparateurs significatifs, tree-sitter ne pouvait pas distinguer
    Solution: External scanner pour newlines significatifs (scanner.c)
    Status: ✅ FIXÉ - Appels chaînés et unpacking fonctionnent simultanément
    """

    def test_bug_vmfunction_returns_string(self):
        """
        Régression critique: Lambda ne doit PAS retourner une string.

        Si le bug revient, ce test retournera 'xx' au lieu de 10.
        """
        result = execute("double = (x) => { x * 2 }; double(5)")

        # Assert strict: DOIT être un int, JAMAIS une string
        assert isinstance(result, int), f"Lambda returned {type(result).__name__} instead of int. Bug detected!"
        assert result == 10, f"Expected 10, got {result}"

    def test_bug_chained_calls_smoke_test(self):
        """
        Régression: Smoke test appels chaînés (tests complets en Rust).

        Bug historique: make_adder(5)(3) causait ambiguïté avec unpacking
        Solution: External scanner pour newlines significatifs
        Status: ✅ FIXÉ

        NOTE: Tests complets (10 tests) dans catnip_rs/tests/regression_chained_calls.rs
        """
        # Smoke test simple pour vérifier que l'intégration Python fonctionne
        result = execute("make_adder = (x) => { (y) => { x + y } }; make_adder(5)(3)")
        assert result == 8

    def test_bug_different_args_different_results(self):
        """
        Si le paramètre était constant, tous les appels retourneraient la même valeur.
        """
        # Même fonction, arguments différents
        result1 = execute("f = (x) => { x * 2 }; f(5)")
        result2 = execute("f = (x) => { x * 2 }; f(10)")
        result3 = execute("f = (x) => { x * 2 }; f(100)")

        assert result1 == 10
        assert result2 == 20
        assert result3 == 200

    def test_bug_param_used_in_complex_expr(self):
        """Paramètre dans expression complexe."""
        result = execute("calc = (a, b, c) => { (a + b) * c - a }; calc(2, 3, 4)")
        # (2 + 3) * 4 - 2 = 5 * 4 - 2 = 20 - 2 = 18
        assert result == 18
