# FILE: tests/language/test_pragma.py
"""
Tests for the pragma system.

Tests pragma directives, context management, and state push/pop.
"""

import pytest

from catnip.exc import CatnipInternalError, CatnipPragmaError
from catnip.pragma import Pragma, PragmaContext, PragmaType


class TestPragmaType:
    """Test PragmaType enum."""

    def test_all_pragma_types_exist(self):
        """Verify all documented pragma types are defined."""
        expected = [
            "OPTIMIZE",
            "WARNING",
            "INLINE",
            "PURE",
            "CACHE",
            "DEBUG",
            "TCO",
            "JIT",
            "UNKNOWN",
        ]
        for name in expected:
            assert hasattr(PragmaType, name)

    def test_pragma_types_are_unique(self):
        """Each pragma type has a unique value."""
        values = [t.value for t in PragmaType.all_variants()]
        assert len(values) == len(set(values))


class TestPragma:
    """Test Pragma dataclass."""

    def test_pragma_creation(self):
        """Create a simple pragma."""
        p = Pragma(
            type=PragmaType.OPTIMIZE,
            directive="optimize",
            value=3,
            options={},
        )
        assert p.type == PragmaType.OPTIMIZE
        assert p.directive == "optimize"
        assert p.value == 3
        assert p.options == {}
        assert p.line is None

    def test_pragma_with_options(self):
        """Pragma with options dict."""
        p = Pragma(
            type=PragmaType.WARNING,
            directive="warning",
            value="off",
            options={"name": "unused"},
            line=42,
        )
        assert p.options["name"] == "unused"
        assert p.line == 42

    def test_pragma_repr(self):
        """Pragma has readable repr."""
        p = Pragma(
            type=PragmaType.TCO,
            directive="tco",
            value="on",
            options={},
        )
        assert "tco" in repr(p)
        assert "on" in repr(p)


