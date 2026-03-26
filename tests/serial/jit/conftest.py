# FILE: tests/serial/jit/conftest.py
"""
Fixtures communes pour les tests JIT.
"""

import os

import pytest

# Force VM mode for all JIT tests
os.environ['CATNIP_EXECUTOR'] = 'vm'

from catnip._rs import VM, Compiler

from catnip import Catnip


@pytest.fixture
def vm_with_jit():
    """VM instance with JIT enabled."""
    c = Catnip(vm_mode='on')
    vm = VM()
    vm.set_context(c.context)
    vm.enable_jit()
    return vm, c


@pytest.fixture
def vm_without_jit():
    """VM instance with JIT disabled (for comparison)."""
    c = Catnip(vm_mode='on')
    vm = VM()
    vm.set_context(c.context)
    return vm, c


@pytest.fixture
def compiler():
    """Bytecode compiler instance."""
    return Compiler()


@pytest.fixture
def catnip_with_jit():
    """Catnip instance with JIT enabled for easy testing."""

    class CatnipJIT:
        def __init__(self):
            self.catnip = Catnip(vm_mode='on')
            self.vm = VM()
            self.vm.set_context(self.catnip.context)
            self.vm.enable_jit()
            self.compiler = Compiler()

        def eval(self, code_str):
            """Compile and execute code with JIT."""
            self.catnip.parse(code_str)
            bytecode = self.compiler.compile(self.catnip.code)
            return self.vm.execute(bytecode, (), {}, None)

    return CatnipJIT()


def compile_code(code_str):
    """Helper to compile Catnip code to bytecode."""
    c = Catnip(vm_mode='on')
    c.parse(code_str)
    compiler = Compiler()
    return compiler.compile(c.code)


def jit_eval(code_str):
    """Execute code through the full pipeline with JIT enabled."""
    c = Catnip(vm_mode='on')
    c.pragma_context.jit_enabled = True
    c.pragma_context.jit_all = True
    c.parse(code_str)
    return c.execute()
