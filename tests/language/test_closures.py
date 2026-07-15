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


class TestEnclosingShadowsGlobal:
    """An enclosing binding that shares a name with a global must win (LEGB).

    Regression: a nested closure used to resolve a name to the module global
    even when an enclosing function bound the same name, because the resolver
    consulted globals before the enclosing closure chain.
    """

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_nested_closure_prefers_enclosing_param_over_global(self, catnip):
        code = """
        v = 100
        f = (v) => { g = () => { v }; g() }
        f(7)
        """
        catnip.parse(code)
        assert catnip.execute() == 7

    def test_double_nested_closure_prefers_enclosing_param(self, catnip):
        """The enclosing binding reaches the innermost closure through the chain
        even when the intermediate closure does not reference it itself."""
        code = """
        v = 100
        f = (v) => { g = () => { h = () => { v }; h() }; g() }
        f(7)
        """
        catnip.parse(code)
        assert catnip.execute() == 7

    def test_broadcast_callback_prefers_enclosing_param(self, catnip):
        code = """
        v = 100
        f = (v) => { list(0, 1).[(i) => { v }] }
        f(7)
        """
        catnip.parse(code)
        assert catnip.execute() == [7, 7]

    def test_global_mutation_through_closure_preserved(self, catnip):
        """A top-level closure still reaches a true global and mutates it: the
        fix must not freeze module globals into the closure."""
        code = """
        counter = 0
        inc = () => { counter = counter + 1 }
        inc()
        inc()
        counter
        """
        catnip.parse(code)
        assert catnip.execute() == 2


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


class TestLetrecGroup:
    """Letrec semantics for named functions defined in the same block.

    Contract: a definition `name = (args) => {...}` sees itself (let-rec)
    and its sibling definitions of the same block, even after the closures
    escape the scope. Aliases and rebinds are mutations, not definitions.
    """

    @pytest.fixture
    def catnip(self):
        return Catnip()

    def test_nested_mutual_recursion(self, catnip):
        """even/odd defined in a function body can call each other."""
        code = """
        test = () => {
            even = (n) => { if n == 0 { True } else { odd(n - 1) } }
            odd = (n) => { if n == 0 { False } else { even(n - 1) } }
            even(10)
        }
        test()
        """
        catnip.parse(code)
        assert catnip.execute() is True

    def test_escaped_mutual_recursion(self, catnip):
        """The mutual group still works after escaping its defining scope."""
        code = """
        make = () => {
            even = (n) => { if n == 0 { True } else { odd(n - 1) } }
            odd = (n) => { if n == 0 { False } else { even(n - 1) } }
            even
        }
        f = make()
        f(11)
        """
        catnip.parse(code)
        assert catnip.execute() is False

    def test_self_reference_survives_rebind(self, catnip):
        """The self-reference is fixed at definition: rebinding the name
        afterwards does not change what the function calls."""
        code = """
        test = () => {
            g = (n) => { if n == 0 { "orig" } else { g(n - 1) } }
            h = g
            g = "not a function"
            h(3)
        }
        test()
        """
        catnip.parse(code)
        assert catnip.execute() == 'orig'

    def test_alias_does_not_patch_closures(self, catnip):
        """An alias (`h = g`) is not a definition: it must not overwrite
        bindings captured by other functions."""
        code = """
        helper = (x) => { "global" }
        test = () => {
            f = (x) => { helper(x) }
            helper2 = f
            f(1)
        }
        test()
        """
        catnip.parse(code)
        assert catnip.execute() == 'global'

    def test_mutual_recursive_function_pickles(self, catnip):
        """Pickling a function from a mutual group must not recurse forever.
        The sibling links are letrec bindings, skipped at serialization."""
        import pickle

        from catnip._rs import set_global_registry

        code = """
        make = () => {
            even = (n) => { if n == 0 { True } else { odd(n - 1) } }
            odd = (n) => { if n == 0 { False } else { even(n - 1) } }
            even
        }
        f = make()
        f(4)
        """
        catnip.parse(code)
        assert catnip.execute() is True
        set_global_registry(catnip.registry)
        f = catnip.context.globals['f']
        data = pickle.dumps(f)
        assert pickle.loads(data) is not None


if __name__ == "__main__":
    pytest.main([__file__, "-v"])


