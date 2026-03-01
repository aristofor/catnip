#!/usr/bin/env python3
# FILE: tests/jit_functions/test_debug_jit.py
"""Debug JIT issue"""

from catnip._rs import VM, Compiler

from catnip import Catnip

code = """
square = (x) => { x * x }
result = square(10)
result
"""

print("Testing WITHOUT JIT...")
c1 = Catnip(vm_mode='on')
ast1 = c1.parse(code)
compiler1 = Compiler()
compiled1 = compiler1.compile(ast1)

vm1 = VM()
vm1.set_context(c1.context)

result1 = vm1.execute(compiled1, ())
print(f"Result: {result1}")
print("Expected: 100\n")

print("Testing WITH JIT...")
c2 = Catnip(vm_mode='on')
ast2 = c2.parse(code)
compiler2 = Compiler()
compiled2 = compiler2.compile(ast2)

vm2 = VM()
vm2.set_context(c2.context)
vm2.enable_jit()

# Call multiple times to trigger compilation
code_loop = """
square = (x) => { x * x }
result = 0
count = 0
while count < 110 {
    result = square(10)
    count = count + 1
}
result
"""

c3 = Catnip(vm_mode='on')
ast3 = c3.parse(code_loop)
compiler3 = Compiler()
compiled3 = compiler3.compile(ast3)

vm3 = VM()
vm3.set_context(c3.context)
vm3.enable_jit()

result3 = vm3.execute(compiled3, ())
print(f"Result after JIT: {result3}")
print("Expected: 100")
