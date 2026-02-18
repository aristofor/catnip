# FILE: tests/optimization/test_tail_call.py
import sys
import unittest

import pytest

from catnip import Catnip


class TestTailCallDetection(unittest.TestCase):
    """
    Tests for tail-call position detection.

    Verify that the semantic analyzer correctly detects:
    - Recursive calls in tail position
    - Non-tail calls (with post-processing)
    - Tail positions in if/elif/else
    - Tail positions in match
    - Tail positions in lambdas
    """

    def test_factorial_tail_recursive(self):
        """Tail-recursive factorial with accumulator."""
        catnip = Catnip()

        # Tail-recursive factorial: all calls are in tail position
        code = catnip.parse("""
            factorial = (n, acc=1) => {
                if n <= 1 {
                    acc
                } else {
                    factorial(n - 1, n * acc)
                }
            }

            factorial(5)
        """)

        result = catnip.execute()
        self.assertEqual(result, 120)

        # Large value to ensure we do not exceed the stack (if TCO works)
        code = catnip.parse("factorial(100)")
        result = catnip.execute()
        # 100! is a very large number
        self.assertGreater(result, 0)

    def test_factorial_non_tail_recursive(self):
        """Non-tail-recursive factorial."""
        catnip = Catnip()

        # Non-tail factorial: recursive call has post-processing (* n)
        code = catnip.parse("""
            factorial_bad = (n) => {
                if n <= 1 {
                    1
                } else {
                    n * factorial_bad(n - 1)
                }
            }

            factorial_bad(5)
        """)

        result = catnip.execute()
        self.assertEqual(result, 120)

        # This version would blow the stack for large values
        # Use a moderate value only
        code = catnip.parse("factorial_bad(10)")
        result = catnip.execute()
        self.assertEqual(result, 3628800)

    def test_mutual_recursion_even_odd(self):
        """Mutual recursion for even/odd."""
        # Test 1: is_even(10)
        catnip = Catnip()
        code = catnip.parse("""
            is_even = (n) => {
                if n == 0 {
                    True
                } else {
                    is_odd(n - 1)
                }
            }

            is_odd = (n) => {
                if n == 0 {
                    False
                } else {
                    is_even(n - 1)
                }
            }

            is_even(10)
        """)
        result = catnip.execute()
        self.assertEqual(result, True)

        # Test 2: is_odd(10) - new instance to avoid state leakage
        catnip = Catnip()
        code = catnip.parse("""
            is_even = (n) => {
                if n == 0 {
                    True
                } else {
                    is_odd(n - 1)
                }
            }

            is_odd = (n) => {
                if n == 0 {
                    False
                } else {
                    is_even(n - 1)
                }
            }

            is_odd(10)
        """)
        result = catnip.execute()
        self.assertEqual(result, False)

        # Test 3: is_even(100)
        catnip = Catnip()
        code = catnip.parse("""
            is_even = (n) => {
                if n == 0 {
                    True
                } else {
                    is_odd(n - 1)
                }
            }

            is_odd = (n) => {
                if n == 0 {
                    False
                } else {
                    is_even(n - 1)
                }
            }

            is_even(100)
        """)
        result = catnip.execute()
        self.assertEqual(result, True)

        # Test 4: is_odd(99)
        catnip = Catnip()
        code = catnip.parse("""
            is_even = (n) => {
                if n == 0 {
                    True
                } else {
                    is_odd(n - 1)
                }
            }

            is_odd = (n) => {
                if n == 0 {
                    False
                } else {
                    is_even(n - 1)
                }
            }

            is_odd(99)
        """)
        result = catnip.execute()
        self.assertEqual(result, True)

    def test_tail_call_in_if_elif_else(self):
        """Test appel tail dans toutes les branches if/elif/else."""
        catnip = Catnip()

        code = catnip.parse("""
            sign = (n, depth=0) => {
                if depth > 1000 {
                    "max"
                } elif n > 0 {
                    sign(n, depth + 1)
                } elif n < 0 {
                    sign(n, depth + 1)
                } else {
                    "zero"
                }
            }

            sign(5)
        """)

        result = catnip.execute()
        self.assertEqual(result, "max")

    def test_tail_call_in_match(self):
        """Test appel tail dans toutes les cases d'un match."""
        catnip = Catnip()

        code = catnip.parse("""
            countdown = (n, acc="") => {
                match n {
                    0 => { acc }
                    1 => { countdown(0, acc + "1") }
                    _ => { countdown(n - 1, acc + str(n)) }
                }
            }

            countdown(5)
        """)

        result = catnip.execute()
        self.assertEqual(result, "54321")

    def test_tail_call_in_lambda(self):
        """Test appel tail dans une lambda."""
        catnip = Catnip()

        code = catnip.parse("""
            sum_range = (n, acc=0) => {
                if n <= 0 {
                    acc
                } else {
                    sum_range(n - 1, acc + n)
                }
            }

            sum_range(10)
        """)

        result = catnip.execute()
        self.assertEqual(result, 55)  # 1+2+3+...+10 = 55

    def test_non_tail_call_with_post_processing(self):
        """Test que les appels avec post-traitement ne sont PAS tail."""
        catnip = Catnip()

        # Call is followed by an addition, so not tail
        code = catnip.parse("""
            sum_plus_one = (n) => {
                if n <= 0 {
                    0
                } else {
                    sum_plus_one(n - 1) + 1
                }
            }

            sum_plus_one(10)
        """)

        result = catnip.execute()
        self.assertEqual(result, 10)

    def test_non_tail_call_in_loop(self):
        """Test que les appels dans les boucles ne sont PAS tail."""
        catnip = Catnip()

        # A call inside a loop is not tail since iterations remain
        code = catnip.parse("""
            helper = (x) => { x * 2 }

            result = 0
            for i in range(5) {
                result = result + helper(i)
            }
            result
        """)

        result = catnip.execute()
        self.assertEqual(result, 20)  # 0 + 2 + 4 + 6 + 8 = 20


