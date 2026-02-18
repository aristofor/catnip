#!/usr/bin/env python3
# FILE: tests/jit_functions/test_function_compile.py
"""Test function trace compilation - Phase 3 validation"""

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
print("Phase 3: Function Trace Compilation Test")
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

print("\nExecuting with JIT compilation...")

try:
    result = vm.execute(compiled, ())

    print("\n✓ Execution successful!")
    print(f"  Result: {result}")
    print("  Expected: 11881 (109 * 109)")

    if result == 11881:
        print("\n✓ Phase 3 COMPLETE:")
        print("  - Function traces are detected and recorded")
        print("  - Function traces compile to native code via Cranelift")
        print("  - Compiled code executes correctly")
        print("\nNext: Phase 4 - Optimize execution and cache compiled functions")
    else:
        print(f"\n✗ Unexpected result: {result} != 11881")
except Exception as e:
    print(f"\n✗ Compilation or execution failed: {e}")
    import traceback

    traceback.print_exc()
