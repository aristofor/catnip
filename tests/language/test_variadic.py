# FILE: tests/language/test_variadic.py
"""Tests for variadic functions (*args syntax)."""

import pytest

from catnip import Catnip


class TestVariadicLambdas:
    """Test variadic lambda functions."""

    def test_simple_variadic_lambda(self):
        """Test lambda with only variadic parameter."""
        c = Catnip()
        c.parse("f = (*args) => { args }; result = f(1, 2, 3)")
        c.execute()
        assert c.context.globals['result'] == [1, 2, 3]

    def test_variadic_lambda_sum(self):
        """Test variadic lambda that sums all arguments."""
        c = Catnip()
        c.parse("""
            somme = (*nums) => {
                total = 0
                for x in nums {
                    total = total + x
                }
                total
            }
            result = somme(1, 2, 3, 4, 5)
        """)
        c.execute()
        assert c.context.globals['result'] == 15

    def test_variadic_lambda_no_args(self):
        """Test calling variadic lambda with no arguments."""
        c = Catnip()
        c.parse("f = (*args) => { args }; result = f()")
        c.execute()
        assert c.context.globals['result'] == []

    def test_mixed_params_lambda(self):
        """Test lambda with regular and variadic parameters."""
        c = Catnip()
        c.parse("""
            f = (first, *rest) => {
                dict(("first", first), ("rest", rest))
            }
            result = f(1, 2, 3, 4)
        """)
        c.execute()
        assert c.context.globals['result'] == {'first': 1, 'rest': [2, 3, 4]}

    def test_multiple_regular_params_with_variadic(self):
        """Test lambda with multiple regular + variadic parameters."""
        c = Catnip()
        c.parse("""
            f = (a, b, *rest) => {
                dict(("a", a), ("b", b), ("rest", rest))
            }
            result = f(1, 2, 3, 4, 5)
        """)
        c.execute()
        assert c.context.globals['result'] == {'a': 1, 'b': 2, 'rest': [3, 4, 5]}

    def test_variadic_with_list_operations(self):
        """Test using Python list operations on variadic args."""
        c = Catnip()
        c.parse("""
            get_length = (*items) => { len(items) }
            result = get_length("a", "b", "c", "d")
        """)
        c.execute()
        assert c.context.globals['result'] == 4

    def test_variadic_with_min_max(self):
        """Test finding min/max of variadic args."""
        c = Catnip()
        c.parse("""
            find_range = (*nums) => {
                dict(("min", min(nums)), ("max", max(nums)))
            }
            result = find_range(5, 2, 8, 1, 9, 3)
        """)
        c.execute()
        assert c.context.globals['result'] == {'min': 1, 'max': 9}


class TestVariadicFunctions:
    """Test variadic named functions."""

    def test_simple_variadic_function(self):
        """Test named function with variadic parameter."""
        c = Catnip()
        c.parse("""
            collect = (*items) => {
                items
            }
            result = collect(1, 2, 3)
        """)
        c.execute()
        assert c.context.globals['result'] == [1, 2, 3]

    def test_variadic_function_sum(self):
        """Test variadic function that computes sum."""
        c = Catnip()
        c.parse("""
            sum_all = (*numbers) => {
                total = 0
                for n in numbers {
                    total = total + n
                }
                total
            }
            result = sum_all(10, 20, 30, 40)
        """)
        c.execute()
        assert c.context.globals['result'] == 100

    def test_mixed_params_function(self):
        """Test function with regular and variadic parameters."""
        c = Catnip()
        c.parse("""
            greet = (greeting, *names) => {
                result = list()
                for name in names {
                    result = result + list(greeting + " " + name)
                }
                result
            }
            greet("Hello", "Alice", "Bob", "Charlie")
        """)
        assert c.execute() == ["Hello Alice", "Hello Bob", "Hello Charlie"]

    def test_variadic_with_defaults(self):
        """Test mixing default parameters with variadic."""
        c = Catnip()
        c.parse("""
            make_list = (prefix=0, *items) => {
                list(prefix) + list(items)
            }
            result1 = make_list(100, 1, 2, 3)
            result2 = make_list()
            list(result1, result2)
        """)
        assert c.execute() == [[100, [1, 2, 3]], [0, []]]


class TestVariadicEdgeCases:
    """Test edge cases for variadic parameters."""

    def test_variadic_empty_call(self):
        """Test variadic function called with no extra args."""
        c = Catnip()
        c.parse("""
            f = (required, *optional) => {
                dict(("required", required), ("optional", optional))
            }
            result = f(42)
        """)
        c.execute()
        assert c.context.globals['result'] == {'required': 42, 'optional': []}

    def test_variadic_single_arg(self):
        """Test variadic with exactly one vararg."""
        c = Catnip()
        c.parse("""
            f = (first, *rest) => { rest }
            result = f(1, 2)
        """)
        c.execute()
        assert c.context.globals['result'] == [2]

    def test_nested_variadic_calls(self):
        """Test calling one variadic function from another."""
        c = Catnip()
        c.parse("""
            inner = (*args) => { len(args) }
            outer = (*args) => { inner(args.__getitem__(0), args.__getitem__(1)) }
            result = outer(1, 2, 3, 4)
        """)
        c.execute()
        assert c.context.globals['result'] == 2


class TestVariadicWithLiterals:
    """Test combining variadic functions with new literals."""

    def test_variadic_returns_list_literal(self):
        """Test variadic function returning list literal (single arg wraps)."""
        c = Catnip()
        c.parse("""
            wrap = (*items) => { list(items) }
            result = wrap(1, 2, 3)
        """)
        c.execute()
        assert c.context.globals['result'] == [[1, 2, 3]]

    def test_variadic_with_dict_result(self):
        """Test variadic function returning dict with args."""
        c = Catnip()
        c.parse("""
            make_info = (*values) => {
                dict(("count", len(values)), ("values", values))
            }
            result = make_info(10, 20, 30)
        """)
        c.execute()
        assert c.context.globals['result'] == {'count': 3, 'values': [10, 20, 30]}

    def test_variadic_sum(self):
        """Test variadic function with sum."""
        c = Catnip()
        c.parse("""
            f = (*args) => { sum(args) }
            f(1, 2, 3, 4, 5)
        """)
        assert c.execute() == 15
