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


def test_jit_deopt_no_double_count_accumulator():
    """Un guard fail en milieu de boucle ne doit pas double-compter un accumulateur.

    Le corps committe `total += i` (StoreLocal), puis une branche rare devient un
    guard (false pendant le warm-up, true à l'itération du flip). Au side-exit, le
    deopt rejoue toute l'itération dans l'interpréteur depuis le header : si le
    code natif réécrit les locals mi-itération au lieu de leur valeur début
    d'itération, `total += i` du flip est appliqué deux fois.

    Chaque cas tourne dans un VM frais : le map JIT `compiled` est keyé par
    loop_offset seul, donc partager un VM entre deux programmes 500-iters de même
    offset ferait réutiliser la mauvaise trace (bug séparé).
    """
    from catnip._rs import VM, Compiler

    from catnip import Catnip

    def run(code_str, jit):
        c = Catnip(vm_mode='on')
        vm = VM()
        vm.set_context(c.context)
        if jit:
            vm.enable_jit()
        c.parse(code_str)
        return vm, vm.execute(Compiler().compile(c.code), (), {}, None)

    # Flip à i == 400, bien après le warm-up JIT (~100 itérations).
    cases = [
        # while + un accumulateur
        '''
        total = 0
        i = 0
        while i < 500 {
            total = total + i
            if i == 400 { total = total + 0 }
            i = i + 1
        }
        total
        ''',
        # for-range + un accumulateur
        '''
        total = 0
        for i in range(500) {
            total = total + i
            if i == 400 { total = total + 0 }
        }
        total
        ''',
        # deux accumulateurs committés avant le guard
        '''
        a = 0
        b = 0
        i = 0
        while i < 500 {
            a = a + i
            b = b + 2 * i
            if i == 300 { a = a + 1 }
            i = i + 1
        }
        a * 1000000 + b
        ''',
        # accumulateur float : trace float avec side-exit (même classe de bug)
        '''
        total = 0.0
        i = 0
        while i < 500 {
            total = total + 1.5
            if i == 400 { total = total + 0.0 }
            i = i + 1
        }
        total
        ''',
        # deopt via le guard d'overflow SmallInt (pas une branche) : `a` est
        # committé avant l'op qui overflow, il ne doit pas être double-compté.
        # `big` reste SmallInt au trace (~i=100), overflow vers i~352.
        '''
        a = 0
        big = 0
        i = 0
        while i < 500 {
            a = a + 1
            big = big + 200000000000
            i = i + 1
        }
        tuple(a, big)
        ''',
    ]

    compiled_any = False
    for code_str in cases:
        vm_jit, result_jit = run(code_str, jit=True)
        _, result_interp = run(code_str, jit=False)
        assert result_jit == result_interp, f'deopt double-compte: {code_str}'
        compiled_any = compiled_any or vm_jit.get_jit_stats()['compiled_loops'] >= 1

    # Au moins un hot loop doit réellement passer par du code natif compilé (sinon
    # le test passe pour la mauvaise raison : aucun deopt exercé).
    assert compiled_any, 'au moins un hot loop aurait dû compiler'


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


def test_jit_bigint_in_hot_loop_no_crash(vm_with_jit):
    """Une valeur BigInt vivante au moment du trace abandonne la trace, sans crash.

    Repro du crash Cranelift (def_var F64/I64): `b` overflow SmallInt -> BigInt
    avant que la boucle ne devienne chaude. Un BigInt n'est ni int ni float pour
    le JIT; le tracer ne doit pas le traiter comme un float (GuardFloat -> slot
    F64) sous peine de panic au StoreLocal. Le fix abandonne la trace; la boucle
    reste interprétée et donne le bon résultat.
    """
    vm, catnip = vm_with_jit

    code = compile_code('''
    b = 1
    i = 0
    while i < 200 {
        b = b * 3
        i = i + 1
    }
    b
    ''')

    # Si le bug était présent, ce vm.execute aborterait le process (SIGABRT).
    result = vm.execute(code, (), {}, None)
    assert result == 3**200

    # La boucle BigInt n'est pas compilable: elle est abandonnée, pas JIT-compilée.
    stats = vm.get_jit_stats()
    assert stats['compiled_loops'] == 0, 'Une boucle BigInt ne doit pas être JIT-compilée'


