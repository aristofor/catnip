#!/usr/bin/env python3
"""
VM Mode Performance Benchmark

Demonstrates the performance difference between AST mode and VM mode.
VM mode offers ~190x speedup on iterative loops by eliminating Python/VM
boundary crossings.
"""
import time
from catnip import Catnip


def benchmark(code: str, iterations: int = 3) -> dict:
    """Run benchmark in both AST and VM modes."""
    results = {}

    # AST mode (default)
    cat_ast = Catnip(vm_mode="off")
    cat_ast.parse(code)

    times_ast = []
    for _ in range(iterations):
        start = time.perf_counter()
        result_ast = cat_ast.execute()
        end = time.perf_counter()
        times_ast.append((end - start) * 1000)

    results["ast"] = {
        "result": result_ast,
        "time_ms": min(times_ast),
        "times": times_ast,
    }

    # VM mode
    cat_vm = Catnip(vm_mode="on")
    cat_vm.parse(code)

    times_vm = []
    for _ in range(iterations):
        start = time.perf_counter()
        result_vm = cat_vm.execute()
        end = time.perf_counter()
        times_vm.append((end - start) * 1000)

    results["vm"] = {
        "result": result_vm,
        "time_ms": min(times_vm),
        "times": times_vm,
    }

    results["speedup"] = results["ast"]["time_ms"] / results["vm"]["time_ms"]

    return results


if __name__ == "__main__":
    print("VM Mode Performance Benchmark")
    print("=" * 60)

    # Benchmark 1: Simple loop
    print("\n1. Simple Loop (sum 1..100000)")
    code_sum = """
total = 0
for i in range(1, 100001) {
    total = total + i
}
total
"""
    results = benchmark(code_sum)
    print(f"   AST mode:  {results['ast']['result']:>12} in {results['ast']['time_ms']:>8.2f}ms")
    print(f"   VM mode:   {results['vm']['result']:>12} in {results['vm']['time_ms']:>8.2f}ms")
    print(f"   Speedup:   {results['speedup']:>12.1f}x")

    # Benchmark 2: Nested loops
    print("\n2. Nested Loops (10x1000 iterations)")
    code_nested = """
total = 0
for i in range(1, 11) {
    for j in range(1, 1001) {
        total = total + 1
    }
}
total
"""
    results = benchmark(code_nested)
    print(f"   AST mode:  {results['ast']['result']:>12} in {results['ast']['time_ms']:>8.2f}ms")
    print(f"   VM mode:   {results['vm']['result']:>12} in {results['vm']['time_ms']:>8.2f}ms")
    print(f"   Speedup:   {results['speedup']:>12.1f}x")

    # Benchmark 3: Arithmetic operations
    print("\n3. Arithmetic (50000 iterations)")
    code_arith = """
result = 0
for i in range(1, 50001) {
    result = result + (i * 2)
}
result
"""
    results = benchmark(code_arith)
    print(f"   AST mode:  {results['ast']['result']:>12} in {results['ast']['time_ms']:>8.2f}ms")
    print(f"   VM mode:   {results['vm']['result']:>12} in {results['vm']['time_ms']:>8.2f}ms")
    print(f"   Speedup:   {results['speedup']:>12.1f}x")

    print("\n" + "=" * 60)
    print("Recommendation: Use --vm on for compute-intensive scripts")
    print("Example: catnip --vm on script.cat")
