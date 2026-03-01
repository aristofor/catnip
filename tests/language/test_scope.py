# FILE: tests/language/test_scope.py
"""
Test for the Scope module
"""

import unittest

from catnip import Catnip, Scope


class TestForLoopScoping(unittest.TestCase):
    """Test that for loops create local scopes correctly"""

    def test_for_loop_variable_is_scoped(self):
        # Loop variables should not leak into the global scope
        c = Catnip()
        c.parse("""
            x = 10
            for i in list(1, 2, 3) {
                x = x + i
            }
        """)
        c.execute()

        # x should be updated (10 + 1 + 2 + 3 = 16)
        self.assertEqual(c.context.globals["x"], 16)
        # i should not exist in globals
        self.assertNotIn("i", c.context.globals)

    def test_for_loop_modifies_parent_scope_variable(self):
        # A parent-scope variable can be updated from a for loop
        c = Catnip()
        c.parse("""
            total = 0
            for n in list(1, 2, 3, 4, 5) {
                total = total + n
            }
        """)
        c.execute()
        self.assertEqual(c.context.globals["total"], 15)

    def test_for_loop_variable_does_not_conflict(self):
        # Reusing a variable name after a for loop should not conflict
        c = Catnip()
        c.parse("""
            for i in list(1, 2, 3) {
                x = i * 2
            }
            i = 100
        """)
        c.execute()

        # i should have the new value, not the loop's value
        self.assertEqual(c.context.globals["i"], 100)

    def test_nested_for_loops_have_separate_scopes(self):
        # Nested for loops have separate scopes
        c = Catnip()
        c.parse("""
            result = list()
            for i in list(1, 2) {
                for j in list(10, 20) {
                    result = result + list(i + j)
                }
            }
        """)
        c.execute()

        # result should contain [11, 21, 12, 22]
        self.assertEqual(c.context.globals["result"], [11, 21, 12, 22])
        # neither i nor j should be in globals
        self.assertNotIn("i", c.context.globals)
        self.assertNotIn("j", c.context.globals)

    def test_for_loop_with_same_variable_name_twice(self):
        # Two successive for loops with the same variable name
        # The second should not be affected by the first
        c = Catnip()
        c.parse("""
            sum1 = 0
            for i in list(1, 2, 3) {
                sum1 = sum1 + i
            }

            sum2 = 0
            for i in list(10, 20, 30) {
                sum2 = sum2 + i
            }
        """)
        c.execute()

        self.assertEqual(c.context.globals["sum1"], 6)
        self.assertEqual(c.context.globals["sum2"], 60)
        # i should not be in globals
        self.assertNotIn("i", c.context.globals)


if __name__ == "__main__":
    unittest.main()
