#!/usr/bin/env python3
# FILE: tests/jit_functions/test_function_compile_verbose.py
"""Test function trace compilation with verbose output"""

from catnip._rs import VM, Compiler

from catnip import Catnip

code = """
# Simple function to compile
square = (x) => { x * x }

# Call it 110 times to exceed threshold and trigger compilation
result = 0
count = 0
while count < 110 {
    result = square(count)
    count = count + 1
}
result
"""

print("=" * 60)
print("Phase 3: Function Trace Compilation (Verbose)")
print("=" * 60)

# Parse and compile
c = Catnip(vm_mode='on')
ast = c.parse(code)
compiler = Compiler()
compiled = compiler.compile(ast)

# Execute with JIT enabled
vm = VM()
vm.set_context(c.context)
vm.enable_jit()
# Note: Tracing is enabled at Rust level if CATNIP_JIT_TRACE env var is set

print("\nExecuting with JIT compilation...\n")

result = vm.execute(compiled, ())

print(f"\n{'=' * 60}")
print(f"Result: {result}")
print("Expected: 11881 (109 * 109)")
print(f"Status: {'✓ PASS' if result == 11881 else '✗ FAIL'}")
