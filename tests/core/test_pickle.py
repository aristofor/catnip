# FILE: tests/core/test_pickle.py
"""
Tests for pickle serialization of Catnip runtime objects.

Pickling is essential for:
- Multiprocessing (sending code between workers)
- Debug/inspection serialization
"""

import pickle

import pytest
from catnip._rs import Scope

from catnip import Catnip


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
