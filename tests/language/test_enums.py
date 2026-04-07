# FILE: tests/language/test_enums.py
"""Tests for enum type definitions."""

import tempfile
from pathlib import Path

import pytest
from catnip import Catnip


class TestEnumBasic:
    """Basic enum declaration and variant access."""

    def test_enum_declaration(self):
        cat = Catnip()
        code = """
enum Color { red; green; blue }
Color
"""
        cat.parse(code)
        result = cat.execute()
        assert result is not None

    def test_variant_access(self):
        cat = Catnip()
        code = """
enum Color { red; green; blue }
Color.red
"""
        cat.parse(code)
        result = cat.execute()
        assert result is not None

    def test_variant_equality(self):
        cat = Catnip()
        code = """
enum Color { red; green; blue }
Color.red == Color.red
"""
        cat.parse(code)
        result = cat.execute()
        assert result is True

    def test_variant_inequality(self):
        cat = Catnip()
        code = """
enum Color { red; green; blue }
Color.red == Color.blue
"""
        cat.parse(code)
        result = cat.execute()
        assert result is False

    def test_variant_ne(self):
        cat = Catnip()
        code = """
enum Color { red; green; blue }
Color.red != Color.blue
"""
        cat.parse(code)
        result = cat.execute()
        assert result is True

    def test_variant_assign_compare(self):
        cat = Catnip()
        code = """
enum Direction { up; down; left; right }
d = Direction.up
d == Direction.up
"""
        cat.parse(code)
        result = cat.execute()
        assert result is True

    def test_single_variant(self):
        cat = Catnip()
        code = """
enum Unit { value }
Unit.value == Unit.value
"""
        cat.parse(code)
        result = cat.execute()
        assert result is True


class TestEnumErrors:
    """Error handling for enum declarations."""

    def test_empty_enum(self):
        cat = Catnip()
        code = "enum Empty { }"
        with pytest.raises(Exception):
            cat.parse(code)
            cat.execute()

    def test_duplicate_variant(self):
        cat = Catnip()
        code = "enum Bad { a; a }"
        with pytest.raises(Exception):
            cat.parse(code)
            cat.execute()

    def test_nonexistent_variant(self):
        cat = Catnip()
        code = """
enum Color { red; green; blue }
Color.yellow
"""
        cat.parse(code)
        with pytest.raises(Exception):
            cat.execute()


class TestEnumMatch:
    """Pattern matching with enums."""

    def test_match_basic(self):
        cat = Catnip()
        code = """
enum Color { red; green; blue }
c = Color.green
match c {
    Color.red => { 1 }
    Color.green => { 2 }
    Color.blue => { 3 }
}
"""
        cat.parse(code)
        result = cat.execute()
        assert result == 2

    def test_match_with_wildcard(self):
        cat = Catnip()
        code = """
enum Color { red; green; blue }
c = Color.blue
match c {
    Color.red => { 1 }
    _ => { 0 }
}
"""
        cat.parse(code)
        result = cat.execute()
        assert result == 0


class TestEnumMultiple:
    """Multiple enum types in same scope."""

    def test_two_enums(self):
        cat = Catnip()
        code = """
enum Color { red; blue }
enum Direction { up; down }
Color.red != Direction.up
"""
        cat.parse(code)
        result = cat.execute()
        assert result is True

    def test_same_variant_name_different_enum(self):
        cat = Catnip()
        code = """
enum A { x; y }
enum B { x; y }
A.x != B.x
"""
        cat.parse(code)
        result = cat.execute()
        assert result is True


class TestEnumTruthiness:
    """Enum values are always truthy."""

    def test_enum_truthy(self):
        cat = Catnip()
        code = """
enum Color { red; green; blue }
if Color.red { 1 } else { 0 }
"""
        cat.parse(code)
        result = cat.execute()
        assert result == 1


class TestEnumTypeof:
    """typeof() returns the enum type name."""

    def test_typeof_variant(self):
        cat = Catnip()
        cat.parse('enum Color { red; green; blue }; typeof(Color.red)')
        assert cat.execute() == "Color"

    def test_typeof_after_roundtrip(self):
        cat = Catnip()
        cat.parse("""
enum Color { red; green; blue }
id = (x) => { x }
typeof(id(Color.red))
""")
        assert cat.execute() == "Color"


class TestEnumOpacity:
    """Enum variants are opaque -- not equal to strings."""

    def test_not_equal_to_string(self):
        cat = Catnip()
        cat.parse('enum Color { red }; Color.red == "Color.red"')
        assert cat.execute() is False

    def test_no_string_methods(self):
        cat = Catnip()
        cat.parse('enum Color { red }; Color.red.upper()')
        with pytest.raises(Exception):
            cat.execute()


class TestEnumImport:
    """Enum types imported from .cat modules."""

    def _catnip_in(self, tmpdir):
        c = Catnip()
        c.context.globals['META'].file = str(Path(tmpdir) / '__test__.cat')
        return c

    def test_import_enum_type(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "colors.cat").write_text(
                'enum Color { red; green; blue }\n'
                'describe = (c) => {\n'
                '    match c {\n'
                '        Color.red => { "r" }\n'
                '        _ => { "?" }\n'
                '    }\n'
                '}\n'
            )
            cat = self._catnip_in(tmpdir)
            cat.parse('m = import("colors"); m.describe(m.Color.red)')
            assert cat.execute() == "r"

    def test_import_closure_capturing_enum(self):
        """Closure exported from module captures an enum variant."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "modes.cat").write_text(
                'enum Mode { fast; slow }\n' 'default_mode = Mode.fast\n' 'get_default = () => { default_mode }\n'
            )
            cat = self._catnip_in(tmpdir)
            cat.parse("""
m = import("modes")
result = m.get_default()
typeof(result)
""")
            result = cat.execute()
            assert result == "Mode"

    def test_import_nested_closure_capturing_enum_via_scope_chain(self):
        """Nested closure captures enum variant through parent scope chain.

        module defines:
          enum S { on; off }
          outer closes over S.on
          outer returns inner which closes over outer's capture
        importer calls m.make()() -- inner must resolve S.on correctly.
        """
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "nested.cat").write_text(
                'enum S { on; off }\n'
                'val = S.on\n'
                'make = () => {\n'
                '    captured = val\n'
                '    inner = () => { captured }\n'
                '    inner\n'
                '}\n'
            )
            cat = self._catnip_in(tmpdir)
            cat.parse("""
m = import("nested")
getter = m.make()
result = getter()
typeof(result) == "S"
""")
            assert cat.execute() is True