class TestTailCallOptimization(unittest.TestCase):
    """
    Tests pour l'optimisation tail-call (TCO).

    Verify that:
    - TCO can be toggled via pragma
    - Tail-recursive functions do not blow the stack
    - Semantics are preserved after optimization
    """

    def test_tco_pragma_enable(self):
        """Test activation du TCO via pragma."""
        catnip = Catnip()

        code = catnip.parse("""
            pragma("tco", "enable")

            countdown = (n) => {
                if n <= 0 {
                    "done"
                } else {
                    countdown(n - 1)
                }
            }

            countdown(10)
        """)

        result = catnip.execute()
        self.assertEqual(result, "done")

    def test_tco_pragma_disable(self):
        """Disable TCO via pragma."""
        catnip = Catnip()

        code = catnip.parse("""
            pragma("tco", "disable")

            countdown = (n) => {
                if n <= 0 {
                    "done"
                } else {
                    countdown(n - 1)
                }
            }

            countdown(10)
        """)

        result = catnip.execute()
        self.assertEqual(result, "done")

    def test_deep_recursion_with_tco(self):
        """Deep recursion with TCO (should work)."""
        catnip = Catnip()

        # Save the Python recursion limit
        old_limit = sys.getrecursionlimit()

        try:
            # Lower limit to force the test path
            sys.setrecursionlimit(100)

            code = catnip.parse("""
                pragma("tco", "enable")

                deep = (n, acc=0) => {
                    if n <= 0 {
                        acc
                    } else {
                        deep(n - 1, acc + 1)
                    }
                }

                deep(500)
            """)

            result = catnip.execute()
            self.assertEqual(result, 500)

        finally:
            # Restore the limit
            sys.setrecursionlimit(old_limit)

    def test_fibonacci_tail_recursive(self):
        """Test Fibonacci tail-recursive."""
        catnip = Catnip()

        code = catnip.parse("""
            fib = (n, a=0, b=1) => {
                if n == 0 {
                    a
                } elif n == 1 {
                    b
                } else {
                    fib(n - 1, b, a + b)
                }
            }

            fib(10)
        """)

        result = catnip.execute()
        self.assertEqual(result, 55)  # 10th Fibonacci number

    def test_kwargs_in_tail_call(self):
        """Ensure kwargs are handled correctly in tail calls."""
        catnip = Catnip()

        code = catnip.parse("""
            accumulate = (n, result=0, multiplier=1) => {
                if n <= 0 {
                    result
                } else {
                    accumulate(n - 1, result=result + n * multiplier, multiplier=multiplier)
                }
            }

            accumulate(5, multiplier=2)
        """)

        result = catnip.execute()
        self.assertEqual(result, 30)  # (5+4+3+2+1) * 2 = 30


