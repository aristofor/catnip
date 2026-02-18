#!/usr/bin/env python3
"""
Comparaison de Performances - Pipelines Catnip

Compare différentes configurations du pipeline d'optimisation.
"""

import time
import statistics
from catnip import Catnip


def benchmark_execution(cat_instance, iterations=10, warmup=3):
    """Benchmark l'exécution avec warmup."""
    # Warmup
    for _ in range(warmup):
        cat_instance.execute()

    # Mesures
    times = []
    for _ in range(iterations):
        start = time.perf_counter()
        result = cat_instance.execute()
        elapsed = (time.perf_counter() - start) * 1000
        times.append(elapsed)

    avg = statistics.mean(times)
    std = statistics.stdev(times) if len(times) > 1 else 0

    return avg, std, result


def main():
    print("=" * 80)
    print("COMPARAISON DE PERFORMANCES - PIPELINES CATNIP")
    print("=" * 80)

    # Test cases
    test_cases = {
        "Sum loop": """
sum = 0
for i in range(1, 10001) {
    sum = sum + i
}
sum
""",
        "Factorial tail-recursive": """
factorial = (n, acc=1) => {
    if n <= 1 { acc }
    else { factorial(n - 1, n * acc) }
}
factorial(100)
""",
        "Nested if": """
x = 50
y = 0
if x > 0 {
    if x > 25 {
        y = x * 2
    } else {
        y = x
    }
}
y
""",
        "Arithmetic expression": """
x = 10
y = 20
z = (x + y) * (x - y) + x * y
z
""",
    }

    configs = [
        ("Opt 0 (aucune)", dict(optimize=0, vm_mode='on')),
        ("Opt 1 (basic)", dict(optimize=1, vm_mode='on')),
        ("Opt 2 (standard)", dict(optimize=2, vm_mode='on')),
        ("Opt 3 (aggressive)", dict(optimize=3, vm_mode='on')),
        ("VM mode", dict(optimize=3, vm_mode='on')),
        ("AST mode", dict(optimize=3, vm_mode='off')),
    ]

    results = {}

    for test_name, code in test_cases.items():
        print(f"\n{'=' * 80}")
        print(f"TEST: {test_name}")
        print("=" * 80)

        test_results = []

        for config_name, config_opts in configs:
            cat = Catnip(**config_opts)
            cat.parse(code)

            avg, std, result = benchmark_execution(cat)
            test_results.append((config_name, avg, std, result))

            print(f"{config_name:20s}: {avg:7.2f}ms ± {std:5.2f}ms")

        # Vérifier que tous les résultats sont identiques
        first_result = test_results[0][3]
        for config_name, avg, std, result in test_results:
            if result != first_result:
                print(f"⚠️  WARNING: {config_name} produced different result!")

        results[test_name] = test_results

        # Comparaison optimisations
        print(f"\n  Comparaison optimisations:")
        opt0_time = test_results[0][1]
        opt3_time = test_results[3][1]
        speedup_opt = opt0_time / opt3_time
        print(f"    Opt 0 → Opt 3: {speedup_opt:.2f}x speedup")

        # Comparaison VM vs AST
        print(f"\n  Comparaison exécuteurs:")
        vm_time = test_results[4][1]
        ast_time = test_results[5][1]
        speedup_vm = ast_time / vm_time
        print(f"    AST → VM: {speedup_vm:.2f}x speedup")

    # Résumé global
    print(f"\n{'=' * 80}")
    print("RÉSUMÉ GLOBAL")
    print("=" * 80)

    print("\nSpeedup moyen par optimisation:")
    for i, (config_name, _) in enumerate(configs[:4]):  # Opt 0-3
        speedups = []
        for test_name in test_cases:
            opt0_time = results[test_name][0][1]
            opti_time = results[test_name][i][1]
            speedups.append(opt0_time / opti_time)
        avg_speedup = statistics.mean(speedups)
        print(f"  {config_name:20s}: {avg_speedup:.2f}x")

    print("\nSpeedup VM vs AST:")
    vm_speedups = []
    for test_name in test_cases:
        vm_time = results[test_name][4][1]
        ast_time = results[test_name][5][1]
        vm_speedups.append(ast_time / vm_time)
    avg_vm_speedup = statistics.mean(vm_speedups)
    std_vm_speedup = statistics.stdev(vm_speedups)
    print(f"  Moyenne: {avg_vm_speedup:.2f}x ± {std_vm_speedup:.2f}x")

    # Recommandations
    print(f"\n{'=' * 80}")
    print("RECOMMANDATIONS")
    print("=" * 80)

    if avg_speedup >= 1.5:
        print("\n✅ Les optimisations apportent un gain significatif")
        print(f"   Niveau 3 recommandé (speedup: {avg_speedup:.2f}x)")
    else:
        print("\n≈ Les optimisations ont un impact modéré")
        print(f"   Niveau 2-3 suffisant (speedup: {avg_speedup:.2f}x)")

    if avg_vm_speedup >= 2.0:
        print("\n✅ VM mode apporte un speedup important")
        print(f"   VM recommandé (speedup: {avg_vm_speedup:.2f}x)")
    elif avg_vm_speedup >= 1.2:
        print("\n✅ VM mode plus rapide que AST")
        print(f"   VM recommandé (speedup: {avg_vm_speedup:.2f}x)")
    else:
        print("\n≈ Performance VM et AST similaires")
        print(f"   Les deux modes sont équivalents ({avg_vm_speedup:.2f}x)")


if __name__ == "__main__":
    main()
