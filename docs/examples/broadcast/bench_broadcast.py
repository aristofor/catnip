#!/usr/bin/env python3
"""
Benchmark de performance : Python natif vs Catnip broadcasting.

Compare les performances des opérations vectorisées entre :

- Listes Python avec list comprehension
- NumPy arrays
- Catnip broadcasting

Usage:
    python bench_broadcast.py
"""

import time
import sys
from pathlib import Path

# Ajouter le parent au path pour importer catnip
sys.path.insert(0, str(Path(__file__).parent.parent.parent))

from catnip import Catnip

try:
    import numpy as np

    HAS_NUMPY = True
except ImportError:
    HAS_NUMPY = False
    print("⚠️  NumPy non installé, certains benchmarks seront ignorés")


def benchmark(name: str, func, iterations: int = 5):
    """
    Exécute un benchmark et affiche les résultats.

    :param name: Nom du test
    :param func: Fonction à benchmarker
    :param iterations: Nombre d'itérations
    """
    times = []
    result = None

    for _ in range(iterations):
        start = time.perf_counter()
        result = func()
        end = time.perf_counter()
        times.append(end - start)

    avg_time = sum(times) / len(times)
    min_time = min(times)
    max_time = max(times)

    print(f"\n  {name}")
    print(f"    Moyenne: {avg_time*1000:.3f} ms")
    print(f"    Min: {min_time*1000:.3f} ms")
    print(f"    Max: {max_time*1000:.3f} ms")

    return avg_time, result


def run_catnip(catnip: Catnip, code: str):
    """Helper : parse puis exécute le code Catnip avec le contexte courant."""
    catnip.parse(code)
    return catnip.execute()


def test_simple_addition(size: int = 10000):
    """Test d'addition simple."""
    print(f"\n{'='*60}")
    print(f"Test 1: Addition de {size:,} éléments")
    print('=' * 60)

    data = list(range(size))

    # Python natif (list comprehension)
    def python_native():
        return [x + 10 for x in data]

    time_python, result_python = benchmark("Python (list comp)", python_native)

    # NumPy
    if HAS_NUMPY:
        np_data = np.array(data)

        def numpy_version():
            return np_data + 10

        time_numpy, result_numpy = benchmark("NumPy", numpy_version)
        speedup_numpy = time_python / time_numpy
        print(f"    Speedup vs Python: {speedup_numpy:.2f}x")

    # Catnip
    catnip = Catnip()
    catnip.context.globals['data'] = data

    def catnip_version():
        return run_catnip(catnip, "data.[+ 10]")

    time_catnip, result_catnip = benchmark("Catnip broadcasting", catnip_version)
    speedup_catnip = time_python / time_catnip
    print(f"    Speedup vs Python: {speedup_catnip:.2f}x")

    # Vérification
    assert result_python == result_catnip, "Les résultats diffèrent!"


def test_chained_operations(size: int = 10000):
    """Test d'opérations enchaînées."""
    print(f"\n{'='*60}")
    print(f"Test 2: Opérations enchaînées sur {size:,} éléments")
    print("Expression: (x * 2 + 5) / 3")
    print('=' * 60)

    data = list(range(size))

    # Python natif
    def python_native():
        return [(x * 2 + 5) / 3 for x in data]

    time_python, result_python = benchmark("Python (list comp)", python_native)

    # NumPy
    if HAS_NUMPY:
        np_data = np.array(data)

        def numpy_version():
            return (np_data * 2 + 5) / 3

        time_numpy, result_numpy = benchmark("NumPy", numpy_version)
        speedup_numpy = time_python / time_numpy
        print(f"    Speedup vs Python: {speedup_numpy:.2f}x")

    # Catnip
    catnip = Catnip()
    catnip.context.globals['data'] = data

    def catnip_version():
        return run_catnip(catnip, "data.[* 2].[+ 5].[/ 3]")

    time_catnip, result_catnip = benchmark("Catnip broadcasting", catnip_version)
    speedup_catnip = time_python / time_catnip
    print(f"    Speedup vs Python: {speedup_catnip:.2f}x")
    assert result_python == result_catnip, "Les résultats diffèrent!"


def test_comparisons(size: int = 10000):
    """Test de comparaisons vectorisées."""
    print(f"\n{'='*60}")
    print(f"Test 3: Comparaisons sur {size:,} éléments")
    print("Expression: x > 5000")
    print('=' * 60)

    data = list(range(size))

    # Python natif
    def python_native():
        return [x > 5000 for x in data]

    time_python, result_python = benchmark("Python (list comp)", python_native)

    # NumPy
    if HAS_NUMPY:
        np_data = np.array(data)

        def numpy_version():
            return np_data > 5000

        time_numpy, result_numpy = benchmark("NumPy", numpy_version)
        speedup_numpy = time_python / time_numpy
        print(f"    Speedup vs Python: {speedup_numpy:.2f}x")

    # Catnip
    catnip = Catnip()
    catnip.context.globals['data'] = data

    def catnip_version():
        return run_catnip(catnip, "data.[> 5000]")

    time_catnip, result_catnip = benchmark("Catnip broadcasting", catnip_version)
    speedup_catnip = time_python / time_catnip
    print(f"    Speedup vs Python: {speedup_catnip:.2f}x")
    assert result_python == result_catnip, "Les résultats diffèrent!"


def test_two_lists(size: int = 10000):
    """Test d'opérations sur deux listes."""
    print(f"\n{'='*60}")
    print(f"Test 4: Addition de deux listes de {size:,} éléments")
    print('=' * 60)

    data_a = list(range(size))
    data_b = list(range(size, size * 2))

    # Python natif
    def python_native():
        return [a + b for a, b in zip(data_a, data_b)]

    time_python, result_python = benchmark("Python (list comp + zip)", python_native)

    # NumPy
    if HAS_NUMPY:
        np_a = np.array(data_a)
        np_b = np.array(data_b)

        def numpy_version():
            return np_a + np_b

        time_numpy, result_numpy = benchmark("NumPy", numpy_version)
        speedup_numpy = time_python / time_numpy
        print(f"    Speedup vs Python: {speedup_numpy:.2f}x")

    # Catnip
    catnip = Catnip()
    catnip.context.globals['data_a'] = data_a
    catnip.context.globals['data_b'] = data_b

    def catnip_version():
        return run_catnip(catnip, "data_a.[+ data_b]")

    time_catnip, result_catnip = benchmark("Catnip broadcasting", catnip_version)
    speedup_catnip = time_python / time_catnip
    print(f"    Speedup vs Python: {speedup_catnip:.2f}x")
    assert result_python == result_catnip, "Les résultats diffèrent!"


def main():
    """Exécute tous les benchmarks."""
    print("\n" + "=" * 60)
    print("Benchmark Broadcasting : Python vs NumPy vs Catnip")
    print("=" * 60)

    # Tests avec différentes tailles
    sizes = [1000, 10000]

    for size in sizes:
        test_simple_addition(size)
        test_chained_operations(size)
        test_comparisons(size)
        test_two_lists(size)

    print("\n" + "=" * 60)
    print("Benchmark terminé")
    print("=" * 60 + "\n")

    print("\nNotes:")
    print("  - NumPy est généralement le plus rapide (C optimisé)")
    print("  - Catnip broadcasting est comparable aux list comprehensions Python")
    print("  - La syntaxe Catnip est plus concise et lisible")
    print("  - L'overhead de parsing Catnip est inclus dans les mesures")


if __name__ == '__main__':
    main()
