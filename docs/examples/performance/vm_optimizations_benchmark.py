#!/usr/bin/env python3
"""
VM Optimizations Benchmark (v2) - Fevrier 2026

Mesure des hot paths VM Catnip avec un focus explicite sur BigInt:
1. ForRangeInt: boucle numerique range()
2. TailRecursionToLoopPass: tail recursion -> loop
3. BigInt growth: multiplications successives
4. BigInt div/mod: // et % sur gros entiers

Le script compare Catnip VM et Python natif avec:
- warmup
- min/median/mean/p95
- verification de resultat
"""

from __future__ import annotations

import statistics
import time
from dataclasses import dataclass
from typing import Any, Callable

import click

from catnip import Catnip


@dataclass
class BenchStats:
    min_ms: float
    median_ms: float
    mean_ms: float
    p95_ms: float


@dataclass
class ScenarioResult:
    name: str
    catnip_stats: BenchStats
    python_stats: BenchStats
    catnip_result: Any
    python_result: Any


def percentile(values: list[float], q: float) -> float:
    """Percentile simple (nearest-rank) pour q in [0, 100]."""
    if not values:
        return 0.0
    if q <= 0:
        return min(values)
    if q >= 100:
        return max(values)
    ordered = sorted(values)
    idx = int(round((q / 100) * (len(ordered) - 1)))
    return ordered[idx]


def summarize(samples_ms: list[float]) -> BenchStats:
    return BenchStats(
        min_ms=min(samples_ms),
        median_ms=statistics.median(samples_ms),
        mean_ms=statistics.fmean(samples_ms),
        p95_ms=percentile(samples_ms, 95),
    )


def benchmark_callable(fn: Callable[[], Any], iterations: int, warmup: int) -> tuple[BenchStats, Any]:
    for _ in range(warmup):
        fn()

    samples_ms: list[float] = []
    last_result = None
    for _ in range(iterations):
        start = time.perf_counter()
        last_result = fn()
        samples_ms.append((time.perf_counter() - start) * 1000.0)
    return summarize(samples_ms), last_result


def benchmark_catnip(code: str, iterations: int, warmup: int) -> tuple[BenchStats, Any]:
    c = Catnip(vm_mode="on")
    c.parse(code)
    return benchmark_callable(c.execute, iterations=iterations, warmup=warmup)


def print_stats_pair(name: str, cat_stats: BenchStats, py_stats: BenchStats) -> None:
    click.echo(f"\n{name}")
    click.echo("-" * 78)
    click.echo("Metric               Catnip (ms)      Python (ms)      Ratio Catnip/Python")
    click.echo("-" * 78)
    for metric in ("min_ms", "median_ms", "mean_ms", "p95_ms"):
        c = getattr(cat_stats, metric)
        p = getattr(py_stats, metric)
        ratio = (c / p) if p > 0 else float("inf")
        click.echo(f"{metric:<20} {c:>12.3f}      {p:>11.3f}      {ratio:>8.2f}x")


def scenario_for_range(iterations: int, warmup: int) -> ScenarioResult:
    code = """
total = 0
for i in range(1, 100001) {
    total = total + i
}
total
"""

    def py_fn():
        total = 0
        for i in range(1, 100001):
            total = total + i
        return total

    cat_stats, cat_res = benchmark_catnip(code, iterations, warmup)
    py_stats, py_res = benchmark_callable(py_fn, iterations, warmup)
    return ScenarioResult("ForRangeInt / Sum(1..100000)", cat_stats, py_stats, cat_res, py_res)


def scenario_tail_factorial(iterations: int, warmup: int, n: int) -> ScenarioResult:
    code = f"""
factorial = (n, acc=1) => {{
    if n <= 1 {{ acc }}
    else {{ factorial(n - 1, n * acc) }}
}}
factorial({n})
"""

    def py_fn():
        acc = 1
        x = n
        while x > 1:
            acc *= x
            x -= 1
        return acc

    cat_stats, cat_res = benchmark_catnip(code, iterations, warmup)
    py_stats, py_res = benchmark_callable(py_fn, iterations, warmup)
    return ScenarioResult(
        f"TailRecursionToLoop / factorial({n})",
        cat_stats,
        py_stats,
        cat_res,
        py_res,
    )