class TestPragmaContext:
    """Test PragmaContext state management."""

    def test_default_values(self):
        """Context has sensible defaults."""
        ctx = PragmaContext()
        assert ctx.optimize_level == 0  # Changed to 0 for faster compile-time
        assert ctx.cache_enabled is True
        assert ctx.debug_mode is False
        assert ctx.tco_enabled is True
        assert ctx.jit_enabled is False  # JIT off by default
        assert ctx.pragmas == []
        assert ctx.warnings == {}
        assert ctx.inline_hints == {}
        assert len(ctx.pure_functions) == 0

    def test_add_pragma(self):
        """Adding pragma stores it and applies effects."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.OPTIMIZE,
            directive="optimize",
            value=1,
            options={},
        )
        ctx.add(p)
        assert len(ctx.pragmas) == 1
        assert ctx.optimize_level == 1


class TestOptimizePragma:
    """Test optimization level pragmas."""

    @pytest.mark.parametrize("level", [0, 1, 2, 3])
    def test_numeric_levels(self, level):
        """Set optimization via integer."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.OPTIMIZE,
            directive="optimize",
            value=level,
            options={},
        )
        ctx.add(p)
        assert ctx.optimize_level == level

    @pytest.mark.parametrize(
        "name,expected",
        [
            ("none", 0),
            ("off", 0),
            ("basic", 1),
            ("low", 1),
            ("medium", 2),
            ("default", 2),
            ("high", 3),
            ("full", 3),
            ("aggressive", 3),
        ],
    )
    def test_string_levels(self, name, expected):
        """Set optimization via string name."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.OPTIMIZE,
            directive="optimize",
            value=name,
            options={},
        )
        ctx.add(p)
        assert ctx.optimize_level == expected

    def test_case_insensitive(self):
        """String levels are case-insensitive."""
        ctx = PragmaContext()
        for variant in ["FULL", "Full", "fUlL"]:
            p = Pragma(
                type=PragmaType.OPTIMIZE,
                directive="optimize",
                value=variant,
                options={},
            )
            ctx.add(p)
            assert ctx.optimize_level == 3

    def test_invalid_numeric_level(self):
        """Reject optimization level outside 0-3."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.OPTIMIZE,
            directive="optimize",
            value=5,
            options={},
        )
        with pytest.raises(CatnipPragmaError, match="must be 0-3"):
            ctx.add(p)

    def test_invalid_string_level(self):
        """Reject unknown optimization level name."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.OPTIMIZE,
            directive="optimize",
            value="turbo",
            options={},
        )
        with pytest.raises(CatnipPragmaError, match="Unknown optimization level"):
            ctx.add(p)


class TestWarningPragma:
    """Test warning control pragmas."""

    @pytest.mark.parametrize("action", ["on", "yes"])
    def test_enable_warning(self, action):
        """Enable a warning."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.WARNING,
            directive="warning",
            value=action,
            options={"name": "unused"},
        )
        ctx.add(p)
        assert ctx.warnings["unused"] is True

    @pytest.mark.parametrize("action", ["off", "no"])
    def test_disable_warning(self, action):
        """Disable a warning."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.WARNING,
            directive="warning",
            value=action,
            options={"name": "deprecated"},
        )
        ctx.add(p)
        assert ctx.warnings["deprecated"] is False

    def test_warning_default_name(self):
        """Without name option, uses 'all'."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.WARNING,
            directive="warning",
            value="off",
            options={},
        )
        ctx.add(p)
        assert ctx.warnings["all"] is False

    def test_invalid_warning_action(self):
        """Reject unknown warning action."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.WARNING,
            directive="warning",
            value="maybe",
            options={},
        )
        with pytest.raises(CatnipPragmaError, match="Unknown warning action"):
            ctx.add(p)

    def test_is_warning_enabled_default(self):
        """is_warning_enabled returns True by default."""
        ctx = PragmaContext()
        assert ctx.is_warning_enabled("anything") is True

    def test_is_warning_enabled_specific(self):
        """is_warning_enabled respects specific warning setting."""
        ctx = PragmaContext()
        ctx.warnings["unused"] = False
        assert ctx.is_warning_enabled("unused") is False
        assert ctx.is_warning_enabled("other") is True

    def test_is_warning_enabled_all_fallback(self):
        """is_warning_enabled uses 'all' as fallback."""
        ctx = PragmaContext()
        ctx.warnings["all"] = False
        assert ctx.is_warning_enabled("anything") is False


class TestInlinePragma:
    """Test inline hint pragmas."""

    @pytest.mark.parametrize("hint", ["always", "never", "auto"])
    def test_valid_hints(self, hint):
        """Accept valid inline hints."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.INLINE,
            directive="inline",
            value=hint,
            options={"function": "my_func"},
        )
        ctx.add(p)
        assert ctx.inline_hints["my_func"] == hint

    def test_default_function_name(self):
        """Without function option, uses __next__."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.INLINE,
            directive="inline",
            value="always",
            options={},
        )
        ctx.add(p)
        assert ctx.inline_hints["__next__"] == "always"

    def test_invalid_hint(self):
        """Reject unknown inline hint."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.INLINE,
            directive="inline",
            value="sometimes",
            options={},
        )
        with pytest.raises(CatnipPragmaError, match="Unknown inline hint"):
            ctx.add(p)

    def test_should_inline_default(self):
        """should_inline returns 'auto' by default."""
        ctx = PragmaContext()
        assert ctx.should_inline("unknown_func") == "auto"

    def test_should_inline_specific(self):
        """should_inline returns specific hint."""
        ctx = PragmaContext()
        ctx.inline_hints["my_func"] = "always"
        assert ctx.should_inline("my_func") == "always"


class TestPurePragma:
    """Test pure function marking."""

    def test_mark_pure(self):
        """Mark function as pure."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.PURE,
            directive="pure",
            value="my_func",
            options={},
        )
        ctx.add(p)
        assert ctx.is_pure("my_func") is True
        assert ctx.is_pure("other") is False

    def test_unmark_pure(self):
        """Unmark function as pure."""
        ctx = PragmaContext()
        ctx.pure_functions.add("my_func")

        p = Pragma(
            type=PragmaType.PURE,
            directive="pure",
            value="my_func",
            options={"enable": False},
        )
        ctx.add(p)
        assert ctx.is_pure("my_func") is False


class TestCachePragma:
    """Test cache control pragmas."""

    @pytest.mark.parametrize("value", [True, "on", "yes", "true"])
    def test_enable_cache(self, value):
        """Enable cache via various formats."""
        ctx = PragmaContext()
        ctx.cache_enabled = False  # Start disabled
        p = Pragma(
            type=PragmaType.CACHE,
            directive="cache",
            value=value,
            options={},
        )
        ctx.add(p)
        assert ctx.cache_enabled is True

    @pytest.mark.parametrize("value", [False, "off", "no", "false"])
    def test_disable_cache(self, value):
        """Disable cache via various formats."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.CACHE,
            directive="cache",
            value=value,
            options={},
        )
        ctx.add(p)
        assert ctx.cache_enabled is False


