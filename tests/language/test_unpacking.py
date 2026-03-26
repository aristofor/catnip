# FILE: tests/language/test_unpacking.py
"""Tests for unpacking/destructuring syntax"""

import pytest

from catnip import Catnip
from catnip.exc import CatnipRuntimeError, CatnipTypeError


class TestBasicUnpacking:
    """Test basic unpacking without star operator"""

    def test_tuple_unpacking_with_parens(self):
        """Unpacking with parentheses: (a, b) = tuple(1, 2)"""
        cat = Catnip()
        code = """
        (a, b) = tuple(10, 20)
        a
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 10
        assert cat.context.globals['b'] == 20

    def test_list_unpacking_with_parens(self):
        """Unpacking list with parentheses: (x, y) = list(1, 2)"""
        cat = Catnip()
        code = """
        (x, y) = list(100, 200)
        y
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 200
        assert cat.context.globals['x'] == 100

    def test_unpacking_without_parens(self):
        """Unpacking without parentheses: a, b = list(1, 2)"""
        cat = Catnip()
        code = """
        x, y, z = list(1, 2, 3)
        y
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 2
        assert cat.context.globals['x'] == 1
        assert cat.context.globals['z'] == 3

    def test_unpacking_three_values(self):
        """Unpacking three values"""
        cat = Catnip()
        code = """
        (a, b, c) = tuple(5, 10, 15)
        a + b + c
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 30


