# FILE: tests/optimization/test_nd_memoization.py
"""
Tests for ND memoization (Phase 6).

Memoization caches ND-recursion results to avoid
redundant computation.
"""

import time

import pytest

from catnip import Catnip


def exec_catnip(code: str):
    """Helper to execute Catnip code."""
    catnip = Catnip()
    catnip.parse(code)
    return catnip.execute()


class TestNDMemoization:
    """ND memoization tests."""

    def test_pragma_memoize_on(self):
        """
        Enable memoization via pragma.
        """
        code = """
        pragma("nd_memoize", True)

        ~~(10, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """

        result = exec_catnip(code)
        assert result == 3628800  # 10!

    def test_pragma_memoize_off(self):
        """
        Disable memoization via pragma.
        """
        code = """
        pragma("nd_memoize", False)

        ~~(10, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """

        result = exec_catnip(code)
        assert result == 3628800  # 10!

    def test_memoization_fibonacci(self):
        """
        Memoization on Fibonacci - classic redundancy case.

        Without memoization, fib(20) needs ~21,891 recursive calls.
        With memoization, fib(20) needs 20 calls (one per unique value).
        """
        code = """
        pragma("nd_memoize", True)

        ~~(20, (n, recur) => {
            if n <= 1 { n }
            else { recur(n - 1) + recur(n - 2) }
        })
        """

        result = exec_catnip(code)
        assert result == 6765  # fib(20)

    def test_memoization_performance_gain(self):
        """
        Ensure memoization significantly improves performance.

        Compare fib(20) with and without memoization.
        """
        code_with_memoize = """
        pragma("nd_memoize", True)
        ~~(20, (n, recur) => {
            if n <= 1 { n }
            else { recur(n - 1) + recur(n - 2) }
        })
        """

        code_without_memoize = """
        pragma("nd_memoize", False)
        ~~(20, (n, recur) => {
            if n <= 1 { n }
            else { recur(n - 1) + recur(n - 2) }
        })
        """

        # Time with memoization
        start = time.time()
        result_memoized = exec_catnip(code_with_memoize)
        time_memoized = time.time() - start

        # Time without memoization
        start = time.time()
        result_no_memoize = exec_catnip(code_without_memoize)
        time_no_memoize = time.time() - start

        # Validate results
        assert result_memoized == result_no_memoize == 6765

        # Memoization should be at least 3x faster
        speedup = time_no_memoize / time_memoized
        assert speedup > 3, f"Memoization speedup {speedup:.1f}x < 3x"

    def test_memoization_broadcast(self):
        """
        Memoization with ND broadcast.

        Compute factorials on multiple values, some redundant.
        """
        code = """
        pragma("nd_memoize", True)

        list(5, 3, 5, 4, 3).[~~ (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        }]
        """

        result = exec_catnip(code)
        # 5! = 120, 3! = 6, 4! = 24
        assert result == [120, 6, 120, 24, 6]

    def test_memoization_correctness(self):
        """
        Ensure memoization does not change results.

        Compare results with and without memoization across multiple values.
        """
        test_values = [5, 10, 15]

        for n in test_values:
            code_with = f"""
            pragma("nd_memoize", True)
            ~~({n}, (n, recur) => {{
                if n <= 1 {{ 1 }}
                else {{ n * recur(n - 1) }}
            }})
            """

            code_without = f"""
            pragma("nd_memoize", False)
            ~~({n}, (n, recur) => {{
                if n <= 1 {{ 1 }}
                else {{ n * recur(n - 1) }}
            }})
            """

            result_with = exec_catnip(code_with)
            result_without = exec_catnip(code_without)

            assert result_with == result_without, f"Memoization changed result for {n}!"


class TestNDMemoizationEdgeCases:
    """Memoization edge case tests."""

    def test_memoization_with_zero(self):
        """Memoization with n=0."""
        code = """
        pragma("nd_memoize", True)
        ~~(0, (n, recur) => { n })
        """
        result = exec_catnip(code)
        assert result == 0

    def test_memoization_with_one(self):
        """Memoization with n=1."""
        code = """
        pragma("nd_memoize", True)
        ~~(1, (n, recur) => { n })
        """
        result = exec_catnip(code)
        assert result == 1

    def test_memoization_parallel_mode(self):
        """
        Memoization in parallel mode.

        Memoization should work even in parallel mode.
        """
        code = """
        pragma("nd_mode", ND.process)
        pragma("nd_workers", 4)
        pragma("nd_memoize", True)

        list(5, 3, 5, 4).[~~ (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        }]
        """

        result = exec_catnip(code)
        assert result == [120, 6, 120, 24]
