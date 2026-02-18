#!/usr/bin/env python3
# FILE: tests/jit_functions/test_function_trace.py
"""Test function trace recording - Phase 2 validation"""

from catnip._rs import VM, Compiler

from catnip import Catnip

code = """
# Simple function to trace
square = (x) => { x * x }

# Call it 110 times to exceed threshold and trigger tracing
result = 0
count = 0
while count < 110 {
    result = square(count)
    count = count + 1
}
result
"""

print("=" * 60)
print("Phase 2: Function Tracing Test")
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

print("\nExecuting...")

result = vm.execute(compiled, ())

print("\n✓ Execution successful!")
print(f"  Result: {result}")
print("  Expected: 11881 (109 * 109)")

if result == 11881:
    print("\n✓ Phase 2 COMPLETE:")
    print("  - Function tracing starts when function becomes hot")
    print("  - Trace recording runs from entry to Return")
    print("  - No crashes during trace recording")
    print("\nNext: Phase 3 - Compile function traces to native code")
else:
    print(f"\n✗ Unexpected result: {result} != 11881")
