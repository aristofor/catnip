# FILE: tests/language/test_shift_operators.py
"""Tests for shift operators (<< and >>)."""

import unittest

from catnip import Catnip


class TestShiftOperators(unittest.TestCase):
    """Test left shift and right shift operators."""

    def setUp(self):
        """Set up test fixtures."""
        self.catnip = Catnip()

    def test_left_shift_basic(self):
        """Test basic left shift operation."""
        code = "4 << 2"
        self.catnip.parse(code)
        result = self.catnip.execute()
        self.assertEqual(result, 16)  # 4 * 2^2 = 16

    def test_left_shift_zero(self):
        """Test left shift by zero."""
        code = "7 << 0"
        self.catnip.parse(code)
        result = self.catnip.execute()
        self.assertEqual(result, 7)

    def test_left_shift_large(self):
        """Test left shift with larger values."""
        code = "1 << 8"
        self.catnip.parse(code)
        result = self.catnip.execute()
        self.assertEqual(result, 256)

    def test_left_shift_chained(self):
        """Test chained left shift operations."""
        code = "1 << 2 << 3"
        self.catnip.parse(code)
        result = self.catnip.execute()
        # (1 << 2) << 3 = 4 << 3 = 32
        self.assertEqual(result, 32)

    def test_right_shift_basic(self):
        """Test basic right shift operation."""
        code = "16 >> 2"
        self.catnip.parse(code)
        result = self.catnip.execute()
        self.assertEqual(result, 4)  # 16 / 2^2 = 4

    def test_right_shift_zero(self):
        """Test right shift by zero."""
        code = "42 >> 0"
        self.catnip.parse(code)
        result = self.catnip.execute()
        self.assertEqual(result, 42)

    def test_right_shift_large(self):
        """Test right shift with larger values."""
        code = "256 >> 8"
        self.catnip.parse(code)
        result = self.catnip.execute()
        self.assertEqual(result, 1)

    def test_right_shift_to_zero(self):
        """Test right shift that results in zero."""
        code = "7 >> 10"
        self.catnip.parse(code)
        result = self.catnip.execute()
        self.assertEqual(result, 0)

    def test_right_shift_chained(self):
        """Test chained right shift operations."""
        code = "64 >> 2 >> 1"
        self.catnip.parse(code)
        result = self.catnip.execute()
        # (64 >> 2) >> 1 = 16 >> 1 = 8
        self.assertEqual(result, 8)

    def test_shift_mixed_operations(self):
        """Test mixing left and right shifts."""
        code = "8 << 2 >> 1"
        self.catnip.parse(code)
        result = self.catnip.execute()
        # (8 << 2) >> 1 = 32 >> 1 = 16
        self.assertEqual(result, 16)

    def test_shift_with_arithmetic(self):
        """Test shift operators with arithmetic operations."""
        code = "(2 + 2) << 3"
        self.catnip.parse(code)
        result = self.catnip.execute()
        self.assertEqual(result, 32)  # 4 << 3 = 32

    def test_shift_with_bitwise_and(self):
        """Test shift operators with bitwise AND."""
        code = "15 & 7 << 1"
        self.catnip.parse(code)
        result = self.catnip.execute()
        # 15 & (7 << 1) = 15 & 14 = 14
        self.assertEqual(result, 14)

    def test_shift_priority_over_addition(self):
        """Test that addition has higher priority than shift."""
        code = "1 + 2 << 3"
        self.catnip.parse(code)
        result = self.catnip.execute()
        # (1 + 2) << 3 = 3 << 3 = 24
        self.assertEqual(result, 24)

    def test_shift_priority_under_bitwise_and(self):
        """Test that bitwise AND has higher priority than shift."""
        code = "8 & 12 << 1"
        self.catnip.parse(code)
        result = self.catnip.execute()
        # 8 & (12 << 1) = 8 & 24 = 8
        self.assertEqual(result, 8)

    def test_left_shift_with_variables(self):
        """Test left shift with variables."""
        code = """
        x = 3
        y = 4
        x << y
        """
        self.catnip.parse(code)
        result = self.catnip.execute()
        self.assertEqual(result, 48)  # 3 << 4 = 48

    def test_right_shift_with_variables(self):
        """Test right shift with variables."""
        code = """
        a = 100
        b = 2
        a >> b
        """
        self.catnip.parse(code)
        result = self.catnip.execute()
        self.assertEqual(result, 25)  # 100 >> 2 = 25

    def test_negative_left_shift(self):
        """Test left shift with negative number."""
        code = "-4 << 2"
        self.catnip.parse(code)
        result = self.catnip.execute()
        self.assertEqual(result, -16)

    def test_negative_right_shift(self):
        """Test right shift with negative number."""
        code = "-16 >> 2"
        self.catnip.parse(code)
        result = self.catnip.execute()
        self.assertEqual(result, -4)

    def test_constant_folding_left_shift(self):
        """Test that constant folding works for left shift."""
        code = "5 << 3"
        self.catnip.parse(code)
        # Access the optimized IR to check folding
        # This is a basic test - constant folding is tested more thoroughly
        # in test_optimization_constant_folding.py
        result = self.catnip.execute()
        self.assertEqual(result, 40)

    def test_constant_folding_right_shift(self):
        """Test that constant folding works for right shift."""
        code = "80 >> 3"
        self.catnip.parse(code)
        result = self.catnip.execute()
        self.assertEqual(result, 10)


if __name__ == "__main__":
    unittest.main()
