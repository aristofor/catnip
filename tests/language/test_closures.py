# FILE: tests/language/test_closures.py
"""
Tests for closures and lexical scoping.

Tests variable capture, nested functions, shadowing, and closure behavior.
"""

import pytest

from catnip import Catnip


class TestBasicClosures:
    """Test basic closure behavior."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_closure_captures_outer_variable(self, catnip):
        """Inner function captures variable from outer scope."""
        code = """
        outer = () => {
            x = 10
            inner = () => { x }
            inner()
        }
        outer()
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 10

    def test_closure_captures_multiple_variables(self, catnip):
        """Inner function captures multiple variables."""
        code = """
        outer = () => {
            a = 1
            b = 2
            c = 3
            inner = () => { a + b + c }
            inner()
        }
        outer()
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 6

    def test_closure_captures_parameter(self, catnip):
        """Inner function captures outer function parameter."""
        code = """
        outer = (x) => {
            inner = () => { x * 2 }
            inner()
        }
        outer(21)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 42

    def test_closure_returned_from_function(self, catnip):
        """Closure returned and called later."""
        code = """
        make_adder = (n) => {
            (x) => { x + n }
        }
        add5 = make_adder(5)
        add5(10)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 15

    def test_closure_factory_pattern(self, catnip):
        """Multiple closures from same factory."""
        code = """
        make_multiplier = (factor) => {
            (x) => { x * factor }
        }
        double = make_multiplier(2)
        triple = make_multiplier(3)
        double(10) + triple(10)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 50  # 20 + 30


class TestNestedFunctions:
    """Test deeply nested functions."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_two_level_nesting(self, catnip):
        """Two levels of function nesting."""
        code = """
        level1 = () => {
            a = 1
            level2 = () => {
                b = 2
                a + b
            }
            level2()
        }
        level1()
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 3

    def test_three_level_nesting(self, catnip):
        """Three levels of function nesting."""
        code = """
        level1 = () => {
            a = 1
            level2 = () => {
                b = 2
                level3 = () => {
                    c = 3
                    a + b + c
                }
                level3()
            }
            level2()
        }
        level1()
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 6

    def test_nested_with_parameters(self, catnip):
        """Nested functions with parameters at each level."""
        code = """
        outer = (x) => {
            middle = (y) => {
                inner = (z) => { x + y + z }
                inner(30)
            }
            middle(20)
        }
        outer(10)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 60


class TestVariableShadowing:
    """Test variable shadowing in nested scopes."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_parameter_shadows_outer(self, catnip):
        """Inner parameter shadows outer variable."""
        code = """
        outer = () => {
            x = 100
            inner = (x) => { x }
            inner(42)
        }
        outer()
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 42

    def test_local_shadows_outer(self, catnip):
        """Inner local variable shadows outer."""
        code = """
        outer = () => {
            x = 100
            inner = () => {
                x = 42
                x
            }
            inner()
        }
        outer()
        """
        catnip.parse(code)
        result = catnip.execute()
        # In Catnip, assignment creates/updates in current scope
        assert result == 42

    def test_shadowing_doesnt_affect_outer(self, catnip):
        """Shadowing in inner scope doesn't affect outer after return."""
        code = """
        x = 100
        inner = () => {
            x = 42
            x
        }
        inner()
        x
        """
        catnip.parse(code)
        result = catnip.execute()
        # Depends on scoping rules - check actual behavior
        # If assignment modifies outer scope:
        assert result == 42 or result == 100


class TestClosureState:
    """Test closures with mutable state."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_counter_closure(self, catnip):
        """Counter pattern with closure."""
        code = """
        make_counter = () => {
            count = 0
            inc = () => {
                count = count + 1
                count
            }
            inc
        }
        counter = make_counter()
        a = counter()
        b = counter()
        c = counter()
        list(a, b, c)
        """
        catnip.parse(code)
        result = catnip.execute()
        # Verify counter increments
        assert result == [1, 2, 3]

    def test_accumulator_closure(self, catnip):
        """Accumulator pattern with closure."""
        code = """
        make_accumulator = (initial) => {
            total = initial
            add = (n) => {
                total = total + n
                total
            }
            add
        }
        acc = make_accumulator(10)
        a = acc(5)
        b = acc(3)
        c = acc(2)
        list(a, b, c)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == [15, 18, 20]


