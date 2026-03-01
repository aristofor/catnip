# FILE: tests/language/test_setattr.py
"""Tests for attribute assignment (setattr) feature."""

import types
import unittest

from catnip import Catnip


class TestSetAttr(unittest.TestCase):
    """Test attribute assignment on objects."""

    def test_simple_setattr(self):
        """Simple attribute assignment: obj.x = value."""
        c = Catnip()
        obj = types.SimpleNamespace()
        c.context.globals['obj'] = obj

        c.parse('obj.x = 10')
        c.execute()

        self.assertEqual(obj.x, 10)

    def test_nested_setattr(self):
        """Nested attribute assignment: obj.a.b = value."""
        c = Catnip()
        obj = types.SimpleNamespace()
        obj.nested = types.SimpleNamespace()
        c.context.globals['obj'] = obj

        c.parse('obj.nested.value = 42')
        c.execute()

        self.assertEqual(obj.nested.value, 42)

    def test_multiple_levels_setattr(self):
        """Deep nested attribute assignment: obj.a.b.c = value."""
        c = Catnip()
        obj = types.SimpleNamespace()
        obj.level1 = types.SimpleNamespace()
        obj.level1.level2 = types.SimpleNamespace()
        c.context.globals['obj'] = obj

        c.parse('obj.level1.level2.value = 99')
        c.execute()

        self.assertEqual(obj.level1.level2.value, 99)

    def test_setattr_with_expression(self):
        """Attribute assignment with expression as value."""
        c = Catnip()
        obj = types.SimpleNamespace()
        obj.count = 5
        c.context.globals['obj'] = obj

        c.parse('obj.count = obj.count + 10')
        c.execute()

        self.assertEqual(obj.count, 15)

    def test_multiple_setattr(self):
        """Multiple attribute assignments in sequence."""
        c = Catnip()
        obj = types.SimpleNamespace()
        c.context.globals['obj'] = obj

        c.parse('''
obj.x = 1
obj.y = 2
obj.z = 3
''')
        c.execute()

        self.assertEqual(obj.x, 1)
        self.assertEqual(obj.y, 2)
        self.assertEqual(obj.z, 3)

    def test_setattr_with_getattr(self):
        """Attribute assignment combined with attribute access."""
        c = Catnip()
        obj = types.SimpleNamespace()
        obj.a = 10
        obj.b = 20
        c.context.globals['obj'] = obj

        c.parse('obj.result = obj.a + obj.b')
        c.execute()

        self.assertEqual(obj.result, 30)

    def test_setattr_string_value(self):
        """Attribute assignment with string value."""
        c = Catnip()
        obj = types.SimpleNamespace()
        c.context.globals['obj'] = obj

        c.parse('obj.name = "test"')
        c.execute()

        self.assertEqual(obj.name, "test")

    def test_setattr_list_value(self):
        """Attribute assignment with list value."""
        c = Catnip()
        obj = types.SimpleNamespace()
        c.context.globals['obj'] = obj

        c.parse('obj.items = list(1, 2, 3)')
        c.execute()

        self.assertEqual(obj.items, [1, 2, 3])


class TestSetAttrEdgeCases(unittest.TestCase):
    """Test edge cases and error handling for setattr."""

    def test_setattr_overwrite_existing(self):
        """Overwriting existing attribute value."""
        c = Catnip()
        obj = types.SimpleNamespace()
        obj.x = 10
        c.context.globals['obj'] = obj

        c.parse('obj.x = 20')
        c.execute()

        self.assertEqual(obj.x, 20)

    def test_setattr_on_new_object(self):
        """Setting attribute on freshly created object."""
        c = Catnip()
        obj = types.SimpleNamespace()
        c.context.globals['obj'] = obj

        c.parse('obj.value = 42')
        c.execute()

        self.assertEqual(obj.value, 42)


if __name__ == '__main__':
    unittest.main()
