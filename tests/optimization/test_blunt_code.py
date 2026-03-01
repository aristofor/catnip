# FILE: tests/optimization/test_blunt_code.py
"""Integration tests for blunt-code simplifications."""

import pytest

from catnip import Catnip


def _cat(executor: str, optimize: int = 2) -> Catnip:
    vm_mode = "on" if executor == "vm" else "off"
    return Catnip(vm_mode=vm_mode, optimize=optimize)


@pytest.mark.parametrize("executor", ["vm", "ast"])
def test_blunt_code_in_function(executor):
    """Simplifications are applied inside function bodies."""
    cat = _cat(executor, optimize=2)
    cat.parse("""
        f = (x) => {
            x + 0
        }
        f(42)
        """)
    assert cat.execute() == 42


@pytest.mark.parametrize("executor", ["vm", "ast"])
def test_blunt_code_in_loop(executor):
    """Simplifications in loops preserve semantics."""
    cat = _cat(executor, optimize=2)
    cat.parse("""
        sum = 0
        for i in range(5) {
            if i * 1 > 2 {
                sum = sum + i
            }
        }
        sum
        """)
    assert cat.execute() == 7


@pytest.mark.parametrize("executor", ["vm", "ast"])
def test_blunt_code_side_effect_case_executes(executor):
    """Side-effect expression executes in the full pipeline."""
    cat = _cat(executor, optimize=2)
    cat.parse("""
        count = 0
        increment = () => {
            count = count + 1
            count
        }
        result = 0 * increment()
        result
        """)
    assert cat.execute() == 0
