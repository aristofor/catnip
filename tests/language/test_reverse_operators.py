# FILE: tests/language/test_reverse_operators.py
"""Tests for reverse (reflected) operator dispatch on structs.

Semantics: when `scalar OP struct` is evaluated and the scalar doesn't handle
the struct type, Python falls back to `struct.__rOP__(scalar)`. Catnip dispatches
this to the same `op_X(self, rhs)` method where self is always the struct.

For commutative ops (+, *, &, |, ^) the result is the same as forward.
For non-commutative ops (-, /, etc.) the result is self.val OP rhs, not rhs OP self.val.
"""

import pytest

from catnip import Catnip
from catnip.exc import CatnipTypeError


def val(cat, code):
    cat.parse(code)
    return cat.execute()


# ---------------------------------------------------------------------------
# 1. Commutative ops — result is identical to forward
# ---------------------------------------------------------------------------


class TestReverseCommutative:
    def test_radd_int(self, cat):
        assert (
            val(
                cat,
                """\
struct S { val; op +(self, rhs) => { S(self.val + rhs) } }
r = 5 + S(10); r.val""",
            )
            == 15
        )

    def test_radd_float(self, cat):
        assert (
            val(
                cat,
                """\
struct S { val; op +(self, rhs) => { S(self.val + rhs) } }
r = 1.5 + S(2.5); r.val""",
            )
            == 4.0
        )

    def test_rmul_int(self, cat):
        assert (
            val(
                cat,
                """\
struct S { val; op *(self, rhs) => { S(self.val * rhs) } }
r = 3 * S(7); r.val""",
            )
            == 21
        )

    def test_rmul_float(self, cat):
        assert (
            val(
                cat,
                """\
struct S { val; op *(self, rhs) => { S(self.val * rhs) } }
r = 2.0 * S(3.5); r.val""",
            )
            == 7.0
        )

    def test_rand(self, cat):
        assert (
            val(
                cat,
                """\
struct S { val; op &(self, rhs) => { S(self.val & rhs) } }
r = 0xFF & S(0x0F); r.val""",
            )
            == 15
        )

    def test_ror(self, cat):
        assert (
            val(
                cat,
                """\
struct S { val; op |(self, rhs) => { S(self.val | rhs) } }
r = 0xF0 | S(0x0F); r.val""",
            )
            == 255
        )

    def test_rxor(self, cat):
        assert (
            val(
                cat,
                """\
struct S { val; op ^(self, rhs) => { S(self.val ^ rhs) } }
r = 0xFF ^ S(0x0F); r.val""",
            )
            == 240
        )


# ---------------------------------------------------------------------------
# 2. Non-commutative ops — self is always the struct
# ---------------------------------------------------------------------------


class TestReverseNonCommutative:
    def test_rsub(self, cat):
        """10 - S(3) => S(3).__rsub__(10) => op_sub(S(3), 10) => S(3 - 10)"""
        assert (
            val(
                cat,
                """\
struct S { val; op -(self, rhs) => { S(self.val - rhs) } }
r = 10 - S(3); r.val""",
            )
            == -7
        )

    def test_rtruediv(self, cat):
        """100 / S(4) => S(4).__rtruediv__(100) => op_div(S(4), 100) => S(4/100)"""
        assert (
            val(
                cat,
                """\
struct S { val; op /(self, rhs) => { S(self.val / rhs) } }
r = 100 / S(4); r.val""",
            )
            == 0.04
        )

    def test_rfloordiv(self, cat):
        """10 // S(7) => S(7).__rfloordiv__(10) => op_floordiv(S(7), 10) => S(7//10)"""
        assert (
            val(
                cat,
                """\
struct S { val; op //(self, rhs) => { S(self.val // rhs) } }
r = 10 // S(7); r.val""",
            )
            == 0
        )

    def test_rmod(self, cat):
        """10 % S(7) => S(7).__rmod__(10) => op_mod(S(7), 10) => S(7%10)"""
        assert (
            val(
                cat,
                """\
struct S { val; op %(self, rhs) => { S(self.val % rhs) } }
r = 10 % S(7); r.val""",
            )
            == 7
        )

    def test_rpow(self, cat):
        """2 ** S(3) => S(3).__rpow__(2) => op_pow(S(3), 2) => S(3**2)"""
        assert (
            val(
                cat,
                """\
struct S { val; op **(self, rhs) => { S(self.val ** rhs) } }
r = 2 ** S(3); r.val""",
            )
            == 9
        )

    def test_rlshift(self, cat):
        """2 << S(1) => S(1).__rlshift__(2) => op_lshift(S(1), 2) => S(1<<2)"""
        assert (
            val(
                cat,
                """\
struct S { val; op <<(self, rhs) => { S(self.val << rhs) } }
r = 2 << S(1); r.val""",
            )
            == 4
        )

    def test_rrshift(self, cat):
        """2 >> S(8) => S(8).__rrshift__(2) => op_rshift(S(8), 2) => S(8>>2)"""
        assert (
            val(
                cat,
                """\
struct S { val; op >>(self, rhs) => { S(self.val >> rhs) } }
r = 2 >> S(8); r.val""",
            )
            == 2
        )


# ---------------------------------------------------------------------------
# 3. Edge cases
# ---------------------------------------------------------------------------


class TestReverseEdgeCases:
    def test_forward_still_works(self, cat):
        assert (
            val(
                cat,
                """\
struct S { val; op +(self, rhs) => { S(self.val + rhs) } }
r = S(10) + 5; r.val""",
            )
            == 15
        )

    def test_no_reverse_without_op(self, cat):
        """No op defined => reverse also fails."""
        cat.parse("""\
struct S { val }
5 + S(10)""")
        with pytest.raises((TypeError, CatnipTypeError)):
            cat.execute()

    def test_left_dispatch_priority(self, cat):
        """Left operand's forward op takes priority over right's reverse."""
        assert (
            val(
                cat,
                """\
struct A { val; op +(self, rhs) => { A(self.val + 100) } }
struct B { val; op +(self, rhs) => { B(self.val + 200) } }
r = A(1) + B(2); r.val""",
            )
            == 101
        )

    def test_both_structs_no_reverse_needed(self, cat):
        """When both have op+, left wins — no reverse dispatch."""
        assert (
            val(
                cat,
                """\
struct A { val; op +(self, rhs) => { A(self.val + rhs.val + 10) } }
struct B { val; op +(self, rhs) => { B(self.val + rhs.val + 20) } }
r = A(1) + B(2); r.val""",
            )
            == 13
        )

    def test_chained_reverse(self, cat):
        assert (
            val(
                cat,
                """\
struct S { val; op *(self, rhs) => { S(self.val * rhs) } }
r = 2 * (3 * S(5)); r.val""",
            )
            == 30
        )

    def test_reverse_with_complex_body(self, cat):
        assert (
            val(
                cat,
                """\
struct Vec2 {
    x, y
    op +(self, rhs) => { Vec2(self.x + rhs, self.y + rhs) }
}
r = 10 + Vec2(1, 2)
list(r.x, r.y)""",
            )
            == [11, 12]
        )
