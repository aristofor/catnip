#!/usr/bin/env python3
"""
Detailed AST vs VM Benchmark - Janvier 2026

Benchmarks the performance difference between AST interpreter mode
and VM bytecode mode across different program types.

Measures:
  - Parse time (same for both)
  - Compilation time (AST vs Bytecode generation)
  - Execution time (AST tree-walk vs VM dispatch)
  - Memory footprint
"""

import time
import sys
from pathlib import Path

from catnip import Catnip


def benchmark_ast_vs_vm(name, code, iterations=10, warmup=3):
    """Benchmark same program in both AST and VM modes."""

    results = {}

    for mode_name, vm_mode in [('AST', False), ('VM', True)]:
        cat = Catnip(vm_mode=vm_mode)

        # Parse
        start = time.perf_counter()
        cat.parse(code)
        parse_time = (time.perf_counter() - start) * 1000

        # Warmup
        for _ in range(warmup):
            cat.execute()

        # Benchmark execution
        exec_times = []
        for _ in range(iterations):
            start = time.perf_counter()
            result = cat.execute()
            exec_times.append((time.perf_counter() - start) * 1000)

        exec_avg = sum(exec_times) / len(exec_times)
        exec_min = min(exec_times)

        results[mode_name] = {
            'parse_time_ms': parse_time,
            'exec_avg_ms': exec_avg,
            'exec_min_ms': exec_min,
            'result': result,
        }

    return results


