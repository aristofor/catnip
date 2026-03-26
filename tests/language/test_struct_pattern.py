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