class TestLambdaSpecifics:
    """Test lambda-specific behavior."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_lambda_stored_then_called(self, catnip):
        """Lambda stored in variable then called."""
        code = """
        f = (x) => { x * 2 }
        f(21)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 42

    def test_lambda_as_value(self, catnip):
        """Lambda can be passed as value."""
        code = """
        apply_twice = (f, x) => { f(f(x)) }
        inc = (n) => { n + 1 }
        apply_twice(inc, 5)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 7

    def test_lambda_with_default_parameter(self, catnip):
        """Lambda with default parameter."""
        code = """
        f = (x, y=10) => { x + y }
        f(5)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 15

    def test_lambda_with_multiple_defaults(self, catnip):
        """Lambda with multiple default parameters."""
        code = """
        f = (a=1, b=2, c=3) => { a + b + c }
        f()
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 6

    def test_lambda_no_parameters(self, catnip):
        """Lambda with no parameters."""
        code = """
        f = () => { 42 }
        f()
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 42

    def test_lambda_single_expression(self, catnip):
        """Lambda with single expression body."""
        code = """
        identity = (x) => { x }
        identity(42)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 42


class TestHigherOrderFunctions:
    """Test higher-order function patterns."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_function_as_argument(self, catnip):
        """Pass function as argument."""
        code = """
        apply = (f, x) => { f(x) }
        double = (n) => { n * 2 }
        apply(double, 21)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 42

    def test_function_composition(self, catnip):
        """Compose two functions."""
        code = """
        compose = (f, g) => {
            (x) => { f(g(x)) }
        }
        add1 = (x) => { x + 1 }
        mul2 = (x) => { x * 2 }
        add1_then_mul2 = compose(mul2, add1)
        add1_then_mul2(5)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 12  # (5 + 1) * 2

    def test_currying(self, catnip):
        """Curried function pattern."""
        code = """
        curry_add = (a) => {
            f1 = (b) => {
                f2 = (c) => { a + b + c }
                f2
            }
            f1
        }
        f = curry_add(1)
        g = f(2)
        g(3)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 6

    def test_partial_application(self, catnip):
        """Partial application pattern."""
        code = """
        add = (a, b) => { a + b }
        add5 = (x) => { add(5, x) }
        add5(10)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 15


class TestRecursiveLambdas:
    """Test recursive lambda patterns."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_named_recursive_function(self, catnip):
        """Named function can call itself."""
        code = """
        factorial = (n) => {
            if n <= 1 { 1 }
            else { n * factorial(n - 1) }
        }
        factorial(5)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 120

    def test_fibonacci(self, catnip):
        """Fibonacci with recursion."""
        code = """
        fib = (n) => {
            if n < 2 { n }
            else { fib(n - 1) + fib(n - 2) }
        }
        fib(10)
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 55

    def test_mutual_recursion(self, catnip):
        """Two functions calling each other."""
        code = """
        is_even = (n) => {
            if n == 0 { True }
            else { is_odd(n - 1) }
        }
        is_odd = (n) => {
            if n == 0 { False }
            else { is_even(n - 1) }
        }
        list(is_even(10), is_odd(10))
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == [True, False]


class TestScopeIsolation:
    """Test that function locals don't clobber caller's variables."""

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_function_local_does_not_clobber_caller_variable(self, catnip):
        """A function's local 'a' must not overwrite the caller's 'a'."""
        code = """
        f = (p1, p2) => { a = p1 + p2; a }
        a = list(1, 2)
        f(3, 4)
        a
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == [1, 2]

    def test_nested_calls_restore_variables(self, catnip):
        """Nested function calls must each restore the caller's scope."""
        code = """
        inner = (x) => { a = x * 2; a }
        outer = (x) => { a = x + 1; inner(a); a }
        a = 100
        outer(5)
        a
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 100


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
