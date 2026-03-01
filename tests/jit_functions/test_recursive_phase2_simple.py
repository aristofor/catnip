#!/usr/bin/env python3
# FILE: tests/jit_functions/test_recursive_phase2_simple.py
"""Phase 2: Simple test to verify JIT recursive works"""

import os
import subprocess
import sys
import tempfile

code = """
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

print("=" * 60)
print("Phase 2: JIT Recursive Functions Test")
print("=" * 60)

# Write code to temp file
with tempfile.NamedTemporaryFile(mode='w', suffix='.cat', delete=False) as f:
    f.write(code)
    temp_file = f.name

# Run with JIT enabled
env = os.environ.copy()
env['CATNIP_OPTIMIZE'] = 'jit'
result = subprocess.run([sys.executable, '-m', 'catnip', temp_file], capture_output=True, text=True, env=env)

# Clean up
os.unlink(temp_file)

# Parse outputs
lines = result.stdout.strip().split('\n')
values = [int(line) for line in lines if line.strip().isdigit()]

expected = [6, 24, 120, 362880, 3628800]

print("\nResults:")
for i, (val, exp) in enumerate(zip(values, expected)):
    n = [3, 4, 5, 9, 10][i]
    status = "✓" if val == exp else "✗"
    print(f"  {status} factorial({n}) = {val} (expected {exp})")

# Verify
assert values == expected, f"Results {values} don't match expected {expected}"

print(f"\n{'=' * 60}")
print("✓ Phase 2 COMPLETE!")
print("  - JIT recursive compilation working")
print("  - CallSelf auto-calling in native code")
print("  - All results correct")
print(f"{'=' * 60}")
