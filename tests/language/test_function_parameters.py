# FILE: tests/language/test_function_parameters.py
"""
Tests for function and lambda parameters.

Critical tests to prevent regressions in parameter binding,
especially for None defaults.
"""

import pytest

from catnip import Catnip


class TestFunctionDefaultParameters:
    """Tests for parameter defaults."""

    def test_function_with_none_default(self):
        """Ensure a parameter with default=None is bound correctly.

        Regression: In earlier versions, `if default is not None`
        prevented binding when default was None.
        """
        catnip = Catnip()
        code = """
        f = (x=None) => {
            if x == None {
                x = 10
            }
            x
        }

        f()
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 10

    def test_function_with_none_default_not_overridden(self):
        """Ensure default None can be used as-is."""
        catnip = Catnip()
        code = """
        f = (x=None) => { x }

        f()
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result is None

    def test_function_with_none_default_can_be_overridden(self):
        """Ensure an argument can override None."""
        catnip = Catnip()
        code = """
        f = (x=None) => {
            if x == None {
                x = 5
            }
            x
        }

        f(42)
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 42

    def test_function_multiple_params_with_none(self):
        """Multiple parameters with None defaults."""
        catnip = Catnip()
        code = """
        f = (a=None, b=None, c=None) => {
            if a == None { a = 1 }
            if b == None { b = 2 }
            if c == None { c = 3 }
            a + b + c
        }

        f()
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 6

    def test_function_mixed_defaults_with_none(self):
        """Mix of None and other defaults."""
        catnip = Catnip()
        code = """
        f = (a=None, b=10, c=None) => {
            if a == None { a = 1 }
            if c == None { c = 3 }
            a + b + c
        }

        f()
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 14

    def test_lambda_with_none_default(self):
        """Ensure a lambda with default=None also works."""
        catnip = Catnip()
        code = """
        lam = (x=None) => {
            if x == None { x = 100 }
            x
        }

        lam()
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 100

    def test_nested_function_with_none_default(self):
        """Nested functions with None defaults."""
        catnip = Catnip()
        code = """
        outer = (x=None) => {
            if x == None { x = 5 }
            inner = (y=None) => {
                if y == None { y = x * 2 }
                y
            }
            inner()
        }

        outer()
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 10

    def test_function_none_default_in_condition(self):
        """Real-world test case : binary_search pattern avec right=None.

        This test reproduces the original bug found in binary_search.cat
        where right=None was not bound correctly.
        """
        catnip = Catnip()
        code = """
        # Simulate binary_search pattern
        check_bounds = (list_param, right=None) => {
            if right == None {
                right = len(list_param) - 1
            }
            right
        }

        check_bounds(list(1, 2, 3, 4, 5))
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 4  # len(list_param) - 1 = 5 - 1 = 4

    def test_function_none_default_with_comparison(self):
        """Ensure None can be compared in conditions."""
        catnip = Catnip()
        code = """
        f = (x=None) => {
            # Toutes ces comparaisons doivent fonctionner
            a = x == None
            b = x != None
            c = None == x
            list(a, b, c)
        }

        f()
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [True, False, True]

    def test_function_none_default_can_be_reassigned(self):
        """Ensure a parameter with default=None can be reassigned."""
        catnip = Catnip()
        code = """
        f = (x=None) => {
            x = 1
            x = 2
            x = 3
            x
        }

        f()
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 3


class TestFunctionRequiredParameters:
    """Tests for required parameters (no defaults)."""

    def test_function_missing_required_parameter(self):
        """Missing parameter is None (permissive arity, unlike strict struct constructors)."""
        catnip = Catnip()
        code = """
        f = (x) => { x }
        f()
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result is None

    def test_function_required_and_default_mixed(self):
        """Test mixing required and optional parameters."""
        catnip = Catnip()
        code = """
        f = (a, b=None) => {
            if b == None { b = a * 2 }
            a + b
        }

        f(5)
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 15  # 5 + 10


class TestFunctionOtherDefaults:
    """Tests for other default value types."""

    def test_function_with_integer_default(self):
        """Test les defaults avec des entiers."""
        catnip = Catnip()
        code = """
        f = (x=42) => { x }
        f()
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 42

    def test_function_with_string_default(self):
        """Test les defaults avec des strings."""
        catnip = Catnip()
        code = """
        f = (x="hello") => { x }
        f()
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == "hello"

    def test_function_with_boolean_default(self):
        """Defaults with booleans."""
        catnip = Catnip()
        code = """
        f = (x=True, y=False) => { list(x, y) }
        f()
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [True, False]


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
