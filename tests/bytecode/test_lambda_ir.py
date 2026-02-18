# FILE: tests/bytecode/test_lambda_ir.py
"""
Tests AST pour lambdas.

Vérifie que les lambdas génèrent le bon AST, notamment:
- Paramètres → Ref nodes (pas Identifier/String)
- Variables capturées → Ref nodes
- Structure correcte des Op nodes
"""

import pytest

from catnip.semantic.opcode import OpCode
from tests.bytecode.helpers import ASTInspector, inspect_ast


class TestLambdaIRStructure:
    """Tests la structure IR des lambdas."""

    def test_lambda_creates_lambda_op(self):
        """Lambda génère un Op avec opcode OpLambda."""
        inspector = inspect_ast("(x) => { x * 2 }")

        # Devrait contenir au moins un OpLambda
        lambda_ops = inspector._find_ops_by_opcode(inspector.ast, OpCode.OP_LAMBDA)
        assert len(lambda_ops) >= 1, "Should have at least one OpLambda node"

    def test_lambda_param_is_ref(self):
        """Paramètre de lambda utilisé dans le corps est un Ref."""
        inspector = inspect_ast("double = (x) => { x * 2 }; double(5)")

        # Le paramètre x utilisé dans le corps devrait être un Ref
        refs = inspector.assert_contains_ref("x")
        assert len(refs) >= 1, "Parameter x should be referenced as Ref in body"

    def test_lambda_multiple_params_all_refs(self):
        """Plusieurs paramètres sont tous des Ref dans le corps."""
        inspector = inspect_ast("add = (a, b) => { a + b }; add(3, 4)")

        # a et b devraient être des Ref
        refs_a = inspector.assert_contains_ref("a")
        refs_b = inspector.assert_contains_ref("b")

        assert len(refs_a) >= 1, "Parameter a should be Ref"
        assert len(refs_b) >= 1, "Parameter b should be Ref"


class TestLambdaClosureIR:
    """Tests closures dans l'IR."""

    def test_closure_captures_as_ref(self):
        """Variable capturée est un Ref dans le corps de la lambda."""
        inspector = inspect_ast("x = 100; f = () => { x }; f()")

        # x devrait être un Ref dans le corps de la lambda
        refs = inspector.assert_contains_ref("x")
        assert len(refs) >= 1, "Captured variable x should be Ref"

    def test_closure_with_param_both_refs(self):
        """Closure avec paramètre ET variable capturée, tous deux Ref."""
        inspector = inspect_ast("multiplier = 10; times = (n) => { n * multiplier }; times(5)")

        # n et multiplier devraient être des Ref
        refs_n = inspector.assert_contains_ref("n")
        refs_mult = inspector.assert_contains_ref("multiplier")

        assert len(refs_n) >= 1, "Parameter n should be Ref"
        assert len(refs_mult) >= 1, "Captured multiplier should be Ref"


class TestNestedLambdasIR:
    """Tests lambdas imbriquées dans l'IR."""

    def test_nested_lambdas_multiple_lambda_ops(self):
        """Lambdas imbriquées créent plusieurs OpLambda nodes."""
        inspector = inspect_ast("(x) => { (y) => { x + y } }")

        # Devrait avoir 2 OpLambda (externe et interne)
        lambda_ops = inspector._find_ops_by_opcode(inspector.ast, OpCode.OP_LAMBDA)
        assert len(lambda_ops) >= 2, "Should have at least 2 OpLambda nodes for nested lambdas"

    def test_nested_lambda_params_both_refs(self):
        """Lambdas imbriquées: paramètres des deux niveaux sont Ref."""
        inspector = inspect_ast("make_adder = (x) => { (y) => { x + y } }; g = make_adder(5); g(3)")

        # x et y devraient être des Ref
        refs_x = inspector.assert_contains_ref("x")
        refs_y = inspector.assert_contains_ref("y")

        assert len(refs_x) >= 1, "Outer parameter x should be Ref"
        assert len(refs_y) >= 1, "Inner parameter y should be Ref"


class TestLambdaIRRegression:
    """Tests de régression pour l'IR des lambdas."""

    def test_param_not_identifier_string(self):
        """
        Régression: Paramètres ne doivent PAS être des Identifier/String.

        Bug historique: Identifier("x") converti en PyString au lieu de Ref
        Fix: convert.rs traitement spécial OpLambda
        """
        inspector = inspect_ast("double = (x) => { x * 2 }; double(5)")

        # x DOIT être un Ref
        refs = inspector.assert_contains_ref("x")
        assert len(refs) >= 1, "Parameter x MUST be Ref (not Identifier/String)"

    def test_multiple_uses_same_param(self):
        """Paramètre utilisé plusieurs fois génère plusieurs Ref."""
        inspector = inspect_ast("square = (n) => { n * n }; square(7)")

        # n utilisé 2 fois → 2 Ref
        refs = inspector.assert_contains_ref("n")
        assert len(refs) >= 2, f"Parameter n used twice should have 2+ Refs, got {len(refs)}"
