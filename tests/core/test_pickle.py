# FILE: tests/core/test_pickle.py
"""
Tests for pickle serialization of Catnip AST and runtime objects.

Pickling is essential for:
- Disk caching of parsed AST
- Multiprocessing (sending code between workers)
- Debug/inspection serialization
"""

import pickle

import pytest
from catnip._rs import Op, Scope

from catnip import Catnip


class TestOpPickle:
    """Tests for Op (AST node) pickling."""

    def test_pickle_simple_op(self):
        """Pickle a simple Op node."""
        c = Catnip()
        ast = c.parse("1 + 2")

        # AST should be an Op
        if isinstance(ast, Op):
            data = pickle.dumps(ast)
            restored = pickle.loads(data)

            assert isinstance(restored, Op)
            assert restored.ident == ast.ident

    def test_pickle_op_preserves_fields(self):
        """Pickle preserves all Op fields."""
        c = Catnip()
        ast = c.parse("1 + 2")

        if isinstance(ast, Op):
            original_ident = ast.ident
            original_tail = ast.tail
            original_start = ast.start_byte
            original_end = ast.end_byte

            data = pickle.dumps(ast)
            restored = pickle.loads(data)

            assert restored.ident == original_ident
            assert restored.tail == original_tail
            assert restored.start_byte == original_start
            assert restored.end_byte == original_end

    def test_pickle_nested_op(self):
        """Pickle nested Op nodes."""
        c = Catnip()
        ast = c.parse("(1 + 2) * 3")

        if isinstance(ast, Op):
            data = pickle.dumps(ast)
            restored = pickle.loads(data)

            # Execute both and compare
            c1 = Catnip()
            c1.code = ast
            r1 = c1.execute()

            c2 = Catnip()
            c2.code = restored
            r2 = c2.execute()

            assert r1 == r2 == 9


class TestScopePickle:
    """Tests for Scope pickling."""

    def test_pickle_empty_scope(self):
        """Pickle an empty scope."""
        scope = Scope()

        data = pickle.dumps(scope)
        restored = pickle.loads(data)

        assert isinstance(restored, Scope)

    def test_pickle_scope_with_int(self):
        """Pickle a scope with integer values."""
        scope = Scope()
        scope._set('x', 42)
        scope._set('y', 100)

        data = pickle.dumps(scope)
        restored = pickle.loads(data)

        assert restored._resolve('x') == 42
        assert restored._resolve('y') == 100

    def test_pickle_scope_with_string(self):
        """Pickle a scope with string values."""
        scope = Scope()
        scope._set('name', "hello")

        data = pickle.dumps(scope)
        restored = pickle.loads(data)

        assert restored._resolve('name') == "hello"

    def test_pickle_scope_with_list(self):
        """Pickle a scope with list values."""
        scope = Scope()
        scope._set('items', [1, 2, 3])

        data = pickle.dumps(scope)
        restored = pickle.loads(data)

        assert restored._resolve('items') == [1, 2, 3]


class TestExecuteAfterPickle:
    """Test execution after pickle roundtrip."""

    def test_arithmetic_after_pickle(self):
        """Execute arithmetic after pickle."""
        c1 = Catnip()
        ast = c1.parse("2 * 3 + 4")

        data = pickle.dumps(ast)
        restored = pickle.loads(data)

        c2 = Catnip()
        c2.code = restored
        result = c2.execute()

        assert result == 10

    def test_comparison_after_pickle(self):
        """Execute comparison after pickle."""
        c1 = Catnip()
        ast = c1.parse("5 > 3")

        data = pickle.dumps(ast)
        restored = pickle.loads(data)

        c2 = Catnip()
        c2.code = restored
        result = c2.execute()

        assert result == True

    def test_boolean_logic_after_pickle(self):
        """Execute boolean logic after pickle."""
        c1 = Catnip()
        # Use True/False (Python builtins) instead of true/false (Catnip keywords)
        # because the restored AST runs in a fresh context
        ast = c1.parse("True and False or True")

        data = pickle.dumps(ast)
        restored = pickle.loads(data)

        c2 = Catnip()
        c2.code = restored
        result = c2.execute()

        assert result == True


class TestFunctionPickle:
    """Tests for Function/Lambda pickling."""

    def test_pickle_simple_lambda(self):
        """Pickle a simple lambda."""
        from catnip._rs import set_global_registry

        c = Catnip()
        c.parse("f = (x) => { x * 2 }")
        c.execute()

        # Set global registry for pickle reconstruction
        set_global_registry(c.registry)

        # Get f from globals
        f = c.context.globals.get('f')
        assert f is not None, "Lambda 'f' not found in globals"

        # Pickle and restore the lambda
        data = pickle.dumps(f)
        restored = pickle.loads(data)

        # Lambda should be a VMFunction
        assert restored is not None

    def test_execute_pickled_lambda(self):
        """Execute a pickled lambda."""
        from catnip._rs import set_global_registry

        c = Catnip()
        c.parse("double = (x) => { x * 2 }")
        c.execute()

        # Set global registry for pickle reconstruction
        set_global_registry(c.registry)

        # Get double from globals
        double = c.context.globals.get('double')
        assert double is not None, "Lambda 'double' not found in globals"

        # Pickle and restore the lambda
        data = pickle.dumps(double)
        restored = pickle.loads(data)

        # Call the restored lambda via Catnip
        c2 = Catnip()
        c2.context.globals['f'] = restored
        c2.parse("f(21)")
        result = c2.execute()

        assert result == 42


class TestPickleProtocols:
    """Test different pickle protocols."""

    @pytest.mark.parametrize("protocol", [2, 3, 4, 5])
    def test_scope_all_protocols(self, protocol):
        """Test Scope pickling with protocol versions 2+.

        Protocols 0 and 1 are too old and don't support __reduce_ex__ properly.
        """
        if protocol > pickle.HIGHEST_PROTOCOL:
            pytest.skip(f"Protocol {protocol} not supported")

        scope = Scope()
        scope._set('value', 42)

        data = pickle.dumps(scope, protocol=protocol)
        restored = pickle.loads(data)

        assert restored._resolve('value') == 42

    @pytest.mark.parametrize("protocol", [0, 1, 2, 3, 4, 5])
    def test_op_all_protocols(self, protocol):
        """Test Op pickling with all protocol versions."""
        if protocol > pickle.HIGHEST_PROTOCOL:
            pytest.skip(f"Protocol {protocol} not supported")

        c = Catnip()
        ast = c.parse("1 + 2")

        if isinstance(ast, Op):
            data = pickle.dumps(ast, protocol=protocol)
            restored = pickle.loads(data)

            assert isinstance(restored, Op)
            assert restored.ident == ast.ident


class TestPickleSize:
    """Test pickle serialization size."""

    def test_scope_pickle_size(self):
        """Scope pickle should be reasonably compact."""
        scope = Scope()
        for i in range(100):
            scope._set(f'var_{i}', i * 10)

        data = pickle.dumps(scope)

        # Should be less than 10KB for 100 variables
        assert len(data) < 10000

    def test_op_pickle_size(self):
        """Op pickle should be reasonably compact."""
        c = Catnip()
        ast = c.parse("1 + 2 + 3 + 4 + 5")

        if isinstance(ast, Op):
            data = pickle.dumps(ast)

            # Should be less than 1KB for simple expression
            assert len(data) < 1000
