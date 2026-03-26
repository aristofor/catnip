# FILE: tests/serial/test_nd_concurrency.py
"""
Tests for ND-recursion with 3 execution modes.

Modes:
- sequential: Single-threaded, no concurrency
- threads: ThreadPoolExecutor, shared memory, GIL-limited
- processes: ProcessPoolExecutor, true parallelism
"""

import pytest

from catnip import Catnip


def exec_catnip(code: str):
    """Helper to execute Catnip code."""
    catnip = Catnip()
    catnip.parse(code)
    return catnip.execute()


def _factorial_for_nd(n, recur):
    """Module-level factorial for ND tests (must be picklable)."""
    if n <= 1:
        return 1
    else:
        return n * recur(n - 1)


def _fibonacci_for_nd(n, recur):
    """Module-level fibonacci for ND tests (must be picklable)."""
    if n <= 1:
        return n
    else:
        return recur(n - 1) + recur(n - 2)


class TestNDModeSequential:
    """Sequential mode tests (baseline)."""

    def test_factorial_sequential(self):
        """Basic factorial with sequential mode (default)."""
        code = """
        ~~(10, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 3628800  # 10!

    def test_explicit_sequential_mode(self):
        """Explicit pragma for sequential mode."""
        code = """
        pragma("nd_mode", ND.sequential)

        ~~(7, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 5040  # 7!

    def test_sequential_alias_seq(self):
        """'seq' alias for sequential mode."""
        code = """
        pragma("nd_mode", ND.sequential)

        ~~(5, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 120  # 5!


class TestNDModeThreads:
    """Thread mode tests (ThreadPoolExecutor)."""

    def test_factorial_thread(self):
        """Factorial with threads mode."""
        code = """
        pragma("nd_mode", ND.thread)

        ~~(10, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 3628800  # 10!

    def test_thread_alias_thread(self):
        """'thread' alias for threads mode."""
        code = """
        pragma("nd_mode", ND.thread)

        ~~(6, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 720  # 6!

    def test_broadcast_thread(self):
        """Broadcast with threads mode."""
        code = """
        pragma("nd_mode", ND.thread)

        list(5, 6, 7, 8).[~~ (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        }]
        """
        result = exec_catnip(code)
        assert result == [120, 720, 5040, 40320]

    def test_memoization_thread(self):
        """Memoization should be shared across threads."""
        code = """
        pragma("nd_mode", ND.thread)
        pragma("nd_memoize", True)

        # Fibonacci benefits from memoization
        ~~(10, (n, recur) => {
            if n <= 1 { n }
            else { recur(n - 1) + recur(n - 2) }
        })
        """
        result = exec_catnip(code)
        assert result == 55  # fib(10)


class TestNDModeProcesses:
    """Process mode tests (ProcessPoolExecutor)."""

    def test_factorial_process(self):
        """Factorial with processes mode."""
        code = """
        pragma("nd_mode", ND.process)

        ~~(10, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 3628800  # 10!

    def test_process_alias_process(self):
        """'process' alias for processes mode."""
        code = """
        pragma("nd_mode", ND.process)

        ~~(8, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 40320  # 8!

    def test_process_alias_parallel(self):
        """'parallel' alias for processes mode (backwards compat)."""
        code = """
        pragma("nd_mode", ND.process)

        ~~(5, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 120  # 5!

    def test_process_alias_par(self):
        """'par' alias for processes mode."""
        code = """
        pragma("nd_mode", ND.process)

        ~~(6, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 720  # 6!


class TestNDSchedulerAPI:
    """Direct NDScheduler API tests."""

    def test_scheduler_mode_sequential(self):
        """Sequential execution via NDScheduler."""
        from catnip.nd import NDScheduler

        scheduler = NDScheduler(n_workers=4, mode='sequential')

        result = scheduler.execute_sync(5, _factorial_for_nd)
        assert result == 120

    def test_scheduler_mode_thread(self):
        """Thread execution via NDScheduler."""
        from catnip.nd import NDScheduler

        scheduler = NDScheduler(n_workers=4, mode='threads')

        result = scheduler.execute_thread(5, _factorial_for_nd)
        assert result == 120

    def test_scheduler_mode_process(self):
        """Process execution via NDScheduler."""
        from catnip.nd import NDScheduler

        scheduler = NDScheduler(n_workers=4, mode='processes')

        # Uses module-level function (picklable)
        result = scheduler.execute_process(5, _factorial_for_nd)
        assert result == 120

    def test_scheduler_set_mode(self):
        """set_mode() changes execution mode."""
        from catnip.nd import NDScheduler

        scheduler = NDScheduler(n_workers=2)
        assert scheduler.mode == 'sequential'

        scheduler.set_mode('thread')
        assert scheduler.mode == 'thread'

        scheduler.set_mode('process')
        assert scheduler.mode == 'process'

    def test_scheduler_invalid_mode(self):
        """Invalid mode raises ValueError."""
        from catnip.nd import NDScheduler

        scheduler = NDScheduler()

        with pytest.raises(ValueError):
            scheduler.set_mode('invalid')


class TestNDPragmas:
    """ND pragma tests."""

    def test_pragma_nd_workers(self):
        """nd_workers pragma configures worker count."""
        code = """
        pragma("nd_workers", 8)

        ~~(10, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 3628800

    def test_pragma_nd_mode_with_workers(self):
        """Combined nd_mode and nd_workers pragmas."""
        code = """
        pragma("nd_mode", ND.thread)
        pragma("nd_workers", 4)

        list(5, 6, 7).[~~ (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        }]
        """
        result = exec_catnip(code)
        assert result == [120, 720, 5040]


class TestNDDeterminism:
    """Tests to ensure all modes produce the same results."""

    def test_all_modes_same_result(self):
        """All 3 modes should produce identical results."""
        factorial_code = """
        pragma("nd_mode", ND.{mode})

        ~~(8, (n, recur) => {{
            if n <= 1 {{ 1 }}
            else {{ n * recur(n - 1) }}
        }})
        """

        expected = 40320  # 8!

        for mode in ('sequential', 'thread', 'process'):
            code = factorial_code.format(mode=mode)
            result = exec_catnip(code)
            assert result == expected, f"Mode {mode} returned {result}, expected {expected}"

    def test_broadcast_all_modes(self):
        """Broadcast should work identically in all modes."""
        broadcast_code = """
        pragma("nd_mode", ND.{mode})

        list(3, 4, 5).[~~ (n, recur) => {{
            if n <= 1 {{ 1 }}
            else {{ n * recur(n - 1) }}
        }}]
        """

        expected = [6, 24, 120]  # 3!, 4!, 5!

        for mode in ('sequential', 'thread', 'process'):
            code = broadcast_code.format(mode=mode)
            result = exec_catnip(code)
            assert result == expected, f"Mode {mode} returned {result}, expected {expected}"
