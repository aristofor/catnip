# FILE: tests/language/test_runtime.py
"""
Tests for Catnip runtime introspection.

Tests the `catnip` builtin namespace for inspecting interpreter state.
"""

import pytest

from catnip import Catnip, __version__


class TestCatnipBuiltin:
    """Test catnip builtin namespace."""

    @pytest.fixture
    def cat(self):
        return Catnip()

    def test_catnip_exists(self, cat):
        """catnip builtin exists."""
        cat.parse("catnip")
        result = cat.execute()
        assert result is not None

    def test_catnip_version(self, cat):
        """catnip.version returns version string."""
        cat.parse("catnip.version")
        result = cat.execute()
        assert result == __version__

    def test_catnip_tco_default(self, cat):
        """catnip.tco is True by default."""
        cat.parse("catnip.tco")
        result = cat.execute()
        assert result is True

    def test_catnip_tco_after_change(self, cat):
        """catnip.tco reflects pragma changes."""
        cat.pragma_context.tco_enabled = False
        cat.parse("catnip.tco")
        result = cat.execute()
        assert result is False

    def test_catnip_optimize_default(self, cat):
        """catnip.optimize is 0 by default (faster compile-time)."""
        cat.parse("catnip.optimize")
        result = cat.execute()
        assert result == 0

    def test_catnip_optimize_after_change(self, cat):
        """catnip.optimize reflects pragma changes."""
        cat.pragma_context.optimize_level = 0
        cat.parse("catnip.optimize")
        result = cat.execute()
        assert result == 0

    def test_catnip_debug_default(self, cat):
        """catnip.debug is False by default."""
        cat.parse("catnip.debug")
        result = cat.execute()
        assert result is False

    def test_catnip_debug_after_change(self, cat):
        """catnip.debug reflects pragma changes."""
        cat.pragma_context.debug_mode = True
        cat.parse("catnip.debug")
        result = cat.execute()
        assert result is True

    def test_catnip_jit_default(self, cat):
        """catnip.jit is False by default (JIT has bugs with break/continue)."""
        cat.parse("catnip.jit")
        result = cat.execute()
        assert result is False

    def test_catnip_cache_default(self, cat):
        """catnip.cache is True by default."""
        cat.parse("catnip.cache")
        result = cat.execute()
        assert result is True

    def test_catnip_modules_default(self, cat):
        """catnip.modules is empty list by default."""
        cat.parse("catnip.modules")
        result = cat.execute()
        assert result == []


class TestCatnipIntrospectionInCode:
    """Test using introspection in Catnip code."""

    @pytest.fixture
    def cat(self):
        return Catnip()

    def test_conditional_on_tco(self, cat):
        """Code can branch based on catnip.tco."""
        code = """
        if catnip.tco {
            "tco enabled"
        } else {
            "tco disabled"
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == "tco enabled"

    def test_conditional_on_optimize(self, cat):
        """Code can branch based on catnip.optimize."""
        code = """
        if catnip.optimize > 0 {
            "optimizing"
        } else {
            "no optimization"
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == "no optimization"  # Default is optimize=0

    def test_store_version(self, cat):
        """Version can be stored in variable."""
        code = """
        v = catnip.version
        v
        """
        cat.parse(code)
        result = cat.execute()
        assert result == __version__


class TestRuntimeModuleTracking:
    """Test module tracking in runtime."""

    def test_add_module(self):
        """Runtime tracks added modules."""
        cat = Catnip()
        cat.runtime._add_module("math")
        cat.runtime._add_module("numpy")

        cat.parse("catnip.modules")
        result = cat.execute()
        assert result == ["math", "numpy"]

    def test_add_module_no_duplicates(self):
        """Adding same module twice doesn't duplicate."""
        cat = Catnip()
        cat.runtime._add_module("math")
        cat.runtime._add_module("math")

        cat.parse("catnip.modules")
        result = cat.execute()
        assert result == ["math"]


class TestRuntimeRepr:
    """Test runtime repr."""

    def test_repr_simple(self):
        """Repr is simple and clean."""
        cat = Catnip()
        cat.parse("str(catnip)")
        result = cat.execute()
        assert result == "<CatnipRuntime>"


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
