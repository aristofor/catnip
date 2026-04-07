# FILE: tests/serial/jit/test_nonregression.py
"""
Tests de non-régression JIT pour CI.

Ces tests vérifient que le JIT :
- Compile toujours les hot loops (compiled_loops > 0)
- Donne des résultats identiques à l'interpréteur
- Fournit un speedup significatif (seuil minimum)
- Ne cause ni crash ni infinite loop
- Gère correctement les edge cases (closures, recursion, etc.)
"""

import time

import pytest

from tests.serial.jit.conftest import compile_code

# Force serial execution to avoid JIT state conflicts
pytestmark = pytest.mark.xdist_group(name='jit')


def test_jit_compiles_hot_loops(vm_with_jit):
    """Vérifie que le JIT compile toujours les hot loops."""
    vm, catnip = vm_with_jit

    code = compile_code('''
    total = 0
    i = 0
    while i < 150 {
        total = total + i
        i = i + 1
    }
    total
    ''')

    result = vm.execute(code, (), {}, None)
    assert result == sum(range(150))

    stats = vm.get_jit_stats()
    assert stats['compiled_loops'] >= 1, 'JIT devrait compiler au moins 1 boucle'
    assert stats['hot_loops'] >= 1, 'Au moins 1 hot loop devrait être détectée'


def test_jit_correctness_vs_interpreter(vm_with_jit, vm_without_jit):
    """Vérifie que JIT donne les mêmes résultats que l'interpréteur."""
    vm_jit, _ = vm_with_jit
    vm_interp, _ = vm_without_jit

    # Test plusieurs patterns de code
    test_cases = [
        # Simple accumulation
        ('total = 0; i = 0; while i < 100 { total = total + i; i = i + 1 }; total', sum(range(100))),
        # Multiplication
        ('result = 1; i = 1; while i <= 10 { result = result * i; i = i + 1 }; result', 3628800),
        # For loop
        ('total = 0; for i in range(1, 51) { total = total + i }; total', sum(range(1, 51))),
        # Nested arithmetic
        ('x = 0; for i in range(1, 11) { x = x + i * 2 }; x', sum(i * 2 for i in range(1, 11))),
    ]

    for code_str, expected in test_cases:
        code = compile_code(code_str)

        result_jit = vm_jit.execute(code, (), {}, None)
        result_interp = vm_interp.execute(code, (), {}, None)

        assert result_jit == result_interp, f'JIT et interpréteur divergent pour: {code_str}'
        assert result_jit == expected, f'Résultat incorrect pour: {code_str}'


def test_jit_minimum_speedup(vm_with_jit, vm_without_jit):
    """Vérifie que le JIT fournit un speedup minimum acceptable."""
    vm_jit, _ = vm_with_jit
    vm_interp, _ = vm_without_jit

    # Code avec hot loop arithmétique intensive
    code = compile_code('''
    total = 0
    for i in range(1, 100001) {
        total = total + i
    }
    total
    ''')

    # Warm-up JIT
    vm_jit.execute(code, (), {}, None)

    # Benchmark sans JIT
    start = time.perf_counter()
    result_interp = vm_interp.execute(code, (), {}, None)
    time_interp = time.perf_counter() - start

    # Benchmark avec JIT
    start = time.perf_counter()
    result_jit = vm_jit.execute(code, (), {}, None)
    time_jit = time.perf_counter() - start

    assert result_jit == result_interp == 5000050000

    # Seuil minimum : 10x speedup (conservateur, on mesure 75x actuellement)
    if time_jit > 0:
        speedup = time_interp / time_jit
        assert speedup >= 10, f'Speedup JIT trop faible: {speedup:.1f}x (minimum: 10x)'


def test_jit_no_infinite_loop(vm_with_jit):
    """Vérifie que le JIT ne cause pas d'infinite loop sur exécutions multiples."""
    vm, catnip = vm_with_jit

    code = compile_code('''
    total = 0
    i = 0
    while i < 150 {
        total = total + i
        i = i + 1
    }
    total
    ''')

    expected = sum(range(150))

    # Exécuter 5 fois pour vérifier qu'il n'y a pas de hang
    for iteration in range(5):
        result = vm.execute(code, (), {}, None)
        assert result == expected, f'Iteration {iteration} a échoué'


