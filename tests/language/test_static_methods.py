# FILE: tests/language/test_static_methods.py
"""Tests for @static methods on structs and traits."""

import pytest

from catnip import Catnip


def test_static_method_basic(cat):
    """@static method with no self, called on the type."""
    code = """struct Counter {
    value

    @static
    zero() => {
        Counter(0)
    }
}
c = Counter.zero()
c.value"""
    cat.parse(code)
    assert cat.execute() == 0


def test_static_method_on_instance(cat):
    """@static method callable on an instance too."""
    code = """struct Counter {
    value

    @static
    zero() => {
        Counter(0)
    }
}
c = Counter(5)
c2 = c.zero()
c2.value"""
    cat.parse(code)
    assert cat.execute() == 0


def test_static_with_params(cat):
    """@static method with parameters (no self)."""
    code = """struct Point {
    x, y

    @static
    origin() => {
        Point(0, 0)
    }

    @static
    from_scalar(n) => {
        Point(n, n)
    }
}
p = Point.from_scalar(7)
list(p.x, p.y)"""
    cat.parse(code)
    assert cat.execute() == [7, 7]


def test_static_and_instance_mixed(cat):
    """Mix of @static and instance methods in one struct."""
    code = """struct Vec2 {
    x, y

    length_sq(self) => {
        self.x * self.x + self.y * self.y
    }

    @static
    zero() => {
        Vec2(0, 0)
    }
}
v = Vec2.zero()
v.length_sq()"""
    cat.parse(code)
    assert cat.execute() == 0


def test_static_self_param_error(cat):
    """@static method with self as first param is a parse error."""
    code = """struct Bad {
    x

    @static
    broken(self) => {
        self.x
    }
}"""
    with pytest.raises(Exception, match="self"):
        cat.parse(code)


def test_static_in_trait(cat):
    """@static method in a trait, accessible on struct type."""
    code = """trait Factory {
    @static
    create() => {
        42
    }
}
struct Widget implements(Factory) { v }
Widget.create()"""
    cat.parse(code)
    assert cat.execute() == 42


def test_abstract_static_in_trait(cat):
    """@abstract @static in a trait, implemented by struct."""
    code = """trait Buildable {
    @abstract
    @static
    build()
}
struct Thing implements(Buildable) {
    x

    @static
    build() => {
        Thing(99)
    }
}
t = Thing.build()
t.x"""
    cat.parse(code)
    assert cat.execute() == 99


def test_abstract_static_not_implemented(cat):
    """Struct must implement @abstract @static methods."""
    code = """trait Buildable {
    @abstract
    @static
    build()
}
struct Broken implements(Buildable) { x }"""
    with pytest.raises(Exception, match="abstract|implement"):
        cat.parse(code)
        cat.execute()


def test_static_with_extends(cat):
    """@static methods inherited through extends."""
    code = """struct Base {
    x

    @static
    default_val() => {
        10
    }
}
struct Child extends(Base) { y }
Child.default_val()"""
    cat.parse(code)
    assert cat.execute() == 10


def test_static_override_in_child(cat):
    """Child struct can override parent's @static method."""
    code = """struct Base {
    x

    @static
    make() => {
        Base(0)
    }
}
struct Child extends(Base) {
    y

    @static
    make() => {
        Child(0, 1)
    }
}
c = Child.make()
c.y"""
    cat.parse(code)
    assert cat.execute() == 1


def test_static_on_instance_from_extends(cat):
    """Inherited @static callable on instance."""
    code = """struct Base {
    x

    @static
    label() => {
        "base"
    }
}
struct Child extends(Base) { y }
c = Child(1, 2)
c.label()"""
    cat.parse(code)
    assert cat.execute() == "base"


def test_static_conflict_between_traits(cat):
    """Conflicting @static methods from unrelated traits."""
    code = """trait A {
    @static
    foo() => { 1 }
}
trait B {
    @static
    foo() => { 2 }
}
struct S implements(A, B) { x }"""
    with pytest.raises(Exception, match="conflict"):
        cat.parse(code)
        cat.execute()