class TestDebugPragma:
    """Test debug mode pragmas."""

    @pytest.mark.parametrize("value", [True, "on", "yes", "true"])
    def test_enable_debug(self, value):
        """Enable debug mode."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.DEBUG,
            directive="debug",
            value=value,
            options={},
        )
        ctx.add(p)
        assert ctx.debug_mode is True

    @pytest.mark.parametrize("value", [False, "off", "no", "false"])
    def test_disable_debug(self, value):
        """Disable debug mode."""
        ctx = PragmaContext()
        ctx.debug_mode = True  # Start enabled
        p = Pragma(
            type=PragmaType.DEBUG,
            directive="debug",
            value=value,
            options={},
        )
        ctx.add(p)
        assert ctx.debug_mode is False


class TestTCOPragma:
    """Test tail-call optimization pragmas."""

    @pytest.mark.parametrize("value", [True, "on", "yes", "true"])
    def test_enable_tco(self, value):
        """Enable TCO."""
        ctx = PragmaContext()
        ctx.tco_enabled = False  # Start disabled
        p = Pragma(
            type=PragmaType.TCO,
            directive="tco",
            value=value,
            options={},
        )
        ctx.add(p)
        assert ctx.tco_enabled is True

    @pytest.mark.parametrize("value", [False, "off", "no", "false"])
    def test_disable_tco(self, value):
        """Disable TCO."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.TCO,
            directive="tco",
            value=value,
            options={},
        )
        ctx.add(p)
        assert ctx.tco_enabled is False


class TestJITPragma:
    """Test JIT compilation pragmas."""

    @pytest.mark.parametrize("value", [True, "on", "yes", "true"])
    def test_enable_jit(self, value):
        """Enable JIT."""
        ctx = PragmaContext()
        ctx.jit_enabled = False  # Start disabled
        p = Pragma(
            type=PragmaType.JIT,
            directive="jit",
            value=value,
            options={},
        )
        ctx.add(p)
        assert ctx.jit_enabled is True

    @pytest.mark.parametrize("value", [False, "off", "no", "false"])
    def test_disable_jit(self, value):
        """Disable JIT."""
        ctx = PragmaContext()
        p = Pragma(
            type=PragmaType.JIT,
            directive="jit",
            value=value,
            options={},
        )
        ctx.add(p)
        assert ctx.jit_enabled is False


class TestPragmaContextStateStack:
    """Test push/pop state functionality."""

    def test_push_pop_restore_state(self):
        """Push and pop restore previous state."""
        ctx = PragmaContext()

        # Modify state
        ctx.optimize_level = 1
        ctx.cache_enabled = False
        ctx.debug_mode = True

        # Push state
        ctx.push_state()

        # Modify again
        ctx.optimize_level = 3
        ctx.cache_enabled = True
        ctx.debug_mode = False

        # Pop restores
        ctx.pop_state()
        assert ctx.optimize_level == 1
        assert ctx.cache_enabled is False
        assert ctx.debug_mode is True

    def test_push_pop_warnings(self):
        """Push/pop preserve warning settings."""
        ctx = PragmaContext()
        ctx.warnings = {"unused": False, "deprecated": True}

        ctx.push_state()
        ctx.warnings = {"all": False}

        ctx.pop_state()
        assert ctx.warnings == {"unused": False, "deprecated": True}

    def test_push_pop_inline_hints(self):
        """Push/pop preserve inline hints."""
        ctx = PragmaContext()
        ctx.inline_hints = {"func1": "always"}

        ctx.push_state()
        ctx.inline_hints = {"func2": "never"}

        ctx.pop_state()
        assert ctx.inline_hints == {"func1": "always"}

    def test_push_pop_pure_functions(self):
        """Push/pop preserve pure function set."""
        ctx = PragmaContext()
        ctx.pure_functions = {"fn1", "fn2"}

        ctx.push_state()
        ctx.pure_functions = {"fn3"}

        ctx.pop_state()
        assert ctx.pure_functions == {"fn1", "fn2"}

    def test_multiple_push_pop(self):
        """Multiple push/pop levels work correctly."""
        ctx = PragmaContext()

        ctx.optimize_level = 0
        ctx.push_state()

        ctx.optimize_level = 1
        ctx.push_state()

        ctx.optimize_level = 2
        ctx.push_state()

        ctx.optimize_level = 3

        ctx.pop_state()
        assert ctx.optimize_level == 2

        ctx.pop_state()
        assert ctx.optimize_level == 1

        ctx.pop_state()
        assert ctx.optimize_level == 0

    def test_pop_empty_stack_raises(self):
        """Pop on empty stack raises error."""
        ctx = PragmaContext()
        with pytest.raises(CatnipInternalError, match="No state to pop"):
            ctx.pop_state()


