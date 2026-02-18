# FILE: tests/language/test_broadcast_simd.py
"""
Tests de correctness pour les fast paths SIMD du broadcasting.

Vérifie que les résultats des opérations sur listes numériques homogènes
sont identiques entre le chemin SIMD Rust et le chemin Python standard.
"""

import pytest

from catnip import Catnip
from catnip.exc import CatnipRuntimeError


def exec_catnip(code):
    c = Catnip()
    c.parse(code)
    return c.execute()


# --- Arithmétique sur listes d'entiers ---


class TestSIMDArithmeticInt:
    def test_add(self):
        assert exec_catnip("list(1, 2, 3).[+ 10]") == [11, 12, 13]

    def test_sub(self):
        assert exec_catnip("list(10, 20, 30).[- 5]") == [5, 15, 25]

    def test_mul(self):
        assert exec_catnip("list(2, 3, 4).[* 3]") == [6, 9, 12]

    def test_div(self):
        result = exec_catnip("list(10, 20, 30).[/ 5]")
        assert result == [2.0, 4.0, 6.0]
        assert all(isinstance(x, float) for x in result)

    def test_floordiv(self):
        assert exec_catnip("list(7, 10, 15).[// 3]") == [2, 3, 5]

    def test_mod(self):
        assert exec_catnip("list(7, 10, 15).[% 3]") == [1, 1, 0]

    def test_pow(self):
        assert exec_catnip("list(2, 3, 4).[** 2]") == [4, 9, 16]

    def test_add_negative(self):
        assert exec_catnip("list(1, 2, 3).[+ -5]") == [-4, -3, -2]

    def test_sub_negative(self):
        assert exec_catnip("list(1, 2, 3).[- -5]") == [6, 7, 8]


# --- Arithmétique sur listes de floats ---


class TestSIMDArithmeticFloat:
    def test_add(self):
        assert exec_catnip("list(1.5, 2.5, 3.5).[+ 10.0]") == [11.5, 12.5, 13.5]

    def test_sub(self):
        assert exec_catnip("list(10.0, 20.0, 30.0).[- 5.0]") == [5.0, 15.0, 25.0]

    def test_mul(self):
        assert exec_catnip("list(2.0, 3.0, 4.0).[* 2.5]") == [5.0, 7.5, 10.0]

    def test_div(self):
        assert exec_catnip("list(10.0, 20.0, 30.0).[/ 5.0]") == [2.0, 4.0, 6.0]

    def test_floordiv(self):
        assert exec_catnip("list(7.0, 10.5, 15.0).[// 3.0]") == [2.0, 3.0, 5.0]

    def test_mod(self):
        result = exec_catnip("list(7.5, 10.0, 15.5).[% 3.0]")
        assert result == pytest.approx([1.5, 1.0, 0.5])

    def test_pow(self):
        assert exec_catnip("list(2.0, 3.0, 4.0).[** 2.0]") == [4.0, 9.0, 16.0]


# --- Comparaisons sur listes d'entiers ---


class TestSIMDComparisonInt:
    def test_gt(self):
        assert exec_catnip("list(1, 5, 10).[> 3]") == [False, True, True]

    def test_lt(self):
        assert exec_catnip("list(1, 5, 10).[< 5]") == [True, False, False]

    def test_ge(self):
        assert exec_catnip("list(1, 5, 10).[>= 5]") == [False, True, True]

    def test_le(self):
        assert exec_catnip("list(1, 5, 10).[<= 5]") == [True, True, False]

    def test_eq(self):
        assert exec_catnip("list(1, 5, 10).[== 5]") == [False, True, False]

    def test_ne(self):
        assert exec_catnip("list(1, 5, 10).[!= 5]") == [True, False, True]


# --- Comparaisons sur listes de floats ---


class TestSIMDComparisonFloat:
    def test_gt(self):
        assert exec_catnip("list(1.0, 5.0, 10.0).[> 3.0]") == [False, True, True]

    def test_lt(self):
        assert exec_catnip("list(1.0, 5.0, 10.0).[< 5.0]") == [True, False, False]

    def test_eq(self):
        assert exec_catnip("list(1.0, 5.0, 10.0).[== 5.0]") == [False, True, False]


