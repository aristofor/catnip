# FILE: tests/language/test_control_flow.py
import pytest

from catnip import Catnip
from catnip.nodes import BreakLoop, ContinueLoop


class TestBreak:
    """Test break statement in loops"""

    def test_break_in_while(self):
        """Break should exit while loop"""
        cat = Catnip()
        code = """
        i = 0
        result = 0
        while i < 10 {
            if i == 5 {
                break
            }
            result = result + i
            i = i + 1
        }
        result
        """
        cat.parse(code)
        result = cat.execute()
        # Sum 0+1+2+3+4 = 10
        assert result == 10

    def test_break_in_for(self):
        """Break should exit for loop"""
        cat = Catnip()
        code = """
        result = 0
        for i in list(1, 2, 3, 4, 5, 6, 7, 8, 9, 10) {
            if i == 6 {
                break
            }
            result = result + i
        }
        result
        """
        cat.parse(code)
        result = cat.execute()
        # Sum 1+2+3+4+5 = 15
        assert result == 15

    def test_break_with_nested_if(self):
        """Break in nested if should exit loop"""
        cat = Catnip()
        code = """
        found = False
        for i in list(1, 2, 3, 4, 5) {
            if i > 2 {
                if i == 4 {
                    found = True
                    break
                }
            }
        }
        found
        """
        cat.parse(code)
        result = cat.execute()
        assert result is True

    def test_break_in_nested_loops(self):
        """Break should only exit innermost loop"""
        cat = Catnip()
        code = """
        outer_count = 0
        inner_count = 0
        for i in list(1, 2, 3) {
            outer_count = outer_count + 1
            for j in list(1, 2, 3, 4, 5) {
                inner_count = inner_count + 1
                if j == 2 {
                    break
                }
            }
        }
        list(outer_count, inner_count)
        """
        cat.parse(code)
        result = cat.execute()
        # Outer loop runs 3 times
        # Inner loop runs 2 times per outer iteration (breaks at j==2)
        # Total: outer=3, inner=6
        assert result == [3, 6]


class TestContinue:
    """Test continue statement in loops"""

    def test_continue_in_while(self):
        """Continue should skip to next iteration in while loop"""
        cat = Catnip()
        code = """
        i = 0
        result = 0
        while i < 10 {
            i = i + 1
            if i == 5 {
                continue
            }
            result = result + i
        }
        result
        """
        cat.parse(code)
        result = cat.execute()
        # Sum 1+2+3+4+6+7+8+9+10 = 50 (skips 5)
        assert result == 50

    def test_continue_in_for(self):
        """Continue should skip to next iteration in for loop"""
        cat = Catnip()
        code = """
        result = 0
        for i in list(1, 2, 3, 4, 5, 6, 7, 8, 9, 10) {
            if i == 5 {
                continue
            }
            result = result + i
        }
        result
        """
        cat.parse(code)
        result = cat.execute()
        # Sum all except 5: 1+2+3+4+6+7+8+9+10 = 50
        assert result == 50

    def test_continue_with_multiple_conditions(self):
        """Continue with multiple skip conditions"""
        cat = Catnip()
        code = """
        result = 0
        for i in list(1, 2, 3, 4, 5, 6, 7, 8, 9, 10) {
            if i == 3 or i == 7 {
                continue
            }
            result = result + i
        }
        result
        """
        cat.parse(code)
        result = cat.execute()
        # Sum all except 3 and 7: 1+2+4+5+6+8+9+10 = 45
        assert result == 45

    def test_continue_in_nested_loops(self):
        """Continue should only affect innermost loop"""
        cat = Catnip()
        code = """
        result = 0
        for i in list(1, 2, 3) {
            for j in list(1, 2, 3) {
                if j == 2 {
                    continue
                }
                result = result + 1
            }
        }
        result
        """
        cat.parse(code)
        result = cat.execute()
        # Each inner loop runs 2 times (skips j==2), outer runs 3 times
        # Total: 3 * 2 = 6
        assert result == 6


