# FILE: tests/language/test_multiple_inheritance.py
"""Tests for multiple inheritance with C3 linearization."""

import pytest

# ── Diamond: basic fields and construction ────────────────────────────────


def test_diamond_fields(cat):
    """Diamond inheritance merges fields via MRO (first-seen wins)."""
    code = """
struct A { x; }
struct B extends(A) { y; }
struct C extends(A) { z; }
struct D extends(B, C) { w; }

d = D(1, 2, 3, 4)
list(d.x, d.y, d.z, d.w)
"""
    cat.parse(code)
    result = cat.execute()
    assert result == [1, 2, 3, 4]


def test_diamond_no_duplicate_fields(cat):
    """Shared ancestor field 'x' appears only once in diamond."""
    code = """
struct A { x; }
struct B extends(A) { y; }
struct C extends(A) { z; }
struct D extends(B, C) { w; }

d = D(10, 20, 30, 40)
d.x
"""
    cat.parse(code)
    result = cat.execute()
    assert result == 10


# ── Method resolution (left priority) ─────────────────────────────────────


def test_method_left_priority(cat):
    """Left parent's method wins over right parent's."""
    code = """
struct A {
    value(self) => { "A" }
}
struct B extends(A) {
    value(self) => { "B" }
}
struct C extends(A) {
    value(self) => { "C" }
}
struct D extends(B, C) {}

d = D()
d.value()
"""
    cat.parse(code)
    result = cat.execute()
    assert result == "B"


def test_method_inherited_from_ancestor(cat):
    """Method from shared ancestor is accessible."""
    code = """
struct A {
    x;
    greet(self) => { "hello from A" }
}
struct B extends(A) {}
struct C extends(A) {}
struct D extends(B, C) {}

d = D(42)
d.greet()
"""
    cat.parse(code)
    result = cat.execute()
    assert result == "hello from A"


# ── Cooperative super (MRO-based) ────────────────────────────────────────


def test_cooperative_super_diamond(cat):
    """super follows MRO: D -> B -> C -> A."""
    code = """
struct A {
    value(self) => { "A" }
}
struct B extends(A) {
    value(self) => { "B->" + super.value() }
}
struct C extends(A) {
    value(self) => { "C->" + super.value() }
}
struct D extends(B, C) {
    value(self) => { "D->" + super.value() }
}

d = D()
d.value()
"""
    cat.parse(code)
    result = cat.execute()
    # MRO: [D, B, C, A]
    # D.value -> super -> B.value -> super -> C.value -> super -> A.value
    assert result == "D->B->C->A"


def test_super_init_chaining(cat):
    """init methods chain via super following MRO."""
    code = """
struct A {
    trace = "";
    init(self) => { self.trace = self.trace + "A" }
}
struct B extends(A) {
    init(self) => {
        super.init()
        self.trace = self.trace + "B"
    }
}
struct C extends(A) {
    init(self) => {
        super.init()
        self.trace = self.trace + "C"
    }
}
struct D extends(B, C) {
    init(self) => {
        super.init()
        self.trace = self.trace + "D"
    }
}

d = D()
d.trace
"""
    cat.parse(code)
    result = cat.execute()
    # MRO: [D, B, C, A]
    # D.init -> super.init (B) -> super.init (C) -> super.init (A)
    # A writes "A", C appends "C", B appends "B", D appends "D"
    assert result == "ACBD"


# ── C3 linearization failure ─────────────────────────────────────────────


def test_c3_inconsistent_hierarchy(cat):
    """Inconsistent hierarchy must fail with C3 error."""
    code = """
struct X {}
struct Y {}
struct A extends(X, Y) {}
struct B extends(Y, X) {}
struct D extends(A, B) {}
"""
    cat.parse(code)
    with pytest.raises(Exception) as exc_info:
        cat.execute()
    assert "C3 linearization failed" in str(exc_info.value)


# ── Extends + implements interaction ──────────────────────────────────────


def test_extends_and_implements(cat):
    """Multiple inheritance works alongside trait implements."""
    code = """
trait Greetable {
    greet(self) => { "hi from trait" }
}

struct A {
    x;
}
struct B extends(A) {
    y;
}
struct C implements(Greetable) extends(B) {
    z;
}

c = C(1, 2, 3)
list(c.x, c.y, c.z, c.greet())
"""
    cat.parse(code)
    result = cat.execute()
    assert result == [1, 2, 3, "hi from trait"]


# ── Single inheritance retrocompatibility ─────────────────────────────────


def test_single_inheritance_still_works(cat):
    """Single extends(Base) continues to work as before."""
    code = """
struct Base {
    x;
    value(self) => { self.x * 2 }
}
struct Child extends(Base) {
    y;
    both(self) => { self.x + self.y }
}

c = Child(3, 7)
list(c.value(), c.both())
"""
    cat.parse(code)
    result = cat.execute()
    assert result == [6, 10]


def test_single_inheritance_super(cat):
    """Single inheritance super still works correctly."""
    code = """
struct Base {
    x;
    value(self) => { self.x }
}
struct Child extends(Base) {
    value(self) => { super.value() + 100 }
}

c = Child(5)
c.value()
"""
    cat.parse(code)
    result = cat.execute()
    assert result == 105


# ── Child cannot redefine inherited field ─────────────────────────────────


def test_redefine_inherited_field_error(cat):
    """Redefining a parent field must be an error."""
    code = """
struct A { x; }
struct B extends(A) { x; }
"""
    cat.parse(code)
    with pytest.raises(Exception) as exc_info:
        cat.execute()
    assert "redefines inherited field" in str(exc_info.value).lower()


# ── Multiple parents with defaults ────────────────────────────────────────


def test_multiple_parents_with_defaults(cat):
    """Default values from parents are preserved."""
    code = """
struct A { x = 10; }
struct B { y = 20; }
struct C extends(A, B) { z = 30; }

c = C()
list(c.x, c.y, c.z)
"""
    cat.parse(code)
    result = cat.execute()
    assert result == [10, 20, 30]


# ── Static methods inheritance ────────────────────────────────────────────


def test_static_method_inherited(cat):
    """Static methods are inherited through MRO."""
    code = """
struct A {
    x;
    @static
    create() => { A(42) }
}
struct B extends(A) {
    y;
}

b = B(1, 2)
a = A.create()
a.x
"""
    cat.parse(code)
    result = cat.execute()
    assert result == 42


# ── Three-level linear chain ─────────────────────────────────────────────


def test_three_level_super_chain(cat):
    """Super chain works through three levels."""
    code = """
struct A {
    value(self) => { 1 }
}
struct B extends(A) {
    value(self) => { super.value() + 10 }
}
struct C extends(B) {
    value(self) => { super.value() + 100 }
}

c = C()
c.value()
"""
    cat.parse(code)
    result = cat.execute()
    assert result == 111


# ── Child override in diamond ─────────────────────────────────────────────


def test_child_override_in_diamond(cat):
    """Child's own method overrides all parents."""
    code = """
struct A {
    value(self) => { "A" }
}
struct B extends(A) {
    value(self) => { "B" }
}
struct C extends(A) {
    value(self) => { "C" }
}
struct D extends(B, C) {
    value(self) => { "D" }
}

d = D()
d.value()
"""
    cat.parse(code)
    result = cat.execute()
    assert result == "D"