# --- Filtres SIMD ---


class TestSIMDFilter:
    def test_filter_gt_int(self):
        assert exec_catnip("list(3, 8, 2, 9, 5).[if > 5]") == [8, 9]

    def test_filter_lt_int(self):
        assert exec_catnip("list(3, 8, 2, 9, 5).[if < 5]") == [3, 2]

    def test_filter_ge_int(self):
        assert exec_catnip("list(3, 8, 2, 9, 5).[if >= 5]") == [8, 9, 5]

    def test_filter_le_int(self):
        assert exec_catnip("list(3, 8, 2, 9, 5).[if <= 5]") == [3, 2, 5]

    def test_filter_eq_int(self):
        assert exec_catnip("list(3, 8, 2, 9, 5).[if == 5]") == [5]

    def test_filter_ne_int(self):
        assert exec_catnip("list(3, 3, 3).[if != 3]") == []

    def test_filter_gt_float(self):
        assert exec_catnip("list(1.5, 3.5, 5.5).[if > 3.0]") == [3.5, 5.5]

    def test_filter_lt_float(self):
        assert exec_catnip("list(1.5, 3.5, 5.5).[if < 4.0]") == [1.5, 3.5]


# --- Edge cases ---


class TestSIMDEdgeCases:
    def test_single_element_int(self):
        assert exec_catnip("list(42).[+ 1]") == [43]

    def test_single_element_float(self):
        assert exec_catnip("list(3.14).[* 2.0]") == [6.28]

    def test_heterogeneous_fallback(self):
        """Liste hétérogène -> fallback au chemin Python."""
        assert exec_catnip('list("a", "b", "c").[+ "x"]') == ["ax", "bx", "cx"]

    def test_chaining_simd(self):
        """Chaînage map + filter sur listes numériques."""
        result = exec_catnip("list(1, 2, 3, 4, 5).[* 2].[if > 5]")
        assert result == [6, 8, 10]

    def test_large_vector(self):
        """Grand vecteur pour vérifier la consistance."""
        code = """
        n = 10000
        data = list()
        i = 0
        while i < n {
            data = data + list(i)
            i = i + 1
        }
        data.[+ 1]
        """
        result = exec_catnip(code)
        assert len(result) == 10000
        assert result[0] == 1
        assert result[9999] == 10000

    def test_division_by_zero_int(self):
        """Division par zéro doit lever une erreur (fallback Python)."""
        with pytest.raises((ZeroDivisionError, CatnipRuntimeError)):
            exec_catnip("list(1, 2, 3).[/ 0]")

    def test_mod_by_zero_int(self):
        """Modulo par zéro doit lever une erreur (fallback Python)."""
        with pytest.raises((ZeroDivisionError, CatnipRuntimeError)):
            exec_catnip("list(1, 2, 3).[% 0]")

    def test_floordiv_by_zero_int(self):
        """Floor division par zéro doit lever une erreur (fallback Python)."""
        with pytest.raises((ZeroDivisionError, CatnipRuntimeError)):
            exec_catnip("list(1, 2, 3).[// 0]")

    def test_result_is_list(self):
        """Le résultat doit être une list Python."""
        result = exec_catnip("list(1, 2, 3).[+ 1]")
        assert isinstance(result, list)

    def test_bool_list_not_optimized(self):
        """Liste de bools ne doit pas prendre le fast path int."""
        result = exec_catnip("list(True, False, True).[== True]")
        assert result == [True, False, True]

    def test_filter_empty_result(self):
        """Filtre qui ne garde rien."""
        assert exec_catnip("list(1, 2, 3).[if > 100]") == []

    def test_filter_all_pass(self):
        """Filtre qui garde tout."""
        assert exec_catnip("list(1, 2, 3).[if > 0]") == [1, 2, 3]

    def test_negative_pow_fallback(self):
        """Puissance négative sur ints -> fallback Python (float result)."""
        result = exec_catnip("list(2, 4).[** -1]")
        assert result == [0.5, 0.25]
