# FILE: tests/language/test_setitem.py
"""Tests for index assignment (setitem) feature."""

import unittest

from catnip import Catnip


class TestSetItem(unittest.TestCase):
    """Test index assignment on containers."""

    def test_simple_list_index(self):
        """Simple list index assignment: lst[0] = 10."""
        c = Catnip()
        c.context.globals['lst'] = [1, 2, 3]

        c.parse('lst[0] = 10')
        c.execute()

        self.assertEqual(c.context.globals['lst'], [10, 2, 3])

    def test_dict_key_assignment(self):
        """Dict key assignment: d["key"] = val."""
        c = Catnip()
        c.context.globals['d'] = {}

        c.parse('d["key"] = 42')
        c.execute()

        self.assertEqual(c.context.globals['d'], {"key": 42})

    def test_variable_index(self):
        """Variable as index: lst[i] = val."""
        c = Catnip()
        c.context.globals['lst'] = [0, 0, 0]

        c.parse('''
i = 2
lst[i] = 99
''')
        c.execute()

        self.assertEqual(c.context.globals['lst'], [0, 0, 99])

    def test_expression_index_and_value(self):
        """Expressions in index and value: lst[i + 1] = x * 2."""
        c = Catnip()
        c.context.globals['lst'] = [0, 0, 0]

        c.parse('''
i = 0
x = 5
lst[i + 1] = x * 2
''')
        c.execute()

        self.assertEqual(c.context.globals['lst'], [0, 10, 0])

    def test_nested_index(self):
        """Nested index assignment: matrix[0][1] = 42."""
        c = Catnip()
        c.context.globals['matrix'] = [[0, 0], [0, 0]]

        c.parse('matrix[0][1] = 42')
        c.execute()

        self.assertEqual(c.context.globals['matrix'], [[0, 42], [0, 0]])

    def test_mixed_getattr_getitem(self):
        """Mix getattr + getitem: obj.items[0] = val."""
        import types

        c = Catnip()
        obj = types.SimpleNamespace()
        obj.items = [1, 2, 3]
        c.context.globals['obj'] = obj

        c.parse('obj.items[0] = 99')
        c.execute()

        self.assertEqual(obj.items, [99, 2, 3])

    def test_read_write_same_object(self):
        """Read + write same object: lst[0] = lst[0] + 1."""
        c = Catnip()
        c.context.globals['lst'] = [10, 20, 30]

        c.parse('lst[0] = lst[0] + 1')
        c.execute()

        self.assertEqual(c.context.globals['lst'], [11, 20, 30])

    def test_overwrite_existing(self):
        """Overwrite existing value at index."""
        c = Catnip()
        c.context.globals['d'] = {"a": 1}

        c.parse('d["a"] = 999')
        c.execute()

        self.assertEqual(c.context.globals['d'], {"a": 999})

    def test_index_out_of_range(self):
        """Index out of range raises error."""
        c = Catnip()
        c.context.globals['lst'] = [1, 2, 3]

        c.parse('lst[10] = 0')

        with self.assertRaises(IndexError):
            c.execute()


if __name__ == '__main__':
    unittest.main()