class TestBreakContinueCombined:
    """Test break and continue used together"""

    def test_break_and_continue_in_same_loop(self):
        """Break and continue can be used in same loop with different conditions"""
        cat = Catnip()
        code = """
        result = 0
        for i in list(1, 2, 3, 4, 5, 6, 7, 8, 9, 10) {
            if i == 3 {
                continue
            }
            if i == 8 {
                break
            }
            result = result + i
        }
        result
        """
        cat.parse(code)
        result = cat.execute()
        # Sum 1+2+4+5+6+7 = 25 (skips 3, breaks at 8)
        assert result == 25

    def test_complex_nested_control_flow(self):
        """Complex nested control flow with break and continue"""
        cat = Catnip()
        code = """
        result = list()
        for i in list(1, 2, 3, 4, 5) {
            if i == 2 {
                continue
            }
            for j in list(10, 20, 30) {
                if j == 20 {
                    continue
                }
                if i == 4 and j == 30 {
                    break
                }
                result = result + list(list(i, j))
            }
        }
        result
        """
        cat.parse(code)
        result = cat.execute()
        # Expected: [[1,10], [1,30], [3,10], [3,30], [4,10], [5,10], [5,30]]
        # i=2 skipped, j=20 always skipped, breaks when i=4 and j=30
        expected = [[1, 10], [1, 30], [3, 10], [3, 30], [4, 10], [5, 10], [5, 30]]
        assert result == expected


class TestLoopEdgeCases:
    """Test edge cases for break and continue"""

    def test_break_outside_loop_raises_error(self, vm_mode):
        """Break outside a loop should raise BreakLoop exception (or SyntaxError in VM)"""
        cat = Catnip(vm_mode=vm_mode)
        code = "break"
        cat.parse(code)
        # VM catches at compile-time (SyntaxError), AST at runtime (BreakLoop)
        with pytest.raises((BreakLoop, SyntaxError)):
            cat.execute()

    def test_continue_outside_loop_raises_error(self, vm_mode):
        """Continue outside a loop should raise ContinueLoop exception (or SyntaxError in VM)"""
        cat = Catnip(vm_mode=vm_mode)
        code = "continue"
        cat.parse(code)
        # VM catches at compile-time (SyntaxError), AST at runtime (ContinueLoop)
        with pytest.raises((ContinueLoop, SyntaxError)):
            cat.execute()

    def test_break_in_function_inside_loop(self, vm_mode):
        """Break in function definition should not affect loop.

        Note: VM mode detects 'break' outside loop at compile-time (SyntaxError),
        while AST mode allows defining the function but raises BreakLoop when called.
        """
        cat = Catnip(vm_mode=vm_mode)
        code = """
        # Define a function that tries to break
        breaker = () => { break }

        # Use it in a loop - calling it will raise BreakLoop
        result = 0
        for i in list(1, 2, 3) {
            result = result + i
        }
        result
        """
        cat.parse(code)

        if vm_mode == "on":
            # VM catches break outside loop at compile-time
            with pytest.raises(SyntaxError):
                cat.execute()
        else:
            result = cat.execute()
            # Should sum all: 1+2+3 = 6
            assert result == 6

    def test_empty_while_with_break(self):
        """Infinite loop broken immediately"""
        cat = Catnip()
        code = """
        count = 0
        while True {
            count = count + 1
            if count == 1 {
                break
            }
        }
        count
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 1

    def test_for_with_immediate_continue(self):
        """For loop where every iteration continues"""
        cat = Catnip()
        code = """
        result = 0
        for i in list(1, 2, 3) {
            continue
            result = result + i  # Never executed
        }
        result
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 0


class TestBreakContinueWithReturn:
    """Test interaction between break/continue and return"""

    def test_return_takes_precedence_over_break(self):
        """Return should exit function even if in loop"""
        cat = Catnip()
        code = """
        f = () => {
            for i in list(1, 2, 3) {
                if i == 2 {
                    return 42
                }
            }
            return 99
        }
        f()
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 42

    def test_return_takes_precedence_over_continue(self):
        """Return should exit function even with continue"""
        cat = Catnip()
        code = """
        f = () => {
            for i in list(1, 2, 3) {
                if i == 1 {
                    continue
                }
                return i
            }
            return 99
        }
        f()
        """
        cat.parse(code)
        result = cat.execute()
        assert result == 2
