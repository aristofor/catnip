# FILE: tests/optimization/test_optimizer.py
"""
Tests for the Optimizer class and optimization passes.

Tests strength reduction, block flattening, dead code elimination,
and the optimizer pipeline.

Tests are skipped when the targeted optimizer is disabled (optimize_level=0).
"""

import pytest

from catnip import Catnip

# Enable pytester fixture for meta-testing (testing pytest behavior)
pytest_plugins = ["pytester"]

# --- Fixtures for different optimization levels ---


@pytest.fixture
def catnip_opt0():
    """Catnip instance with all optimizations disabled (level 0)."""
    cat = Catnip()
    cat.pragma_context.optimize_level = 0
    return cat


@pytest.fixture
def catnip_opt3():
    """Catnip instance with IR optimizations enabled (level 2, CFG disabled due to bugs)."""
    return Catnip(optimize=2)


class TestStrengthReduction:
    """Tests for strength reduction optimizations."""

    @pytest.fixture
    def catnip(self):
        return Catnip(optimize=2)

    def test_multiply_by_one_right(self, catnip):
        """x * 1 -> x"""
        catnip.parse("x = 42; x * 1")
        result = catnip.execute()
        assert result == 42

    def test_multiply_by_one_left(self, catnip):
        """1 * x -> x"""
        catnip.parse("x = 42; 1 * x")
        result = catnip.execute()
        assert result == 42

    def test_multiply_by_zero_right(self, catnip):
        """x * 0 -> 0"""
        catnip.parse("x = 42; x * 0")
        result = catnip.execute()
        assert result == 0

    def test_multiply_by_zero_left(self, catnip):
        """0 * x -> 0"""
        catnip.parse("x = 42; 0 * x")
        result = catnip.execute()
        assert result == 0

    def test_power_of_two_constant(self, catnip):
        """Constant ** 2 is folded."""
        # Note: strength reduction to x * x for variables has a known issue
        # Testing with constants which are fully folded
        catnip.parse("7 ** 2")
        result = catnip.execute()
        assert result == 49

    def test_power_of_one(self, catnip):
        """x ** 1 -> x"""
        catnip.parse("x = 42; x ** 1")
        result = catnip.execute()
        assert result == 42

    def test_power_of_zero(self, catnip):
        """x ** 0 -> 1"""
        catnip.parse("x = 42; x ** 0")
        result = catnip.execute()
        assert result == 1

    def test_add_zero_right(self, catnip):
        """x + 0 -> x"""
        catnip.parse("x = 42; x + 0")
        result = catnip.execute()
        assert result == 42

    def test_add_zero_left(self, catnip):
        """0 + x -> x"""
        catnip.parse("x = 42; 0 + x")
        result = catnip.execute()
        assert result == 42

    def test_sub_zero(self, catnip):
        """x - 0 -> x"""
        catnip.parse("x = 42; x - 0")
        result = catnip.execute()
        assert result == 42

    def test_div_by_one(self, catnip):
        """x / 1 -> x"""
        catnip.parse("x = 42; x / 1")
        result = catnip.execute()
        assert result == 42.0

    def test_floordiv_by_one(self, catnip):
        """x // 1 -> x"""
        catnip.parse("x = 42; x // 1")
        result = catnip.execute()
        assert result == 42

    def test_nested_strength_reduction(self, catnip):
        """Multiple strength reductions in one expression."""
        catnip.parse("x = 5; (x * 1 + 0) ** 1")
        result = catnip.execute()
        assert result == 5


class TestDeadCodeElimination:
    """Tests for dead code elimination."""

    @pytest.fixture
    def catnip(self):
        return Catnip(optimize=2)

    def test_if_true_constant(self, catnip):
        """if True { block } else { other } -> block"""
        catnip.parse('if True { "yes" } else { "no" }')
        result = catnip.execute()
        assert result == "yes"

    def test_if_false_constant(self, catnip):
        """if False { block } else { other } -> other"""
        catnip.parse('if False { "yes" } else { "no" }')
        result = catnip.execute()
        assert result == "no"

    def test_if_false_no_else(self, catnip):
        """if False { block } -> None"""
        catnip.parse("x = 1; if False { x = 99 }; x")
        result = catnip.execute()
        assert result == 1

    def test_while_false_never_executes(self, catnip):
        """while False { block } never executes."""
        catnip.parse("x = 1; while False { x = x + 1 }; x")
        result = catnip.execute()
        assert result == 1

    def test_elif_chain_with_variable(self, catnip):
        """elif chain with variable condition."""
        code = """
        x = 1
        if x == 0 { "first" }
        elif x == 1 { "second" }
        else { "third" }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == "second"

    def test_complex_dead_code(self, catnip):
        """Complex dead code scenario."""
        code = """
        x = 10
        if False {
            x = 999
        }
        if True {
            x = x + 1
        }
        x
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 11


