# FILE: tests/language/test_struct_edge_cases.py
"""Edge-case tests for struct init, super, extends+implements, scopes."""

import pytest

# -- init constructor --


def test_init_basic(cat):
    """init is called automatically after field assignment."""
    code = """
struct S {
    x
    init(self) => { self.x = self.x + 1 }
}
S(10).x
"""
    cat.parse(code)
    assert cat.execute() == 11


def test_init_return_ignored(cat):
    """init return value is discarded, instance is returned."""
    code = """
struct S {
    x
    init(self) => { self.x = self.x * 2; 999 }
}
S(5).x
"""
    cat.parse(code)
    assert cat.execute() == 10


def test_init_with_defaults(cat):
    """init works with default field values."""
    code = """
struct Config {
    host, port = 8080
    init(self) => { self.host = self.host + ":auto" }
}
list(Config("localhost").host, Config("localhost").port)
"""
    cat.parse(code)
    assert cat.execute() == ["localhost:auto", 8080]


def test_init_with_kwargs(cat):
    """init works with keyword argument instantiation."""
    code = """
struct S {
    x, y
    init(self) => { self.x = self.x + self.y }
}
S(x=10, y=5).x
"""
    cat.parse(code)
    assert cat.execute() == 15


def test_init_no_init(cat):
    """Struct without init works normally."""
    code = """
struct S { x }
S(42).x
"""
    cat.parse(code)
    assert cat.execute() == 42


# -- super --


def test_super_basic(cat):
    """super.method() calls parent version."""
    code = """
struct Base {
    x
    value(self) => { self.x }
}
struct Child extends(Base) {
    value(self) => { super.value() + 10 }
}
Child(5).value()
"""
    cat.parse(code)
    assert cat.execute() == 15


def test_super_chain(cat):
    """super works through a multi-level inheritance chain."""
    code = """
struct A {
    x
    value(self) => { self.x }
}
struct B extends(A) {
    value(self) => { super.value() + 10 }
}
struct C extends(B) {
    value(self) => { super.value() + 100 }
}
C(1).value()
"""
    cat.parse(code)
    assert cat.execute() == 111


def test_super_with_init(cat):
    """super.init() calls parent init."""
    code = """
struct Base {
    x
    init(self) => { self.x = self.x + 1 }
}
struct Child extends(Base) {
    init(self) => {
        super.init()
        self.x = self.x * 10
    }
}
Child(5).x
"""
    cat.parse(code)
    assert cat.execute() == 60


def test_super_no_parent_error(cat):
    """Accessing super without extends raises an error."""
    code = """
struct S {
    x
    value(self) => { super.value() }
}
S(1).value()
"""
    cat.parse(code)
    with pytest.raises(Exception):
        cat.execute()


# -- extends + implements combined --


def test_extends_then_implements(cat):
    """struct S extends(B) implements(T) works."""
    code = """
trait Loggable { log(self) => { "logged" } }
struct Base { x }
struct Child extends(Base) implements(Loggable) { y }
c = Child(1, 2)
list(c.x, c.y, c.log())
"""
    cat.parse(code)
    assert cat.execute() == [1, 2, "logged"]


def test_implements_then_extends(cat):
    """struct S implements(T) extends(B) also works."""
    code = """
trait Loggable { log(self) => { "logged" } }
struct Base { x }
struct Child implements(Loggable) extends(Base) { y }
c = Child(1, 2)
list(c.x, c.y, c.log())
"""
    cat.parse(code)
    assert cat.execute() == [1, 2, "logged"]


# -- Scope edge cases --


def test_struct_redefinition_shadowing(cat):
    """Struct defined in inner scope shadows outer."""
    code = """
struct S { x }
a = S(1)
result = {
    struct S { x, y }
    S(10, 20).y
}
list(a.x, result)
"""
    cat.parse(code)
    assert cat.execute() == [1, 20]


def test_init_in_struct_with_traits(cat):
    """init interacts correctly with trait methods."""
    code = """
trait T { m(self) => { self.x * 2 } }
struct S implements(T) {
    x
    init(self) => { self.x = self.x + 1 }
}
s = S(5)
list(s.x, s.m())
"""
    cat.parse(code)
    assert cat.execute() == [6, 12]


def test_method_uses_mutated_field(cat):
    """Method sees field values after init mutation."""
    code = """
struct S {
    x
    init(self) => { self.x = self.x * 3 }
    get(self) => { self.x }
}
S(7).get()
"""
    cat.parse(code)
    assert cat.execute() == 21


def test_multiple_structs_independent(cat):
    """Multiple structs with init don't interfere."""
    code = """
struct A {
    x
    init(self) => { self.x = self.x + 1 }
}
struct B {
    x
    init(self) => { self.x = self.x * 2 }
}
list(A(10).x, B(10).x)
"""
    cat.parse(code)
    assert cat.execute() == [11, 20]


def test_super_only_for_overridden(cat):
    """super provides access to parent methods, non-overridden methods work directly."""
    code = """
struct Base {
    x
    base_only(self) => { self.x * 100 }
    shared(self) => { self.x }
}
struct Child extends(Base) {
    shared(self) => { super.shared() + 1 }
}
c = Child(5)
list(c.base_only(), c.shared())
"""
    cat.parse(code)
    assert cat.execute() == [500, 6]
