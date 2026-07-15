# FILE: tests/serial/jit/test_execution.py
"""
Tests d'exécution du code JIT compilé.

Vérifie la correction fonctionnelle du code généré.
"""

import pytest

from tests.serial.jit.conftest import compile_code

# Force serial execution to avoid JIT state conflicts
pytestmark = pytest.mark.xdist_group(name="jit")


class TestBasicExecution:
    """Tests d'exécution basiques."""

    def test_simple_increment(self, vm_with_jit):
        """Boucle simple d'incrémentation."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            i = 0
            while i < 150 {
                i = i + 1
            }
            i
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 150

    def test_accumulator(self, vm_with_jit):
        """Boucle avec accumulateur."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            total = 0
            i = 1
            while i <= 100 {
                total = total + i
                i = i + 1
            }
            total
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 5050  # sum(1..100)

    def test_multiple_variables(self, vm_with_jit):
        """Boucle avec plusieurs variables."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            a = 0
            b = 1
            i = 0
            while i < 150 {
                temp = a + b
                a = b
                b = temp
                i = i + 1
            }
            a
        }
        """)

        result = vm.execute(code, (), {}, None)
        # Vérifie juste que ça s'exécute sans crash
        assert isinstance(result, int)


class TestMultipleExecutions:
    """Tests d'exécutions multiples (non-régression infinite loop)."""

    def test_reexecution_no_hang(self, vm_with_jit):
        """Réexécuter du code JIT ne cause pas de hang."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            i = 0
            while i < 150 {
                i = i + 1
            }
            i
        }
        """)

        # Première exécution
        result1 = vm.execute(code, (), {}, None)
        assert result1 == 150

        # Deuxième exécution (test de non-régression)
        result2 = vm.execute(code, (), {}, None)
        assert result2 == 150


class TestCorrectnessVsInterpreter:
    """Tests de correction par rapport à l'interpréteur."""

    def test_same_result_as_interpreter(self, vm_with_jit, vm_without_jit):
        """Le JIT produit le même résultat que l'interpréteur."""
        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit

        code = compile_code("""
        {
            total = 0
            i = 1
            while i <= 100 {
                total = total + i
                i = i + 1
            }
            total
        }
        """)

        result_jit = vm_jit.execute(code, (), {}, None)
        result_no_jit = vm_no_jit.execute(code, (), {}, None)

        assert result_jit == result_no_jit == 5050


class TestFloatDivisionSemantics:
    """Float division must keep ZeroDivisionError semantics under hot loops.

    Native `fdiv` yields inf on a zero divisor, so float division is kept out of
    compiled traces (tracer emits a Fallback). These tests pin that the divisor
    reaching zero arithmetically inside a hot loop still raises, not returns inf.
    """

    def test_float_div_by_zero_in_hot_loop_raises(self, vm_with_jit):
        """A divisor that reaches 0.0 arithmetically (no branch) must raise."""
        vm, _ = vm_with_jit
        # divisor = 1000.0 - i hits 0.0 at i == 1000, well past the JIT threshold.
        code = compile_code("""
        {
            i = 0
            last = 0.0
            while i < 2000 {
                last = 1000.0 / (1000.0 - i * 1.0)
                i = i + 1
            }
            last
        }
        """)
        with pytest.raises(Exception):
            vm.execute(code, (), {}, None)

    def test_float_div_no_zero_matches_interpreter(self, vm_with_jit, vm_without_jit):
        """Non-zero float division stays correct (and matches the interpreter)."""
        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit
        code = compile_code("""
        {
            i = 0
            acc = 0.0
            while i < 500 {
                acc = acc + 10.0 / (i * 1.0 + 1.0)
                i = i + 1
            }
            acc
        }
        """)
        assert vm_jit.execute(code, (), {}, None) == vm_no_jit.execute(code, (), {}, None)

    def test_int_true_division_is_float_in_hot_loop(self, vm_with_jit):
        """`/` on ints is true division (7/2 == 3.5), never integer truncation.

        The tracer used to type `/` as DivInt (is_int_value defaults to true),
        which would truncate to 3 and SIGFPE on /0. True division is now kept
        interpreted, so the float result is preserved under a hot loop.
        """
        vm, _ = vm_with_jit
        code = compile_code("""
        {
            i = 0
            acc = 0.0
            n = 7
            d = 2
            while i < 2000 {
                acc = acc + n / d
                i = i + 1
            }
            acc
        }
        """)
        assert vm.execute(code, (), {}, None) == 3.5 * 2000

    def test_int_true_division_by_zero_raises_in_hot_loop(self, vm_with_jit):
        """`/0` on ints must raise (not SIGFPE) even inside a compiled hot loop."""
        vm, _ = vm_with_jit
        # divisor = 1000 - i reaches 0 at i == 1000, past the JIT threshold.
        code = compile_code("""
        {
            i = 0
            last = 0.0
            n = 1000
            while i < 2000 {
                last = n / (1000 - i)
                i = i + 1
            }
            last
        }
        """)
        with pytest.raises(Exception):
            vm.execute(code, (), {}, None)


