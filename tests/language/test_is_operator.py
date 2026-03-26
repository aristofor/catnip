# FILE: tests/language/test_is_operator.py
# Tests for `is` / `is not` identity operators

import pytest

from catnip import Catnip


@pytest.fixture
def cat():
    return Catnip()


def run(cat, code):
    cat.parse(code)
    return cat.execute()


# --- is ---


class TestIs:
    def test_none_is_none(self, cat):
        assert run(cat, "None is None") is True

    def test_true_is_true(self, cat):
        assert run(cat, "True is True") is True

    def test_false_is_false(self, cat):
        assert run(cat, "False is False") is True

    def test_none_is_not_true(self, cat):
        assert run(cat, "None is True") is False

    def test_variable_is_none(self, cat):
        assert run(cat, "x = None\nx is None") is True

    def test_variable_is_not_none(self, cat):
        assert run(cat, "x = 42\nx is None") is False

    def test_same_variable(self, cat):
        assert run(cat, "x = list(1, 2, 3)\nx is x") is True

    def test_different_lists(self, cat):
        assert run(cat, "a = list(1, 2, 3)\nb = list(1, 2, 3)\na is b") is False


# --- is not ---


class TestIsNot:
    def test_none_is_not_none(self, cat):
        assert run(cat, "None is not None") is False

    def test_none_is_not_true(self, cat):
        assert run(cat, "None is not True") is True

    def test_variable_is_not_none(self, cat):
        assert run(cat, "x = 42\nx is not None") is True

    def test_variable_is_none(self, cat):
        assert run(cat, "x = None\nx is not None") is False

    def test_different_lists(self, cat):
        assert run(cat, "a = list(1, 2, 3)\nb = list(1, 2, 3)\na is not b") is True

    def test_same_variable_is_not(self, cat):
        assert run(cat, "x = list(1, 2, 3)\nx is not x") is False


# --- in control flow ---


class TestIsControlFlow:
    def test_if_is_none(self, cat):
        code = """
x = None
if x is None {
    "null"
} else {
    "not null"
}
"""
        assert run(cat, code) == "null"

    def test_if_is_not_none(self, cat):
        code = """
x = 42
if x is not None {
    "has value"
} else {
    "null"
}
"""
        assert run(cat, code) == "has value"


# --- VM mode explicit ---


class TestIsVM:
    def test_is_none_vm(self):
        cat = Catnip(vm_mode='on')
        assert run(cat, "None is None") is True

    def test_is_not_none_vm(self):
        cat = Catnip(vm_mode='on')
        assert run(cat, "42 is not None") is True


# --- AST mode explicit ---


class TestIsAST:
    def test_is_none_ast(self):
        cat = Catnip(vm_mode='off')
        assert run(cat, "None is None") is True

    def test_is_not_none_ast(self):
        cat = Catnip(vm_mode='off')
        assert run(cat, "42 is not None") is True
