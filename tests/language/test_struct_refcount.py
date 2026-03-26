# FILE: tests/language/test_struct_refcount.py
"""Tests for struct instance refcounting (regression tests for double-free bugs)."""


def test_struct_reassignment(cat):
    """Reassigning a struct variable must not crash (double-free on StoreScope)."""
    code = """
struct S { n; }
s = S(1)
s = S(2)
s.n
"""
    cat.parse(code)
    assert cat.execute() == 2


def test_struct_reassignment_multiple(cat):
    """Multiple sequential reassignments."""
    code = """
struct S { n; }
s = S(1)
s = S(2)
s = S(3)
s = S(4)
s.n
"""
    cat.parse(code)
    assert cat.execute() == 4


def test_struct_reassignment_in_for_loop(cat):
    """Struct reassignment inside a for loop."""
    code = """
struct S { n; }
result = list()
for i in range(5) {
    s = S(i)
    result.append(s.n)
}
result
"""
    cat.parse(code)
    assert cat.execute() == [0, 1, 2, 3, 4]


def test_struct_reassignment_in_while_loop(cat):
    """Struct reassignment inside a while loop."""
    code = """
struct S { n; }
i = 0
last = None
while i < 3 {
    last = S(i)
    i = i + 1
}
last.n
"""
    cat.parse(code)
    assert cat.execute() == 2


def test_struct_created_in_function_loop(cat):
    """Struct created by a function, reassigned in a loop."""
    code = """
struct Book { id; title; }
make = (i) => { Book(i, f"T{i}") }
result = list()
for i in range(3) {
    b = make(i)
    result.append(b.title)
}
result
"""
    cat.parse(code)
    assert cat.execute() == ["T0", "T1", "T2"]


def test_struct_in_list_multiple_iterations(cat):
    """Iterate over a list of structs multiple times."""
    code = """
struct P { x; }
items = list(P(1), P(2), P(3))
a = list()
for p in items { a.append(p.x) }
b = list()
for p in items { b.append(p.x * 10) }
list(a, b)
"""
    cat.parse(code)
    assert cat.execute() == [[1, 2, 3], [10, 20, 30]]


def test_struct_build_list_then_iterate(cat):
    """Build a list of structs via accumulation, then iterate."""
    code = """
struct S { n; }
items = list()
for i in range(3) {
    items = items + list(S(i))
}
result = list()
for s in items { result.append(s.n) }
result
"""
    cat.parse(code)
    assert cat.execute() == [0, 1, 2]


def test_struct_field_access_after_reassignment(cat):
    """Access fields of a struct that was previously reassigned."""
    code = """
struct Point { x; y; }
p = Point(1, 2)
old_x = p.x
p = Point(10, 20)
list(old_x, p.x)
"""
    cat.parse(code)
    assert cat.execute() == [1, 10]


def test_struct_reassignment_with_field_mutation(cat):
    """Mutate fields, then reassign."""
    code = """
struct S { x; }
s = S(1)
s.x = 10
s = S(2)
s.x
"""
    cat.parse(code)
    assert cat.execute() == 2


def test_struct_returned_from_function_reassigned(cat):
    """Function returns a struct, caller reassigns."""
    code = """
struct S { n; }
make = (x) => { S(x * 2) }
s = make(1)
s = make(2)
s = make(3)
s.n
"""
    cat.parse(code)
    assert cat.execute() == 6


def test_struct_in_conditional_reassignment(cat):
    """Struct reassigned inside if/else branches."""
    code = """
struct S { n; }
s = S(0)
if True {
    s = S(1)
}
s.n
"""
    cat.parse(code)
    assert cat.execute() == 1


def test_struct_match_with_reassignment(cat):
    """Pattern match on struct, then reassign."""
    code = """
struct S { kind; val; }
s = S("a", 1)
result = match s.kind {
    "a" => { s.val * 10 }
    _ => { 0 }
}
s = S("b", 2)
list(result, s.val)
"""
    cat.parse(code)
    assert cat.execute() == [10, 2]


def test_struct_broadcast_after_reassignment(cat):
    """Broadcast over structs that were built via reassignment."""
    code = """
struct S { n; }
items = list()
for i in range(3) {
    items = items + list(S(i))
}
items.[(s) => { s.n * 2 }]
"""
    cat.parse(code)
    assert cat.execute() == [0, 2, 4]


def test_struct_nested_field_struct_reassignment(cat):
    """Struct with a struct field, outer reassigned."""
    code = """
struct Inner { x; }
struct Outer { inner; }
o = Outer(Inner(1))
o = Outer(Inner(2))
o.inner.x
"""
    cat.parse(code)
    assert cat.execute() == 2
