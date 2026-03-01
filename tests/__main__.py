# FILE: tests/__main__.py
import unittest
from pathlib import Path


def run_tests():
    # Resolve the current directory (tests folder).
    test_dir = Path(__file__).parent
    # Discover all test modules starting with "test_"
    suite = unittest.defaultTestLoader.discover(start_dir=test_dir, pattern="test_*.py")
    # Run the test suite with verbose output.
    runner = unittest.TextTestRunner(verbosity=2)
    result = runner.run(suite)
    return result


if __name__ == "__main__":
    run_tests()