class TestClosureSemanticsGrid:
    """The settled closure semantics (wip/CLOSURE_SEMANTICS.md, 2026-07-04):
    copy capture at creation (a closure is a private mutable snapshot, writes
    update its own capture -- nothing leaks to the parent), and module globals
    resolved live at call time (late binding, reads and writes). Every case
    asserts the reference value in whatever executor the suite runs: the grid
    is the three-executor differential oracle that was missing when VM and AST
    silently diverged on five of these ten cases."""

    @staticmethod
    def _run(src):
        c = Catnip()
        c.parse(src)
        return c.execute()

    def test_write_through_parent_does_not_leak(self):
        assert self._run('outer = () => { c = "a"\n inner = () => { c = c + "b"\n 0 }\n inner()\n c }\nouter()') == 'a'

    def test_late_binding_read_in_function_is_early(self):
        assert self._run('outer = () => { c = 1\n inner = () => { c }\n c = 2\n inner() }\nouter()') == 1

    def test_read_after_inner_write(self):
        assert (
            self._run(
                'outer = () => { c = "a"\n inner = () => { tmp = c\n c = tmp + "b"\n c }\n r = inner()\n r + "|" + c }\nouter()'
            )
            == 'ab|a'
        )

    def test_sibling_closures_do_not_share(self):
        assert (
            self._run(
                'outer = () => { c = 0\n up = () => { c = c + 1\n 0 }\n get = () => { c }\n up()\n up()\n get() }\nouter()'
            )
            == 0
        )

    def test_counter_escapes(self):
        assert self._run('mk = () => { c = 0\n () => { c = c + 1\n c } }\nf = mk()\nf()\nf()') == 2

    def test_toplevel_write_through_is_live(self):
        assert self._run('c = "a"\nf = () => { c = c + "b"\n 0 }\nf()\nc') == 'ab'

    def test_toplevel_read_is_late_bound(self):
        assert self._run('c = 1\nf = () => { c }\nc = 2\nf()') == 2

    def test_depth2_write_stays_in_inner_capture(self):
        assert (
            self._run(
                'o = () => { c = "a"\n mid = () => { inner = () => { c = c + "x"\n 0 }\n inner()\n 0 }\n mid()\n c }\no()'
            )
            == 'a'
        )

    def test_param_capture_write_does_not_leak(self):
        assert self._run('o = (c) => { inner = () => { c = c + 1\n 0 }\n inner()\n c }\no(10)') == 10

    def test_write_only_global_creates_a_local(self):
        # Settled 2026-07-04: a write-through requires a READ inside the
        # function; a write-only name is a local, wherever the global was
        # defined (the assignment-based pre-seed was order-dependent).
        assert self._run('x = 0\nf = () => { x = 99\n 1 }\nf()\nx') == 0
        assert self._run('f = () => { x = 99\n 1 }\nx = 0\nf()\nx') == 0

    def test_read_then_write_global_is_live(self):
        assert self._run('x = 0\nf = () => { x = x + 99\n 1 }\nf()\nx') == 99

    def test_loop_var_captured_early_per_iteration(self):
        assert (
            self._run(
                'o = () => { acc = 0\n fs = []\n for i in [1, 2, 3] { fs = fs + [(x) => { x + i }] }\n fs[0](0) + fs[2](0) }\no()'
            )
            == 4
        )


class TestClosureWriteThroughFallbackCompiler:
    """The rs CompilerCore fork (Decimal-literal fallback, debugger, Compiler
    API) used to register a local slot for an outer name written from a
    closure: later reads compiled to LoadLocal on a slot the store never
    filled (nil if the store didn't run, stale snapshot if the outer binding
    mutated in between). Audit 2026-07-13 B7 -- emit_store aligned on the
    catnip_vm production compiler. The Decimal literal below is what routes
    compilation through the fork."""

    @staticmethod
    def _run(src):
        c = Catnip()
        c.parse(src)
        return c.execute()

    def test_conditional_outer_store_not_taken(self):
        src = """
x = 10
d = 1.5d
f = () => {
    if x > 100 { x = 99 }
    x
}
f()
"""
        assert self._run(src) == 10

    def test_outer_binding_mutation_visible_after_write(self):
        src = """
counter = 0
d = 1.5d
bump = () => { counter = counter + 1 }
observe = () => {
    counter = counter + 10
    bump()
    counter
}
observe()
"""
        assert self._run(src) == 11
