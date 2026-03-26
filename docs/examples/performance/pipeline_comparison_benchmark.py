#!/usr/bin/env python3
"""
Benchmark des niveaux d'optimisation Catnip.

Ce script compare `optimize=0..3` sur quelques charges simples.
Il ne compare pas plusieurs runtimes distincts et ne doit pas être lu
comme une comparaison de "pipelines" au sens architectural.
"""

from __future__ import annotations

import statistics
import time

import click

from catnip import Catnip

TEST_CASES = {
    "Redundant computation in loop": """
result = 0
x = 7
y = 13
for i in range(1, 100001) {
    result = result + (x * y) + (x * y) + (x + y) * 2
}
result
""",
    "Loop with invariant expression": """
total = 0
a = 3
b = 5
c = 11
for i in range(1, 100001) {
    total = total + (a * b + c) * (a + b * c) + (a * b + c)
}
total
""",
    "Nested loop with redundancy": """
sum = 0
x = 4
y = 9
for i in range(1, 501) {
    for j in range(1, 501) {
        sum = sum + (x * y) + (x + y) * (x - y)
    }
}
sum
""",
    "Deep tail recursion": """
countdown = (n, acc=0) => {
    if n <= 0 { acc }
    else { countdown(n - 1, acc + n * 2 + 1) }
}
countdown(50000)
""",
    "Chained function calls": """
f = (x) => { x * 3 + x * 3 + 7 }
g = (x) => { f(x) + f(x) + 1 }
total = 0
for i in range(1, 50001) {
    total = total + g(i)
}
total
""",
}


def benchmark_execution(cat_instance: Catnip, iterations: int, warmup: int):
    """Mesure `execute()` après une phase de chauffe."""
    for _ in range(warmup):
        cat_instance.execute()

    times = []
    result = None
    for _ in range(iterations):
        start = time.perf_counter()
        result = cat_instance.execute()
        times.append((time.perf_counter() - start) * 1000.0)

    avg = statistics.mean(times)
    std = statistics.stdev(times) if len(times) > 1 else 0.0
    return avg, std, result


@click.command()
@click.option("-n", "--iterations", default=10, show_default=True, help="Iterations par mesure.")
@click.option("-w", "--warmup", default=3, show_default=True, help="Iterations de chauffe.")
@click.option(
    "-l",
    "--levels",
    default="0-3",
    show_default=True,
    help="Niveaux d'optimisation (ex: '0-3', '0,2,3', '2').",
)
@click.option("-t", "--test", "tests", multiple=True, help="Filtrer par nom de test (sous-chaine).")
@click.option("-x", "--executor", default="vm", show_default=True, type=click.Choice(["vm", "ast"]))
def main(iterations: int, warmup: int, levels: str, tests: tuple[str, ...], executor: str) -> None:
    """Benchmark des niveaux d'optimisation Catnip (optimize=0..3)."""
    opt_levels = _parse_levels(levels)
    vm_mode = "on" if executor == "vm" else "off"

    selected = TEST_CASES
    if tests:
        selected = {k: v for k, v in TEST_CASES.items() if any(t.lower() in k.lower() for t in tests)}
        if not selected:
            raise click.BadParameter(f"aucun test ne matche {tests!r}", param_hint="--test")

    configs = [(f"opt={lvl}", dict(optimize=lvl, vm_mode=vm_mode)) for lvl in opt_levels]

    click.echo(f"Catnip optimize benchmark  executor={executor}  iterations={iterations}  warmup={warmup}")
    click.echo("=" * 80)

    results: dict[str, list[tuple[str, float, float, object]]] = {}

    for test_name, code in selected.items():
        click.echo(f"\n{test_name}")
        click.echo("-" * 80)

        test_results = []
        for config_name, config_opts in configs:
            cat = Catnip(**config_opts)
            cat.parse(code)

            avg, std, result = benchmark_execution(cat, iterations, warmup)
            test_results.append((config_name, avg, std, result))
            click.echo(f"{config_name:8s} {avg:8.3f} ms +/- {std:6.3f}")

        first_result = test_results[0][3]
        for config_name, _, _, result in test_results:
            if result != first_result:
                click.echo(f"WARNING: {config_name} produced a different result")

        results[test_name] = test_results

        baseline = test_results[0][1]
        best = test_results[-1][1]
        speedup = baseline / best if best > 0 else float("inf")
        click.echo(f"{test_results[0][0]} -> {test_results[-1][0]}: {speedup:.2f}x")

    click.echo("\nSummary")
    click.echo("=" * 80)
    click.echo("Average speedup relative to opt=0:")
    for i, (config_name, _) in enumerate(configs):
        speedups = []
        for test_name in selected:
            opt0_time = results[test_name][0][1]
            opti_time = results[test_name][i][1]
            speedups.append(opt0_time / opti_time if opti_time > 0 else float("inf"))
        avg_speedup = statistics.mean(speedups)
        click.echo(f"{config_name:8s} {avg_speedup:.2f}x")


def _parse_levels(spec: str) -> list[int]:
    """Parse '0-3', '0,2,3', ou '2' en liste d'entiers."""
    if "-" in spec and "," not in spec:
        lo, hi = spec.split("-", 1)
        return list(range(int(lo), int(hi) + 1))
    return [int(x.strip()) for x in spec.split(",")]


if __name__ == "__main__":
    main()