class TestStarOperator:
    """Test unpacking with star operator"""

    def test_star_at_end(self):
        """Star operator at end: (a, *rest) = ..."""
        cat = Catnip()
        code = """
        (first, *rest) = list(1, 2, 3, 4, 5)
        first
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 1
        assert cat.context.globals['rest'] == [2, 3, 4, 5]

    def test_star_at_beginning(self):
        """Star operator at beginning: (*init, last) = ..."""
        cat = Catnip()
        code = """
        (*init, last) = tuple(10, 20, 30, 40)
        last
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 40
        assert cat.context.globals['init'] == [10, 20, 30]

    def test_star_in_middle(self):
        """Star operator in middle: (a, *mid, z) = ..."""
        cat = Catnip()
        code = """
        (a, *middle, z) = list(1, 2, 3, 4, 5)
        z
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 5
        assert cat.context.globals['a'] == 1
        assert cat.context.globals['middle'] == [2, 3, 4]

    def test_star_with_minimal_values(self):
        """Star captures empty list when minimal values"""
        cat = Catnip()
        code = """
        (a, *rest, b) = list(10, 20)
        rest
        """
        cat.parse(code)
        result = cat.execute()
        assert result == []
        assert cat.context.globals['a'] == 10
        assert cat.context.globals['b'] == 20

    def test_star_captures_many_values(self):
        """Star captures many values"""
        cat = Catnip()
        code = """
        (x, *ys) = list(1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
        len(ys)
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 9
        assert cat.context.globals['x'] == 1
        assert cat.context.globals['ys'] == [2, 3, 4, 5, 6, 7, 8, 9, 10]


class TestNestedUnpacking:
    """Test nested/recursive unpacking patterns"""

    def test_simple_nested(self):
        """Simple nested pattern: (a, (b, c)) = ..."""
        cat = Catnip()
        code = """
        (i, (j, k)) = list(100, list(200, 300))
        j
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 200
        assert cat.context.globals['i'] == 100
        assert cat.context.globals['k'] == 300

    def test_deeply_nested(self):
        """Deeply nested pattern: (a, (b, (c, d))) = ..."""
        cat = Catnip()
        code = """
        (a, (b, (c, d))) = list(1, list(2, list(3, 4)))
        c
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 3
        assert cat.context.globals['a'] == 1
        assert cat.context.globals['b'] == 2
        assert cat.context.globals['d'] == 4

    def test_star_in_nested_pattern(self):
        """Star in nested pattern: (a, *mid, (x, y)) = ..."""
        cat = Catnip()
        code = """
        (a, *mid, (x, y)) = list(1, 2, 3, 4, list(10, 20))
        mid
        """
        cat.parse(code)
        result = cat.execute()
        assert result == [2, 3, 4]
        assert cat.context.globals['a'] == 1
        assert cat.context.globals['x'] == 10
        assert cat.context.globals['y'] == 20

    def test_nested_with_star_inside(self):
        """Star inside nested pattern: ((a, *rest), b) = ..."""
        cat = Catnip()
        code = """
        ((p, *q), r) = list(list(1, 2, 3), 4)
        q
        """
        cat.parse(code)
        result = cat.execute()
        assert result == [2, 3]
        assert cat.context.globals['p'] == 1
        assert cat.context.globals['r'] == 4

    def test_multiple_nested_levels(self):
        """Multiple levels of nesting"""
        cat = Catnip()
        code = """
        ((a, b), (c, d)) = list(list(1, 2), list(3, 4))
        a + b + c + d
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 10


class TestUnpackingInForLoops:
    """Test unpacking in for loop contexts"""

    def test_for_simple_unpacking(self):
        """for (a, b) in pairs"""
        cat = Catnip()
        code = """
        sum = 0
        for (i, j) in list(tuple(1, 2), tuple(3, 4), tuple(5, 6)) {
            sum = sum + i + j
        }
        sum
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 21  # 1+2+3+4+5+6 = 21

    def test_for_unpacking_without_parens(self):
        """for a, b in pairs (without parens)"""
        cat = Catnip()
        code = """
        product = 1
        for x, y in list(tuple(2, 3), tuple(4, 5)) {
            product = product * x * y
        }
        product
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 120  # 1 * 2 * 3 * 4 * 5 = 120

    def test_for_with_star(self):
        """for (head, *tail) in chunks"""
        cat = Catnip()
        code = """
        heads = list()
        for (h, *t) in list(list(1, 2, 3), list(4, 5, 6, 7)) {
            heads = heads + list(h)
        }
        heads
        """
        cat.parse(code)
        result = cat.execute()
        assert result == [1, 4]

    def test_for_nested_unpacking(self):
        """for (a, (b, c)) in nested"""
        cat = Catnip()
        code = """
        result = list()
        for (x, (y, z)) in list(list(1, list(2, 3)), list(4, list(5, 6))) {
            result = result + list(x + y + z)
        }
        result
        """
        cat.parse(code)
        result = cat.execute()
        assert result == [6, 15]  # (1+2+3), (4+5+6)


class TestUnpackingErrors:
    """Test error handling in unpacking"""

    def test_too_many_values(self):
        """Error when too many values to unpack"""
        cat = Catnip()
        code = """
        (a, b) = list(1, 2, 3)
        """
        cat.parse(code)
        with pytest.raises(CatnipRuntimeError, match="Cannot unpack 3 values into 2 variables"):
            cat.execute()

    def test_too_few_values(self):
        """Error when too few values to unpack"""
        cat = Catnip()
        code = """
        (a, b, c) = list(1, 2)
        """
        cat.parse(code)
        with pytest.raises(CatnipRuntimeError, match="Cannot unpack 2 values into 3 variables"):
            cat.execute()

    def test_non_iterable(self):
        """Error when unpacking non-iterable"""
        cat = Catnip()
        code = """
        (a, b) = 42
        """
        cat.parse(code)
        with pytest.raises(CatnipTypeError, match="Cannot unpack non-iterable"):
            cat.execute()

    def test_star_too_few_values(self):
        """Error when star pattern requires more values"""
        cat = Catnip()
        code = """
        (a, b, *rest, c, d) = list(1, 2)
        """
        cat.parse(code)
        with pytest.raises(CatnipRuntimeError, match="Not enough values to unpack"):
            cat.execute()

    def test_nested_wrong_structure(self):
        """Error when nested pattern doesn't match structure"""
        cat = Catnip()
        code = """
        (a, (b, c)) = list(1, 2)
        """
        cat.parse(code)
        with pytest.raises(CatnipTypeError, match="Cannot unpack non-iterable"):
            cat.execute()


class TestUnpackingEdgeCases:
    """Test edge cases and special scenarios"""

    def test_single_element_unpacking(self):
        """Unpack single element (still valid)"""
        cat = Catnip()
        code = """
        (x,) = list(42)
        x
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 42

    def test_empty_star(self):
        """Star captures empty list"""
        cat = Catnip()
        code = """
        (a, *rest) = list(1)
        len(rest)
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 0
        assert cat.context.globals['rest'] == []

    def test_unpacking_range(self):
        """Unpack from range() result"""
        cat = Catnip()
        code = """
        (a, b, c) = range(3)
        a + b + c
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 3  # 0 + 1 + 2

    def test_reassignment_with_unpacking(self):
        """Reassign variables using unpacking"""
        cat = Catnip()
        code = """
        a = 10
        b = 20
        (a, b) = tuple(b, a)
        a
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 20
        assert cat.context.globals['b'] == 10
