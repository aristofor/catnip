# FILE: tests/optimization/test_cse.py
"""Integration tests for Common Subexpression Elimination (CSE)."""

import pytest

from catnip import Catnip


def _cat(executor: str, optimize: int = 2) -> Catnip:
    vm_mode = "on" if executor == "vm" else "off"
    return Catnip(vm_mode=vm_mode, optimize=optimize)


@pytest.mark.parametrize("executor", ["vm", "ast"])
def test_cse_block_pipeline_smoke(executor):
    """Pipeline optimizes block and computes the expected value."""
    cat = _cat(executor, optimize=2)
    code = """
        {
            a = 10
            b = 5
            result = a + b * 2 + b * 2
            result
        }
        """
    cat.parse(code)

    # Verify IR is produced; the block is preserved (it declares locals,
    # flattening it would leak the bindings into the enclosing scope)
    ir = cat._pipeline.get_prepared_ir_nodes()
    assert ir
    assert any(n.kind == "Op" and n.opcode == "OpBlock" for n in ir)

    assert cat.execute() == 30
