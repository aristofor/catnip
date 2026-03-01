#!/usr/bin/env python3
# FILE: tests/jit_functions/test_function_benchmark.py
"""Benchmark function JIT performance"""

import time

from catnip._rs import VM, Compiler

from catnip import Catnip

code = """
# Fibonacci function
fib = (n) => {
    if n <= 1 {
        n
    } else {
        fib(n - 1) + fib(n - 2)
    }
}

# Call it many times with warmup
result = 0
count = 0
while count < 200 {
    result = fib(15)
    count = count + 1
}
result
"""

print("=" * 60)
print("Function JIT Performance Benchmark")
print("=" * 60)

# Test WITHOUT JIT
print("\n1. Testing WITHOUT JIT...")
c1 = Catnip(vm_mode='on')
ast1 = c1.parse(code)
compiler1 = Compiler()
compiled1 = compiler1.compile(ast1)

vm1 = VM()
vm1.set_context(c1.context)
# No JIT enabled

start = time.time()
result1 = vm1.execute(compiled1, ())
time_no_jit = time.time() - start

print(f"   Result: {result1}")
print(f"   Time: {time_no_jit:.3f}s")

# Test WITH JIT
print("\n2. Testing WITH JIT...")
c2 = Catnip(vm_mode='on')
ast2 = c2.parse(code)
compiler2 = Compiler()
compiled2 = compiler2.compile(ast2)

vm2 = VM()
vm2.set_context(c2.context)
vm2.enable_jit()  # JIT enabled

start = time.time()
result2 = vm2.execute(compiled2, ())
time_with_jit = time.time() - start

print(f"   Result: {result2}")
print(f"   Time: {time_with_jit:.3f}s")

# Compare
print(f"\n{'=' * 60}")
print(f"Results match: {result1 == result2}")
if time_with_jit > 0:
    speedup = time_no_jit / time_with_jit
    print(f"Speedup: {speedup:.1f}x")

    if speedup > 1.5:
        print("\n✓ JIT provides significant speedup!")
    elif speedup > 1.0:
        print("\n~ JIT provides modest speedup")
    else:
        print("\n⚠ No speedup observed (JIT may not be active)")
else:
    print("\n⚠ Cannot compute speedup")
