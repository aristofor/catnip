# FILE: tests/language/test_traits.py
"""Tests for trait composition."""

import pytest

from catnip import Catnip

# -- Basic trait with method --


def test_trait_basic_method(cat):
    code = """
trait Greetable {
    greet(self) => { "hello" }
}
struct Person implements(Greetable) { name; }
Person("Alice").greet()
"""
    cat.parse(code)
    assert cat.execute() == "hello"


def test_trait_method_accesses_self(cat):
    code = """
trait Describable {
    describe(self) => { self.label }
}
struct Widget implements(Describable) { label; }
Widget("btn").describe()
"""
    cat.parse(code)
    assert cat.execute() == "btn"


# -- Trait extends --


def test_trait_extends(cat):
    code = """
trait A {
    foo(self) => { 1 }
}
trait B extends(A) {
    bar(self) => { 2 }
}
struct S implements(B) { }
list(S().foo(), S().bar())
"""
    cat.parse(code)
    assert cat.execute() == [1, 2]


# -- Struct override --


def test_struct_override_trait_method(cat):
    code = """
trait T {
    m(self) => { 10 }
}
struct S implements(T) {
    m(self) => { 20 }
}
S().m()
"""
    cat.parse(code)
    assert cat.execute() == 20


# -- Conflict detection --


def test_trait_conflict_error(cat):
    code = """
trait X {
    f(self) => { 1 }
}
trait Y {
    f(self) => { 2 }
}
struct S implements(X, Y) { }
"""
    cat.parse(code)
    with pytest.raises(Exception, match="conflicting"):
        cat.execute()


def test_trait_conflict_resolved_by_override(cat):
    code = """
trait X {
    f(self) => { 1 }
}
trait Y {
    f(self) => { 2 }
}
struct S implements(X, Y) {
    f(self) => { 3 }
}
S().f()
"""
    cat.parse(code)
    assert cat.execute() == 3


# -- Diamond (dedupe silencieux) --


def test_diamond_dedupe(cat):
    code = """
trait Base {
    m(self) => { 0 }
}
trait L extends(Base) { }
trait R extends(Base) { }
struct S implements(L, R) { }
S().m()
"""
    cat.parse(code)
    assert cat.execute() == 0


def test_diamond_with_method_conflict(cat):
    code = """
trait Base2 { }
trait L2 extends(Base2) {
    f(self) => { 1 }
}
trait R2 extends(Base2) {
    f(self) => { 2 }
}
struct S2 implements(L2, R2) { }
"""
    cat.parse(code)
    with pytest.raises(Exception, match="conflicting"):
        cat.execute()


def test_diamond_conflict_resolved(cat):
    code = """
trait Base2 { }
trait L2 extends(Base2) {
    f(self) => { 1 }
}
trait R2 extends(Base2) {
    f(self) => { 2 }
}
struct S3 implements(L2, R2) {
    f(self) => { 3 }
}
S3().f()
"""
    cat.parse(code)
    assert cat.execute() == 3


# -- Multiple traits, no conflict --


def test_multiple_traits_no_conflict(cat):
    code = """
trait Readable {
    read(self) => { "read" }
}
trait Writable {
    write(self) => { "write" }
}
struct File implements(Readable, Writable) { path; }
f = File("/tmp")
list(f.read(), f.write())
"""
    cat.parse(code)
    assert cat.execute() == ["read", "write"]


# -- Trait without methods (fields only via struct) --


def test_trait_empty_body(cat):
    code = """
trait Marker { }
struct S implements(Marker) { x; }
S(42).x
"""
    cat.parse(code)
    assert cat.execute() == 42


# -- Child override parent method (not a conflict) --


def test_trait_child_override_parent_method(cat):
    """B extends(A), both define m() -> B.m wins, no conflict."""
    code = """
trait A {
    m(self) => { 1 }
}
trait B extends(A) {
    m(self) => { 2 }
}
struct S implements(B) { }
S().m()
"""
    cat.parse(code)
    assert cat.execute() == 2


def test_diamond_child_override_base_method(cat):
    """L overrides Base.m, R does not -> L.m wins (most derived)."""
    code = """
trait Base {
    m(self) => { 0 }
}
trait L extends(Base) {
    m(self) => { 1 }
}
trait R extends(Base) { }
struct S implements(L, R) { }
S().m()
"""
    cat.parse(code)
    assert cat.execute() == 1