class TestPragmaIntegration:
    """Integration tests with Catnip interpreter."""

    def test_pragma_tco_via_interpreter(self):
        """pragma("tco", ...) works in Catnip code."""
        from catnip import Catnip

        catnip = Catnip()
        catnip.parse('pragma("tco", "off")')
        catnip.execute()
        assert catnip.pragma_context.tco_enabled is False

    def test_pragma_debug_via_interpreter(self):
        """pragma("debug", ...) works in Catnip code."""
        from catnip import Catnip

        catnip = Catnip()
        catnip.parse('pragma("debug", "on")')
        catnip.execute()
        assert catnip.pragma_context.debug_mode is True

    def test_pragma_cache_via_interpreter(self):
        """pragma("cache", ...) works in Catnip code."""
        from catnip import Catnip

        catnip = Catnip()
        catnip.parse('pragma("cache", "off")')
        catnip.execute()
        assert catnip.pragma_context.cache_enabled is False

    def test_multiple_pragmas(self):
        """Multiple pragmas in sequence."""
        from catnip import Catnip

        catnip = Catnip()
        catnip.parse("""
            pragma("tco", "off")
            pragma("debug", "on")
            pragma("cache", "off")
        """)
        catnip.execute()
        assert catnip.pragma_context.tco_enabled is False
        assert catnip.pragma_context.debug_mode is True
        assert catnip.pragma_context.cache_enabled is False


class TestOptimizationLevel:
    """Test optimization level settings."""

    def test_optimize_level_0_disables_constant_folding(self):
        """Level 0 disables all optimizations including constant folding."""
        from catnip import Catnip

        catnip = Catnip()
        catnip.pragma_context.optimize_level = 0
        catnip.parse("2 + 3")
        # With opt level 0, should NOT fold to 5
        assert len(catnip.code) == 1
        # Should be an Op ADD, not a literal 5
        from catnip._rs import Op

        assert isinstance(catnip.code[0], Op)

    def test_optimize_level_3_enables_constant_folding(self):
        """Level 3 enables all optimizations including constant folding."""
        from catnip import Catnip

        catnip = Catnip()
        catnip.pragma_context.optimize_level = 3
        catnip.parse("2 + 3")
        # With opt level 3, should fold to 5
        assert catnip.code == [5]

    def test_pragma_optimize_none(self):
        """pragma('optimize', 'none') sets level to 0."""
        from catnip import Catnip

        catnip = Catnip()
        catnip.parse('pragma("optimize", "none")')
        catnip.execute()
        assert catnip.pragma_context.optimize_level == 0

    def test_pragma_optimize_full(self):
        """pragma('optimize', 'full') sets level to 3."""
        from catnip import Catnip

        catnip = Catnip()
        catnip.pragma_context.optimize_level = 0  # Start at 0
        catnip.parse('pragma("optimize", "full")')
        catnip.execute()
        assert catnip.pragma_context.optimize_level == 3

    def test_optimize_level_affects_execution(self):
        """Optimization level affects actual execution results correctly."""
        from catnip import Catnip

        # Both should produce same result, just different IR
        catnip0 = Catnip()
        catnip0.pragma_context.optimize_level = 0
        catnip0.parse("2 + 3")
        result0 = catnip0.execute()

        catnip3 = Catnip()
        catnip3.pragma_context.optimize_level = 3
        catnip3.parse("2 + 3")
        result3 = catnip3.execute()

        assert result0 == result3 == 5


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
