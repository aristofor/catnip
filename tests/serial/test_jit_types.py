# FILE: tests/serial/test_jit_types.py
"""Tests for JIT type support (int, float, bool, nested loops)."""

import platform

import pytest
from catnip._rs import VM, Compiler

from catnip import Catnip

# Skip if not x86-64
pytestmark = pytest.mark.skipif(
    platform.machine().lower() not in ("x86_64", "amd64"),
    reason=f"JIT only supports x86-64, got {platform.machine()}",
)


def compile_code(code_str):
    """Helper to compile Catnip code to bytecode."""
    c = Catnip(vm_mode='on')
    ast = c.parse(code_str)
    compiler = Compiler()
    return compiler.compile(ast)


class TestJITTypes:
    """Test JIT compilation with different types."""

    def test_float_accumulation(self):
        """Test JIT compiles float accumulation loop."""
        c = Catnip(vm_mode='on')
        vm = VM()
        vm.set_context(c.context)
        vm.enable_jit()

        code = compile_code('''
        {
            x = 0.0
            for i in range(2000) {
                x = x + 1.5
            }
            x
        }
        ''')

        result = vm.execute(code, (), {}, None)
        expected = 2000 * 1.5

        assert result == expected

        stats = vm.get_jit_stats()
        assert stats['compiled_loops'] >= 1, "Float loop should be compiled"

    def test_float_multiplication(self):
        """Test JIT compiles float multiplication."""
        c = Catnip(vm_mode='on')
        vm = VM()
        vm.set_context(c.context)
        vm.enable_jit()

        code = compile_code('''
        {
            x = 1.0
            for i in range(2000) {
                x = x * 1.001
            }
            x
        }
        ''')

        result = vm.execute(code, (), {}, None)
        # x = 1.001^2000
        expected = 1.001**2000

        # Float comparison with tolerance
        assert abs(result - expected) / expected < 1e-10

        stats = vm.get_jit_stats()
        assert stats['compiled_loops'] >= 1

    def test_mixed_int_float(self):
        """Test loop with both int counter and float accumulator."""
        c = Catnip(vm_mode='on')
        vm = VM()
        vm.set_context(c.context)
        vm.enable_jit()

        code = compile_code('''
        {
            sum = 0.0
            for i in range(2000) {
                sum = sum + 0.5
            }
            sum
        }
        ''')

        result = vm.execute(code, (), {}, None)
        expected = 2000 * 0.5

        assert result == expected

        stats = vm.get_jit_stats()
        assert stats['compiled_loops'] >= 1

    def test_int_loop_still_works(self):
        """Verify int-only loops still compile."""
        c = Catnip(vm_mode='on')
        vm = VM()
        vm.set_context(c.context)
        vm.enable_jit()

        code = compile_code('''
        {
            x = 0
            for i in range(2000) {
                x = x + 1
            }
            x
        }
        ''')

        result = vm.execute(code, (), {}, None)
        assert result == 2000

        stats = vm.get_jit_stats()
        assert stats['compiled_loops'] >= 1

    def test_nested_loops(self):
        """Test nested loops - inner loop should be compiled."""
        c = Catnip(vm_mode='on')
        vm = VM()
        vm.set_context(c.context)
        vm.enable_jit()

        code = compile_code('''
        {
            sum = 0
            for i in range(100) {
                for j in range(100) {
                    sum = sum + 1
                }
            }
            sum
        }
        ''')

        result = vm.execute(code, (), {}, None)
        assert result == 100 * 100

        stats = vm.get_jit_stats()
        # Inner loop should be compiled
        assert stats['compiled_loops'] >= 1

    def test_boolean_variable(self):
        """Test JIT with boolean variable in condition."""
        c = Catnip(vm_mode='on')
        vm = VM()
        vm.set_context(c.context)
        vm.enable_jit()

        code = compile_code('''
        {
            count = 0
            flag = True
            for i in range(2000) {
                if flag {
                    count = count + 1
                }
            }
            count
        }
        ''')

        result = vm.execute(code, (), {}, None)
        assert result == 2000

        stats = vm.get_jit_stats()
        assert stats['compiled_loops'] >= 1
