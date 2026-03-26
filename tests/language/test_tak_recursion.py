# FILE: tests/language/test_tak_recursion.py
"""
Test for TAK (Takeuchi) function - deep recursion stress test.

The TAK function is a classic benchmark for evaluating recursion performance.
It creates a tree of recursive calls that exercises both if/else branches.
"""

import unittest

from catnip import Catnip


class TestTAKRecursion(unittest.TestCase):
    """Test cases for TAK function recursion."""

    def test_tak_simple(self):
        """Test simple TAK case."""
        code = """
        pragma("tco", False)

        tak = (x, y, z) => {
            if y >= x { z }
            else { tak(tak(x-1,y,z), tak(y-1,z,x), tak(z-1,x,y)) }
        }

        tak(6, 4, 2)
        """

        cat = Catnip()
        cat.parse(code)
        cat.execute()
        result = cat.context.result
        self.assertEqual(result, 3)

    def test_tak_complex(self):
        """Test complex TAK case with deep recursion."""
        code = """
        pragma("tco", False)

        tak = (x, y, z) => {
            if y >= x { z }
            else { tak(tak(x-1,y,z), tak(y-1,z,x), tak(z-1,x,y)) }
        }

        tak(18, 12, 6)
        """

        cat = Catnip()
        cat.parse(code)
        cat.execute()
        result = cat.context.result
        self.assertEqual(result, 7)

    def test_nested_recursive_calls(self):
        """Test nested recursive calls in else branch."""
        code = """
        pragma("tco", False)

        test = (x, y) => {
            if x <= 0 { y }
            else { test(test(x - 1, y), y - 1) }
        }

        test(3, 10)
        """

        cat = Catnip()
        cat.parse(code)
        cat.execute()
        result = cat.context.result
        self.assertEqual(result, 4)


if __name__ == "__main__":
    unittest.main()