def test_jit_no_cross_program_trace_reuse(vm_with_jit):
    """Un second programme ne réutilise pas la trace native du premier.

    Les traces compilées en mémoire sont keyées par (bytecode_hash, loop_offset),
    pas par loop_offset seul. Deux programmes de structure identique (même offset
    de boucle) mais de corps différent (hash différents), exécutés dans le même
    VM, ne doivent pas partager de trace.
    """
    vm, catnip = vm_with_jit

    # Structures identiques -> même loop_offset ; corps différents -> hash différents.
    code_a = compile_code('{ total = 0; i = 0; while i < 500 { total = total + i; i = i + 1 }; total }')
    code_b = compile_code('{ total = 0; i = 0; while i < 500 { total = total + 2; i = i + 1 }; total }')

    result_a = vm.execute(code_a, (), {}, None)
    assert result_a == sum(range(500))
    assert vm.get_jit_stats()['compiled_loops'] >= 1, 'Le premier programme doit être JIT-compilé'

    # Sans le fix, B réutiliserait la trace de A et renverrait 124752 au lieu de 1000.
    result_b = vm.execute(code_b, (), {}, None)
    assert result_b == 1000


def test_jit_recompiles_cross_program_loop(vm_with_jit):
    """Un second programme à boucle chaude au même offset est suivi comme une entrée distincte du detector.

    Le HotLoopDetector est keyé par (bytecode_hash, loop_offset). Program A porte son
    offset à chaud ; program B, même offset mais hash différent, doit être compté comme une
    boucle distincte au lieu d'être vu « déjà chaud » et jamais re-tracé.

    Discriminant : `total_loops_tracked` (compteurs du detector), pas `compiled_loops`.
    `compiled_loops` est confondu par le warm-start disque -- `try_compile_from_cache` compile
    B depuis ~/.cache/catnip indépendamment du detector, donc il passerait même fix reverté sur
    un cache chaud. `total_loops_tracked` vient de `record_loop_header`, keyé par (hash, offset) :
    sans le re-key, B réutiliserait la clé de A et le compteur resterait figé. Robuste à l'état
    du cache disque.
    """
    vm, catnip = vm_with_jit

    code_a = compile_code('{ total = 0; i = 0; while i < 500 { total = total + i; i = i + 1 }; total }')
    code_b = compile_code('{ total = 0; i = 0; while i < 500 { total = total + 2; i = i + 1 }; total }')

    vm.execute(code_a, (), {}, None)
    tracked_after_a = vm.get_jit_stats()['total_loops_tracked']
    assert tracked_after_a >= 1, 'Le premier programme doit enregistrer sa boucle dans le detector'

    vm.execute(code_b, (), {}, None)
    tracked_after_b = vm.get_jit_stats()['total_loops_tracked']
    assert tracked_after_b > tracked_after_a, (
        f"La boucle de B doit être une entrée distincte du detector, keyée par hash "
        f"(total_loops_tracked {tracked_after_a} -> {tracked_after_b})"
    )


def test_jit_type_flip_never_wrong():
    """Un local qui change de type numérique entre deux appels de la même
    fonction ne doit JAMAIS produire un résultat faux.

    Avant le fix 2026-07-13, le prédicat d'entrée JIT du for laissait entrer
    un float dans un slot tracé int (unbox garbage silencieux : 0 au lieu de
    750.0), et le chemin warm-start re-traçait/recompilait sur échec de guards
    -- recompilation qui échouait en silence (symbole Cranelift dupliqué) en
    laissant des guards frais sur du code périmé. Deux rounds : le premier
    exerce le chemin froid, le second le warm-start depuis le cache disque.
    """
    from catnip import Catnip

    def run(code):
        c = Catnip()
        c.parse(code)
        r = c.execute()
        del c
        return r

    bodies = dict(
        for_loop="""
f = (x) => {{
  s = 0
  for i in range(300) {{
    s = s + x
  }}
  s
}}
list(f({a}), f({b}))
""",
        while_loop="""
f = (x) => {{
  s = 0
  i = 0
  while i < 300 {{
    s = s + x
    i = i + 1
  }}
  s
}}
list(f({a}), f({b}))
""",
    )

    flips = ((1, 2.5), (2.5, 2))
    for round_ in (1, 2):  # round 2 = warm-start depuis le trace cache disque
        for name, tpl in bodies.items():
            for a, b in flips:
                src = tpl.format(a=a, b=b)
                expected = run('pragma("jit", False)\n' + src)
                got = run('pragma("jit", True)\n' + src)
                assert got == expected, f"round {round_} {name} flip {a}->{b}: jit={got} vs interp={expected}"
