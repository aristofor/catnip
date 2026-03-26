# FILE: tests/language/test_decimal.py
"""Tests for Decimal exact type (base-10, rust_decimal backend)."""

import unittest
from decimal import Decimal

from catnip import Catnip
from catnip.exc import CatnipRuntimeError, CatnipTypeError


def run(code):
    c = Catnip()
    c.parse(code)
    c.execute()
    return c


def val(code):
    c = run(code)
    return c.context.result


class TestDecimalParsing(unittest.TestCase):

    def test_integer_suffix(self):
        assert val("42d") == Decimal("42")

    def test_float_suffix(self):
        assert val("0.1d") == Decimal("0.1")

    def test_multi_decimal(self):
        assert val("123.456d") == Decimal("123.456")

    def test_uppercase_suffix(self):
        assert val("0D") == Decimal("0")

    def test_zero(self):
        assert val("0d") == Decimal("0")


class TestDecimalArithmetic(unittest.TestCase):

    def test_canonical_exact(self):
        """0.1d + 0.2d == 0.3d -- the canonical base-10 test."""
        assert val("0.1d + 0.2d == 0.3d") is True

    def test_int_promotion_right(self):
        assert val("2 + 0.5d") == Decimal("2.5")

    def test_int_promotion_left(self):
        assert val("0.5d + 2") == Decimal("2.5")

    def test_sub(self):
        assert val("10d - 3d") == Decimal("7")

    def test_mul(self):
        assert val("2d * 3d") == Decimal("6")

    def test_div_exact(self):
        assert val("10d / 2d") == Decimal("5")


class TestDecimalFloatMixing(unittest.TestCase):

    def test_add_decimal_float(self):
        with self.assertRaises((CatnipRuntimeError, CatnipTypeError)):
            val("0.1d + 0.2")

    def test_add_float_decimal(self):
        with self.assertRaises((CatnipRuntimeError, CatnipTypeError)):
            val("0.2 + 0.1d")

    def test_mul_decimal_float(self):
        with self.assertRaises((CatnipRuntimeError, CatnipTypeError)):
            val("0.1d * 2.0")


class TestDecimalErrors(unittest.TestCase):

    def test_div_by_zero(self):
        with self.assertRaises((CatnipRuntimeError, ZeroDivisionError)):
            val("1d / 0d")

    def test_div_rounded(self):
        """1d / 3d rounds to 28 digits (Python decimal semantics)."""
        result = val("1d / 3d")
        assert result == Decimal("1") / Decimal("3")

    def test_zero_div_zero(self):
        with self.assertRaises((CatnipRuntimeError, ZeroDivisionError)):
            val("0d / 0d")


class TestDecimalComparisons(unittest.TestCase):

    def test_eq_same_value_different_scale(self):
        assert val("1.0d == 1d") is True

    def test_lt(self):
        assert val("1d < 2d") is True

    def test_gt(self):
        assert val("2d > 1d") is True

    def test_eq_int(self):
        assert val("1d == 1") is True

    def test_gt_int(self):
        assert val("2d > 1") is True

    def test_le(self):
        assert val("1d <= 1d") is True

    def test_ge(self):
        assert val("2d >= 1d") is True


class TestDecimalNegationTruthiness(unittest.TestCase):

    def test_neg(self):
        assert val("-1d") == Decimal("-1")

    def test_neg_fraction(self):
        assert val("-(0.5d)") == Decimal("-0.5")

    def test_zero_falsy(self):
        c = run("x = if 0d { 1 } else { 0 }")
        assert c.context.globals['x'] == 0

    def test_nonzero_truthy(self):
        c = run("x = if 1d { 1 } else { 0 }")
        assert c.context.globals['x'] == 1


class TestDecimalBuiltin(unittest.TestCase):

    def test_builtin_string(self):
        result = val('import("decimal", "Decimal"); Decimal("3.14")')
        assert result == Decimal("3.14")

    def test_builtin_int(self):
        result = val('import("decimal", "Decimal"); Decimal(42)')
        assert result == Decimal("42")


class TestDecimalEdgeCases(unittest.TestCase):

    def test_max_precision(self):
        # 28 digits: should work
        result = val("1234567890123456789012345678d")
        assert result == Decimal("1234567890123456789012345678")

    def test_neg_zero_eq_zero(self):
        assert val("-0d == 0d") is True

    def test_add_neg_eq_zero(self):
        assert val("1d + (-1d) == 0d") is True


if __name__ == "__main__":
    unittest.main()
