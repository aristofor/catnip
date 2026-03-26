# FILE: tests/language/test_fold.py
"""Tests for fold and reduce builtins."""

import pytest

from catnip import Catnip


def exec_catnip(code):
    c = Catnip()
    c.parse(code)
    return c.execute()


class TestFold:

    def test_sum(self):
        assert exec_catnip("fold(list(1,2,3), 0, (acc,x)=>{acc+x})") == 6

    def test_string_concat(self):
        assert exec_catnip('fold(list("a","b","c"), "", (acc,x)=>{acc+x})') == "abc"

    def test_logical_and(self):
        assert exec_catnip("fold(list(True,True,False), True, (acc,x)=>{acc and x})") is False

    def test_empty(self):
        assert exec_catnip("fold(list(), 0, (acc,x)=>{acc+x})") == 0

    def test_single_element(self):
        assert exec_catnip("fold(list(42), 0, (acc,x)=>{acc+x})") == 42

    def test_nested_one_level(self):
        result = exec_catnip("fold(list(list(1,2), list(3,4)), 0, (acc,row)=>{acc + len(row)})")
        assert result == 4

    def test_with_broadcast(self):
        assert exec_catnip("fold(list(1,2,3).[* 10], 0, (acc,x)=>{acc+x})") == 60

    def test_product(self):
        assert exec_catnip("fold(list(1,2,3,4), 1, (acc,x)=>{acc*x})") == 24

    def test_callback_with_closure_capture(self):
        """Lambda capturing a variable from enclosing scope, passed as callback."""
        result = exec_catnip("""
            f = (predicate) => {
                fold(list(1, 2, 3), list(), (acc, x) => {
                    if predicate(x) { acc + list(x) } else { acc }
                })
            }
            is_gt1 = (x) => { x > 1 }
            f(is_gt1)
        """)
        assert result == [2, 3]

    def test_callback_with_value_capture(self):
        """Lambda capturing a plain value from enclosing scope."""
        result = exec_catnip("""
            make_adder = (n) => {
                fold(list(1, 2, 3), list(), (acc, x) => { acc + list(x + n) })
            }
            make_adder(10)
        """)
        assert result == [11, 12, 13]


class TestReduce:

    def test_sum(self):
        assert exec_catnip("reduce(list(1,2,3), (acc,x)=>{acc+x})") == 6

    def test_single_element(self):
        assert exec_catnip("reduce(list(42), (acc,x)=>{acc+x})") == 42

    def test_empty_raises(self):
        with pytest.raises(ValueError, match="empty sequence"):
            exec_catnip("reduce(list(), (acc,x)=>{acc+x})")

    def test_max(self):
        result = exec_catnip("""
            reduce(list(3,1,4,1,5,9), (acc,x)=>{
                if x > acc { x } else { acc }
            })
        """)
        assert result == 9
