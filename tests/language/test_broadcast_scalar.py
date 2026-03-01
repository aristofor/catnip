# FILE: tests/language/test_broadcast_scalar.py
"""
Tests pour le broadcasting sur scalaires.
"""

import pytest

from catnip import Catnip


def exec_catnip(code):
    """Helper to execute Catnip code"""
    c = Catnip()
    c.parse(code)
    return c.execute()


class TestScalarBroadcastArithmetic:
    """Tests for arithmetic operations on scalars"""

    def test_scalar_addition(self):
        """5.[+ 10] additionne un scalaire"""
        result = exec_catnip("5.[+ 10]")
        assert result == 15

    def test_scalar_multiplication(self):
        """5.[* 2] multiplie un scalaire"""
        result = exec_catnip("5.[* 2]")
        assert result == 10

    def test_scalar_expression_as_target(self):
        """(2 + 3).[* 2] broadcast over expression result"""
        result = exec_catnip("(2 + 3).[* 2]")
        assert result == 10

    def test_scalar_division(self):
        """100.[/ 4] divise un scalaire"""
        result = exec_catnip("100.[/ 4]")
        assert result == 25.0

    def test_scalar_power(self):
        """2.[** 8] raises a scalar to a power"""
        result = exec_catnip("2.[** 8]")
        assert result == 256


class TestScalarBroadcastUnary:
    """Tests for unary operations on scalars"""

    def test_scalar_abs_positive(self):
        """5.[abs] applies abs to a positive scalar"""
        result = exec_catnip("5.[abs]")
        assert result == 5

    def test_scalar_abs_negative(self):
        """(-5).[abs] applies abs to a negative scalar"""
        result = exec_catnip("(-5).[abs]")
        assert result == 5


class TestScalarBroadcastChaining:
    """Tests for chaining scalar operations"""

    def test_scalar_chain_basic(self):
        """5.[* 2].[+ 1] chains multiplication then addition"""
        # 5 * 2 = 10, 10 + 1 = 11
        result = exec_catnip("5.[* 2].[+ 1]")
        assert result == 11

    def test_scalar_chain_complex(self):
        """10.[- 3].[* 2].[+ 5] chains multiple operations"""
        # (10 - 3) * 2 + 5 = 7 * 2 + 5 = 19
        result = exec_catnip("10.[- 3].[* 2].[+ 5]")
        assert result == 19


class TestScalarBroadcastFilter:
    """Tests pour le filtrage conditionnel sur scalaires"""

    def test_scalar_filter_pass(self):
        """5.[if > 0] retourne [5] quand condition vraie"""
        result = exec_catnip("5.[if > 0]")
        assert result == [5]

    def test_scalar_filter_fail(self):
        """5.[if < 0] retourne [] quand condition fausse"""
        result = exec_catnip("5.[if < 0]")
        assert result == []


class TestScalarBroadcastCallable:
    """Tests pour le broadcast de fonctions sur scalaires"""

    def test_scalar_with_lambda(self):
        """5.[double] applies a lambda to a scalar"""
        code = """
        double = (x) => { x * 2 }
        5.[double]
        """
        result = exec_catnip(code)
        assert result == 10

    def test_scalar_with_inline_lambda(self):
        """21.[(x) => { x * 2 }] applique une lambda inline"""
        result = exec_catnip("21.[(x) => { x * 2 }]")
        assert result == 42


class TestScalarVariableSymmetry:
    """Tests for symmetry between literal and variable scalars"""

    def test_literal_vs_variable_same_result(self):
        """Broadcasting on literal and variable gives the same result"""
        code = """
        a = 5
        x = a.[+ 10]
        y = 5.[+ 10]
        y - x
        """
        result = exec_catnip(code)
        assert result == 0  # x et y sont identiques
