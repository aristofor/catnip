# FILE: tests/language/test_operator_overload.py
"""Tests for operator overloading on structs via op <symbol> syntax."""

import pytest

from catnip import Catnip
from catnip.exc import CatnipTypeError

# --- Binary operators ---


def test_add(cat):
    code = """struct Vec2 {
    x; y;
    op +(self, rhs) => { Vec2(self.x + rhs.x, self.y + rhs.y) }
}
a = Vec2(1, 2)
b = Vec2(3, 4)
c = a + b
list(c.x, c.y)"""
    cat.parse(code)
    assert cat.execute() == [4, 6]


def test_sub(cat):
    code = """struct Vec2 {
    x; y;
    op -(self, rhs) => { Vec2(self.x - rhs.x, self.y - rhs.y) }
}
a = Vec2(5, 8)
b = Vec2(2, 3)
c = a - b
list(c.x, c.y)"""
    cat.parse(code)
    assert cat.execute() == [3, 5]


def test_mul_scalar(cat):
    code = """struct Vec2 {
    x; y;
    op *(self, rhs) => { Vec2(self.x * rhs, self.y * rhs) }
}
a = Vec2(2, 3)
b = a * 4
list(b.x, b.y)"""
    cat.parse(code)
    assert cat.execute() == [8, 12]


def test_div(cat):
    code = """struct Vec2 {
    x; y;
    op /(self, rhs) => { Vec2(self.x / rhs, self.y / rhs) }
}
a = Vec2(10, 20)
b = a / 2
list(b.x, b.y)"""
    cat.parse(code)
    assert cat.execute() == [5.0, 10.0]


def test_floordiv(cat):
    code = """struct Vec2 {
    x; y;
    op //(self, rhs) => { Vec2(self.x // rhs, self.y // rhs) }
}
a = Vec2(7, 11)
b = a // 3
list(b.x, b.y)"""
    cat.parse(code)
    assert cat.execute() == [2, 3]


def test_mod(cat):
    code = """struct Vec2 {
    x; y;
    op %(self, rhs) => { Vec2(self.x % rhs, self.y % rhs) }
}
a = Vec2(7, 11)
b = a % 3
list(b.x, b.y)"""
    cat.parse(code)
    assert cat.execute() == [1, 2]


def test_pow(cat):
    code = """struct Vec2 {
    x; y;
    op **(self, rhs) => { Vec2(self.x ** rhs, self.y ** rhs) }
}
a = Vec2(2, 3)
b = a ** 3
list(b.x, b.y)"""
    cat.parse(code)
    assert cat.execute() == [8, 27]


# --- Unary operators ---


def test_neg(cat):
    code = """struct Vec2 {
    x; y;
    op -(self) => { Vec2(-self.x, -self.y) }
}
a = Vec2(3, -5)
b = -a
list(b.x, b.y)"""
    cat.parse(code)
    assert cat.execute() == [-3, 5]


def test_pos(cat):
    code = """struct Val {
    x;
    op +(self) => { Val(abs(self.x)) }
}
a = Val(-7)
b = +a
b.x"""
    cat.parse(code)
    assert cat.execute() == 7


# --- Chaining ---


def test_chaining(cat):
    code = """struct Vec2 {
    x; y;
    op +(self, rhs) => { Vec2(self.x + rhs.x, self.y + rhs.y) }
}
a = Vec2(1, 1)
b = Vec2(2, 2)
c = Vec2(3, 3)
d = a + b + c
list(d.x, d.y)"""
    cat.parse(code)
    assert cat.execute() == [6, 6]


def test_print_between_ops(cat):
    """Regression: print() between operator calls must not corrupt struct values."""
    code = """struct Vec2 {
    x; y;
    op +(self, rhs) => { Vec2(self.x + rhs.x, self.y + rhs.y) }
    op -(self, rhs) => { Vec2(self.x - rhs.x, self.y - rhs.y) }
}
a = Vec2(1, 2)
b = Vec2(3, 4)
c = a + b
str(c)
d = a - b
list(d.x, d.y)"""
    cat.parse(code)
    assert cat.execute() == [-2, -2]


# --- No overload: builtins still work ---


def test_no_overload_builtins(cat):
    code = "1 + 2"
    cat.parse(code)
    assert cat.execute() == 3


def test_struct_without_overload(cat):
    code = """struct Point { x; y; }
a = Point(1, 2)
1 + 2"""
    cat.parse(code)
    assert cat.execute() == 3


# --- Inheritance ---


def test_inherited_op(cat):
    code = """struct Base {
    x; y;
    op +(self, rhs) => { Base(self.x + rhs.x, self.y + rhs.y) }
}
struct Child extends(Base) { }
a = Child(1, 2)
b = Child(3, 4)
c = a + b
list(c.x, c.y)"""
    cat.parse(code)
    assert cat.execute() == [4, 6]