class TestIntModuloSemantics:
    """Integer modulo must keep ZeroDivisionError semantics under hot loops.

    Native `srem` traps (SIGFPE) on a zero divisor, where the interpreter raises
    ZeroDivisionError. ModInt is compiled with a zero-divisor side-exit: the
    divisor is guarded before the srem and a zero bails to the interpreter (which
    replays the modulo and raises). Modulo loops stay JIT-compilable.
    """

    def test_mod_by_zero_in_hot_loop_raises(self, vm_with_jit):
        """A divisor that reaches 0 arithmetically must raise, not SIGFPE."""
        vm, _ = vm_with_jit
        # divisor = 1000 - i reaches 0 at i == 1000, past the JIT threshold.
        code = compile_code("""
        {
            i = 0
            last = 0
            n = 1000
            while i < 2000 {
                last = n % (1000 - i)
                i = i + 1
            }
            last
        }
        """)
        with pytest.raises(ZeroDivisionError):
            vm.execute(code, (), {}, None)

    def test_mod_no_zero_matches_interpreter(self, vm_with_jit, vm_without_jit):
        """Non-zero modulo stays correct and matches the interpreter."""
        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit
        code = compile_code("""
        {
            i = 1
            total = 0
            while i < 500 {
                total = total + (1000 % i)
                i = i + 1
            }
            total
        }
        """)
        result_jit = vm_jit.execute(code, (), {}, None)
        assert result_jit == vm_no_jit.execute(code, (), {}, None)
        assert result_jit == sum(1000 % i for i in range(1, 500))
        # The modulo loop must actually compile (the guard preserves that).
        assert vm_jit.get_jit_stats()['compiled_loops'] >= 1

    def test_mod_negative_operands_match_interpreter(self, vm_with_jit, vm_without_jit):
        """Catnip `%` is Python floored (sign of divisor), not truncated.

        Native `srem` is truncated (sign of dividend), so a compiled modulo of
        opposite-sign operands would silently diverge from the interpreter unless
        codegen replicates the floored-modulo fixup.
        """
        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit
        # Negative dividend, positive divisor: floored -> {0,1,2}, truncated -> {0,-1,-2}.
        code = compile_code("""
        {
            i = 0
            total = 0
            while i < 600 {
                total = total + ((0 - i) % 3)
                i = i + 1
            }
            total
        }
        """)
        result_jit = vm_jit.execute(code, (), {}, None)
        assert result_jit == vm_no_jit.execute(code, (), {}, None)
        assert result_jit == sum((-i) % 3 for i in range(600))
        assert vm_jit.get_jit_stats()['compiled_loops'] >= 1

    def test_mod_negative_divisor_matches_interpreter(self, vm_with_jit, vm_without_jit):
        """Positive dividend, negative divisor: floored result takes divisor sign."""
        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit
        code = compile_code("""
        {
            i = 0
            total = 0
            while i < 600 {
                total = total + (i % (0 - 7))
                i = i + 1
            }
            total
        }
        """)
        result_jit = vm_jit.execute(code, (), {}, None)
        assert result_jit == vm_no_jit.execute(code, (), {}, None)
        assert result_jit == sum(i % -7 for i in range(600))
        assert vm_jit.get_jit_stats()['compiled_loops'] >= 1
