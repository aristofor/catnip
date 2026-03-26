#!/usr/bin/env python3
# FILE: tests/jit_functions/test_recursive_phase2_simple.py
"""Phase 2: Simple test to verify JIT recursive works"""

import os
import subprocess
import sys
import tempfile

import pytest

_CODE = """
factorial = (n) => {
    if n <= 1 { 1 } else { n * factorial(n - 1) }
}

for i in range(120) { factorial(5) }

print(factorial(3))
print(factorial(4))
print(factorial(5))
print(factorial(9))
print(factorial(10))
"""


def test_jit_recursive_phase2():
    """Verify JIT recursive functions produce correct results."""
    with tempfile.NamedTemporaryFile(mode='w', suffix='.cat', delete=False) as f:
        f.write(_CODE)
        temp_file = f.name

    try:
        env = os.environ.copy()
        env['CATNIP_OPTIMIZE'] = 'jit'
        env['XDG_CACHE_HOME'] = tempfile.mkdtemp()
        result = subprocess.run(
            [sys.executable, '-m', 'catnip', temp_file],
            capture_output=True,
            text=True,
            env=env,
        )

        lines = result.stdout.strip().split('\n')
        values = [int(line) for line in lines if line.strip().isdigit()]
        expected = [6, 24, 120, 362880, 3628800]
        assert values == expected, f"Results {values} don't match expected {expected}"
    finally:
        os.unlink(temp_file)
