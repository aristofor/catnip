#!/usr/bin/env python3
"""JIT compilation benchmark for Catnip.

Demonstrates the trace-based JIT compiler using Cranelift.
Compares interpreter vs JIT-compiled execution for hot loops and hot functions.

The JIT compiler supports:
- Loops: Int, float, boolean types, nested loops, conditional branches
- Functions : Recursive and non-recursive functions called frequently
- Typical speedups: 100-200x on numeric loops, 1.1x on simple functions

Usage:
    python docs/examples/performance/jit_benchmark.py
"""

import time


def benchmark_sum_loop():
    """Compare interpreter vs JIT for a simple sum loop."""
    from catnip import Catnip
    from catnip._rs import Compiler

    code = """
    total = 0
    for i in range(1, 100001) {
        total = total + i
    }
    total
    """

    # Parse and compile to bytecode
    c = Catnip(vm_mode='on')
    ast = c.parse(code)
    compiler = Compiler()
    bytecode = compiler.compile(ast)

    # Get VM
    from catnip._rs import VM
    vm = VM()
    vm.set_context(c.context)

    # Warm up (also triggers JIT compilation)
    vm.execute(bytecode, (), {}, None)

    # Without JIT
    vm.disable_jit()
    start = time.perf_counter()
    result_interp = vm.execute(bytecode, (), {}, None)
    interp_time = (time.perf_counter() - start) * 1000

    # With JIT
    vm.enable_jit()
    start = time.perf_counter()
    result_jit = vm.execute(bytecode, (), {}, None)
    jit_time = (time.perf_counter() - start) * 1000

    # Get stats
    stats = vm.get_jit_stats()

    print("Sum(1..100000) Benchmark")
    print("=" * 40)
    print(f"Result: {result_jit} (expected: 5000050000)")
    print(f"Interpreter: {interp_time:.2f}ms")
    print(f"JIT:         {jit_time:.2f}ms")
    if jit_time > 0:
        print(f"Speedup:     {interp_time / jit_time:.1f}x")
    print()
    print("JIT Stats:")
    for key, value in stats.items():
        print(f"  {key}: {value}")


def benchmark_float_loop():
    """Benchmark float accumulation."""
    from catnip import Catnip
    from catnip._rs import Compiler, VM

    code = """
    x = 0.0
    for i in range(1000000) {
        x = x + 1.5
    }
    x
    """

    c = Catnip(vm_mode='on')
    ast = c.parse(code)
    compiler = Compiler()
    bytecode = compiler.compile(ast)

    vm = VM()
    vm.set_context(c.context)
    vm.execute(bytecode, (), {}, None)

    # Without JIT
    vm.disable_jit()
    start = time.perf_counter()
    result_interp = vm.execute(bytecode, (), {}, None)
    interp_time = (time.perf_counter() - start) * 1000

    # With JIT
    vm.enable_jit()
    start = time.perf_counter()
    result_jit = vm.execute(bytecode, (), {}, None)
    jit_time = (time.perf_counter() - start) * 1000

    print()
    print("Float Accumulation (1M iterations)")
    print("=" * 40)
    print(f"Result: {result_jit} (expected: 1500000.0)")
    print(f"Interpreter: {interp_time:.2f}ms")
    print(f"JIT:         {jit_time:.2f}ms")
    if jit_time > 0:
        print(f"Speedup:     {interp_time / jit_time:.1f}x")


def benchmark_conditional_loop():
    """Benchmark loop with conditional branch (side exits)."""
    from catnip import Catnip
    from catnip._rs import Compiler, VM

    code = """
    count = 0
    for i in range(1000000) {
        if i > 500000 {
            count = count + 1
        }
    }
    count
    """

    c = Catnip(vm_mode='on')
    ast = c.parse(code)
    compiler = Compiler()
    bytecode = compiler.compile(ast)

    vm = VM()
    vm.set_context(c.context)
    vm.execute(bytecode, (), {}, None)

    # Without JIT
    vm.disable_jit()
    start = time.perf_counter()
    result_interp = vm.execute(bytecode, (), {}, None)
    interp_time = (time.perf_counter() - start) * 1000

    # With JIT
    vm.enable_jit()
    start = time.perf_counter()
    result_jit = vm.execute(bytecode, (), {}, None)
    jit_time = (time.perf_counter() - start) * 1000

    print()
    print("Conditional Loop (1M iterations, 50% side exits)")
    print("=" * 40)
    print(f"Result: {result_jit} (expected: 499999)")
    print(f"Interpreter: {interp_time:.2f}ms")
    print(f"JIT:         {jit_time:.2f}ms")
    if jit_time > 0:
        speedup = interp_time / jit_time
        print(f"Speedup:     {speedup:.2f}x")
        if speedup < 1:
            print("  Note: Side exits cause overhead when guards fail frequently")


