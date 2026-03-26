# FILE: tests/language/test_type.py
"""Tests for the native typeof() builtin."""

import pytest

from catnip import Catnip


def run(code):
    cat = Catnip()
    cat.parse(code)
    return cat.execute()


class TestTypeOf:
    """Basic typeof() returns."""

    def test_int(self):
        assert run('typeof(42)') == "int"

    def test_float(self):
        assert run('typeof(3.14)') == "float"

    def test_bool_true(self):
        assert run('typeof(True)') == "bool"

    def test_bool_false(self):
        assert run('typeof(False)') == "bool"

    def test_nil(self):
        assert run('typeof(None)') == "nil"

    def test_string(self):
        assert run('typeof("hello")') == "string"

    def test_list(self):
        assert run('typeof(list(1, 2, 3))') == "list"

    def test_tuple(self):
        assert run('typeof(tuple(1, 2, 3))') == "tuple"

    def test_empty_list(self):
        assert run('typeof(list())') == "list"

    def test_empty_tuple(self):
        assert run('typeof(tuple())') == "tuple"


class TestTypeOfFunctions:
    """typeof() on callables."""

    def test_lambda(self):
        assert run('typeof(() => { 1 })') == "function"

    def test_named_function(self):
        assert run('f = (x) => { x }; typeof(f)') == "function"


class TestTypeOfStruct:
    """typeof() on struct instances."""

    def test_struct_instance(self):
        assert run('struct Point { x; y }; typeof(Point(1, 2))') == "Point"

    def test_struct_different_types(self):
        code = '''
        struct Foo { a }
        struct Bar { b }
        f = Foo(1)
        b = Bar(2)
        typeof(f) + " " + typeof(b)
        '''
        assert run(code) == "Foo Bar"


class TestTypeOfExpressions:
    """typeof() on computed values."""

    def test_arithmetic_result(self):
        assert run('typeof(1 + 2)') == "int"

    def test_division_result(self):
        assert run('typeof(1 / 2)') == "float"

    def test_string_concat(self):
        assert run('typeof("a" + "b")') == "string"

    def test_comparison_result(self):
        assert run('typeof(1 < 2)') == "bool"

    def test_type_in_condition(self):
        assert run('if typeof(42) == "int" { "yes" } else { "no" }') == "yes"

    def test_bigint(self):
        assert run('typeof(2 ** 100)') == "int"