def scenario_bigint_growth(iterations: int, warmup: int, steps: int) -> ScenarioResult:
    code = f"""
x = 1
for i in range(0, {steps}) {{
    x = x * 3
}}
x
"""

    def py_fn():
        x = 1
        for _ in range(steps):
            x = x * 3
        return x

    cat_stats, cat_res = benchmark_catnip(code, iterations, warmup)
    py_stats, py_res = benchmark_callable(py_fn, iterations, warmup)
    return ScenarioResult(
        f"BigInt Growth / 3^{steps}",
        cat_stats,
        py_stats,
        cat_res,
        py_res,
    )


def scenario_bigint_divmod(iterations: int, warmup: int, growth_steps: int, divmod_steps: int) -> ScenarioResult:
    code = f"""
x = 1
for i in range(0, {growth_steps}) {{
    x = x * 3
}}
for j in range(0, {divmod_steps}) {{
    q = x // 7
    r = x % 7
    x = q + r
}}
x
"""

    def py_fn():
        x = 1
        for _ in range(growth_steps):
            x = x * 3
        for _ in range(divmod_steps):
            q = x // 7
            r = x % 7
            x = q + r
        return x

    cat_stats, cat_res = benchmark_catnip(code, iterations, warmup)
    py_stats, py_res = benchmark_callable(py_fn, iterations, warmup)
    return ScenarioResult(
        f"BigInt Div/Mod / growth={growth_steps}, loops={divmod_steps}",
        cat_stats,
        py_stats,
        cat_res,
        py_res,
    )


@click.command()
@click.option("-n", "--iterations", default=12, show_default=True, help="Iterations de mesure par scenario.")
@click.option("-w", "--warmup", default=3, show_default=True, help="Iterations de chauffe.")
@click.option("--factorial-n", default=1000, show_default=True, help="Input pour le scenario factorial.")
@click.option("--bigint-growth-steps", default=1200, show_default=True, help="Steps de multiplication BigInt.")
@click.option("--bigint-divmod-steps", default=2000, show_default=True, help="Iterations div/mod BigInt.")
@click.option("--fast", is_flag=True, help="Preset rapide: iterations=6, warmup=2, divmod-steps<=800.")
def main(
    iterations: int,
    warmup: int,
    factorial_n: int,
    bigint_growth_steps: int,
    bigint_divmod_steps: int,
    fast: bool,
) -> None:
    """Benchmark VM optimizations et hot paths BigInt (Catnip vs Python)."""
    if fast:
        iterations = 6
        warmup = 2
        bigint_divmod_steps = min(bigint_divmod_steps, 800)

    click.echo("=" * 78)
    click.echo("Catnip VM Optimizations Benchmark (v2) - Fevrier 2026")
    click.echo("=" * 78)
    click.echo(f"iterations={iterations}, warmup={warmup}, vm_mode=on")
    click.echo("scenarios: for-range, tail->loop, bigint-growth, bigint-divmod")

    scenarios = [
        scenario_for_range(iterations, warmup),
        scenario_tail_factorial(iterations, warmup, factorial_n),
        scenario_bigint_growth(iterations, warmup, bigint_growth_steps),
        scenario_bigint_divmod(iterations, warmup, bigint_growth_steps, bigint_divmod_steps),
    ]

    click.echo("\nValidation des resultats:")
    for s in scenarios:
        ok = s.catnip_result == s.python_result
        click.echo(f"  {s.name:<50} {'OK' if ok else 'MISMATCH'}")
        if not ok:
            raise click.ClickException(f"Mismatch on scenario: {s.name}")

    click.echo("\nDetails par scenario:")
    for s in scenarios:
        print_stats_pair(s.name, s.catnip_stats, s.python_stats)

    click.echo("\n" + "=" * 78)
    click.echo("Résumé (ratio sur median_ms)")
    click.echo("=" * 78)
    for s in scenarios:
        ratio = s.catnip_stats.median_ms / s.python_stats.median_ms if s.python_stats.median_ms > 0 else float("inf")
        click.echo(f"  {s.name:<50} {ratio:>8.2f}x")

    click.echo("\nLecture rapide:")
    click.echo("  - ForRangeInt et tail->loop représentent les optimisations de contrôle de flux.")
    click.echo("  - BigInt growth/divmod représentent les chemins arithmétiques BigInt de la VM.")
    click.echo("  - Si les ratios BigInt montent avec la taille, le coût est surtout algorithmique (BigInt).")


if __name__ == "__main__":
    main()
