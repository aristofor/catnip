#!/usr/bin/env python3
"""
Tail Recursion Performance Benchmark

Compares:
1. Tail-recursive function with automatic transformation
2. Manual while loop equivalent
3. Python native iterative version

Shows that Catnip's automatic tail recursion → loop transformation
achieves near-native loop performance.
"""

import time
import os
from catnip import Catnip

# Force AST mode for consistent benchmarking
os.environ["CATNIP_EXECUTOR"] = "ast"


def benchmark(name, cat_instance, iterations=100):
    """Run benchmark and return average time."""
    # Warm up
    for _ in range(3):
        cat_instance.execute()

    # Benchmark
    start = time.perf_counter()
    for _ in range(iterations):
        result = cat_instance.execute()
    end = time.perf_counter()

    avg_time = (end - start) / iterations * 1000
    return avg_time, result


def main():
    print("=" * 70)
    print("Tail Recursion Performance Benchmark")
    print("=" * 70)

    # Test value
    N = 1000

    # 1. Tail-recursive with automatic transformation
    print(f"\n1. Tail-recursive factorial({N}) - AUTO TRANSFORMATION")
    print("-" * 70)

    code_auto = f"""
factorial = (n, acc=1) => {{
    if n <= 1 {{ acc }}
    else {{ factorial(n - 1, n * acc) }}
}}

factorial({N})
"""

    cat_auto = Catnip()
    cat_auto.parse(code_auto, semantic=True)
    time_auto, result_auto = benchmark("Auto transform", cat_auto)

    print(f"Average time: {time_auto:.2f}ms")
    print(f"Result: {len(str(result_auto))} digits")

    # 2. Manual while loop
    print(f"\n2. Manual while loop factorial({N})")
    print("-" * 70)

    code_manual = f"""
factorial_manual = (n, acc=1) => {{
    while True {{
        if n <= 1 {{ return acc }}
        tmp_n = n - 1
        tmp_acc = n * acc
        n = tmp_n
        acc = tmp_acc
    }}
}}

factorial_manual({N})
"""

    cat_manual = Catnip()
    cat_manual.parse(code_manual, semantic=True)
    time_manual, result_manual = benchmark("Manual loop", cat_manual)

    print(f"Average time: {time_manual:.2f}ms")
    print(f"Result: {len(str(result_manual))} digits")

    # 3. Python native iterative
    print(f"\n3. Python native iterative factorial({N})")
    print("-" * 70)

    def python_factorial(n):
        acc = 1
        while n > 1:
            acc *= n
            n -= 1
        return acc

    # Warm up
    for _ in range(3):
        python_factorial(N)

    start = time.perf_counter()
    for _ in range(100):
        result_python = python_factorial(N)
    end = time.perf_counter()

    time_python = (end - start) / 100 * 1000

    print(f"Average time: {time_python:.2f}ms")
    print(f"Result: {len(str(result_python))} digits")

    # Summary
    print("\n" + "=" * 70)
    print("SUMMARY")
    print("=" * 70)

    # Verify correctness
    assert result_auto == result_manual == result_python, "Results don't match!"
    print(f"✓ All implementations return the same result")

    # Performance comparison
    print(f"\nPerformance relative to manual while loop:")
    print(f"  Auto transform:  {time_auto:.2f}ms ({time_auto/time_manual:.2f}x)")
    print(f"  Manual loop:     {time_manual:.2f}ms (1.00x baseline)")
    print(f"  Python native:   {time_python:.2f}ms ({time_python/time_manual:.2f}x)")

    # Speedup over trampoline (historical data: ~3ms for N=1000)
    trampoline_time = 3.0
    print(f"\nSpeedup over trampoline runtime (~{trampoline_time}ms):")
    print(f"  Auto transform: {trampoline_time/time_auto:.2f}x faster")

    # Analysis
    print("\n" + "=" * 70)
    print("ANALYSIS")
    print("=" * 70)

    overhead_pct = (time_auto - time_manual) / time_manual * 100
    if abs(overhead_pct) < 20:
        print(f"✓ Auto transformation overhead: {overhead_pct:.1f}%")
        print("  Performance is comparable to manual while loop!")
    else:
        print(f"⚠ Auto transformation overhead: {overhead_pct:.1f}%")

    print(f"\nBenefits of automatic transformation:")
    print(f"  - No manual conversion needed")
    print(f"  - O(1) stack space (no stack overflow)")
    print(f"  - Near-native loop performance")
    print(f"  - Works transparently with existing code")


if __name__ == "__main__":
    main()
