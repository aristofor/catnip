# FILE: tests/language/test_match_structural.py
"""Tests for structural pattern matching in match expressions"""

import pytest

from catnip import Catnip


class TestBasicTuplePatterns:
    """Test basic tuple pattern matching"""

    def test_simple_tuple_pattern(self):
        """Match simple tuple pattern: (a, b)"""
        cat = Catnip()
        code = """
        value = tuple(10, 20)
        match value {
            (a, b) => { a + b }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 30

    def test_tuple_pattern_with_literals(self):
        """Match tuple with literal values"""
        cat = Catnip()
        code = """
        point = tuple(0, 5)
        match point {
            (0, 0) => { "origin" }
            (0, y) => { y }
            (x, 0) => { x * 10 }
            (x, y) => { x + y }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 5

    def test_tuple_pattern_all_literals(self):
        """Match tuple with all literals"""
        cat = Catnip()
        code = """
        coord = tuple(1, 2)
        match coord {
            (0, 0) => { "no" }
            (1, 2) => { "yes" }
            _ => { "maybe" }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == "yes"

    def test_tuple_pattern_with_wildcard(self):
        """Match tuple with wildcard"""
        cat = Catnip()
        code = """
        data = tuple(42, 99)
        match data {
            (x, _) => { x }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 42


class TestStarPatterns:
    """Test patterns with star operator"""

    def test_star_at_end(self):
        """Pattern with star at end: (first, *rest)"""
        cat = Catnip()
        code = """
        values = list(1, 2, 3, 4, 5)
        match values {
            (first, *rest) => { tuple(first, len(rest)) }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == (1, 4)

    def test_star_at_beginning(self):
        """Pattern with star at beginning: (*init, last)"""
        cat = Catnip()
        code = """
        values = tuple(10, 20, 30, 40)
        match values {
            (*init, last) => { last }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 40

    def test_star_in_middle(self):
        """Pattern with star in middle: (a, *middle, z)"""
        cat = Catnip()
        code = """
        data = list(1, 2, 3, 4, 5)
        match data {
            (a, *middle, z) => { tuple(a, len(middle), z) }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == (1, 3, 5)

    def test_star_with_minimal_values(self):
        """Star captures empty list when minimal"""
        cat = Catnip()
        code = """
        data = tuple(10, 20)
        match data {
            (a, *rest, b) => { len(rest) }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 0


class TestNestedPatterns:
    """Test nested tuple patterns"""

    def test_simple_nested(self):
        """Simple nested pattern: (a, (b, c))"""
        cat = Catnip()
        code = """
        data = list(1, tuple(2, 3))
        match data {
            (a, (b, c)) => { a + b + c }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 6

    def test_deeply_nested(self):
        """Deeply nested pattern: (a, (b, (c, d)))"""
        cat = Catnip()
        code = """
        data = list(1, list(2, list(3, 4)))
        match data {
            (a, (b, (c, d))) => { a + b + c + d }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 10

    def test_nested_with_literals(self):
        """Nested pattern with literals: (0, (x, y))"""
        cat = Catnip()
        code = """
        data = tuple(0, tuple(10, 20))
        match data {
            (0, (x, y)) => { x * y }
            _ => { 0 }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 200

    def test_nested_with_star(self):
        """Nested pattern with star: (a, *mid, (x, y))"""
        cat = Catnip()
        code = """
        data = list(1, 2, 3, list(10, 20))
        match data {
            (a, *mid, (x, y)) => { tuple(a, len(mid), x, y) }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == (1, 2, 10, 20)


class TestMixedPatterns:
    """Test combinations of different pattern types"""

    def test_mix_literal_var_wildcard(self):
        """Mix literals, variables, and wildcards"""
        cat = Catnip()
        code = """
        data = tuple(0, 42, 99)
        match data {
            (0, x, _) => { x }
            _ => { 0 }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 42

    def test_pattern_with_guard(self):
        """Tuple pattern with guard"""
        cat = Catnip()
        code = """
        point = tuple(3, 3)
        match point {
            (x, y) if x == y => { "diagonal" }
            (x, y) => { "other" }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == "diagonal"

    def test_or_patterns_with_tuples(self):
        """OR patterns with tuple patterns"""
        cat = Catnip()
        code = """
        value = tuple(1, 1)
        match value {
            (0, 0) | (1, 1) => { "special" }
            _ => { "normal" }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == "special"

    def test_multiple_nested_levels(self):
        """Multiple levels of nesting"""
        cat = Catnip()
        code = """
        data = list(tuple(1, 2), tuple(3, 4))
        match data {
            ((a, b), (c, d)) => { a + b + c + d }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 10


class TestNonMatchingPatterns:
    """Test patterns that don't match"""

    def test_wrong_length(self):
        """Pattern doesn't match due to wrong length"""
        cat = Catnip()
        code = """
        value = tuple(1, 2, 3)
        match value {
            (a, b) => { "two" }
            (a, b, c) => { "three" }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == "three"

    def test_literal_mismatch(self):
        """Pattern doesn't match due to literal mismatch"""
        cat = Catnip()
        code = """
        point = tuple(1, 2)
        match point {
            (0, y) => { "on Y axis" }
            (x, 0) => { "on X axis" }
            _ => { "elsewhere" }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == "elsewhere"

    def test_non_iterable_value(self):
        """Non-iterable value doesn't match tuple pattern"""
        cat = Catnip()
        code = """
        value = 42
        match value {
            (a, b) => { "tuple" }
            x => { "not tuple" }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == "not tuple"

    def test_nested_structure_mismatch(self):
        """Nested pattern doesn't match wrong structure"""
        cat = Catnip()
        code = """
        value = tuple(1, 2)
        match value {
            (a, (b, c)) => { "nested" }
            (x, y) => { "flat" }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == "flat"


class TestEdgeCases:
    """Test edge cases and special scenarios"""

    def test_single_element_tuple(self):
        """Match single element tuple"""
        cat = Catnip()
        code = """
        value = tuple(42)
        match value {
            (x,) => { x * 2 }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 84

    def test_star_captures_empty(self):
        """Star captures empty list"""
        cat = Catnip()
        code = """
        value = list(10, 20)
        match value {
            (a, *rest, b) => { len(rest) }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 0

    def test_match_with_range(self):
        """Match pattern on range result"""
        cat = Catnip()
        code = """
        value = range(3)
        match value {
            (a, b, c) => { a + b + c }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 3  # 0 + 1 + 2

    def test_pattern_with_list_matching_tuple(self):
        """List value matches tuple pattern"""
        cat = Catnip()
        code = """
        value = list(1, 2, 3)
        match value {
            (a, b, c) => { "matched" }
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == "matched"