# --- Comparison operators ---


def test_eq(cat):
    code = """struct Money {
    amount; currency;
    op ==(self, rhs) => { self.amount == rhs.amount and self.currency == rhs.currency }
}
a = Money(100, "EUR")
b = Money(100, "EUR")
c = Money(100, "USD")
list(a == b, a == c)"""
    cat.parse(code)
    assert cat.execute() == [True, False]


def test_ne(cat):
    code = """struct Money {
    amount; currency;
    op !=(self, rhs) => { self.amount != rhs.amount or self.currency != rhs.currency }
}
a = Money(100, "EUR")
b = Money(200, "EUR")
a != b"""
    cat.parse(code)
    assert cat.execute() is True


def test_lt(cat):
    code = """struct Money {
    amount;
    op <(self, rhs) => { self.amount < rhs.amount }
}
a = Money(50)
b = Money(100)
a < b"""
    cat.parse(code)
    assert cat.execute() is True


def test_le(cat):
    code = """struct Money {
    amount;
    op <=(self, rhs) => { self.amount <= rhs.amount }
}
a = Money(100)
b = Money(100)
a <= b"""
    cat.parse(code)
    assert cat.execute() is True


def test_gt(cat):
    code = """struct Money {
    amount;
    op >(self, rhs) => { self.amount > rhs.amount }
}
a = Money(200)
b = Money(100)
a > b"""
    cat.parse(code)
    assert cat.execute() is True


def test_ge(cat):
    code = """struct Money {
    amount;
    op >=(self, rhs) => { self.amount >= rhs.amount }
}
a = Money(100)
b = Money(100)
a >= b"""
    cat.parse(code)
    assert cat.execute() is True


def test_structural_eq_fallback(cat):
    """Without op ==, structural equality is used."""
    code = """struct Point { x; y; }
a = Point(1, 2)
b = Point(1, 2)
c = Point(3, 4)
list(a == b, a == c)"""
    cat.parse(code)
    assert cat.execute() == [True, False]


# --- Bitwise operators ---


def test_band(cat):
    code = """struct Mask {
    bits;
    op &(self, rhs) => { Mask(self.bits % rhs.bits) }
}
a = Mask(7)
b = Mask(3)
c = a & b
c.bits"""
    cat.parse(code)
    assert cat.execute() == 1


def test_bor(cat):
    code = """struct Mask {
    bits;
    op |(self, rhs) => { Mask(self.bits + rhs.bits) }
}
a = Mask(4)
b = Mask(2)
c = a | b
c.bits"""
    cat.parse(code)
    assert cat.execute() == 6


def test_bxor(cat):
    code = """struct Mask {
    bits;
    op ^(self, rhs) => { Mask(self.bits + rhs.bits) }
}
a = Mask(5)
b = Mask(3)
c = a ^ b
c.bits"""
    cat.parse(code)
    assert cat.execute() == 8


def test_lshift(cat):
    code = """struct Bits {
    val;
    op <<(self, rhs) => { Bits(self.val * (2 ** rhs)) }
}
a = Bits(1)
b = a << 3
b.val"""
    cat.parse(code)
    assert cat.execute() == 8


def test_rshift(cat):
    code = """struct Bits {
    val;
    op >>(self, rhs) => { Bits(self.val // (2 ** rhs)) }
}
a = Bits(16)
b = a >> 2
b.val"""
    cat.parse(code)
    assert cat.execute() == 4


def test_bnot(cat):
    code = """struct Bits {
    val;
    op ~(self) => { Bits(-self.val - 1) }
}
a = Bits(5)
b = ~a
b.val"""
    cat.parse(code)
    assert cat.execute() == -6


# --- Disambiguation unary/binary ---


def test_unary_binary_disambiguation(cat):
    """op - with 1 param = unary neg, op - with 2 params = binary sub."""
    code = """struct Num {
    x;
    op -(self) => { Num(-self.x) }
    op -(self, rhs) => { Num(self.x - rhs.x) }
}
a = Num(10)
b = Num(3)
c = -a
d = a - b
list(c.x, d.x)"""
    cat.parse(code)
    assert cat.execute() == [-10, 7]


# --- Error cases ---


def test_missing_binop_returns_type_error(cat):
    code = """struct Empty { x; }
a = Empty(1)
a + 42"""
    cat.parse(code)
    with pytest.raises((TypeError, CatnipTypeError)):
        cat.execute()


def test_missing_unaryop_raises_type_error(cat):
    code = """struct Empty { x; }
a = Empty(1)
-a"""
    cat.parse(code)
    with pytest.raises((TypeError, CatnipTypeError)):
        cat.execute()
