# FILE: tests/language/test_operator_overload_matrix.py
"""Cross-axis tests for operator overloading: ambiguity, broadcast shape, pure/impure, ND stability."""

import pytest

from catnip import Catnip
from catnip.exc import CatnipTypeError

# ---------------------------------------------------------------------------
# 1. Ambiguity - dispatch left-operand-wins, cross-type, missing operators
# ---------------------------------------------------------------------------


class TestAmbiguity:
    def test_left_operand_dispatches(self, cat):
        code = """\
struct A {
    val;
    op +(self, rhs) => { A(self.val + rhs.val + 100) }
}
struct B {
    val;
    op +(self, rhs) => { B(self.val + rhs.val + 200) }
}
r = A(1) + B(2)
r.val"""
        cat.parse(code)
        assert cat.execute() == 103

    def test_swap_operands_swaps_dispatch(self, cat):
        code = """\
struct A {
    val;
    op +(self, rhs) => { A(self.val + rhs.val + 100) }
}
struct B {
    val;
    op +(self, rhs) => { B(self.val + rhs.val + 200) }
}
r = B(2) + A(1)
r.val"""
        cat.parse(code)
        assert cat.execute() == 203

    def test_scalar_plus_struct_reverse(self, cat):
        code = """\
struct S {
    val;
    op +(self, rhs) => { S(self.val + rhs) }
}
r = 5 + S(10)
r.val"""
        cat.parse(code)
        assert cat.execute() == 15

    def test_only_left_has_op_succeeds(self, cat):
        code = """\
struct A {
    val;
    op +(self, rhs) => { A(self.val + rhs.val) }
}
struct B { val; }
r = A(1) + B(2)
r.val"""
        cat.parse(code)
        assert cat.execute() == 3

    def test_only_right_has_op_reverse(self, cat):
        code = """\
struct A { val; }
struct B {
    val;
    op +(self, rhs) => { B(self.val + rhs.val) }
}
r = A(1) + B(2)
r.val"""
        cat.parse(code)
        assert cat.execute() == 3

    def test_chaining_mixed_types(self, cat):
        code = """\
struct A {
    val;
    op +(self, rhs) => { A(self.val + rhs.val) }
}
struct B { val; }
r = A(1) + B(2) + A(3)
r.val"""
        cat.parse(code)
        # A(1)+B(2) => A(3), then A(3)+A(3) => A(6)
        assert cat.execute() == 6


# ---------------------------------------------------------------------------
# 2. Broadcast shape - broadcast with overloaded operators, various shapes
# ---------------------------------------------------------------------------


class TestBroadcastShape:
    def test_broadcast_struct_op_flat(self, cat):
        code = """\
struct Num {
    val;
    op *(self, rhs) => { Num(self.val * rhs) }
}
result = list(Num(1), Num(2), Num(3)).[* 10]
list(result[0].val, result[1].val, result[2].val)"""
        cat.parse(code)
        assert cat.execute() == [10, 20, 30]

    def test_broadcast_struct_op_nested(self, cat):
        code = """\
struct Num {
    val;
    op *(self, rhs) => { Num(self.val * rhs) }
}
result = list(list(Num(1)), list(Num(2), Num(3))).[* 5]
list(result[0][0].val, result[1][0].val, result[1][1].val)"""
        cat.parse(code)
        assert cat.execute() == [5, 10, 15]

    def test_broadcast_missing_op_error(self, cat):
        code = """\
struct HasOp {
    val;
    op *(self, rhs) => { HasOp(self.val * rhs) }
}
struct NoOp { val; }
list(HasOp(1), NoOp(2)).[* 10]"""
        cat.parse(code)
        with pytest.raises((TypeError, CatnipTypeError)):
            cat.execute()

    def test_broadcast_filter_overloaded_cmp(self, cat):
        code = """\
struct Num { val; }
items = list(Num(1), Num(5), Num(3))
items.[if (x) => { x.val > 3 }]"""
        cat.parse(code)
        result = cat.execute()
        assert len(result) == 1
        assert result[0].val == 5

    def test_broadcast_empty_struct_list(self, cat):
        code = """\
struct Num {
    val;
    op *(self, rhs) => { Num(self.val * rhs) }
}
list().[* 10]"""
        cat.parse(code)
        assert cat.execute() == []

    def test_broadcast_chained_struct_ops(self, cat):
        code = """\
struct Num {
    val;
    op *(self, rhs) => { Num(self.val * rhs) }
}
result = list(Num(1), Num(2)).[* 3].[* 2]
list(result[0].val, result[1].val)"""
        cat.parse(code)
        assert cat.execute() == [6, 12]

    def test_broadcast_lambda_struct_field(self, cat):
        code = """\
struct Vec2 { x; y; }
list(Vec2(1, 2), Vec2(3, 4)).[~> (v) => { v.x + v.y }]"""
        cat.parse(code)
        assert cat.execute() == [3, 7]


