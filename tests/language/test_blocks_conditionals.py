# FILE: tests/language/test_blocks_conditionals.py
import pytest

from catnip import Catnip


class TestBlocks:
    """Test block execution"""

    def test_empty_block(self):
        """Empty block should return None"""
        cat = Catnip()
        cat.parse("{ }")
        result = cat.execute()
        assert result is None

    def test_single_value_block(self):
        """Block with single value should return that value"""
        cat = Catnip()
        cat.parse("{ 42 }")
        result = cat.execute()
        assert result == 42

    def test_multiple_statements_block(self):
        """Block should return the last statement value"""
        cat = Catnip()
        cat.parse("{ 1; 2; 3 }")
        result = cat.execute()
        assert result == 3

    def test_block_with_assignment(self):
        """Block with assignments should return last value"""
        cat = Catnip()
        cat.parse("{ a = 10; b = 20; a + b }")
        result = cat.execute()
        assert result == 30

    def test_block_scope_hybrid(self):
        """Block should modify parent scope variables (hybrid scope)"""
        cat = Catnip()
        cat.parse("a = 10; b = { a = 20; a + 5 }; a")
        result = cat.execute()
        assert result == 20  # a was modified in block

    def test_nested_blocks(self):
        """Nested blocks should work correctly"""
        cat = Catnip()
        cat.parse("{ { 1 } + { 2 } }")
        result = cat.execute()
        assert result == 3

    def test_block_as_expression(self):
        """Blocks can be used as expressions"""
        cat = Catnip()
        cat.parse("x = { 10 } + { 20 }; x")
        result = cat.execute()
        assert result == 30


class TestConditionals:
    """Test conditional statements (if/elif/else)"""

    def test_if_true(self):
        """If with true condition should execute block"""
        cat = Catnip()
        cat.parse("if True { 100 }")
        result = cat.execute()
        assert result == 100

    def test_if_false(self):
        """If with false condition should return None"""
        cat = Catnip()
        cat.parse("if False { 100 }")
        result = cat.execute()
        assert result is None

    def test_if_else_true(self):
        """If/else with true condition should execute if block"""
        cat = Catnip()
        cat.parse("if True { 1 } else { 2 }")
        result = cat.execute()
        assert result == 1

    def test_if_else_false(self):
        """If/else with false condition should execute else block"""
        cat = Catnip()
        cat.parse("if False { 1 } else { 2 }")
        result = cat.execute()
        assert result == 2

    def test_if_elif_else_first_true(self):
        """If/elif/else should execute first true branch"""
        cat = Catnip()
        cat.parse("x = 1; if x < 3 { 100 } elif x < 7 { 200 } else { 300 }")
        result = cat.execute()
        assert result == 100

    def test_if_elif_else_second_true(self):
        """If/elif/else should execute elif when if is false"""
        cat = Catnip()
        cat.parse("x = 5; if x < 3 { 100 } elif x < 7 { 200 } else { 300 }")
        result = cat.execute()
        assert result == 200

    def test_if_elif_else_all_false(self):
        """If/elif/else should execute else when all are false"""
        cat = Catnip()
        cat.parse("x = 10; if x < 3 { 100 } elif x < 7 { 200 } else { 300 }")
        result = cat.execute()
        assert result == 300

    def test_multiple_elif(self):
        """Multiple elif clauses should work"""
        cat = Catnip()
        cat.parse("x = 5; if x == 1 { 1 } elif x == 3 { 3 } elif x == 5 { 5 } elif x == 7 { 7 } else { 0 }")
        result = cat.execute()
        assert result == 5

    def test_if_with_comparison(self):
        """If with comparison expression"""
        cat = Catnip()
        cat.parse("a = 10; b = 20; if a < b { 1 } else { 2 }")
        result = cat.execute()
        assert result == 1

    def test_if_with_boolean_operators(self):
        """If with boolean operators"""
        cat = Catnip()
        cat.parse("a = 10; b = 20; if a < 15 and b > 15 { 1 } else { 2 }")
        result = cat.execute()
        assert result == 1

    def test_nested_if(self):
        """Nested if statements"""
        cat = Catnip()
        cat.parse("x = 5; if x > 0 { if x < 10 { 1 } else { 2 } } else { 3 }")
        result = cat.execute()
        assert result == 1

    def test_if_without_else_false(self):
        """If without else, when false, returns None"""
        cat = Catnip()
        cat.parse("a = 10; if a > 20 { 100 }")
        result = cat.execute()
        assert result is None
