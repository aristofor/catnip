# FILE: tests/optimization/test_cse.py
"""Integration tests for Common Subexpression Elimination (CSE)."""

import pytest

from catnip import Catnip
from catnip.nodes import Op
from catnip.semantic.opcode import OpCode


def _cat(executor: str, optimize: int = 2) -> Catnip:
    vm_mode = "on" if executor == "vm" else "off"
    return Catnip(vm_mode=vm_mode, optimize=optimize)


@pytest.mark.parametrize("executor", ["vm", "ast"])
def test_cse_preserves_semantics(executor):
    """CSE keeps observable behavior identical."""
    cat = _cat(executor, optimize=2)

    cat.parse("""
        {
            a = 7
            b = 4
            result = a * b + a * b + a * b
        }
        """)
    result1 = cat.execute()

    cat = _cat(executor, optimize=2)
    cat.parse("""
        {
            a = 7
            b = 4
            result = a * b * 3
        }
        """)
    result2 = cat.execute()

    assert result1 == result2


@pytest.mark.parametrize("executor", ["vm", "ast"])
def test_cse_block_pipeline_smoke(executor):
    """Pipeline emits a CSE temp and still computes the expected value."""
    cat = _cat(executor, optimize=2)
    cat.parse("""
        {
            a = 10
            b = 5
            result = a + b * 2 + b * 2
            result
        }
        """)

    code = cat.code
    assert isinstance(code, list) and code
    block = code[0]
    assert isinstance(block, Op)
    assert block.ident == OpCode.OP_BLOCK

    has_cse_var = False
    for stmt in block.args:
        if isinstance(stmt, Op) and stmt.ident == OpCode.SET_LOCALS:
            names = stmt.args[0]
            if isinstance(names, tuple) and any(isinstance(name, str) and name.startswith("_cse_") for name in names):
                has_cse_var = True
                break

    assert has_cse_var
    assert cat.execute() == 30
