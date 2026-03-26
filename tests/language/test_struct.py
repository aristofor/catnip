# FILE: tests/language/test_struct.py
"""Tests for struct declarations."""

import pytest


def test_struct_basic_creation(cat):
    """Test basic struct declaration and instantiation."""
    code = """struct Point { x; y; }
p = Point(1, 2)
p.x"""
    cat.parse(code)
    result = cat.execute()
    assert result == 1


def test_struct_keyword_args(cat):
    """Test struct instantiation with keyword arguments."""
    code = """struct Point { x; y; }
p = Point(x=10, y=20)
p.y"""
    cat.parse(code)
    result = cat.execute()
    assert result == 20


def test_struct_mixed_args(cat):
    """Test struct with mixed positional and keyword arguments."""
    code = """struct Point { x; y; z; }
p = Point(1, y=2, z=3)
p.z"""
    cat.parse(code)
    result = cat.execute()
    assert result == 3


def test_struct_repr(cat):
    """Test struct repr output."""
    code = """struct Point { x; y; }
p = Point(1, 2)
str(p)"""
    cat.parse(code)
    result = cat.execute()
    assert "Point" in result
    assert "x=1" in result
    assert "y=2" in result


def test_struct_equality(cat):
    """Test struct equality comparison."""
    code = """struct Point { x; y; }
p1 = Point(1, 2)
p2 = Point(1, 2)
p3 = Point(2, 3)
list(p1 == p2, p1 == p3)"""
    cat.parse(code)
    result = cat.execute()
    assert result == [True, False]


def test_struct_attribute_access(cat):
    """Test accessing all struct attributes."""
    code = """struct Vector3D { x; y; z; }
v = Vector3D(10, 20, 30)
list(v.x, v.y, v.z)"""
    cat.parse(code)
    result = cat.execute()
    assert result == [10, 20, 30]


def test_struct_multiple_instances(cat):
    """Test creating multiple instances of the same struct."""
    code = """struct Point { x; y; }
p1 = Point(1, 2)
p2 = Point(3, 4)
list(p1.x, p2.x, p1.y, p2.y)"""
    cat.parse(code)
    result = cat.execute()
    assert result == [1, 3, 2, 4]


def test_struct_nested_in_list(cat):
    """Test structs as elements in collections."""
    code = """struct Point { x; y; }
points = list(Point(1, 2), Point(3, 4))
points[0].x + points[1].y"""
    cat.parse(code)
    result = cat.execute()
    assert result == 5


def test_struct_missing_field_error(cat):
    """Test that missing required fields raise an error."""
    code = """struct Point { x; y; }
Point(1)"""
    cat.parse(code)
    with pytest.raises(Exception) as exc_info:
        cat.execute()
    assert "missing" in str(exc_info.value).lower() and "argument" in str(exc_info.value).lower()


def test_struct_unexpected_keyword_error(cat):
    """Test that unexpected keyword arguments raise an error."""
    code = """struct Point { x; y; }
Point(x=1, y=2, z=3)"""
    cat.parse(code)
    with pytest.raises(Exception) as exc_info:
        cat.execute()
    assert "unexpected keyword argument" in str(exc_info.value).lower()


def test_struct_too_many_positional_error(cat):
    """Test that too many positional arguments raise an error."""
    code = """struct Point { x; y; }
Point(1, 2, 3)"""
    cat.parse(code)
    with pytest.raises(Exception) as exc_info:
        cat.execute()
    assert "takes" in str(exc_info.value).lower() and "argument" in str(exc_info.value).lower()


def test_struct_single_field(cat):
    """Test struct with a single field."""
    code = """struct Scalar { value; }
s = Scalar(42)
s.value"""
    cat.parse(code)
    result = cat.execute()
    assert result == 42


def test_struct_many_fields(cat):
    """Test struct with many fields."""
    code = """struct Record { a; b; c; d; e; }
r = Record(1, 2, 3, 4, 5)
r.c"""
    cat.parse(code)
    result = cat.execute()
    assert result == 3


def test_struct_with_complex_values(cat):
    """Test struct storing complex values like lists and dicts."""
    code = """struct Container { data; meta; }
c = Container(list(1, 2, 3), dict(name="test"))
list(c.data, c.meta["name"])"""
    cat.parse(code)
    result = cat.execute()
    assert result == [[1, 2, 3], "test"]


def test_struct_field_mutation(cat):
    """Test that struct fields can be mutated after creation."""
    code = """struct Point { x; y; }
p = Point(1, 2)
p.x = 10
p.x"""
    cat.parse(code)
    result = cat.execute()
    assert result == 10


def test_struct_multiple_definitions(cat):
    """Test defining multiple different structs."""
    code = """struct Point { x; y; }
struct Color { r; g; b; }
p = Point(1, 2)
c = Color(255, 0, 128)
list(p.x, c.g)"""
    cat.parse(code)
    result = cat.execute()
    assert result == [1, 0]


def test_struct_default_values(cat):
    """Test struct with default field values."""
    code = """struct Point { x; y = 0; }
p = Point(5)
list(p.x, p.y)"""
    cat.parse(code)
    result = cat.execute()
    assert result == [5, 0]


def test_struct_default_override(cat):
    """Test overriding default values with positional args."""
    code = """struct Point { x; y = 0; }
p = Point(1, 2)
list(p.x, p.y)"""
    cat.parse(code)
    result = cat.execute()
    assert result == [1, 2]


def test_struct_default_keyword_override(cat):
    """Test overriding defaults with keyword args."""
    code = """struct Point { x; y = 0; }
p = Point(x=3, y=7)
list(p.x, p.y)"""
    cat.parse(code)
    result = cat.execute()
    assert result == [3, 7]