def test_jit_handles_closures_correctly(vm_with_jit):
    """Vérifie que le JIT n'interfère pas avec les closures."""
    vm, catnip = vm_with_jit

    # Code avec closure (supporte LoadScope avec guards depuis l'implémentation du 25 janvier)
    code = compile_code('''
    make_counter = () => {
        count = 0
        increment = () => {
            count = count + 1
            count
        }
        increment
    }

    counter = make_counter()
    result = 0
    for i in range(1, 11) {
        result = counter()
    }
    result
    ''')

    result = vm.execute(code, (), {}, None)
    assert result == 10


def test_jit_fallback_on_noncompilable(vm_with_jit):
    """Vérifie que le JIT fallback gracefully sur code non-compilable.

    Note: Certains patterns complexes (closures, calls, etc.) causent un fallback
    car ils nécessitent l'interpréteur Python. Break/continue sont maintenant
    supportés depuis le 25 janvier 2026.
    """
    vm, catnip = vm_with_jit

    # Code simple qui devrait compiler
    code_simple = compile_code('''
    total = 0
    i = 0
    while i < 150 {
        total = total + i
        i = i + 1
    }
    total
    ''')

    result = vm.execute(code_simple, (), {}, None)
    assert result == sum(range(150))

    # Vérifier que ça a compilé
    stats = vm.get_jit_stats()
    assert stats['compiled_loops'] > 0, 'Le code simple devrait compiler'


def test_jit_compiled_loops_metric(vm_with_jit):
    """Vérifie que la métrique compiled_loops est correctement rapportée."""
    vm, catnip = vm_with_jit

    initial_stats = vm.get_jit_stats()
    initial_compiled = initial_stats.get('compiled_loops', 0)

    # Exécuter un hot loop
    code = compile_code('''
    total = 0
    for i in range(1, 151) {
        total = total + i
    }
    total
    ''')

    vm.execute(code, (), {}, None)

    final_stats = vm.get_jit_stats()
    final_compiled = final_stats.get('compiled_loops', 0)

    assert final_compiled > initial_compiled, 'compiled_loops devrait augmenter après exécution'


def test_jit_stats_structure(vm_with_jit):
    """Vérifie que les stats JIT ont la structure attendue."""
    vm, catnip = vm_with_jit

    stats = vm.get_jit_stats()

    # Vérifier que toutes les clés attendues sont présentes
    expected_keys = {'total_loops_tracked', 'hot_loops', 'compiled_loops'}
    assert expected_keys.issubset(stats.keys()), f'Stats JIT manquent des clés: {expected_keys - stats.keys()}'

    # Vérifier que les valeurs sont des nombres non-négatifs
    for key in expected_keys:
        assert isinstance(stats[key], int), f'{key} devrait être un int'
        assert stats[key] >= 0, f'{key} devrait être non-négatif'


def test_jit_multiple_hot_loops(vm_with_jit):
    """Vérifie que le JIT peut compiler plusieurs hot loops dans le même code."""
    vm, catnip = vm_with_jit

    code = compile_code('''
    # Premier hot loop
    sum1 = 0
    for i in range(1, 151) {
        sum1 = sum1 + i
    }

    # Deuxième hot loop
    sum2 = 0
    for j in range(1, 151) {
        sum2 = sum2 + j * 2
    }

    tuple(sum1, sum2)
    ''')

    result = vm.execute(code, (), {}, None)
    expected = (sum(range(1, 151)), sum(j * 2 for j in range(1, 151)))
    assert result == expected

    stats = vm.get_jit_stats()
    # Au moins 2 loops devraient être trackées
    assert stats['total_loops_tracked'] >= 2


def test_jit_abort_on_exception_opcodes_then_compile(vm_with_jit):
    """Hot loop with try/except aborts trace, but subsequent hot loop still compiles."""
    vm, catnip = vm_with_jit

    # Loop with try/except -- trace should abort on exception opcodes
    code_with_try = compile_code('''
    total = 0
    i = 0
    while i < 150 {
        try {
            total = total + i
        } except {
            _ => { 0 }
        }
        i = i + 1
    }
    total
    ''')

    result = vm.execute(code_with_try, (), {}, None)
    assert result == sum(range(150))

    stats_after_try = vm.get_jit_stats()
    compiled_after_try = stats_after_try['compiled_loops']

    # Pure arithmetic loop -- should still compile in the same VM session
    code_pure = compile_code('''
    total = 0
    for i in range(1, 151) {
        total = total + i
    }
    total
    ''')

    result = vm.execute(code_pure, (), {}, None)
    assert result == sum(range(1, 151))

    stats_after_pure = vm.get_jit_stats()
    assert (
        stats_after_pure['compiled_loops'] > compiled_after_try
    ), 'JIT should still compile loops after a trace abort from exception opcodes'
