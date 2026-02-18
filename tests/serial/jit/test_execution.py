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