def benchmark_function_loop():
    """Benchmark loop inside a function (tests JIT for function scopes)."""
    from catnip import Catnip
    from catnip._rs import Compiler, VM

    code = """
    counter = (n) => {
        i = 0
        sum = 0
        while i < n {
            sum = sum + 1
            i = i + 1
        }
        sum
    }
    result = counter(1000000)
    result
    """

    c = Catnip(vm_mode='on')
    ast = c.parse(code)
    compiler = Compiler()
    bytecode = compiler.compile(ast)

    vm = VM()
    vm.set_context(c.context)
    vm.execute(bytecode, (), {}, None)

    # Without JIT
    vm.disable_jit()
    start = time.perf_counter()
    result_interp = vm.execute(bytecode, (), {}, None)
    interp_time = (time.perf_counter() - start) * 1000

    # With JIT
    vm.enable_jit()
    start = time.perf_counter()
    result_jit = vm.execute(bytecode, (), {}, None)
    jit_time = (time.perf_counter() - start) * 1000

    print()
    print("Function Loop (1M iterations inside function)")
    print("=" * 40)
    print(f"Result: {result_jit} (expected: 1000000)")
    print(f"Interpreter: {interp_time:.2f}ms")
    print(f"JIT:         {jit_time:.2f}ms")
    if jit_time > 0:
        print(f"Speedup:     {interp_time / jit_time:.1f}x")


def benchmark_hot_function():
    """Benchmark JIT compilation of frequently called functions."""
    from catnip import Catnip
    from catnip._rs import Compiler, VM

    code = """
    square = (x) => { x * x }

    result = 0
    for i in range(10000) {
        result = square(i)
    }
    result
    """

    c = Catnip(vm_mode='on')
    ast = c.parse(code)
    compiler = Compiler()
    bytecode = compiler.compile(ast)

    vm = VM()
    vm.set_context(c.context)
    vm.execute(bytecode, (), {}, None)

    # Without JIT
    vm.disable_jit()
    start = time.perf_counter()
    result_interp = vm.execute(bytecode, (), {}, None)
    interp_time = (time.perf_counter() - start) * 1000

    # With JIT
    vm.enable_jit()
    start = time.perf_counter()
    result_jit = vm.execute(bytecode, (), {}, None)
    jit_time = (time.perf_counter() - start) * 1000

    print()
    print("Hot Function JIT (function called 10k times)")
    print("=" * 40)
    print(f"Result: {result_jit} (expected: 99980001)")
    print(f"Interpreter: {interp_time:.2f}ms")
    print(f"JIT:         {jit_time:.2f}ms")
    if jit_time > 0:
        speedup = interp_time / jit_time
        print(f"Speedup:     {speedup:.2f}x")
        print()
        print("Note: Modest speedup (~1.1x) due to boxing/unboxing overhead.")
        print("      Functions with loops inside benefit more from loop JIT.")


def benchmark_recursive_function():
    """Benchmark recursive function compilation."""
    from catnip import Catnip
    from catnip._rs import Compiler, VM

    code = """
    {
        factorial = (n) => {
            if n <= 1 {
                1
            } else {
                n * factorial(n - 1)
            }
        }

        result = 0
        i = 0
        while i < 120 {
            result = factorial(5)
            i = i + 1
        }
        result
    }
    """

    # Parse and compile
    c = Catnip(vm_mode='on')
    ast = c.parse(code)
    compiler = Compiler()
    bytecode = compiler.compile(ast)

    # Without JIT
    vm_nojit = VM()
    vm_nojit.set_context(c.context)
    start = time.perf_counter()
    result_interp = vm_nojit.execute(bytecode, (), {}, None)
    interp_time = (time.perf_counter() - start) * 1000

    # With JIT (need fresh VM)
    vm_jit = VM()
    vm_jit.set_context(c.context)
    vm_jit.enable_jit()
    start = time.perf_counter()
    result_jit = vm_jit.execute(bytecode, (), {}, None)
    jit_time = (time.perf_counter() - start) * 1000

    stats = vm_jit.get_jit_stats()

    print("Recursive Function (factorial) Benchmark")
    print("=" * 40)
    print(f"Result: {result_jit} (expected: 120)")
    print(f"Calls: 120 × factorial(5) = 600 recursive calls")
    print(f"Interpreter: {interp_time:.2f}ms")
    print(f"JIT:         {jit_time:.2f}ms")
    if jit_time > 0:
        print(f"Speedup:     {interp_time / jit_time:.1f}x")
    print()
    print("Note: Recursive calls compiled via CallSelf with NaN re-boxing.")
    print("      Depth > MAX_RECURSION_DEPTH (10000) triggers graceful fallback.")


def show_jit_info():
    """Display JIT configuration information."""
    from catnip._rs import VM

    vm = VM()
    print("JIT Configuration")
    print("=" * 40)
    print(f"JIT enabled: {vm.is_jit_enabled()}")
    print("Backend: Cranelift (x86-64)")
    print("Strategy: Trace-based compilation")
    print("Hot threshold: 100 iterations/calls")
    print("Supports:")
    print("  - Loops (for/while)")
    print("  - Functions (recursive and non-recursive)")
    print("Scope support: Module-level and function-local variables")
    print()


if __name__ == "__main__":
    show_jit_info()
    benchmark_sum_loop()
    benchmark_float_loop()
    benchmark_conditional_loop()
    benchmark_function_loop()
    benchmark_hot_function()
    benchmark_recursive_function()
