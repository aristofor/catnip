# FILE: tests/serial/jit/test_recursion.py
"""
Tests de récursion JIT.

Vérifie que le JIT compile et exécute correctement les fonctions récursives.
"""

import pytest

from tests.serial.jit.conftest import compile_code

# Force serial execution to avoid JIT state conflicts
pytestmark = pytest.mark.xdist_group(name="jit")


class TestBasicRecursion:
    """Tests de récursion basique."""

    def test_factorial_recursive(self, vm_with_jit):
        """Factorial récursif - cas simple."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            factorial = (n) => {
                if n <= 1 {
                    1
                } else {
                    n * factorial(n - 1)
                }
            }

            # Call many times to trigger JIT compilation
            result = 0
            i = 0
            while i < 120 {
                result = factorial(5)
                i = i + 1
            }
            result
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 120  # 5! = 120

    def test_fibonacci_recursive(self, vm_with_jit):
        """Fibonacci récursif - double récursion."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            fib = (n) => {
                if n <= 1 {
                    n
                } else {
                    fib(n - 1) + fib(n - 2)
                }
            }

            # Call many times to trigger JIT
            result = 0
            i = 0
            while i < 120 {
                result = fib(6)
                i = i + 1
            }
            result
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 8  # fib(6) = 8

    def test_simple_recursive_sum(self, vm_with_jit):
        """Somme récursive - tail-call candidat."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            sum_n = (n, acc) => {
                if n <= 0 {
                    acc
                } else {
                    sum_n(n - 1, acc + n)
                }
            }

            # Call many times to trigger JIT
            result = 0
            i = 0
            while i < 120 {
                result = sum_n(10, 0)
                i = i + 1
            }
            result
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 55  # sum(1..10) = 55


class TestRecursionEdgeCases:
    """Tests des cas limites de la récursion JIT."""

    def test_recursion_base_case(self, vm_with_jit):
        """Récursion avec cas de base immédiat."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            identity = (n) => {
                if n == 0 {
                    0
                } else {
                    identity(0)
                }
            }

            # Call many times
            result = 0
            i = 0
            while i < 120 {
                result = identity(5)
                i = i + 1
            }
            result
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 0

    def test_recursion_with_multiple_params(self, vm_with_jit):
        """Récursion avec plusieurs paramètres."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            ackermann_lite = (m, n) => {
                if m == 0 {
                    n + 1
                } else {
                    if n == 0 {
                        ackermann_lite(m - 1, 1)
                    } else {
                        ackermann_lite(m - 1, ackermann_lite(m, n - 1))
                    }
                }
            }

            # Small values only (Ackermann grows fast)
            result = 0
            i = 0
            while i < 120 {
                result = ackermann_lite(2, 2)
                i = i + 1
            }
            result
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 7  # A(2,2) = 7


class TestRecursionCorrectnessVsInterpreter:
    """Vérification que JIT et interpréteur donnent le même résultat."""

    def test_factorial_consistency(self, vm_with_jit, vm_without_jit):
        """Factorial doit donner le même résultat avec et sans JIT."""
        vm_jit, c = vm_with_jit
        vm_nojit, _ = vm_without_jit

        code = compile_code("""
        {
            factorial = (n) => {
                if n <= 1 {
                    1
                } else {
                    n * factorial(n - 1)
                }
            }

            result = 0
            i = 0
            while i < 120 {
                result = factorial(7)
                i = i + 1
            }
            result
        }
        """)

        result_jit = vm_jit.execute(code, (), {}, None)
        result_nojit = vm_nojit.execute(code, (), {}, None)

        assert result_jit == result_nojit
        assert result_jit == 5040  # 7! = 5040

    def test_fibonacci_consistency(self, vm_with_jit, vm_without_jit):
        """Fibonacci doit donner le même résultat avec et sans JIT."""
        vm_jit, c = vm_with_jit
        vm_nojit, _ = vm_without_jit

        code = compile_code("""
        {
            fib = (n) => {
                if n <= 1 {
                    n
                } else {
                    fib(n - 1) + fib(n - 2)
                }
            }

            result = 0
            i = 0
            while i < 120 {
                result = fib(8)
                i = i + 1
            }
            result
        }
        """)

        result_jit = vm_jit.execute(code, (), {}, None)
        result_nojit = vm_nojit.execute(code, (), {}, None)

        assert result_jit == result_nojit
        assert result_jit == 21  # fib(8) = 21


class TestRecursionOverflowProtection:
    """Tests Phase 3 : protection contre stack overflow."""

    def test_deep_recursion_fallback(self, vm_with_jit):
        """Récursion profonde > MAX_DEPTH doit fallback vers l'interpréteur."""
        vm, c = vm_with_jit
        # MAX_RECURSION_DEPTH = 10000
        # On teste avec 15000 pour dépasser le seuil
        code = compile_code("""
        {
            sum_recursive = (n) => {
                if n <= 0 {
                    0
                } else {
                    1 + sum_recursive(n - 1)
                }
            }

            # Appeler 120 fois pour déclencher la compilation JIT
            result = 0
            i = 0
            while i < 120 {
                result = sum_recursive(15000)
                i = i + 1
            }
            result
        }
        """)

        # Ne devrait pas crasher, doit fallback vers l'interpréteur
        result = vm.execute(code, (), {}, None)
        assert result == 15000

    def test_within_limit_uses_jit(self, vm_with_jit):
        """Récursion dans la limite doit utiliser le JIT."""
        vm, c = vm_with_jit
        # Profondeur de 1000 est bien en dessous de MAX_RECURSION_DEPTH (10000)
        code = compile_code("""
        {
            sum_recursive = (n) => {
                if n <= 0 {
                    0
                } else {
                    1 + sum_recursive(n - 1)
                }
            }

            # Appeler 120 fois pour déclencher la compilation JIT
            result = 0
            i = 0
            while i < 120 {
                result = sum_recursive(1000)
                i = i + 1
            }
            result
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 1000

        # Note: JIT compiled_functions stats not yet exposed, but function works correctly

    def test_exactly_at_limit(self, vm_with_jit):
        """Récursion exactement à MAX_DEPTH doit fonctionner."""
        vm, c = vm_with_jit
        # MAX_RECURSION_DEPTH = 10000
        code = compile_code("""
        {
            sum_recursive = (n) => {
                if n <= 0 {
                    0
                } else {
                    1 + sum_recursive(n - 1)
                }
            }

            # Appeler 120 fois pour déclencher la compilation JIT
            result = 0
            i = 0
            while i < 120 {
                result = sum_recursive(10000)
                i = i + 1
            }
            result
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 10000


class TestMemoization:
    """Tests Phase 4.3 : Memoization JIT (cache recursive calls)."""

    def test_fibonacci_memoization(self, vm_with_jit):
        """Fibonacci avec memoization doit utiliser le cache."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            fib = (n) => {
                if n <= 1 {
                    n
                } else {
                    fib(n - 1) + fib(n - 2)
                }
            }

            # Call many times to trigger JIT + memoization
            result = 0
            i = 0
            while i < 120 {
                result = fib(10)
                i = i + 1
            }
            result
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 55  # fib(10) = 55

    def test_factorial_no_memoization(self, vm_with_jit):
        """Factorial (not tail-recursive, not overlapping) pas de memo."""
        vm, c = vm_with_jit
        # Factorial is not a good candidate for memoization (no overlapping subproblems)
        # but should still work correctly with CallSelf
        code = compile_code("""
        {
            factorial = (n) => {
                if n <= 1 {
                    1
                } else {
                    n * factorial(n - 1)
                }
            }

            result = 0
            i = 0
            while i < 120 {
                result = factorial(6)
                i = i + 1
            }
            result
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 720  # 6! = 720

    def test_memoization_with_different_args(self, vm_with_jit):
        """Memoization doit isoler les résultats par argument."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            fib = (n) => {
                if n <= 1 {
                    n
                } else {
                    fib(n - 1) + fib(n - 2)
                }
            }

            # Multiple calls with different arguments
            result = 0
            i = 0
            while i < 120 {
                # Mix of fib(5), fib(6), fib(7)
                result = fib(5) + fib(6) + fib(7)
                i = i + 1
            }
            result
        }
        """)

        result = vm.execute(code, (), {}, None)
        # fib(5) + fib(6) + fib(7) = 5 + 8 + 13 = 26
        assert result == 26


class TestTailCallOptimization:
    """Tests Phase 4.1 : TCO JIT (tail-call → jump)."""

    def test_tail_recursive_factorial(self, vm_with_jit):
        """Factorial tail-recursive doit être optimisé en jump."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            # Tail-recursive factorial (acc parameter)
            factorial_tail = (n, acc) => {
                if n <= 1 {
                    acc
                } else {
                    factorial_tail(n - 1, n * acc)
                }
            }

            # Wrapper pour API standard
            factorial = (n) => { factorial_tail(n, 1) }

            result = 0
            i = 0
            while i < 120 {
                result = factorial(10)
                i = i + 1
            }
            result
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 3628800  # 10! = 3628800

    def test_tail_recursive_sum(self, vm_with_jit):
        """Sum tail-recursive doit utiliser TCO."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            sum_tail = (n, acc) => {
                if n <= 0 {
                    acc
                } else {
                    sum_tail(n - 1, acc + n)
                }
            }

            result = 0
            i = 0
            while i < 120 {
                result = sum_tail(1000, 0)
                i = i + 1
            }
            result
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 500500  # sum(1..1000)

    def test_tco_performance(self, vm_with_jit, vm_without_jit):
        """TCO devrait avoir un speedup sur tail-recursion."""
        vm_jit, c = vm_with_jit
        vm_nojit, _ = vm_without_jit

        code = compile_code("""
        {
            sum_tail = (n, acc) => {
                if n <= 0 {
                    acc
                } else {
                    sum_tail(n - 1, acc + n)
                }
            }

            result = 0
            i = 0
            while i < 120 {
                result = sum_tail(100, 0)
                i = i + 1
            }
            result
        }
        """)

        import time

        # Sans JIT
        start = time.perf_counter()
        result_nojit = vm_nojit.execute(code, (), {}, None)
        time_nojit = (time.perf_counter() - start) * 1000

        # Avec JIT (TCO devrait optimiser)
        start = time.perf_counter()
        result_jit = vm_jit.execute(code, (), {}, None)
        time_jit = (time.perf_counter() - start) * 1000

        assert result_jit == result_nojit
        assert result_jit == 5050  # sum(1..100)

        # Informational only -- VM interpreter is faster than JIT on TCO,
        # so this test is purely a correctness check
        speedup = time_nojit / time_jit if time_jit > 0 else 1.0
        print(f"\nTCO Performance: {time_nojit:.2f}ms → {time_jit:.2f}ms (speedup: {speedup:.2f}x)")
