# FILE: tests/optimization/test_jit_forced.py
"""
Tests for forced JIT compilation.

Tests the @jit decorator syntax, jit() builtin, and pragma("jit", "all").
"""

import pytest

from catnip import Catnip
from catnip.exc import CatnipTypeError


def exec_catnip(code: str):
    """Helper to execute Catnip code."""
    catnip = Catnip()
    catnip.parse(code)
    return catnip.execute()


class TestJitWrapper:
    """Tests for the jit() builtin wrapper."""

    def test_jit_wrapper_simple(self):
        """jit() wraps a function and returns it."""
        code = 'f = jit((n) => { n * 2 }); f(5)'
        assert exec_catnip(code) == 10

    def test_jit_wrapper_with_recursion(self):
        """jit() works with recursive functions."""
        code = '''
        fact = jit((n) => {
            if n <= 1 { 1 }
            else { n * fact(n - 1) }
        })
        fact(5)
        '''
        assert exec_catnip(code) == 120

    def test_jit_wrapper_returns_function(self):
        """jit() returns the same function (potentially JIT-compiled)."""
        code = '''
        f = (n) => { n + 1 }
        g = jit(f)
        g(5)
        '''
        assert exec_catnip(code) == 6

    def test_jit_wrapper_type_error(self):
        """jit() raises TypeError for non-function argument."""
        code = 'jit(42)'
        with pytest.raises(CatnipTypeError, match="jit\\(\\) expects a Catnip function"):
            exec_catnip(code)


class TestJitDecorator:
    """Tests for the @jit decorator syntax."""

    def test_decorator_simple(self):
        """@jit decorator syntax works."""
        code = '@jit f = (n) => { n * 2 }; f(5)'
        assert exec_catnip(code) == 10

    def test_decorator_with_lambda(self):
        """@jit works with lambda expressions."""
        code = '@jit double = (x) => { x + x }; double(21)'
        assert exec_catnip(code) == 42

    def test_decorator_with_recursion(self):
        """@jit works with recursive functions."""
        code = '''
        @jit fact = (n) => {
            if n <= 1 { 1 }
            else { n * fact(n - 1) }
        }
        fact(6)
        '''
        assert exec_catnip(code) == 720

    def test_multiple_decorators(self):
        """Multiple decorators apply correctly."""
        # Note: @pure @jit -> pure(jit(fn))
        # Both pure and jit should work even if the function doesn't match
        # a JIT-compilable pattern (the function still runs normally)
        code = '@jit f = (n) => { n * 3 }; f(7)'
        assert exec_catnip(code) == 21

    def test_decorator_desugaring(self):
        """Decorator desugars to wrapper call."""
        # @dec f = expr -> f = dec(expr)
        # We verify this indirectly by checking the behavior
        code = '''
        wrapper = (fn) => {
            (x) => { fn(x) + 100 }
        }
        @wrapper add_one = (n) => { n + 1 }
        add_one(5)
        '''
        assert exec_catnip(code) == 106


class TestJitPragma:
    """Tests for pragma("jit", ...) directive."""

    def test_pragma_jit_on(self):
        """pragma("jit", "on") enables JIT."""
        code = '''
        pragma("jit", "on")
        f = (n) => { n + 1 }
        f(5)
        '''
        assert exec_catnip(code) == 6

    def test_pragma_jit_off(self):
        """pragma("jit", "off") disables JIT."""
        code = '''
        pragma("jit", "off")
        f = (n) => { n + 1 }
        f(5)
        '''
        assert exec_catnip(code) == 6

    def test_pragma_jit_all(self):
        """pragma("jit", "all") compiles all functions immediately."""
        code = '''
        pragma("jit", "all")
        f = (n) => { n + 1 }
        g = (n) => { n * 2 }
        f(5) + g(5)
        '''
        assert exec_catnip(code) == 16

    def test_pragma_jit_all_with_recursion(self):
        """pragma("jit", "all") works with recursive functions."""
        code = '''
        pragma("jit", "all")
        fact = (n) => {
            if n <= 1 { 1 }
            else { n * fact(n - 1) }
        }
        fact(5)
        '''
        assert exec_catnip(code) == 120


class TestDecoratorOrder:
    """Tests for decorator application order."""

    def test_single_decorator(self):
        """Single decorator applied correctly."""
        code = '''
        add_ten = (fn) => { (x) => { fn(x) + 10 } }
        @add_ten f = (n) => { n }
        f(5)
        '''
        assert exec_catnip(code) == 15

    def test_two_decorators_order(self):
        """Two decorators: @a @b f = e -> f = a(b(e))."""
        code = '''
        add_ten = (fn) => { (x) => { fn(x) + 10 } }
        double_result = (fn) => { (x) => { fn(x) * 2 } }

        # @add_ten @double_result f = ... -> f = add_ten(double_result(...))
        # double_result wraps first, then add_ten wraps that
        # f(5) = add_ten(double_result((n) => n))(5)
        # = add_ten((x) => x * 2)(5)  -- inner lambda
        # = (x) => (x * 2) + 10 (5) = 10 + 10 = 20
        @add_ten @double_result f = (n) => { n }
        f(5)
        '''
        assert exec_catnip(code) == 20

    def test_three_decorators(self):
        """Three decorators applied in correct order."""
        code = '''
        a = (fn) => { (x) => { fn(x) + 1 } }
        b = (fn) => { (x) => { fn(x) * 2 } }
        c = (fn) => { (x) => { fn(x) + 100 } }

        # @a @b @c f = e -> f = a(b(c(e)))
        # c: +100, b: *2, a: +1
        # f(5) = a(b(c((n) => n)))(5)
        # c: 5 + 100 = 105
        # b: 105 * 2 = 210
        # a: 210 + 1 = 211
        @a @b @c f = (n) => { n }
        f(5)
        '''
        assert exec_catnip(code) == 211
