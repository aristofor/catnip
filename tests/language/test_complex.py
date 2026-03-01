# FILE: tests/language/test_complex.py
"""Tests for complex number support (j/J suffix literals)."""

import pytest

from catnip import Catnip
from catnip.exc import CatnipRuntimeError, CatnipTypeError


def val(code):
    c = Catnip()
    c.parse(code)
    c.execute()
    return c.context.result


# ============================================================================
# Parsing
# ============================================================================


class TestParsing:
    def test_pure_imaginary_int(self):
        assert val("2j") == 2j

    def test_pure_imaginary_float(self):
        assert val("1.5j") == 1.5j

    def test_zero_imaginary(self):
        assert val("0j") == 0j

    def test_uppercase_suffix(self):
        assert val("1J") == 1j


# ============================================================================
# Construction via addition (1+2j)
# ============================================================================


class TestConstruction:
    def test_int_plus_imaginary(self):
        assert val("1 + 2j") == (1 + 2j)

    def test_int_minus_imaginary(self):
        assert val("1 - 2j") == (1 - 2j)

    def test_float_plus_imaginary(self):
        assert val("1.5 + 0.5j") == (1.5 + 0.5j)


# ============================================================================
# Arithmetic
# ============================================================================


class TestArithmetic:
    def test_add(self):
        assert val("(1+2j) + (3+4j)") == (4 + 6j)

    def test_sub(self):
        assert val("(1+2j) - (3+4j)") == (-2 - 2j)

    def test_mul(self):
        assert val("(1+2j) * (3+4j)") == (-5 + 10j)

    def test_div(self):
        assert val("(1+2j) / (1+0j)") == (1 + 2j)

    def test_pow(self):
        assert val("(2+0j) ** 2") == (4 + 0j)


# ============================================================================
# Mixed numeric
# ============================================================================


class TestMixed:
    def test_int_plus_complex(self):
        result = val("1 + 2j")
        assert isinstance(result, complex)
        assert result == (1 + 2j)

    def test_float_plus_complex(self):
        result = val("1.5 + 2j")
        assert isinstance(result, complex)

    def test_complex_plus_int(self):
        assert val("2j + 1") == (1 + 2j)

    def test_complex_times_int(self):
        assert val("2j * 3") == 6j


# ============================================================================
# Equality
# ============================================================================


class TestEquality:
    def test_equal(self):
        assert val("(1+2j) == (1+2j)") is True

    def test_not_equal(self):
        assert val("(1+2j) != (1+3j)") is True

    def test_zero_eq_int(self):
        assert val("0j == 0") is True

    def test_real_eq_int(self):
        assert val("(1+0j) == 1") is True


# ============================================================================
# Ordering (forbidden)
# ============================================================================


class TestOrdering:
    def test_lt_raises(self):
        with pytest.raises(CatnipTypeError):
            val("1j < 2j")

    def test_le_raises(self):
        with pytest.raises(CatnipTypeError):
            val("1j <= 2j")

    def test_gt_raises(self):
        with pytest.raises(CatnipTypeError):
            val("1j > 2j")

    def test_ge_raises(self):
        with pytest.raises(CatnipTypeError):
            val("1j >= 2j")


# ============================================================================
# Negation
# ============================================================================


class TestNegation:
    def test_negate_imaginary(self):
        assert val("-2j") == -2j

    def test_negate_complex(self):
        assert val("-(1+2j)") == (-1 - 2j)


# ============================================================================
# Attributes and functions
# ============================================================================


class TestAttributes:
    def test_real(self):
        assert val("(1+2j).real") == 1.0

    def test_imag(self):
        assert val("(1+2j).imag") == 2.0

    def test_conjugate(self):
        assert val("(1+2j).conjugate()") == (1 - 2j)

    def test_abs(self):
        assert val("abs(3+4j)") == 5.0


# ============================================================================
# Errors
# ============================================================================


class TestErrors:
    def test_div_by_zero(self):
        with pytest.raises(CatnipRuntimeError):
            val("(1+2j) / 0j")

    def test_div_by_zero_explicit(self):
        with pytest.raises(CatnipRuntimeError):
            val("(1+2j) / (0+0j)")


# ============================================================================
# Builtin constructor
# ============================================================================


class TestBuiltin:
    def test_complex_constructor(self):
        assert val("complex(1, 2)") == (1 + 2j)

    def test_complex_zero(self):
        assert val("complex(0, 0)") == 0j

    def test_complex_real_only(self):
        assert val("complex(1, 0)") == (1 + 0j)


# ============================================================================
# Truthiness
# ============================================================================


class TestTruthiness:
    def test_zero_falsy(self):
        assert val("0j") == 0j
        assert val("if 0j { 1 } else { 0 }") == 0

    def test_nonzero_truthy(self):
        assert val("if 1j { 1 } else { 0 }") == 1

    def test_real_nonzero_truthy(self):
        assert val("if (1+0j) { 1 } else { 0 }") == 1
