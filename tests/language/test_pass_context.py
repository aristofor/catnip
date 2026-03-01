# FILE: tests/language/test_pass_context.py
"""Tests for @pass_context decorator - context injection into Python functions."""

import pytest

from catnip import Catnip, Context, pass_context


class TestPassContext:
    """Test that @pass_context injects the execution context."""

    def test_pass_context_receives_context(self, cat):
        """Function decorated with @pass_context receives Context as first arg."""
        received = {}

        @pass_context
        def capture_context(ctx, value):
            received['ctx'] = ctx
            received['value'] = value
            return value * 2

        cat.context.globals['capture'] = capture_context
        cat.parse('capture(21)')
        result = cat.execute()

        assert result == 42
        assert received['value'] == 21
        assert isinstance(received['ctx'], Context)

    def test_pass_context_access_globals(self, cat):
        """Context passed to function has access to ctx.globals."""

        @pass_context
        def check_global(ctx, name):
            return name in ctx.globals

        cat.context.globals['has_global'] = check_global
        cat.context.globals['my_var'] = 42
        cat.parse('has_global("my_var")')
        result = cat.execute()

        assert result is True

    def test_pass_context_read_global_value(self, cat):
        """Context passed to function can read global values."""

        @pass_context
        def get_global(ctx, name):
            return ctx.globals.get(name)

        cat.context.globals['get_global'] = get_global
        cat.context.globals['secret'] = 'hello'
        cat.parse('get_global("secret")')
        result = cat.execute()

        assert result == 'hello'

    def test_pass_context_with_kwargs(self, cat):
        """@pass_context works with keyword arguments."""

        @pass_context
        def with_kwargs(ctx, a, b=10):
            return a + b

        cat.context.globals['f'] = with_kwargs
        cat.parse('f(5, b=20)')
        result = cat.execute()

        assert result == 25

    def test_pass_context_multiple_args(self, cat):
        """@pass_context works with multiple positional arguments."""

        @pass_context
        def sum_args(ctx, *args):
            return sum(args)

        cat.context.globals['sum_all'] = sum_args
        cat.parse('sum_all(1, 2, 3, 4, 5)')
        result = cat.execute()

        assert result == 15

    def test_regular_function_no_context(self, cat):
        """Function without @pass_context does NOT receive context."""
        received = []

        def regular_func(*args):
            received.extend(args)
            return sum(args)

        cat.context.globals['regular'] = regular_func
        cat.parse('regular(1, 2, 3)')
        result = cat.execute()

        assert result == 6
        assert received == [1, 2, 3]
        assert not any(isinstance(a, Context) for a in received)


class TestPassContextModes:
    """Test @pass_context works in both VM and AST modes."""

    def test_vm_mode(self):
        """@pass_context works in VM mode."""

        @pass_context
        def get_ctx_type(ctx):
            return type(ctx).__name__

        cat = Catnip(vm_mode='on')
        cat.context.globals['ctx_type'] = get_ctx_type
        cat.parse('ctx_type()')
        result = cat.execute()

        assert result == 'Context'

    def test_vm_mode_tail_call(self):
        """@pass_context works in VM mode when call is in tail position."""

        @pass_context
        def get_ctx_type(ctx):
            return type(ctx).__name__

        cat = Catnip(vm_mode='on')
        cat.context.globals['ctx_type'] = get_ctx_type
        cat.parse('''f = () => { ctx_type() }
f()''')
        result = cat.execute()

        assert result == 'Context'

    def test_ast_mode(self):
        """@pass_context works in AST mode."""

        @pass_context
        def get_ctx_type(ctx):
            return type(ctx).__name__

        cat = Catnip(vm_mode='off')
        cat.context.globals['ctx_type'] = get_ctx_type
        cat.parse('ctx_type()')
        result = cat.execute()

        assert result == 'Context'


class TestPassContextErrorHandling:
    """Test error handling for @pass_context edge cases."""

    def test_decorator_sets_attribute(self):
        """@pass_context sets the pass_context attribute to True."""

        @pass_context
        def my_func(ctx):
            pass

        assert hasattr(my_func, 'pass_context')
        assert my_func.pass_context is True

    def test_decorator_preserves_function(self):
        """@pass_context returns the same function object."""

        def original(ctx):
            pass

        decorated = pass_context(original)
        assert decorated is original

    def test_function_without_decorator_no_attribute(self):
        """Regular function does not have pass_context attribute."""

        def regular(x):
            return x

        assert not hasattr(regular, 'pass_context')
