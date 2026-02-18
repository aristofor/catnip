#!/usr/bin/env python3
"""
Constant and Copy Propagation Benchmark - Janvier 2026

Mesure l'impact potentiel de la propagation de constantes et copies sur:
1. Code size reduction (fewer variable assignments)
2. Execution performance (fewer lookups)
3. Combined effect with dead code elimination

Note: Les passes actuelles sont des stubs (retournent l'IR inchangé).
Ce benchmark montre les patterns que nous optimiserions une fois implémentés.

Patterns optimisés:
- Constant propagation: x = 42; y = x + 1 → y = 42 + 1
- Copy propagation: x = y; z = x → z = y
- Combined: Élimine les variables intermédiaires
"""

import time
import sys
from pathlib import Path

from catnip import Catnip


def benchmark_program(name, code, iterations=10, warmup=3):
    """Benchmark a Catnip program."""
    cat = Catnip(vm_mode='on')

    # Parse and compile
    start = time.perf_counter()
    cat.parse(code)
    parse_time = (time.perf_counter() - start) * 1000

    # Count IR nodes as complexity proxy
    ir_node_count = len(cat.code)

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

    return {
        'name': name,
        'parse_time_ms': parse_time,
        'exec_avg_ms': exec_avg,
        'ir_nodes': ir_node_count,
        'result': result,
    }


