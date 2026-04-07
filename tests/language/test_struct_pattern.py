# FILE: tests/language/test_struct_pattern.py
"""Tests for structural pattern matching with structs: Point{x, y}"""

import pytest

from catnip import Catnip


class TestStructPatternBasic:
    """Basic struct pattern matching"""

    def test_simple_struct_pattern(self):
        cat = Catnip()
        code = """
        struct Point { x; y; }
        p = Point(1, 2)
        match p {
            Point{x, y} => { x + y }
        }
        """
        cat.parse(code)
        assert cat.execute() == 3

    def test_struct_pattern_field_binding(self):
        cat = Catnip()
        code = """
        struct Point { x; y; }
        p = Point(10, 20)
        match p {
            Point{x, y} => { x * y }
        }
        """
        cat.parse(code)
        assert cat.execute() == 200

    def test_struct_pattern_with_wildcard_fallback(self):
        cat = Catnip()
        code = """
        struct Point { x; y; }
        p = Point(5, 3)
        match p {
            _ => { 99 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 99

    def test_struct_pattern_type_mismatch(self):
        cat = Catnip()
        code = """
        struct Point { x; y; }
        struct Vec3 { x; y; z; }
        p = Point(1, 2)
        match p {
            Vec3{x, y, z} => { x + y + z }
            Point{x, y} => { x * y }
        }
        """
        cat.parse(code)
        assert cat.execute() == 2

    def test_struct_pattern_no_match(self):
        cat = Catnip()
        code = """
        struct Point { x; y; }
        struct Color { r; g; b; }
        c = Color(255, 0, 128)
        match c {
            Point{x, y} => { -1 }
            _ => { 42 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 42


class TestStructPatternMultipleFields:
    """Struct patterns with various field counts"""

    def test_single_field_struct(self):
        cat = Catnip()
        code = """
        struct Wrapper { value; }
        w = Wrapper(42)
        match w {
            Wrapper{value} => { value }
        }
        """
        cat.parse(code)
        assert cat.execute() == 42

    def test_three_field_struct(self):
        cat = Catnip()
        code = """
        struct Vec3 { x; y; z; }
        v = Vec3(1, 2, 3)
        match v {
            Vec3{x, y, z} => { x + y + z }
        }
        """
        cat.parse(code)
        assert cat.execute() == 6


class TestStructPatternWithGuards:
    """Struct patterns with guard conditions"""

    def test_struct_pattern_with_guard(self):
        cat = Catnip()
        code = """
        struct Point { x; y; }
        p = Point(0, 5)
        match p {
            Point{x, y} if x == 0 => { y * 10 }
            Point{x, y} => { x + y }
        }
        """
        cat.parse(code)
        assert cat.execute() == 50

    def test_struct_pattern_guard_fails(self):
        cat = Catnip()
        code = """
        struct Point { x; y; }
        p = Point(3, 5)
        match p {
            Point{x, y} if x == 0 => { y * 10 }
            Point{x, y} => { x + y }
        }
        """
        cat.parse(code)
        assert cat.execute() == 8


class TestStructPatternMissingField:
    """Pattern with field absent from the struct falls through to next arm."""

    def test_missing_field_falls_through(self, cat):
        code = """
        struct Point { x; y; }
        match Point(1, 2) {
            Point{x, z} => { -1 }
            _ => { 0 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 0

    def test_missing_field_tries_next_arm(self, cat):
        code = """
        struct Point { x; y; }
        match Point(3, 4) {
            Point{x, z} => { -1 }
            Point{x, y} => { x + y }
        }
        """
        cat.parse(code)
        assert cat.execute() == 7

    def test_partial_missing_fields(self, cat):
        code = """
        struct Vec3 { x; y; z; }
        match Vec3(1, 2, 3) {
            Vec3{x, y, w} => { -1 }
            Vec3{x, y, z} => { x + y + z }
        }
        """
        cat.parse(code)
        assert cat.execute() == 6


class TestStructPatternRedefinition:
    """Redefining a struct with the same name."""

    def test_redefined_struct_matches_new_shape(self, cat):
        code = """
        struct Point { x; y; }
        struct Point { a; b; c; }
        match Point(1, 2, 3) {
            Point{a, b, c} => { a + b + c }
            _ => { -1 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 6

    def test_redefined_struct_old_pattern_no_match(self, cat):
        code = """
        struct Point { x; y; }
        struct Point { a; b; c; }
        match Point(1, 2, 3) {
            Point{x, y} => { -1 }
            _ => { 0 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 0


class TestStructPatternAST:
    """Struct patterns in AST execution mode"""

    def test_struct_pattern_ast_mode(self):
        cat = Catnip(vm_mode='off')
        code = """
        struct Point { x; y; }
        p = Point(10, 20)
        match p {
            Point{x, y} => { x - y }
        }
        """
        cat.parse(code)
        assert cat.execute() == -10

    def test_struct_pattern_mismatch_ast(self):
        cat = Catnip(vm_mode='off')
        code = """
        struct Point { x; y; }
        struct Color { r; g; b; }
        c = Color(1, 2, 3)
        match c {
            Point{x, y} => { -1 }
            _ => { 0 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 0


class TestStructPatternInTuple:
    """Struct patterns nested inside tuple patterns: (Point{x, y}, z)"""

    def test_basic_struct_in_tuple(self):
        cat = Catnip()
        code = """
        struct Point { x; y }
        p = Point(1, 2)
        match tuple(p, 3) {
            (Point{x, y}, z) => { x + y + z }
            _ => { -1 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 6

    def test_struct_in_tuple_type_mismatch(self):
        cat = Catnip()
        code = """
        struct Point { x; y }
        struct Color { r; g; b }
        c = Color(10, 20, 30)
        match tuple(c, 99) {
            (Point{x, y}, z) => { -1 }
            _ => { 42 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 42

    def test_multiple_structs_in_tuple(self):
        cat = Catnip()
        code = """
        struct Point { x; y }
        struct Color { r; g; b }
        p = Point(1, 2)
        c = Color(10, 20, 30)
        match tuple(p, c) {
            (Point{x, y}, Color{r, g, b}) => { x + y + r + g + b }
            _ => { -1 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 63

    def test_struct_in_tuple_with_guard(self):
        cat = Catnip()
        code = """
        struct Point { x; y }
        match tuple(Point(0, 5), 10) {
            (Point{x, y}, z) if x == 0 => { y * z }
            (Point{x, y}, z) => { x + y + z }
        }
        """
        cat.parse(code)
        assert cat.execute() == 50

    def test_struct_in_tuple_with_literal(self):
        cat = Catnip()
        code = """
        struct Point { x; y }
        match tuple(Point(1, 2), "hello") {
            (Point{x, y}, "hello") => { x + y }
            _ => { -1 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 3

    def test_struct_in_tuple_with_wildcard(self):
        cat = Catnip()
        code = """
        struct Point { x; y }
        match tuple(Point(3, 4), 99) {
            (Point{x, y}, _) => { x * y }
        }
        """
        cat.parse(code)
        assert cat.execute() == 12

    def test_struct_in_tuple_ast_mode(self):
        cat = Catnip(vm_mode='off')
        code = """
        struct Point { x; y }
        match tuple(Point(5, 6), 7) {
            (Point{x, y}, z) => { x + y + z }
            _ => { -1 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 18

    def test_struct_in_triple_tuple(self):
        cat = Catnip()
        code = """
        struct Wrap { val }
        match tuple(1, Wrap(42), "end") {
            (a, Wrap{val}, b) => { val }
            _ => { -1 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 42


class TestStructPatternBroadcast:
    """Struct pattern matching on broadcast results (cross-VM identity)."""

    def test_broadcast_struct_match(self):
        """Struct created in broadcast closure is matched in parent scope."""
        cat = Catnip()
        code = """
        struct Wrap { val }
        items = list(1, 2, 3)
        results = items.[(x) => { Wrap(x) }]
        match results[0] {
            Wrap{val} => { val }
            _ => { -1 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 1

    def test_broadcast_nested_struct_match(self):
        """Nested structs from broadcast are matched correctly."""
        cat = Catnip()
        code = """
        struct Leaf { v }
        struct Node { left; right; }
        items = list(1, 2)
        results = items.[(x) => { Node(Leaf(x), Leaf(x + 10)) }]
        match results[0] {
            Node{left, right} => {
                match left {
                    Leaf{v} => { v }
                }
            }
        }
        """
        cat.parse(code)
        assert cat.execute() == 1

    def test_broadcast_chained_struct_match(self):
        """Chained broadcasts preserve struct identity."""
        cat = Catnip()
        code = """
        struct Num { v }
        items = list(1, 2, 3)
        step1 = items.[(x) => { Num(x * 10) }]
        step2 = step1.[(n) => {
            match n {
                Num{v} => { v + 1 }
                _ => { -1 }
            }
        }]
        step2
        """
        cat.parse(code)
        assert cat.execute() == [11, 21, 31]

    def test_broadcast_struct_field_access(self):
        """Field access works on structs returned from broadcast."""
        cat = Catnip()
        code = """
        struct Pair { a; b; }
        items = list(1, 2, 3)
        results = items.[(x) => { Pair(x, x * 2) }]
        results[1].b
        """
        cat.parse(code)
        assert cat.execute() == 4

    def test_broadcast_struct_reused_free_slot(self):
        """Child VM reuses a free-list slot from the parent registry."""
        cat = Catnip()
        code = """
        struct Tmp { x }
        struct Wrap { val }

        t = Tmp(0)
        t = None

        items = list(1, 2, 3)
        results = items.[(x) => { Wrap(x) }]
        match results[0] {
            Wrap{val} => { val }
            _ => { -1 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 1
