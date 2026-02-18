# FILE: tests/optimization/template.py
"""
Template for optimization tests.

Copy this file and adapt it for each new optimization.
Example: test_optimization_constant_folding.py
"""

import unittest

from catnip import Catnip


class TestOptimization_NAME(unittest.TestCase):
    """
    Tests for OPTIMIZATION_NAME optimization.

    Suggested structure:
    1. Correctness tests (semantics preserved)
    2. Detection tests (cases where the optimization applies)
    3. Non-application tests (cases where it must NOT apply)
    4. Edge case tests
    5. Performance tests (optional)
    """

    def test_preserves_semantics(self):
        """Ensure the optimization preserves exact semantics.

        Suggested pattern:
        1. Run the same code with optimization ON and OFF
        2. Verify that results match
        """
        catnip_with_optim = Catnip()
        # TODO: Enable the optimization
        # catnip_with_optim.optimize_settings.FEATURE = True

        catnip_without_optim = Catnip()
        # TODO: Disable the optimization
        # catnip_without_optim.optimize_settings.FEATURE = False

        test_code = """
            # TODO: Test code
        """

        result_with = catnip_with_optim.parse(test_code)
        result_with = catnip_with_optim.execute()

        result_without = catnip_without_optim.parse(test_code)
        result_without = catnip_without_optim.execute()

        self.assertEqual(result_with, result_without)

    def test_simple_case(self):
        """Simple case where the optimization should apply."""
        catnip = Catnip()
        # TODO: Enable the optimization

        code = catnip.parse("""
            # TODO: Code that should be optimized
        """)

        result = catnip.execute()
        # TODO: Validate the result
        self.assertEqual(result, "expected_value")

    def test_complex_case(self):
        """Complex case where the optimization should apply."""
        catnip = Catnip()
        # TODO: Enable the optimization

        code = catnip.parse("""
            # TODO: Complex code that should be optimized
        """)

        result = catnip.execute()
        # TODO: Validate the result
        self.assertEqual(result, "expected_value")

    def test_should_not_optimize(self):
        """Case where the optimization must NOT apply."""
        catnip = Catnip()
        # TODO: Enable the optimization

        code = catnip.parse("""
            # TODO: Code that should NOT be optimized
        """)

        result = catnip.execute()
        # TODO: Ensure optimization preserves semantics
        self.assertEqual(result, "expected_value")

    def test_with_side_effects(self):
        """Ensure the optimization respects side effects."""
        catnip = Catnip()
        # TODO: Enable the optimization

        code = catnip.parse("""
            # TODO: Code with side effects (print, mutation, etc.)
        """)

        result = catnip.execute()
        # TODO: Ensure side effects are preserved
        self.assertEqual(result, "expected_value")

    def test_edge_case_boundary(self):
        """Boundary cases (extremes, degenerate cases)."""
        catnip = Catnip()
        # TODO: Enable the optimization

        code = catnip.parse("""
            # TODO: Code with boundary values (0, None, empty, etc.)
        """)

        result = catnip.execute()
        # TODO: Validate the result
        self.assertEqual(result, "expected_value")

    def test_nested_optimization(self):
        """Nested case where the optimization should apply recursively."""
        catnip = Catnip()
        # TODO: Enable the optimization

        code = catnip.parse("""
            # TODO: Code with nested structures
        """)

        result = catnip.execute()
        # TODO: Validate the result
        self.assertEqual(result, "expected_value")


class TestOptimization_NAME_Performance(unittest.TestCase):
    """
    Performance tests for OPTIMIZATION_NAME optimization.

    These tests are optional but recommended for optimizations
    with measurable performance impact.
    """

    def test_performance_gain(self):
        """Ensure the optimization improves performance."""
        import time

        catnip_with = Catnip()
        # TODO: Enable the optimization

        catnip_without = Catnip()
        # TODO: Disable the optimization

        test_code = """
            # TODO: Code computationally intensive
        """

        # Measure without optimization
        catnip_without.parse(test_code)
        start = time.perf_counter()
        for _ in range(100):
            catnip_without.execute()
        time_without = time.perf_counter() - start

        # Measure with optimization
        catnip_with.parse(test_code)
        start = time.perf_counter()
        for _ in range(100):
            catnip_with.execute()
        time_with = time.perf_counter() - start

        # Optimization should be faster
        # Note: This test can be flaky; adjust the threshold if needed
        speedup = time_without / time_with
        self.assertGreater(speedup, 1.0, f"Expected speedup, got {speedup:.2f}x")


if __name__ == "__main__":
    unittest.main()