class TestTailCallEdgeCases(unittest.TestCase):
    """
    Tests pour les cas limites et edge cases du TCO.
    """

    def test_mutual_recursion_deep(self):
        """Deep mutual recursion with TCO.

        TCO now supports mutual recursion (A calls B calls A).
        This test verifies deep recursion without stack overflow.

        Note: Using depth 1000 instead of 10000 to reduce load when
        multiple test workers run in parallel.
        """
        catnip = Catnip()

        code = catnip.parse("""
            pragma("tco", "enable")

            ping = (n) => {
                if n <= 0 {
                    "done"
                } else {
                    pong(n - 1)
                }
            }

            pong = (n) => {
                if n <= 0 {
                    "done"
                } else {
                    ping(n - 1)
                }
            }

            ping(1000)
        """)

        result = catnip.execute()
        self.assertEqual(result, "done")

    def test_mutual_recursion_is_even_deep(self):
        """Test is_even/is_odd mutual recursion at depth 1000.

        Note: Using depth 1000 instead of 10000 to reduce load when
        multiple test workers run in parallel.
        """
        catnip = Catnip()

        code = catnip.parse("""
            is_even = (n) => {
                if n == 0 { True }
                else { is_odd(n - 1) }
            }

            is_odd = (n) => {
                if n == 0 { False }
                else { is_even(n - 1) }
            }

            is_even(1000)
        """)

        result = catnip.execute()
        self.assertEqual(result, True)  # 1000 is even

    def test_mutual_recursion_is_odd_deep(self):
        """Test is_even/is_odd mutual recursion returns False for odd input.

        Note: Using depth 1001 instead of 10001 to reduce load when
        multiple test workers run in parallel.
        """
        catnip = Catnip()

        code = catnip.parse("""
            is_even = (n) => {
                if n == 0 { True }
                else { is_odd(n - 1) }
            }

            is_odd = (n) => {
                if n == 0 { False }
                else { is_even(n - 1) }
            }

            is_even(1001)
        """)

        result = catnip.execute()
        self.assertEqual(result, False)  # 1001 is odd

    def test_lambda_tail_recursive(self):
        """Test lambda anonyme tail-recursive."""
        catnip = Catnip()

        code = catnip.parse("""
            # Lambda stored in a variable
            countdown = (n) => {
                if n <= 0 {
                    0
                } else {
                    countdown(n - 1)
                }
            }

            countdown(100)
        """)

        result = catnip.execute()
        self.assertEqual(result, 0)

    def test_match_with_guards_tail_position(self):
        """Test match avec guards en position tail."""
        catnip = Catnip()

        code = catnip.parse("""
            categorize = (n, depth=0) => {
                match n {
                    0 => { "zero" }
                    x if x < 0 => { categorize(-x, depth + 1) }
                    x if x > 100 and depth < 10 => { categorize(x // 2, depth + 1) }
                    _ => { "done" }
                }
            }

            categorize(500)
        """)

        result = catnip.execute()
        self.assertEqual(result, "done")

    def test_nested_tail_calls(self):
        """Tail calls nested inside blocks."""
        catnip = Catnip()

        code = catnip.parse("""
            process = (n, mode) => {
                if mode == "inc" {
                    if n > 100 {
                        "max"
                    } else {
                        process(n + 1, "inc")
                    }
                } else {
                    if n <= 0 {
                        "min"
                    } else {
                        process(n - 1, "dec")
                    }
                }
            }

            process(0, "inc")
        """)

        result = catnip.execute()
        self.assertEqual(result, "max")

    def test_tco_preserves_semantics(self):
        """Ensure TCO preserves exact semantics."""
        catnip = Catnip()

        # Same code with TCO on/off should match
        test_code = """
            sum_to_n = (n, acc=0) => {
                if n <= 0 {
                    acc
                } else {
                    sum_to_n(n - 1, acc + n)
                }
            }

            sum_to_n(10)
        """

        # With TCO
        catnip.pragma_context.tco_enabled = True
        code = catnip.parse(test_code)
        result_with_tco = catnip.execute()

        # Without TCO
        catnip2 = Catnip()
        catnip2.pragma_context.tco_enabled = False
        code = catnip2.parse(test_code)
        result_without_tco = catnip2.execute()

        self.assertEqual(result_with_tco, result_without_tco)
        self.assertEqual(result_with_tco, 55)

    def test_non_recursive_tail_call(self):
        """Tail call into another function (non-recursive)."""
        catnip = Catnip()

        code = catnip.parse("""
            final_step = (x) => { x * 2 }

            process = (n) => {
                if n <= 0 {
                    final_step(100)
                } else {
                    process(n - 1)
                }
            }

            process(5)
        """)

        result = catnip.execute()
        self.assertEqual(result, 200)

    def test_tail_call_with_multiple_accumulators(self):
        """Test tail call avec plusieurs accumulateurs."""
        catnip = Catnip()

        code = catnip.parse("""
            sum_and_product = (n, sum_acc=0, prod_acc=1) => {
                if n <= 0 {
                    sum_acc + prod_acc
                } else {
                    sum_and_product(n - 1, sum_acc + n, prod_acc * n)
                }
            }

            sum_and_product(5)
        """)

        result = catnip.execute()
        # sum(1..5) = 15, prod(1..5) = 120, total = 135
        self.assertEqual(result, 135)

    @pytest.mark.serial
    def test_tak_function_non_tail_recursive(self):
        """Tak (Takeuchi) function - classic non-tail recursion.

        Tak is a classic recursion benchmark.
        It is NOT tail-recursive because recursive calls are arguments
        to other recursive calls (nested recursion).

        Also validates that equivalent implementations match.

        Note: Using tak(12, 6, 3) instead of tak(18, 12, 6) to avoid
        excessive recursion (~1000 calls vs ~64000) which causes freezes
        when multiple test workers run in parallel.
        """
        catnip = Catnip()

        code = catnip.parse("""
            tak1 = (x, y, z) => {
                if y < x {
                    tak1(tak1(x - 1, y, z), tak1(y - 1, z, x), tak1(z - 1, x, y))
                } else {
                    z
                }
            }

            tak2 = (x, y, z) => {
                if y < x {
                    tak2(tak2(x - 1, y, z), tak2(y - 1, z, x), tak2(z - 1, x, y))
                } else {
                    z
                }
            }

            list(tak1(12, 6, 3), tak2(12, 6, 3))
        """)

        result = catnip.execute()
        self.assertEqual(result, [4, 4])

    def test_return_with_tail_call(self):
        """Test que return fonctionne correctement avec tail calls.

        Avant le fix, return enveloppait le TailCall dans ReturnValue,
        which prevented the trampoline from running. That caused a
        non-resolved TailCall object to be returned instead of the final result.
        """
        catnip = Catnip()

        code = catnip.parse("""
            countdown = (n) => {
                if n <= 0 {
                    return "done"
                } else {
                    return countdown(n - 1)
                }
            }

            countdown(100)
        """)

        result = catnip.execute()
        self.assertEqual(result, "done")

        # Test with deeper recursion
        catnip2 = Catnip()
        code = catnip2.parse("""
            sum_to = (n, acc=0) => {
                if n <= 0 {
                    return acc
                } else {
                    return sum_to(n - 1, acc + n)
                }
            }

            sum_to(100)
        """)

        result = catnip2.execute()
        self.assertEqual(result, 5050)  # sum(1..100) = 5050


if __name__ == "__main__":
    unittest.main()
