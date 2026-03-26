# FILE: tests/language/test_extensions.py
"""Tests for the compiled extension system."""

import sys
import types

import pytest


from catnip.extensions import ExtensionInfo, discover_extensions, load_extension, validate_extension

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_module(name, descriptor):
    """Create a fake module with __catnip_extension__."""
    m = types.ModuleType(name)
    m.__catnip_extension__ = descriptor
    return m


def _make_context():
    """Minimal context-like object with globals and _extensions."""
    ctx = types.SimpleNamespace()
    ctx.globals = {}
    ctx._extensions = {}
    return ctx


# ---------------------------------------------------------------------------
# validate_extension
# ---------------------------------------------------------------------------


class TestValidateExtension:
    def test_returns_none_for_plain_module(self):
        m = types.ModuleType('plain')
        assert validate_extension(m) is None

    def test_returns_dict_for_valid_extension(self):
        m = _make_module('ext', {'name': 'foo', 'version': '1.0'})
        d = validate_extension(m)
        assert d['name'] == 'foo'
        assert d['version'] == '1.0'

    def test_raises_if_not_dict(self):
        m = types.ModuleType('bad')
        m.__catnip_extension__ = "nope"
        with pytest.raises(ValueError, match="must be a dict"):
            validate_extension(m)

    def test_raises_if_missing_name(self):
        m = _make_module('bad', {'version': '1.0'})
        with pytest.raises(ValueError, match="missing required key 'name'"):
            validate_extension(m)

    def test_raises_if_missing_version(self):
        m = _make_module('bad', {'name': 'foo'})
        with pytest.raises(ValueError, match="missing required key 'version'"):
            validate_extension(m)

    def test_raises_if_name_not_str(self):
        m = _make_module('bad', {'name': 42, 'version': '1.0'})
        with pytest.raises(ValueError, match="must be str"):
            validate_extension(m)

    def test_raises_if_register_not_callable(self):
        m = _make_module('bad', {'name': 'foo', 'version': '1.0', 'register': 'nope'})
        with pytest.raises(ValueError, match="must be callable"):
            validate_extension(m)

    def test_raises_if_exports_not_dict(self):
        m = _make_module('bad', {'name': 'foo', 'version': '1.0', 'exports': [1, 2]})
        with pytest.raises(ValueError, match="must be a dict"):
            validate_extension(m)

    def test_accepts_optional_fields(self):
        m = _make_module(
            'full',
            {
                'name': 'full',
                'version': '2.0',
                'description': 'a thing',
                'register': lambda ctx: None,
                'exports': {'a': 1},
            },
        )
        d = validate_extension(m)
        assert d['description'] == 'a thing'


# ---------------------------------------------------------------------------
# load_extension
# ---------------------------------------------------------------------------


class TestLoadExtension:
    def test_injects_exports(self):
        ctx = _make_context()
        m = _make_module(
            'ext',
            {
                'name': 'test',
                'version': '0.1',
                'exports': {'double': lambda x: x * 2, 'PI': 3.14},
            },
        )
        info = load_extension(m, ctx)
        assert ctx.globals['double'](5) == 10
        assert ctx.globals['PI'] == 3.14
        assert isinstance(info, ExtensionInfo)
        assert info.name == 'test'

    def test_calls_register_hook(self):
        calls = []
        ctx = _make_context()
        m = _make_module(
            'ext',
            {
                'name': 'hooked',
                'version': '0.1',
                'register': lambda c: calls.append(c),
            },
        )
        load_extension(m, ctx)
        assert calls == [ctx]

    def test_register_before_exports(self):
        """register() is called before exports are injected."""
        order = []
        ctx = _make_context()

        def register_hook(c):
            # at register time, exports should not be in globals yet
            order.append(('register', 'x' in c.globals))

        m = _make_module(
            'ext',
            {
                'name': 'ordered',
                'version': '0.1',
                'register': register_hook,
                'exports': {'x': 1},
            },
        )
        load_extension(m, ctx)
        assert order == [('register', False)]
        assert ctx.globals['x'] == 1

    def test_tracked_in_context(self):
        ctx = _make_context()
        m = _make_module('ext', {'name': 'tracked', 'version': '0.1'})
        load_extension(m, ctx)
        assert 'tracked' in ctx._extensions
        assert ctx._extensions['tracked'].version == '0.1'

    def test_raises_for_non_extension(self):
        ctx = _make_context()
        m = types.ModuleType('plain')
        with pytest.raises(ValueError, match="not a Catnip extension"):
            load_extension(m, ctx)


# ---------------------------------------------------------------------------
# Loader integration (import() in Catnip)
# ---------------------------------------------------------------------------


class TestLoaderDetection:
    def test_import_detects_extension(self):
        """import() auto-detects __catnip_extension__ and injects exports."""
        mod_name = '_catnip_test_ext_loader'
        m = types.ModuleType(mod_name)
        m.__catnip_extension__ = {
            'name': 'loader-test',
            'version': '0.1.0',
            'exports': {'ext_fn': lambda x: x + 100},
        }
        sys.modules[mod_name] = m
        try:
            from catnip import Catnip

            c = Catnip()
            c.parse(f'import("{mod_name}"); ext_fn(1)')
            result = c.execute()
            assert result == 101
            assert 'loader-test' in c.context._extensions
        finally:
            sys.modules.pop(mod_name, None)


# ---------------------------------------------------------------------------
# Runtime introspection (catnip.extensions)
# ---------------------------------------------------------------------------


class TestRuntimeExtensions:
    def test_extensions_property(self):
        mod_name = '_catnip_test_ext_runtime'
        m = types.ModuleType(mod_name)
        m.__catnip_extension__ = {
            'name': 'rt-test',
            'version': '1.2.3',
            'description': 'runtime test',
        }
        sys.modules[mod_name] = m
        try:
            from catnip import Catnip

            c = Catnip()
            c.parse(f'import("{mod_name}"); catnip.extensions')
            result = c.execute()
            assert isinstance(result, list)
            assert len(result) == 1
            assert result[0] == {'name': 'rt-test', 'version': '1.2.3', 'description': 'runtime test'}
        finally:
            sys.modules.pop(mod_name, None)


# ---------------------------------------------------------------------------
# discover_extensions (entry points)
# ---------------------------------------------------------------------------


class TestDiscoverExtensions:
    def test_returns_dict(self):
        result = discover_extensions()
        assert isinstance(result, dict)
