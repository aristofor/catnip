# FILE: tests/language/test_abstract_methods.py
"""Tests for @abstract method declarations in traits and structs."""

import pytest

# -- Trait with abstract method --


def test_trait_abstract_method_implemented(cat):
    """Concrete struct implements abstract method from trait."""
    code = """
trait Printable {
    @abstract
    to_string(self)
}
struct Point implements(Printable) {
    x; y;
    to_string(self) => { "Point" }
}
Point(1, 2).to_string()
"""
    cat.parse(code)
    assert cat.execute() == "Point"


def test_trait_abstract_method_not_implemented(cat):
    """Missing implementation of abstract method raises at struct definition."""
    code = """
trait Printable {
    @abstract
    to_string(self)
}
struct Point implements(Printable) { x; y; }
"""
    cat.parse(code)
    with pytest.raises(Exception, match=r"abstract method"):
        cat.execute()


def test_trait_abstract_instantiation_guard(cat):
    """Cannot instantiate struct with unresolved abstract methods."""
    code = """
trait Showable {
    @abstract
    show(self)
}
struct Point implements(Showable) { x; y; }
"""
    cat.parse(code)
    with pytest.raises(Exception, match=r"abstract"):
        cat.execute()


# -- Struct with own abstract method --


def test_struct_abstract_method_inherited(cat):
    """Abstract method in base struct, implemented in child."""
    code = """
struct Animal {
    name;
    @abstract
    speak(self)
}
struct Dog extends(Animal) {
    speak(self) => { self.name + " says woof" }
}
Dog("Rex").speak()
"""
    cat.parse(code)
    assert cat.execute() == "Rex says woof"


def test_struct_abstract_cannot_instantiate(cat):
    """Direct instantiation of struct with abstract method fails."""
    code = """
struct Animal {
    name;
    @abstract
    speak(self)
}
Animal("test")
"""
    cat.parse(code)
    with pytest.raises(Exception, match=r"abstract"):
        cat.execute()


def test_struct_abstract_child_missing_impl(cat):
    """Child struct that doesn't implement parent abstract method fails."""
    code = """
struct Animal {
    name;
    @abstract
    speak(self)
}
struct Cat extends(Animal) { }
Cat("Whiskers")
"""
    cat.parse(code)
    with pytest.raises(Exception, match=r"abstract"):
        cat.execute()


# -- Mixed concrete and abstract methods --


def test_trait_mixed_concrete_and_abstract(cat):
    """Trait with both concrete and abstract methods."""
    code = """
trait Serializable {
    @abstract
    serialize(self)
    type_name(self) => { "object" }
}
struct Config implements(Serializable) {
    key;
    serialize(self) => { self.key }
}
c = Config("debug")
list(c.serialize(), c.type_name())
"""
    cat.parse(code)
    assert cat.execute() == ["debug", "object"]


# -- Trait hierarchy with abstract methods --


def test_trait_extends_abstract_propagation(cat):
    """Abstract method propagates through trait extends chain."""
    code = """
trait Base {
    @abstract
    render(self)
}
trait Styled extends(Base) {
    style(self) => { "default" }
}
struct Widget implements(Styled) {
    name;
    render(self) => { self.name }
}
w = Widget("btn")
list(w.render(), w.style())
"""
    cat.parse(code)
    assert cat.execute() == ["btn", "default"]


def test_trait_extends_abstract_not_implemented(cat):
    """Abstract from parent trait not implemented raises error."""
    code = """
trait Base {
    @abstract
    render(self)
}
trait Styled extends(Base) {
    style(self) => { "default" }
}
struct Widget implements(Styled) { name; }
"""
    cat.parse(code)
    with pytest.raises(Exception, match=r"abstract"):
        cat.execute()


def test_trait_abstract_satisfied_by_child_trait(cat):
    """Child trait provides concrete implementation of parent abstract."""
    code = """
trait Base {
    @abstract
    render(self)
}
trait Concrete extends(Base) {
    render(self) => { "concrete" }
}
struct Widget implements(Concrete) { name; }
Widget("x").render()
"""
    cat.parse(code)
    assert cat.execute() == "concrete"


# -- @abstract on init is forbidden --


def test_abstract_init_forbidden(cat):
    """@abstract on init method should be rejected at parse time."""
    code = """
struct Foo {
    x;
    @abstract
    init(self)
}
"""
    with pytest.raises(Exception, match=r"init cannot be abstract"):
        cat.parse(code)


# -- Multiple abstract methods --


def test_multiple_abstract_methods(cat):
    """Struct must implement all abstract methods from trait."""
    code = """
trait Codec {
    @abstract
    encode(self)
    @abstract
    decode(self)
}
struct Json implements(Codec) {
    data;
    encode(self) => { "encoded" }
    decode(self) => { "decoded" }
}
j = Json("test")
list(j.encode(), j.decode())
"""
    cat.parse(code)
    assert cat.execute() == ["encoded", "decoded"]


def test_multiple_abstract_partial_impl_fails(cat):
    """Implementing only some abstract methods still fails."""
    code = """
trait Codec {
    @abstract
    encode(self)
    @abstract
    decode(self)
}
struct Json implements(Codec) {
    data;
    encode(self) => { "encoded" }
}
"""
    cat.parse(code)
    with pytest.raises(Exception, match=r"abstract"):
        cat.execute()
