# FILE: tests/serial/jit/test_compilation.py
"""
Tests de compilation JIT.

Vérifie que le JIT détecte, trace et compile les boucles chaudes.
"""

import pytest

from tests.serial.jit.conftest import compile_code

# Force serial execution to avoid JIT state conflicts
pytestmark = pytest.mark.xdist_group(name="jit")


class TestHotLoopDetection:
    """Tests de détection des hot loops."""

    def test_while_loop_detected(self, vm_with_jit):
        """Le JIT détecte une boucle while chaude."""
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
        stats = vm.get_jit_stats()

        assert result == 150
        assert stats['hot_loops'] >= 1, "Au moins une hot loop détectée"

    def test_for_loop_detected(self, vm_with_jit):
        """Le JIT détecte une boucle for chaude."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            total = 0
            for i in range(200) {
                total = total + i
            }
            total
        }
        """)

        result = vm.execute(code, (), {}, None)
        stats = vm.get_jit_stats()

        assert result == sum(range(200))
        assert stats['hot_loops'] >= 1


class TestTraceRecording:
    """Tests d'enregistrement de traces."""

    def test_trace_recorded(self, vm_with_jit):
        """Une trace est enregistrée pour une hot loop."""
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
        stats = vm.get_jit_stats()

        assert result == 150
        assert stats['total_loops_tracked'] >= 1


class TestTraceCompilation:
    """Tests de compilation de traces."""

    def test_simple_loop_compiles(self, vm_with_jit):
        """Une boucle simple est compilée par le JIT."""
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
        stats = vm.get_jit_stats()

        assert result == 150
        assert stats['compiled_loops'] >= 1, "Au moins une boucle compilée"

    def test_accumulator_loop_compiles(self, vm_with_jit):
        """Une boucle avec accumulateur est compilée."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            total = 0
            i = 1
            while i <= 150 {
                total = total + i
                i = i + 1
            }
            total
        }
        """)

        result = vm.execute(code, (), {}, None)
        stats = vm.get_jit_stats()

        assert result == sum(range(1, 151))
        assert stats['compiled_loops'] >= 1

    def test_arithmetic_operations_compile(self, vm_with_jit):
        """Les opérations arithmétiques sont JIT-compilables."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            result = 0
            i = 0
            while i < 150 {
                result = result + i * 2 - 1
                i = i + 1
            }
            result
        }
        """)

        vm.execute(code, (), {}, None)
        stats = vm.get_jit_stats()

        assert stats['compiled_loops'] >= 1


class TestNonCompilableTraces:
    """Tests de traces non compilables (fallback)."""

    def test_mixed_types_not_compiled(self, vm_with_jit):
        """Une boucle avec types mixtes (int/float) n'est pas compilée."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            result = 0.0
            i = 0
            while i < 150 {
                result = result + i * 1.5
                i = i + 1
            }
            result
        }
        """)

        vm.execute(code, (), {}, None)
        stats = vm.get_jit_stats()

        # Float support manquant → fallback attendu
        # Mais l'exécution doit réussir
        assert stats['hot_loops'] >= 1
