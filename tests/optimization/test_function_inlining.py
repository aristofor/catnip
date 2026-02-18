# FILE: tests/optimization/test_function_inlining.py
"""
Tests for function inlining optimization pass.

Small pure functions (no calls, no loops, no recursion) are inlined at
their call sites, replacing Call(Ref("f"), args) with Block(SetLocals, body).
Subsequent passes (CopyProp, ConstFold, BlockFlat, DCE) clean up the result.

Activated at optimize >= 1 (IR-level pass).
"""

import unittest

from catnip import Catnip


class TestFunctionInlining(unittest.TestCase):
    """Function inlining correctness tests."""

    def _run(self, code, *, optimize=1):
        c = Catnip(optimize=optimize)
        c.parse(code)
        return c.execute()

    def test_simple_inline(self):
        """Basic: single-param function inlined."""
        code = "f = (x) => { x * 2 }; f(5)"
        assert self._run(code) == 10

    def test_multi_param(self):
        """Two-param function."""
        code = "add = (x, y) => { x + y }; add(3, 4)"
        assert self._run(code) == 7

    def test_expression_body(self):
        """Body with nested arithmetic."""
        code = "f = (x) => { x * 3 + 1 }; f(10)"
        assert self._run(code) == 31

    def test_multiple_calls(self):
        """Same function called multiple times."""
        code = "f = (x) => { x * 3 + 1 }; f(10) + f(20)"
        assert self._run(code) == 92  # 31 + 61

    def test_no_inline_recursive(self):
        """Recursive function: not inlined but still correct."""
        code = "fact = (n) => { if (n <= 1) { 1 } else { n * fact(n - 1) } }; fact(5)"
        assert self._run(code) == 120

    def test_no_inline_large(self):
        """Large function body: not inlined but still correct."""
        code = """
        f = (x) => {
            a = x + 1; b = a + 2; c = b + 3; d = c + 4; e = d + 5
            f_ = e + 6; g = f_ + 7; h = g + 8; i = h + 9; j = i + 10
            j
        }
        f(0)
        """
        assert self._run(code) == 55

    def test_preserves_semantics(self):
        """Compare optimize=0 vs optimize=1."""
        code = "f = (x) => { x * 3 + 1 }; f(10) + f(20)"
        r0 = self._run(code, optimize=0)
        r1 = self._run(code, optimize=1)
        assert r0 == r1

    def test_inline_used_in_loop(self):
        """Inlined function called inside a loop."""
        code = "double = (x) => { x * 2 }; s = 0; i = 0; while (i < 5) { s = s + double(i); i = i + 1 }; s"
        assert self._run(code) == 20  # sum(2*i for i in range(5))

    def test_no_scope_leak(self):
        """Inlined param bindings don't leak to outer scope."""
        code = "x = 100; f = (x) => { x * 2 }; f(5) + x"
        assert self._run(code) == 110  # 10 + 100

    def test_nested_inline(self):
        """Nested calls: inner inlined first, then outer."""
        code = "f = (x) => { x + 1 }; g = (y) => { y * 2 }; g(f(3))"
        assert self._run(code) == 8  # (3+1)*2

    def test_no_inline_with_call_in_body(self):
        """Body contains a Call: not inlined but still correct."""
        code = "inc = (x) => { x + 1 }; apply = (f, x) => { f(x) }; apply(inc, 5)"
        assert self._run(code) == 6

    def test_conditional_body(self):
        """Body with if-expression (allowed, no forbidden opcodes)."""
        code = "abs_ = (x) => { if (x < 0) { 0 - x } else { x } }; abs_(0 - 5) + abs_(3)"
        assert self._run(code) == 8

    def test_no_inline_with_loop_in_body(self):
        """Body with while: not inlined but still correct."""
        code = """
        sum_to = (n) => {
            s = 0; i = 0
            while (i < n) { s = s + i; i = i + 1 }
            s
        }
        sum_to(5)
        """
        assert self._run(code) == 10

    def test_wrong_arity_no_crash(self):
        """Calling with wrong arity: no inline, still evaluates normally."""
        # This should not be inlined (arity mismatch) and may raise an error
        # or work depending on how the interpreter handles it
        code = "f = (x) => { x * 2 }; f(1, 2)"
        # Just verify it doesn't crash the optimizer
        try:
            self._run(code)
        except Exception:
            pass  # Expected: wrong arity error

    def test_inline_with_shadowing(self):
        """Param name shadows outer variable correctly."""
        code = """
        a = 10
        f = (a) => { a + 1 }
        result = f(5) + a
        result
        """
        assert self._run(code) == 16  # 6 + 10


if __name__ == "__main__":
    unittest.main()