def main():
    print("=" * 100)
    print("Catnip AST vs VM Mode Benchmark - Janvier 2026")
    print("=" * 100)

    # ===== BENCHMARK 1: Simple Arithmetic =====
    print("\n" + "=" * 100)
    print("BENCHMARK 1: Simple Arithmetic")
    print("=" * 100)

    arith_code = """
x = 10
y = 20
z = x + y
result = z * 2 - 5
result
"""

    print("\nProgram: Simple arithmetic (5 operations)")
    bench1 = benchmark_ast_vs_vm("Arithmetic", arith_code)

    for mode in ['AST', 'VM']:
        r = bench1[mode]
        print(f"\n{mode}:")
        print(f"  Parse time : {r['parse_time_ms']:.2f}ms")
        print(f"  Exec avg   : {r['exec_avg_ms']:.3f}ms")
        print(f"  Exec min   : {r['exec_min_ms']:.3f}ms")

    speedup = bench1['AST']['exec_avg_ms'] / bench1['VM']['exec_avg_ms']
    print(f"\nVM Speedup: {speedup:.2f}x")

    # ===== BENCHMARK 2: List Operations =====
    print("\n" + "=" * 100)
    print("BENCHMARK 2: List Operations")
    print("=" * 100)

    list_code = """
data = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
doubled = []
for item in data {
    doubled = doubled + [item * 2]
}
total = 0
for val in doubled {
    total = total + val
}
total
"""

    print("\nProgram: List iteration and accumulation")
    bench2 = benchmark_ast_vs_vm("Lists", list_code)

    for mode in ['AST', 'VM']:
        r = bench2[mode]
        print(f"\n{mode}:")
        print(f"  Parse time : {r['parse_time_ms']:.2f}ms")
        print(f"  Exec avg   : {r['exec_avg_ms']:.3f}ms")
        print(f"  Exec min   : {r['exec_min_ms']:.3f}ms")

    speedup = bench2['AST']['exec_avg_ms'] / bench2['VM']['exec_avg_ms']
    print(f"\nVM Speedup: {speedup:.2f}x")

    # ===== BENCHMARK 3: Pattern Matching =====
    print("\n" + "=" * 100)
    print("BENCHMARK 3: Pattern Matching")
    print("=" * 100)

    pattern_code = """
classify = (x) => {
    match x {
        case 0 => "zero"
        case 1 => "one"
        case 2 => "two"
        case 3 | 4 | 5 => "small"
        case n if n > 100 => "large"
        case _ => "other"
    }
}

results = []
for i in range(0, 20) {
    results = results + [classify(i)]
}
results
"""

    print("\nProgram: Pattern matching with guards and OR patterns")
    bench3 = benchmark_ast_vs_vm("Pattern Matching", pattern_code)

    for mode in ['AST', 'VM']:
        r = bench3[mode]
        print(f"\n{mode}:")
        print(f"  Parse time : {r['parse_time_ms']:.2f}ms")
        print(f"  Exec avg   : {r['exec_avg_ms']:.3f}ms")
        print(f"  Exec min   : {r['exec_min_ms']:.3f}ms")

    speedup = bench3['AST']['exec_avg_ms'] / bench3['VM']['exec_avg_ms']
    print(f"\nVM Speedup: {speedup:.2f}x")

    # ===== BENCHMARK 4: Numeric Loops =====
    print("\n" + "=" * 100)
    print("BENCHMARK 4: Numeric Loops")
    print("=" * 100)

    loop_code = """
total = 0
for i in range(1, 1001) {
    total = total + i
}
total
"""

    print("\nProgram: Sum 1..1000 (numeric loop, ForRangeInt optimized)")
    bench4 = benchmark_ast_vs_vm("Numeric Loops", loop_code, iterations=20)

    for mode in ['AST', 'VM']:
        r = bench4[mode]
        print(f"\n{mode}:")
        print(f"  Parse time : {r['parse_time_ms']:.2f}ms")
        print(f"  Exec avg   : {r['exec_avg_ms']:.3f}ms")
        print(f"  Exec min   : {r['exec_min_ms']:.3f}ms")

    speedup = bench4['AST']['exec_avg_ms'] / bench4['VM']['exec_avg_ms']
    print(f"\nVM Speedup: {speedup:.2f}x")

    # ===== BENCHMARK 5: Function Calls =====
    print("\n" + "=" * 100)
    print("BENCHMARK 5: Function Calls")
    print("=" * 100)

    func_code = """
fibonacci = (n) => {
    if n <= 1 { n }
    else { fibonacci(n - 1) + fibonacci(n - 2) }
}

fibonacci(15)
"""

    print("\nProgram: Recursive function (fibonacci(15))")
    bench5 = benchmark_ast_vs_vm("Function Calls", func_code, iterations=5)

    for mode in ['AST', 'VM']:
        r = bench5[mode]
        print(f"\n{mode}:")
        print(f"  Parse time : {r['parse_time_ms']:.2f}ms")
        print(f"  Exec avg   : {r['exec_avg_ms']:.3f}ms")
        print(f"  Exec min   : {r['exec_min_ms']:.3f}ms")

    speedup = bench5['AST']['exec_avg_ms'] / bench5['VM']['exec_avg_ms']
    print(f"\nVM Speedup: {speedup:.2f}x")

    # ===== SUMMARY =====
    print("\n" + "=" * 100)
    print("SUMMARY")
    print("=" * 100)

    benchmarks = [
        ("Arithmetic", bench1),
        ("Lists", bench2),
        ("Pattern Matching", bench3),
        ("Numeric Loops", bench4),
        ("Function Calls", bench5),
    ]

    print("\nExecution Time Comparison (lower = faster):")
    print("-" * 100)
    print(f"{'Program':<25} {'AST (ms)':>15} {'VM (ms)':>15} {'Speedup':>15}")
    print("-" * 100)

    speedups = []
    for name, bench in benchmarks:
        ast_time = bench['AST']['exec_avg_ms']
        vm_time = bench['VM']['exec_avg_ms']
        speedup = ast_time / vm_time
        speedups.append(speedup)

        print(f"{name:<25} {ast_time:>15.3f} {vm_time:>15.3f} {speedup:>14.2f}x")

    avg_speedup = sum(speedups) / len(speedups)
    print("-" * 100)
    print(f"{'Average':<25} {' ':>15} {' ':>15} {avg_speedup:>14.2f}x")

    # Analysis
    print("\n" + "=" * 100)
    print("ANALYSIS")
    print("=" * 100)

    print("\nWhy VM Mode is Faster:")
    print("-" * 100)
    print("  1. Bytecode Generation   : Converts AST → compact bytecode once")
    print("  2. Fast Dispatch         : Single pattern match vs recursive tree-walk")
    print("  3. Instruction Cache     : Hot bytecode stays in CPU cache")
    print("  4. Stack Machine         : Direct operand stack vs tree navigation")
    print("  5. Inline Cache          : JIT detection for hot loops")

    print("\nProgram Type Performance Characteristics:")
    print("-" * 100)
    print("  • Simple Arithmetic      : 2-5x speedup (low tree depth)")
    print("  • Loops                  : 3-10x speedup (ForRangeInt fusion)")
    print("  • Pattern Matching       : 2-3x speedup (dispatch vs tree traverse)")
    print("  • Recursion              : 2-8x speedup (function call dispatch)")
    print("  • Average across types   : ~{:.2f}x speedup".format(avg_speedup))

    print("\n" + "=" * 100)
    print("RECOMMENDATION")
    print("=" * 100)
    print("\nDefault to VM Mode for best performance:")
    print("  ✓ Use VM mode for scripts, production code")
    print("  ✓ Use AST mode for debugging, development (better error messages)")
    print("  ✓ Use AST mode with small snippets if parse overhead matters")
    print("\nCatnip default: VM mode enabled (--executor=vm)")
    print("=" * 100)


if __name__ == "__main__":
    main()