def test_struct_multiple_defaults(cat):
    """Test struct with multiple default fields."""
    code = """struct Config { host; port = 8080; debug = False; }
c = Config("localhost")
list(c.host, c.port, c.debug)"""
    cat.parse(code)
    result = cat.execute()
    assert result == ["localhost", 8080, False]


def test_struct_all_defaults(cat):
    """Test struct where all fields have defaults."""
    code = """struct Opts { verbose = False; retries = 3; }
o = Opts()
list(o.verbose, o.retries)"""
    cat.parse(code)
    result = cat.execute()
    assert result == [False, 3]


def test_struct_default_ordering_error(cat):
    """Test that non-default field after default field raises parse error."""
    code = """struct Bad { x = 0; y; }"""
    with pytest.raises(Exception) as exc_info:
        cat.parse(code)
    assert "non-default field" in str(exc_info.value).lower()


def test_struct_default_with_methods(cat):
    """Test struct with both defaults and methods."""
    code = """
struct Vec2 {
    x; y = 0;

    length(self) => { (self.x ** 2 + self.y ** 2) ** 0.5 }
}
v = Vec2(3, 4)
v.length()
"""
    cat.parse(code)
    result = cat.execute()
    assert result == 5.0


def test_struct_default_with_pattern(cat):
    """Test pattern matching on struct with default fields."""
    code = """
struct Point { x; y = 0; }
p = Point(10)
match p {
    Point{x, y} => { x + y }
}
"""
    cat.parse(code)
    result = cat.execute()
    assert result == 10


def test_struct_extends_inherits_fields_and_methods(cat):
    """Child struct inherits parent fields and methods with extends(Base)."""
    code = """
struct Point {
    x; y;
    sum(self) => { self.x + self.y }
}

struct Point3 extends(Point) {
    z;
}

p = Point3(1, 2, 3)
list(p.x, p.y, p.z, p.sum())
"""
    cat.parse(code)
    result = cat.execute()
    assert result == [1, 2, 3, 3]


def test_struct_extends_method_override(cat):
    """Child methods override inherited methods."""
    code = """
struct Base {
    x;
    value(self) => { self.x }
}

struct Child extends(Base) {
    value(self) => { self.x * 2 }
}

c = Child(7)
c.value()
"""
    cat.parse(code)
    result = cat.execute()
    assert result == 14


def test_struct_extends_unknown_base_error(cat):
    """Extending an unknown base struct must fail at declaration runtime."""
    code = """
struct Child extends(UnknownBase) {
    x;
}
"""
    cat.parse(code)
    with pytest.raises(Exception) as exc_info:
        cat.execute()
    assert "unknown base struct" in str(exc_info.value).lower()


# --- Recursive struct tests (struct instances in collections) ---


def test_struct_recursive_linked_list(cat):
    """Recursive method on a linked list (direct field reference)."""
    code = """
struct Node {
    value; next = None;
    sum_all(self) => {
        if (self.next == None) { self.value }
        else { self.value + self.next.sum_all() }
    }
}
n1 = Node(1, None)
n2 = Node(2, n1)
n3 = Node(3, n2)
n3.sum_all()
"""
    cat.parse(code)
    assert cat.execute() == 6


def test_struct_recursive_tree_children_list(cat):
    """Recursive method on a tree with children stored in list()."""
    code = """
struct TreeNode {
    value; children = None;
    sum_tree(self) => {
        if (self.children == None) { self.value }
        else {
            total = self.value
            for child in self.children {
                total = total + child.sum_tree()
            }
            total
        }
    }
}
leaf1 = TreeNode(1, None)
leaf2 = TreeNode(2, None)
branch = TreeNode(10, list(leaf1, leaf2))
root = TreeNode(100, list(branch))
root.sum_tree()
"""
    cat.parse(code)
    assert cat.execute() == 113


def test_struct_recursive_deep_tree(cat):
    """Multi-level tree with multiple children per node."""
    code = """
struct T {
    v; kids = None;
    total(self) => {
        if (self.kids == None) { self.v }
        else {
            s = self.v
            for k in self.kids { s = s + k.total() }
            s
        }
    }
}
a = T(1, None)
b = T(2, None)
c = T(3, None)
d = T(10, list(a, b))
e = T(20, list(c))
root = T(100, list(d, e))
root.total()
"""
    cat.parse(code)
    assert cat.execute() == 136


def test_struct_in_list_method_no_recursion(cat):
    """Method call on struct extracted from list (no recursion)."""
    code = """
struct Point {
    x; y;
    sum(self) => { self.x + self.y }
}
points = list(Point(1, 2), Point(3, 4), Point(5, 6))
points[0].sum() + points[1].sum() + points[2].sum()
"""
    cat.parse(code)
    assert cat.execute() == 21


def test_struct_in_list_field_access(cat):
    """Field access on struct extracted from list."""
    code = """
struct Vec3 { x; y; z; }
vecs = list(Vec3(1, 2, 3), Vec3(4, 5, 6))
vecs[0].x + vecs[1].z
"""
    cat.parse(code)
    assert cat.execute() == 7


def test_struct_in_list_broadcast(cat):
    code = """
struct Point { x; y; }
points = list(Point(1, 2), Point(3, 4), Point(5, 6))
points.[(p) => { p.x + p.y }]
"""
    cat.parse(code)
    assert cat.execute() == [3, 7, 11]


def test_struct_recursive_with_mutation(cat):
    """Recursive traversal after field mutation."""
    code = """
struct Node {
    value; next = None;
    sum_all(self) => {
        if (self.next == None) { self.value }
        else { self.value + self.next.sum_all() }
    }
}
n1 = Node(1, None)
n2 = Node(2, n1)
n2.value = 20
n2.sum_all()
"""
    cat.parse(code)
    assert cat.execute() == 21
