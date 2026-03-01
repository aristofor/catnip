# FILE: tests/optimization/test_iv_strength_reduction.py
"""
Tests for Induction Variable Strength Reduction.

Detects loop induction variables (BIVs) and derived expressions (DIVs)
like `j = i * 4`, replacing the per-iteration multiplication with a
cheaper addition-based accumulator.

Activated at optimize >= 3 (CFG/SSA level).
"""

import unittest

from catnip import Catnip


class TestIVStrengthReduction(unittest.TestCase):
    """IV strength reduction correctness tests."""

    def setUp(self):
        self.opt3 = Catnip(optimize=3)
        self.opt0 = Catnip(optimize=0)

    def _run(self, code, *, optimize=3):
        c = Catnip(optimize=optimize)
        c.parse(code)
        return c.execute()

    def test_simple_mul_in_while(self):
        """Basic case: i * 4 in a while loop."""
        code = "s = 0; i = 0; while (i < 10) { s = s + i * 4; i = i + 1 }; s"
        assert self._run(code) == 180  # sum(i*4 for i in range(10))

    def test_mul_assigned_to_variable(self):
        """DIV assigned to a named variable."""
        code = """
        s = 0; i = 0
        while (i < 5) {
            idx = i * 3
            s = s + idx
            i = i + 1
        }
        s
        """
        assert self._run(code) == 30  # sum(i*3 for i in range(5))

    def test_step_of_two(self):
        """BIV with step 2."""
        code = "s = 0; i = 0; while (i < 10) { s = s + i * 5; i = i + 2 }; s"
        # i values: 0, 2, 4, 6, 8 -> i*5: 0, 10, 20, 30, 40 -> sum = 100
        assert self._run(code) == 100

    def test_preserves_semantics(self):
        """Compare optimize=0 vs optimize=3."""
        code = "s = 0; i = 0; while (i < 20) { s = s + i * 7; i = i + 1 }; s"
        r0 = self._run(code, optimize=0)
        r3 = self._run(code, optimize=3)
        assert r0 == r3

    def test_no_optimization_variable_scale(self):
        """i * k with variable k must NOT be optimized (and still be correct)."""
        code = "k = 4; s = 0; i = 0; while (i < 5) { s = s + i * k; i = i + 1 }; s"
        assert self._run(code) == 40

    def test_no_optimization_nonlinear(self):
        """i * i is not a DIV (both operands are the BIV)."""
        code = "s = 0; i = 0; while (i < 5) { s = s + i * i; i = i + 1 }; s"
        assert self._run(code) == 30  # sum(i*i for i in range(5))

    def test_commutative_mul(self):
        """Constant on the left: 4 * i."""
        code = "s = 0; i = 0; while (i < 10) { s = s + 4 * i; i = i + 1 }; s"
        assert self._run(code) == 180

    def test_multiple_divs(self):
        """Multiple derived IVs from same BIV."""
        code = """
        s = 0; t = 0; i = 0
        while (i < 5) {
            s = s + i * 3
            t = t + i * 7
            i = i + 1
        }
        s + t
        """
        # sum(i*3 for i=0..4) + sum(i*7 for i=0..4) = 30 + 70 = 100
        assert self._run(code) == 100

    def test_zero_iterations(self):
        """Loop that never executes."""
        code = "s = 0; i = 0; while (i < 0) { s = s + i * 4; i = i + 1 }; s"
        assert self._run(code) == 0

    def test_single_iteration(self):
        """Loop with exactly one iteration."""
        code = "s = 0; i = 0; while (i < 1) { s = s + i * 4; i = i + 1 }; s"
        assert self._run(code) == 0

    def test_large_scale(self):
        """Large constant scale."""
        code = "s = 0; i = 0; while (i < 100) { s = s + i * 1000; i = i + 1 }; s"
        assert self._run(code) == 4950000

    def test_negative_scale(self):
        """Negative constant scale."""
        code = "s = 0; i = 0; while (i < 5) { s = s + i * (0 - 3); i = i + 1 }; s"
        assert self._run(code) == -30

    def test_semantics_with_other_ops(self):
        """Loop body with other operations besides the DIV."""
        code = """
        s = 0; i = 0
        while (i < 5) {
            x = i * 6
            y = x + 1
            s = s + y
            i = i + 1
        }
        s
        """
        # sum(i*6 + 1 for i in range(5)) = 0+1 + 6+1 + 12+1 + 18+1 + 24+1 = 65
        assert self._run(code) == 65


if __name__ == "__main__":
    unittest.main()