# ---------------------------------------------------------------------------
# 3. Pure/impure - purity tracking, side effects in operators
# ---------------------------------------------------------------------------


class TestPureImpure:
    def test_pure_fn_broadcast_struct(self, cat):
        code = """\
struct Num { val; }
@pure
f = (x) => { Num(x * 2) }
result = list(1, 2, 3).[~> f]
list(result[0].val, result[1].val, result[2].val)"""
        cat.parse(code)
        assert cat.execute() == [2, 4, 6]

    def test_impure_op_side_effect_counted(self, cat):
        code = """\
struct Counter {
    val; count = 0;
    op +(self, rhs) => {
        self.count = self.count + 1
        Counter(self.val + rhs.val, self.count)
    }
}
c = Counter(0)
c = c + Counter(1)
c = c + Counter(2)
c = c + Counter(3)
c.count"""
        cat.parse(code)
        assert cat.execute() == 3

    def test_impure_op_broadcast_ordering(self, cat):
        code = """\
struct Logger {
    val; log = list();
    op *(self, rhs) => {
        self.log = self.log + list(self.val)
        Logger(self.val * rhs, self.log)
    }
}
items = list(Logger(1), Logger(2), Logger(3))
result = items.[* 1]
list(result[0].log, result[1].log, result[2].log)"""
        cat.parse(code)
        assert cat.execute() == [[1], [2], [3]]

    def test_pure_fn_same_input_same_output(self, cat):
        code = """\
struct Num { val; }
@pure
f = (x) => { Num(x) }
a = f(5)
b = f(5)
a.val == b.val"""
        cat.parse(code)
        assert cat.execute() is True

    def test_impure_fn_broadcast_accumulates(self, cat):
        code = """\
counter = list(0)
tick = (x) => {
    counter[0] = counter[0] + 1
    x
}
list(10, 20, 30).[~> tick]
counter[0]"""
        cat.parse(code)
        assert cat.execute() == 3

    def test_pure_vs_impure_result_equivalence(self, cat):
        code = """\
struct Num { val; }
@pure
p = (x) => { Num(x * 2) }
i = (x) => { Num(x * 2) }
a = list(1, 2, 3).[~> p]
b = list(1, 2, 3).[~> i]
list(a[0].val == b[0].val, a[1].val == b[1].val, a[2].val == b[2].val)"""
        cat.parse(code)
        assert cat.execute() == [True, True, True]


# ---------------------------------------------------------------------------
# 4. ND stability - ND recursion with structs and operators
# ---------------------------------------------------------------------------


class TestNDStability:
    def test_nd_recursion_struct_seed(self, cat):
        code = """\
struct Num { val; }
~~(Num(5), (v, recur) => {
    if v.val > 0 { recur(Num(v.val - 1)) }
    else { v.val }
})"""
        cat.parse(code)
        assert cat.execute() == 0

    def test_nd_recursion_struct_accumulate(self, cat):
        code = """\
~~(5, (n, recur) => {
    if n <= 1 { 1 }
    else { n * recur(n - 1) }
})"""
        cat.parse(code)
        assert cat.execute() == 120

    def test_nd_recursion_op_in_body(self, cat):
        code = """\
struct Num {
    val;
    op +(self, rhs) => { Num(self.val + rhs.val) }
}
r = ~~(5, (n, recur) => {
    if n <= 0 { Num(0) }
    else { Num(n) + recur(n - 1) }
})
r.val"""
        cat.parse(code)
        assert cat.execute() == 15

    def test_broadcast_nd_struct_result(self, cat):
        code = """\
list(3, 5).[~~ (n, recur) => {
    if n <= 1 { 1 }
    else { n * recur(n - 1) }
}]"""
        cat.parse(code)
        assert cat.execute() == [6, 120]

    def test_nd_determinism_struct(self, cat):
        code = """\
struct Num { val; }
r1 = ~~(Num(10), (v, recur) => {
    if v.val > 0 { recur(Num(v.val - 1)) }
    else { v.val }
})
r2 = ~~(Num(10), (v, recur) => {
    if v.val > 0 { recur(Num(v.val - 1)) }
    else { v.val }
})
r1 == r2"""
        cat.parse(code)
        assert cat.execute() is True

    def test_nd_map_over_structs(self, cat):
        code = """\
struct Num { val; }
~>(list(Num(1), Num(2), Num(3)), (x) => { x.val * 10 })"""
        cat.parse(code)
        assert cat.execute() == [10, 20, 30]

    def test_nd_closure_struct_wrapper(self, cat):
        """Regression: TAG_STRUCT captured in closure leaks into child VM."""
        code = """\
struct Num { val; }
fn = (seed) => {
    ~~(seed, (v, recur) => {
        if v.val > 0 { recur(Num(v.val - 1)) }
        else { v.val }
    })
}
r1 = fn(Num(10))
r2 = fn(Num(10))
r1 == r2"""
        cat.parse(code)
        assert cat.execute() is True
