# FILE: tests/optimization/test_constant_folding.py
"""Integration tests for constant folding behavior."""

import pytest

from catnip import Catnip
from catnip.exc import CatnipRuntimeError


def _cat(executor: str, optimize: int = 2) -> Catnip:
    vm_mode = "on" if executor == "vm" else "off"
    return Catnip(vm_mode=vm_mode, optimize=optimize)


@pytest.mark.parametrize("executor", ["vm", "ast"])
def test_constant_folding_with_variables(executor):
    """Only constant parts are folded; variable semantics are unchanged."""
    cat = _cat(executor, optimize=2)
    cat.parse("""
        x = 10
        (2 + 3) + x
        """)
    assert cat.execute() == 15


@pytest.mark.parametrize("executor", ["vm", "ast"])
def test_constant_folding_does_not_fold_function_calls(executor):
    """Function calls keep runtime behavior (no illegal compile-time fold)."""
    cat = _cat(executor, optimize=2)
    cat.parse("""
        f = (x) => { x + 1 }
        f(5)
        """)
    assert cat.execute() == 6


@pytest.mark.parametrize("executor", ["vm", "ast"])
def test_division_by_zero_happens_at_runtime(executor):
    """Division by zero must not be folded during parse/semantic."""
    cat = _cat(executor, optimize=2)
    cat.parse("1 / 0")
    with pytest.raises((CatnipRuntimeError, ZeroDivisionError)):
        cat.execute()


@pytest.mark.parametrize("executor", ["vm", "ast"])
def test_constant_folding_preserves_call_in_comparison(executor):
    """Call nodes (e.g. len(x)) must not be treated as constants in EQ."""
    cat = _cat(executor, optimize=2)
    cat.parse("""
        f = (xs) => {
            if len(xs) == 0 { 0 }
            else { 1 + f(list()) }
        }
        f(list(1, 2, 3))
    """)
    assert cat.execute() == 1


# Removed: test_strength_reduction_and_folding_compose - sr_pow_one, const_fold_add (CatnipOptimProof.v, CatnipConstFoldProof.v)
# Doublon exact de test_optimization_iterations dans test_optimizer.py; passes prouvees individuellement
