# FILE: tests/serial/test_jit_types.py
"""Tests for JIT type support (int, float, bool, nested loops).

Each test runs in a subprocess for process isolation (separate JIT state).
"""

import platform
import subprocess
import sys

import pytest

pytestmark = pytest.mark.skipif(
    platform.machine().lower() not in ("x86_64", "amd64"),
    reason=f"JIT only supports x86-64, got {platform.machine()}",
)


def jit_run(code_str, expected, tmp_path_factory=None):
    """Run Catnip code with JIT in a subprocess, assert last line == expected."""
    import tempfile

    with tempfile.TemporaryDirectory() as tmpdir:
        env = {**__import__('os').environ, 'CATNIP_CACHE': 'off', 'XDG_CACHE_HOME': tmpdir}
        result = subprocess.run(
            [sys.executable, '-m', 'catnip', '-o', 'jit', '-c', code_str],
            capture_output=True,
            text=True,
            timeout=30,
            env=env,
        )
    assert result.returncode == 0, f"Process failed:\n{result.stderr}"
    output = result.stdout.strip().split('\n')[-1]
    actual = float(output) if '.' in output else int(output)
    assert actual == expected, f"got {actual}, expected {expected}"


class TestJITTypes:
    """Test JIT compilation with different types."""

    def test_float_accumulation(self):
        jit_run('{x = 0.0; for i in range(2000) { x = x + 1.5 }; x}', 3000.0)

    def test_float_multiplication(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmpdir:
            env = {**__import__('os').environ, 'CATNIP_CACHE': 'off', 'XDG_CACHE_HOME': tmpdir}
            result = subprocess.run(
                [
                    sys.executable,
                    '-m',
                    'catnip',
                    '-o',
                    'jit',
                    '-c',
                    '{x = 1.0; for i in range(2000) { x = x * 1.001 }; x}',
                ],
                capture_output=True,
                text=True,
                timeout=30,
                env=env,
            )
        assert result.returncode == 0
        actual = float(result.stdout.strip().split('\n')[-1])
        expected = 1.001**2000
        assert abs(actual - expected) / expected < 1e-10

    def test_mixed_int_float(self):
        jit_run('{sum = 0.0; for i in range(2000) { sum = sum + 0.5 }; sum}', 1000.0)

    def test_int_loop_still_works(self):
        jit_run('{x = 0; for i in range(2000) { x = x + 1 }; x}', 2000)

    def test_nested_loops(self):
        jit_run('{sum = 0; for i in range(100) { for j in range(100) { sum = sum + 1 } }; sum}', 10000)

    def test_boolean_variable(self):
        jit_run('{count = 0; flag = True; for i in range(2000) { if flag { count = count + 1 } }; count}', 2000)