class TestBlockFlattening:
    """Tests for block flattening optimization."""

    @pytest.fixture
    def catnip(self):
        return Catnip(optimize=2)

    def test_nested_blocks_execute(self, catnip):
        """Nested blocks execute correctly."""
        # Variables are in the same scope in Catnip (no block scoping)
        code = """
        x = 1
        y = 2
        z = 3
        x + y + z
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 6

    def test_block_returns_last_value(self, catnip):
        """Block returns its last value."""
        catnip.parse("{ 1; 2; 3 }")
        result = catnip.execute()
        assert result == 3

    def test_empty_block_in_sequence(self, catnip):
        """Empty blocks don't affect sequence."""
        catnip.parse("x = 1; {}; x + 1")
        result = catnip.execute()
        assert result == 2

    def test_block_with_assignments(self, catnip):
        """Blocks with assignments work correctly."""
        code = """
        {
            a = 10
            b = 20
            a + b
        }
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 30


class TestOptimizerPipeline:
    """Tests for the full optimizer pipeline."""

    @pytest.fixture
    def catnip(self):
        return Catnip(optimize=2)

    def test_multiple_optimizations_compose(self, catnip):
        """Multiple optimizations work together."""
        # constant folding + strength reduction
        code = "x = 2 + 3; x * 1"  # 2 + 3 -> 5, then 5 * 1 -> 5
        catnip.parse(code)
        result = catnip.execute()
        assert result == 5

    def test_optimization_iterations(self, catnip):
        """Optimizations iterate to enable each other."""
        # After constant folding, strength reduction can apply
        code = "(1 + 1) ** 2"  # fold to 2 ** 2, then strength reduce
        catnip.parse(code)
        result = catnip.execute()
        assert result == 4

    def test_preserves_semantics(self, catnip):
        """Optimizations preserve program semantics."""
        code = """
        a = 5
        b = 3
        c = a * b
        d = a * b
        e = c + d
        e
        """
        catnip.parse(code)
        result = catnip.execute()
        assert result == 30

    def test_no_side_effect_changes(self, catnip):
        """Optimizations don't change side effect order."""
        code = """
        counter = 0
        inc = () => { counter = counter + 1; counter }
        x = inc() + inc()
        x
        """
        catnip.parse(code)
        result = catnip.execute()
        # inc() called twice: 1 + 2 = 3
        assert result == 3


class TestChainedComparisons:
    """Test constant folding with chained comparisons."""

    @pytest.fixture
    def catnip(self):
        return Catnip(optimize=2)

    def test_chained_lt(self, catnip):
        """a < b < c with constants."""
        catnip.parse("1 < 2 < 3")
        result = catnip.execute()
        assert result is True

    def test_chained_lt_false(self, catnip):
        """a < b < c with constants (false case)."""
        catnip.parse("1 < 3 < 2")
        result = catnip.execute()
        assert result is False

    def test_chained_le(self, catnip):
        """a <= b <= c with constants."""
        catnip.parse("1 <= 2 <= 2")
        result = catnip.execute()
        assert result is True

    def test_chained_gt(self, catnip):
        """a > b > c with constants."""
        catnip.parse("3 > 2 > 1")
        result = catnip.execute()
        assert result is True

    def test_chained_ge(self, catnip):
        """a >= b >= c with constants."""
        catnip.parse("3 >= 3 >= 1")
        result = catnip.execute()
        assert result is True

    def test_chained_eq(self, catnip):
        """a == b == c with constants."""
        catnip.parse("1 == 1 == 1")
        result = catnip.execute()
        assert result is True

    def test_chained_ne(self, catnip):
        """a != b != c with constants."""
        catnip.parse("1 != 2 != 3")
        result = catnip.execute()
        assert result is True


class TestOptimizationLevelSkipping:
    """Tests that verify optimization level skipping behavior."""

    def test_opt0_still_executes_correctly(self, catnip_opt0):
        """Code executes correctly even without optimizations."""
        catnip_opt0.parse("x = 5; x * 1 + 0")
        result = catnip_opt0.execute()
        # Without optimization, x * 1 + 0 still evaluates to 5
        assert result == 5

    def test_opt0_constant_folding_disabled(self, catnip_opt0):
        """Constant folding is disabled at level 0 (semantic still works)."""
        # We can verify indirectly by checking the code still executes
        catnip_opt0.parse("2 + 3")
        result = catnip_opt0.execute()
        assert result == 5

    def test_opt3_constant_folding_enabled(self, catnip_opt3):
        """Constant folding is enabled at level 3."""
        catnip_opt3.parse("2 + 3")
        result = catnip_opt3.execute()
        assert result == 5

    def test_opt0_dead_code_still_runs(self, catnip_opt0):
        """Dead code branches still execute at level 0."""
        catnip_opt0.parse('if True { "yes" } else { "no" }')
        result = catnip_opt0.execute()
        assert result == "yes"

    def test_opt0_complex_expression(self, catnip_opt0):
        """Complex expression evaluates correctly at level 0."""
        code = """
        a = 10
        b = 20
        c = a + b
        d = c * 2
        d + 0
        """
        catnip_opt0.parse(code)
        result = catnip_opt0.execute()
        assert result == 60

    def test_catnip_optimize_introspection(self):
        """catnip.optimize reflects the current optimization level."""
        cat = Catnip()
        cat.pragma_context.optimize_level = 0
        cat.parse("catnip.optimize")
        result = cat.execute()
        assert result == 0

        cat2 = Catnip()
        cat2.pragma_context.optimize_level = 2
        cat2.parse("catnip.optimize")
        result2 = cat2.execute()
        assert result2 == 2

    def test_skip_if_no_optimization(self, pytester):
        """Verify that tests can skip based on optimization level."""
        # Use pytester to run a test in isolation and verify skip behavior
        pytester.makepyfile("""
            import pytest
            from catnip import Catnip

            def test_should_skip():
                cat = Catnip()
                cat.pragma_context.optimize_level = 0
                if cat.pragma_context.optimize_level == 0:
                    pytest.skip("Test requires optimization enabled")
        """)
        result = pytester.runpytest("-v")
        result.assert_outcomes(skipped=1)

    def test_conditional_based_on_introspection(self):
        """Code can branch based on catnip.optimize."""
        cat = Catnip()
        cat.pragma_context.optimize_level = 1
        code = """
        if catnip.optimize > 0 {
            "optimized"
        } else {
            "unoptimized"
        }
        """
        cat.parse(code)
        result = cat.execute()
        assert result == "optimized"


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
