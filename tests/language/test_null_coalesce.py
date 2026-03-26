# FILE: tests/language/test_null_coalesce.py
import pytest

from catnip import Catnip


class TestNullCoalesce:
    """Test ?? (nil-coalescing) operator"""

    @pytest.mark.parametrize(
        "code, expected",
        [
            ("42 ?? 0", 42),
            ("None ?? 0", 0),
            ("None ?? None ?? 3", 3),
            # ?? tests None only, not truthiness
            ("0 ?? 99", 0),
            ("False ?? 99", False),
            ('\"\" ?? 99', ""),
            # Chaining
            ("None ?? None ?? None ?? 42", 42),
            ("1 ?? 2 ?? 3", 1),
        ],
    )
    def test_null_coalesce(self, code, expected):
        cat = Catnip()
        cat.parse(code)
        result = cat.execute()
        assert result == expected

    def test_null_coalesce_short_circuit(self):
        """RHS should not be evaluated when LHS is not None"""
        cat = Catnip()
        cat.parse('x = 0; 42 ?? { x = 1; 99 }; x')
        result = cat.execute()
        assert result == 0

    def test_null_coalesce_evaluates_rhs_when_none(self):
        """RHS should be evaluated when LHS is None"""
        cat = Catnip()
        cat.parse('x = 0; None ?? { x = 1; 99 }; x')
        result = cat.execute()
        assert result == 1


class TestAndOrBool:
    """Test that and/or return boolean values"""

    @pytest.mark.parametrize(
        "code, expected",
        [
            ("True and True", True),
            ("True and False", False),
            ("False and True", False),
            ("False and False", False),
            # Non-bool operands are converted to bool
            ('"ok" and 42', True),
            ("0 and 1", False),
            ("1 and 0", False),
            ("1 and 2", True),
            # or
            ("True or False", True),
            ("False or True", True),
            ("False or False", False),
            ("True or True", True),
            ('0 or "fallback"', True),
            ("False or 1", True),
            ("0 or 0", False),
            ("0 or False", False),
        ],
    )
    def test_and_or_return_bool(self, code, expected):
        cat = Catnip()
        cat.parse(code)
        result = cat.execute()
        assert result is expected

    def test_and_short_circuit(self):
        """and should not evaluate RHS when LHS is falsy"""
        cat = Catnip()
        cat.parse('x = 0; False and { x = 1; True }; x')
        result = cat.execute()
        assert result == 0

    def test_or_short_circuit(self):
        """or should not evaluate RHS when LHS is truthy"""
        cat = Catnip()
        cat.parse('x = 0; True or { x = 1; False }; x')
        result = cat.execute()
        assert result == 0


class TestCombined:
    """Test ?? combined with and/or"""

    @pytest.mark.parametrize(
        "code, expected",
        [
            # ?? has lower precedence than or
            ("None ?? 0 or False", False),
            ("None ?? 1 or False", True),
            # ?? has lower precedence than and: 42 ?? (0 and True) → 42 ?? False → 42
            ("42 ?? 0 and True", 42),
            # With parens: (42 ?? 0) and True → 42 and True → True
            ("(42 ?? 0) and True", True),
        ],
    )
    def test_combined(self, code, expected):
        cat = Catnip()
        cat.parse(code)
        result = cat.execute()
        assert result is expected
