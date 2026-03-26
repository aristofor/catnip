# FILE: tests/language/test_struct_methods.py
"""Tests for struct methods with explicit self."""

import pytest

from catnip import Catnip


def test_method_basic(cat):
    """Test basic method call."""
    code = """struct Point {
    x; y;

    sum(self) => {
        self.x + self.y
    }
}
p = Point(3, 4)
p.sum()"""
    cat.parse(code)
    assert cat.execute() == 7


def test_method_two_params(cat):
    """Test method with two parameters."""
    code = """struct Point {
    x; y;

    add(self, other) => {
        Point(self.x + other.x, self.y + other.y)
    }
}
p1 = Point(1, 2)
p2 = Point(3, 4)
p3 = p1.add(p2)
list(p3.x, p3.y)"""
    cat.parse(code)
    assert cat.execute() == [4, 6]


def test_method_returns_scalar(cat):
    """Test method returning a scalar value."""
    code = """struct Rect {
    w; h;

    area(self) => {
        self.w * self.h
    }
}
Rect(5, 3).area()"""
    cat.parse(code)
    assert cat.execute() == 15


def test_multiple_methods(cat):
    """Test struct with multiple methods."""
    code = """struct Vec2 {
    x; y;

    length_sq(self) => {
        self.x ** 2 + self.y ** 2
    }

    scale(self, factor) => {
        Vec2(self.x * factor, self.y * factor)
    }
}
v = Vec2(3, 4)
v2 = v.scale(2)
list(v.length_sq(), v2.x, v2.y)"""
    cat.parse(code)
    assert cat.execute() == [25, 6, 8]


def test_method_chaining(cat):
    """Test calling methods on method results."""
    code = """struct Counter {
    n;

    inc(self) => {
        Counter(self.n + 1)
    }

    value(self) => {
        self.n
    }
}
Counter(0).inc().inc().inc().value()"""
    cat.parse(code)
    assert cat.execute() == 3


def test_method_with_fields_only_struct(cat):
    """Test that structs without methods still work."""
    code = """struct Simple { a; b; }
s = Simple(1, 2)
s.a + s.b"""
    cat.parse(code)
    assert cat.execute() == 3


def test_method_accessing_globals(cat):
    """Test method accessing global variables."""
    code = """offset = 100
struct Val {
    n;

    shifted(self) => {
        self.n + offset
    }
}
Val(42).shifted()"""
    cat.parse(code)
    assert cat.execute() == 142


class TestMethodLexicalCapture:
    """Struct methods must capture enclosing local variables (VM/AST parity)."""

    CLOSURE_CODE = """
factory = () => {
    offset = 10
    struct Point {
        x; y;
        shifted(self) => { self.x + offset }
    }
    Point
}
P = factory()
P(3, 4).shifted()
"""

    def test_lexical_capture_vm(self):
        cat = Catnip(vm_mode='on')
        cat.parse(self.CLOSURE_CODE)
        assert cat.execute() == 13

    def test_lexical_capture_ast(self):
        cat = Catnip(vm_mode='off')
        cat.parse(self.CLOSURE_CODE)
        assert cat.execute() == 13

    def test_nested_closure_capture(self, cat):
        code = """
outer = () => {
    base = 100
    inner = () => {
        mult = 2
        struct Calc {
            n;
            compute(self) => { self.n * mult + base }
        }
        Calc
    }
    inner()
}
C = outer()
C(5).compute()
"""
        cat.parse(code)
        assert cat.execute() == 110

    def test_inline_method_captures_local(self, cat):
        code = """
make = () => {
    factor = 3
    struct Vec2 {
        x; y;
        scaled_x(self) => { self.x * factor }
    }
    Vec2
}
V = make()
V(7, 0).scaled_x()
"""
        cat.parse(code)
        assert cat.execute() == 21