def main():
    print("=" * 80)
    print("Constant and Copy Propagation Benchmark - Janvier 2026")
    print("=" * 80)

    # ===== BENCHMARK 1: Constant Propagation Pattern =====
    print("\n" + "=" * 80)
    print("BENCHMARK 1: Constant Propagation Pattern")
    print("=" * 80)

    # Code with constant assignments propagated
    constant_prog = """
# Constant propagation candidates
const_pi = 3.14159
const_g = 9.81
const_scale = 100

# Uses of constants (would be replaced by propagation)
radius = const_pi * 2
gravity = const_g * const_scale
area = const_pi * (radius / 2)

result = radius + gravity + area
"""

    print("\nProgram structure:")
    print("  • 3 constants assigned to variables")
    print("  • Constants used in 3 expressions")
    print("  • Candidate for optimization: ~6 references → 3 constants")

    print("\nCompiling and executing...")
    bench1 = benchmark_program("Constant Propagation", constant_prog)

    print(f"\nResults:")
    print(f"  • IR nodes: {bench1['ir_nodes']}")
    print(f"  • Parse time: {bench1['parse_time_ms']:.2f}ms")
    print(f"  • Execution avg: {bench1['exec_avg_ms']:.3f}ms")
    print(f"  • Result: {bench1['result']:.2f}")

    # ===== BENCHMARK 2: Copy Propagation Pattern =====
    print("\n" + "=" * 80)
    print("BENCHMARK 2: Copy Propagation Pattern")
    print("=" * 80)

    # Code with copy assignments that could be eliminated
    copy_prog = """
# Copy propagation candidates
x = 10
y = x          # Simple copy - could be eliminated
z = y + 5      # References copy - could use x directly
w = y * 2      # Another reference - could use x

total = z + w
"""

    print("\nProgram structure:")
    print("  • 1 original value (x)")
    print("  • 1 copy assignment (y = x)")
    print("  • 2 uses of copy (z uses y, w uses y)")
    print("  • Optimization potential: y = x can be eliminated if DCE follows")

    print("\nCompiling and executing...")
    bench2 = benchmark_program("Copy Propagation", copy_prog)

    print(f"\nResults:")
    print(f"  • IR nodes: {bench2['ir_nodes']}")
    print(f"  • Parse time: {bench2['parse_time_ms']:.2f}ms")
    print(f"  • Execution avg: {bench2['exec_avg_ms']:.3f}ms")
    print(f"  • Result: {bench2['result']}")

    # ===== BENCHMARK 3: Combined Pattern =====
    print("\n" + "=" * 80)
    print("BENCHMARK 3: Combined Constant + Copy Propagation")
    print("=" * 80)

    # Combined pattern: constants propagated through copies
    combined_prog = """
# Combined optimization pattern
scale = 10        # Constant assignment
offset = scale    # Copy of constant
factor = 5        # Another constant
temp = factor     # Copy of constant

# Chain of uses
result1 = temp * 2
result2 = offset + result1
result3 = result2 * 2

# Would optimize to: result3 = ((5 * 2) + 10) * 2 = 60
final = result3
"""

    print("\nProgram structure:")
    print("  • 2 constants (scale=10, factor=5)")
    print("  • 2 copy assignments (offset=scale, temp=factor)")
    print("  • Chain of operations on copies")
    print("  • Optimization: propagate constants through copies, fold result")

    print("\nCompiling and executing...")
    bench3 = benchmark_program("Combined Propagation", combined_prog)

    print(f"\nResults:")
    print(f"  • IR nodes: {bench3['ir_nodes']}")
    print(f"  • Parse time: {bench3['parse_time_ms']:.2f}ms")
    print(f"  • Execution avg: {bench3['exec_avg_ms']:.3f}ms")
    print(f"  • Result: {bench3['result']}")

    # ===== BENCHMARK 4: Loop with Propagation =====
    print("\n" + "=" * 80)
    print("BENCHMARK 4: Loop with Constant Propagation")
    print("=" * 80)

    # Constants used in loops are good candidates
    loop_prog = """
# Constants used in loop
step = 10
multiplier = 2
limit = 100

total = 0
i = 0
while i < limit {
    total = total + (i * multiplier)
    i = i + step
}

total
"""

    print("\nProgram structure:")
    print("  • 3 constants (step, multiplier, limit)")
    print("  • Loop using constants in each iteration")
    print("  • Constants are re-evaluated, good for propagation")
    print("  • Benefit: reduce variable lookup overhead in loop body")

    print("\nCompiling and executing...")
    bench4 = benchmark_program("Loop Propagation", loop_prog)

    print(f"\nResults:")
    print(f"  • IR nodes: {bench4['ir_nodes']}")
    print(f"  • Parse time: {bench4['parse_time_ms']:.2f}ms")
    print(f"  • Execution avg: {bench4['exec_avg_ms']:.3f}ms")
    print(f"  • Result: {bench4['result']}")

    # ===== SUMMARY =====
    print("\n" + "=" * 80)
    print("SUMMARY")
    print("=" * 80)

    benchmarks = [bench1, bench2, bench3, bench4]

    print("\nIR Node Count (complexity):")
    print("-" * 80)
    for b in benchmarks:
        print(f"  {b['name']:35} : {b['ir_nodes']:4} IR nodes")

    print("\nParse Time (lower = better):")
    print("-" * 80)
    for b in benchmarks:
        print(f"  {b['name']:35} : {b['parse_time_ms']:7.2f}ms")

    print("\nExecution Time (lower = better, 10 iterations):")
    print("-" * 80)
    for b in benchmarks:
        print(f"  {b['name']:35} : {b['exec_avg_ms']:7.3f}ms avg")

    # Analysis
    print("\n" + "=" * 80)
    print("OPTIMIZATION POTENTIAL ANALYSIS")
    print("=" * 80)

    print("\nConstant Propagation Benefits:")
    print("-" * 80)
    print("  Pattern                         Benefit (estimated)")
    print("  • Numeric constants (+ usage)   2-5% faster (fewer lookups)")
    print("  • String constants              1-2% faster")
    print("  • Boolean constants             1-3% faster")
    print("  • General code                  0-1% faster")

    print("\nCopy Propagation Benefits:")
    print("-" * 80)
    print("  Pattern                         Benefit (estimated)")
    print("  • Single-use copies             1-3% faster (eliminated)")
    print("  • Multi-use copies              0-1% faster")
    print("  • With DCE                      5-10% code size reduction")

    print("\nCombined Effects:")
    print("-" * 80)
    print("  • Constant→Copy chain           3-8% faster (on worst-case)")
    print("  • With constant folding         5-15% on pure arithmetic")
    print("  • With DCE                      5-10% code size reduction")
    print("  • Average realistic code        1-2% improvement")

    print("\nCurrent Status:")
    print("-" * 80)
    print("  ✗ ConstantPropagationPass - Stub (returns IR unchanged)")
    print("  ✗ CopyPropagationPass - Stub (returns IR unchanged)")
    print("  ✓ Integrated in optimizer pipeline")
    print("  ✓ Ready for full implementation")

    print("\nNext Steps:")
    print("-" * 80)
    print("  1. Implement interior mutability (RefCell<HashMap>)")
    print("  2. Add two-pass approach if needed")
    print("  3. Add scope awareness for nested scopes")
    print("  4. Re-run this benchmark to measure actual gains")

    print("\n" + "=" * 80)


if __name__ == "__main__":
    main()
