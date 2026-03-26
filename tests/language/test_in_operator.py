# FILE: tests/language/test_in_operator.py
"""Tests for `in` and `not in` operators."""

from catnip import Catnip
from catnip.exc import CatnipTypeError


def run(code):
    cat = Catnip()
    cat.parse(code)
    return cat.execute()


# --- in ---


class TestInOperator:
    def test_in_list(self):
        assert run("2 in list(1, 2, 3)") is True

    def test_in_list_absent(self):
        assert run("5 in list(1, 2, 3)") is False

    def test_in_tuple(self):
        assert run("2 in tuple(1, 2, 3)") is True

    def test_in_tuple_absent(self):
        assert run("5 in tuple(1, 2, 3)") is False

    def test_in_dict(self):
        assert run('"a" in dict(a=1, b=2)') is True

    def test_in_dict_absent(self):
        assert run('"c" in dict(a=1, b=2)') is False

    def test_in_string(self):
        assert run('"cat" in "catnip"') is True

    def test_in_string_absent(self):
        assert run('"dog" in "catnip"') is False

    def test_in_set(self):
        assert run("2 in set(1, 2, 3)") is True

    def test_in_set_absent(self):
        assert run("5 in set(1, 2, 3)") is False

    def test_in_with_variable(self):
        assert run("xs = list(1, 2, 3); 2 in xs") is True

    def test_in_with_expression(self):
        assert run("(1 + 1) in list(1, 2, 3)") is True


# --- not in ---


class TestNotInOperator:
    def test_not_in_list(self):
        assert run("5 not in list(1, 2, 3)") is True

    def test_not_in_list_present(self):
        assert run("2 not in list(1, 2, 3)") is False

    def test_not_in_tuple(self):
        assert run("5 not in tuple(1, 2, 3)") is True

    def test_not_in_dict(self):
        assert run('"c" not in dict(a=1, b=2)') is True

    def test_not_in_dict_present(self):
        assert run('"a" not in dict(a=1, b=2)') is False

    def test_not_in_string(self):
        assert run('"dog" not in "catnip"') is True

    def test_not_in_string_present(self):
        assert run('"cat" not in "catnip"') is False

    def test_not_in_set(self):
        assert run("5 not in set(1, 2, 3)") is True

    def test_not_in_with_variable(self):
        assert run("xs = list(1, 2, 3); 5 not in xs") is True


# --- in conditionals ---


class TestInConditionals:
    def test_if_in(self):
        assert run('if 2 in list(1, 2, 3) { "yes" } else { "no" }') == "yes"

    def test_if_not_in(self):
        assert run('if 5 not in list(1, 2, 3) { "yes" } else { "no" }') == "yes"

    def test_while_not_in(self):
        code = """
        seen = list()
        i = 0
        while i not in seen {
            seen = seen + list(i)
            i = i + 1
            if i > 3 { break }
        }
        seen
        """
        assert run(code) == [0, 1, 2, 3]


# --- struct overload ---


STRUCT_WITH_OP_IN = """
struct Bag {
    items

    op in(self, item) => {
        item in self.items
    }
}
"""

STRUCT_WITH_OP_NOT_IN = """
struct Bag {
    items

    op in(self, item) => {
        item in self.items
    }

    op not in(self, item) => {
        item not in self.items
    }
}
"""


class TestStructOpIn:
    def test_op_in_true(self):
        assert run(STRUCT_WITH_OP_IN + 'b = Bag(list(1, 2, 3)); 2 in b') is True

    def test_op_in_false(self):
        assert run(STRUCT_WITH_OP_IN + 'b = Bag(list(1, 2, 3)); 5 in b') is False

    def test_not_in_via_op_in_true(self):
        """not in uses negation of op_in (Python __contains__ protocol)."""
        assert run(STRUCT_WITH_OP_IN + 'b = Bag(list(1, 2, 3)); 5 not in b') is True

    def test_not_in_via_op_in_false(self):
        assert run(STRUCT_WITH_OP_IN + 'b = Bag(list(1, 2, 3)); 2 not in b') is False


class TestStructOpNotIn:
    def test_op_not_in_vm_true(self):
        """VM dispatches op_not_in directly."""
        assert run(STRUCT_WITH_OP_NOT_IN + 'b = Bag(list(1, 2, 3)); 5 not in b') is True

    def test_op_not_in_vm_false(self):
        assert run(STRUCT_WITH_OP_NOT_IN + 'b = Bag(list(1, 2, 3)); 2 not in b') is False


class TestStructInNoOp:
    def test_in_without_op_raises(self):
        code = """
        struct Empty { x }
        e = Empty(1)
        1 in e
        """
        import pytest

        with pytest.raises(CatnipTypeError, match="not iterable"):
            run(code)
