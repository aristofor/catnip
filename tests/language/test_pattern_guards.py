# FILE: tests/language/test_pattern_guards.py
"""
Tests for pattern matching guards and OR patterns.

Tests guard conditions, captured variable access in guards, and OR pattern combinations.
"""

import pytest

from catnip import Catnip


class TestBasicGuards:
    """Test basic guard functionality."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_simple_guard_true(self, catnip):
        """Guard that evaluates to true."""
        code = """
        x = 10
        match x {
            n if n > 5 => { "big" }
            _ => { "small" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "big"

    def test_simple_guard_false(self, catnip):
        """Guard that evaluates to false, falls through."""
        code = """
        x = 3
        match x {
            n if n > 5 => { "big" }
            _ => { "small" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "small"

    def test_multiple_guards(self, catnip):
        """Multiple cases with guards."""
        code = """
        x = 50
        match x {
            n if n > 100 => { "huge" }
            n if n > 50 => { "big" }
            n if n > 10 => { "medium" }
            _ => { "small" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "medium"

    def test_guard_with_equality(self, catnip):
        """Guard with equality check."""
        code = """
        x = 42
        match x {
            n if n == 42 => { "found" }
            _ => { "not found" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "found"


class TestGuardsWithTuplePatterns:
    """Test guards with tuple/structural patterns."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_guard_accesses_captured_vars(self, catnip):
        """Guard can access variables captured by pattern."""
        code = """
        point = tuple(3, 3)
        match point {
            (x, y) if x == y => { "diagonal" }
            (x, y) => { "off diagonal" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "diagonal"

    def test_guard_with_multiple_captures(self, catnip):
        """Guard using multiple captured variables."""
        code = """
        data = tuple(10, 20, 30)
        match data {
            (a, b, c) if a + b == c => { "sum match" }
            _ => { "no sum match" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "sum match"

    def test_guard_with_nested_pattern(self, catnip):
        """Guard with nested pattern captures."""
        code = """
        data = list(1, list(2, 3))
        match data {
            (a, (b, c)) if a + b == c => { "match" }
            _ => { "no match" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "match"

    def test_guard_with_star_pattern(self, catnip):
        """Guard with star pattern captures."""
        code = """
        nums = list(1, 2, 3, 4, 5)
        match nums {
            (first, *rest) if first < 5 => { len(rest) }
            _ => { 0 }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 4


class TestGuardExpressions:
    """Test various expressions in guards."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_guard_with_and(self, catnip):
        """Guard with AND condition."""
        code = """
        x = 15
        match x {
            n if n > 10 and n < 20 => { "in range" }
            _ => { "out of range" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "in range"

    def test_guard_with_or(self, catnip):
        """Guard with OR condition."""
        code = """
        x = 5
        match x {
            n if n == 5 or n == 10 => { "special" }
            _ => { "normal" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "special"

    def test_guard_with_not(self, catnip):
        """Guard with NOT condition."""
        code = """
        x = 7
        match x {
            n if not (n == 0) => { "nonzero" }
            _ => { "zero" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "nonzero"

    def test_guard_with_function_call(self, catnip):
        """Guard calling a function."""
        code = """
        is_positive = (n) => { n > 0 }
        x = 42
        match x {
            n if is_positive(n) => { "positive" }
            _ => { "non-positive" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "positive"

    def test_guard_with_builtin(self, catnip):
        """Guard using builtin function."""
        code = """
        s = "hello"
        match s {
            x if len(x) > 3 => { "long" }
            _ => { "short" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "long"

    def test_guard_with_modulo(self, catnip):
        """Guard with modulo operation."""
        code = """
        x = 6
        match x {
            n if n % 2 == 0 => { "even" }
            _ => { "odd" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "even"


class TestBasicORPatterns:
    """Test basic OR pattern functionality."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_simple_or_pattern(self, catnip):
        """OR pattern with two alternatives."""
        code = """
        x = 2
        match x {
            1 | 2 => { "one or two" }
            _ => { "other" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "one or two"

    def test_or_pattern_first_match(self, catnip):
        """OR pattern matches first alternative."""
        code = """
        x = 1
        match x {
            1 | 2 => { "matched" }
            _ => { "not matched" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "matched"

    def test_or_pattern_second_match(self, catnip):
        """OR pattern matches second alternative."""
        code = """
        x = 2
        match x {
            1 | 2 => { "matched" }
            _ => { "not matched" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "matched"

    def test_or_pattern_no_match(self, catnip):
        """OR pattern doesn't match."""
        code = """
        x = 3
        match x {
            1 | 2 => { "matched" }
            _ => { "not matched" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "not matched"

    def test_multiple_or_alternatives(self, catnip):
        """OR pattern with multiple alternatives."""
        code = """
        x = 3
        match x {
            1 | 2 | 3 | 4 => { "small" }
            _ => { "big" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "small"


class TestORPatternsWithTuples:
    """Test OR patterns with tuple patterns."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_or_with_tuple_patterns(self, catnip):
        """OR of tuple patterns."""
        code = """
        point = tuple(0, 0)
        match point {
            (0, 0) | (1, 1) => { "special" }
            _ => { "normal" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "special"

    def test_or_tuple_second_match(self, catnip):
        """OR of tuple patterns, second matches."""
        code = """
        point = tuple(1, 1)
        match point {
            (0, 0) | (1, 1) => { "special" }
            _ => { "normal" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "special"

    def test_or_mixed_with_literals(self, catnip):
        """OR pattern mixing literals and variables."""
        code = """
        x = 0
        match x {
            0 | 1 => { "binary" }
            n => { "other" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "binary"


class TestORPatternsWithGuards:
    """Test combination of OR patterns and guards."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_or_pattern_with_guard(self, catnip):
        """OR pattern combined with guard."""
        code = """
        x = 2
        match x {
            1 | 2 | 3 if x > 1 => { "big small" }
            _ => { "other" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "big small"

    def test_or_pattern_guard_fails(self, catnip):
        """OR pattern matches but guard fails."""
        code = """
        x = 1
        match x {
            1 | 2 | 3 if x > 2 => { "big" }
            1 | 2 | 3 => { "small" }
            _ => { "other" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "small"


class TestComplexPatternCombinations:
    """Test complex combinations of patterns, guards, and OR."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_string_or_patterns(self, catnip):
        """OR patterns with string literals."""
        code = """
        cmd = "quit"
        match cmd {
            "exit" | "quit" | "q" => { "goodbye" }
            "help" | "?" => { "showing help" }
            _ => { "unknown" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "goodbye"

    def test_nested_guards(self, catnip):
        """Complex guard with nested conditions."""
        code = """
        data = tuple(5, 10)
        match data {
            (x, y) if x > 0 and y > 0 and x < y => { "valid" }
            _ => { "invalid" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "valid"

    def test_guard_with_computed_value(self, catnip):
        """Guard comparing to computed value."""
        code = """
        threshold = 10
        x = 15
        match x {
            n if n > threshold => { "above" }
            _ => { "below" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "above"

    def test_fizzbuzz_pattern(self, catnip):
        """FizzBuzz using pattern matching with guards."""
        code = """
        check = (n) => {
            match n {
                x if x % 15 == 0 => { "FizzBuzz" }
                x if x % 3 == 0 => { "Fizz" }
                x if x % 5 == 0 => { "Buzz" }
                x => { x }
            }
        }
        list(check(3), check(5), check(15), check(7))
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == ["Fizz", "Buzz", "FizzBuzz", 7]


class TestGuardEdgeCases:
    """Test edge cases for guards."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_guard_with_none_check(self, catnip):
        """Guard checking for None."""
        code = """
        x = None
        match x {
            val if val == None => { "is none" }
            val => { "has value" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "is none"

    def test_guard_with_boolean(self, catnip):
        """Guard with boolean values."""
        code = """
        flag = True
        match flag {
            b if b => { "truthy" }
            _ => { "falsy" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "truthy"

    def test_guard_false_boolean(self, catnip):
        """Guard with False boolean."""
        code = """
        flag = False
        match flag {
            b if b => { "truthy" }
            _ => { "falsy" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "falsy"

    def test_wildcard_with_guard(self, catnip):
        """Wildcard pattern with guard (using captured value)."""
        code = """
        x = 10
        match x {
            _ if x > 5 => { "big" }
            _ => { "small" }
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "big"


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
