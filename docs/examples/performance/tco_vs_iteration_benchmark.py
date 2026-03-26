#!/usr/bin/env python3
"""
Benchmark Catnip: iteration vs tail recursion.

Objectif:
- comparer une boucle impérative et une récursion terminale équivalente
- mesurer l'impact de `tco` sur la version récursive
- garder un script lisible, utile pour le futur guide de performance
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


@dataclass
class RunResult:
    label: str
    stats: BenchStats | None
    result: Any = None
    error: str | None = None


def summarize(samples_ms: list[float]) -> BenchStats:
    return BenchStats(
        min_ms=min(samples_ms),
        median_ms=statistics.median(samples_ms),
        mean_ms=statistics.fmean(samples_ms),
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


def make_loop_code(n: int) -> str:
    return f"""
total = 0
for i in range(1, {n} + 1) {{
    total = total + i
}}
total
"""


def make_tail_code(n: int) -> str:
    return f"""
sum_to = (n, acc=0) => {{
    if n <= 0 {{ acc }}
    else {{ sum_to(n - 1, acc + n) }}
}}
sum_to({n})
"""


def make_python_loop(n: int) -> Callable[[], int]:
    def run() -> int:
        total = 0
        for i in range(1, n + 1):
            total += i
        return total

    return run


def run_catnip_case(
    *,
    label: str,
    code: str,
    optimize: int,
    iterations: int,
    warmup: int,
    tco_enabled: bool | None,
) -> RunResult:
    try:
        cat = Catnip(vm_mode="on", optimize=optimize)
        cat.parse(code)
        if tco_enabled is not None:
            cat.pragma_context.tco_enabled = tco_enabled
        stats, result = benchmark_callable(cat.execute, iterations=iterations, warmup=warmup)
        return RunResult(label=label, stats=stats, result=result)
    except Exception as exc:  # noqa: BLE001
        return RunResult(label=label, stats=None, error=f"{type(exc).__name__}: {exc}")


def print_result(run: RunResult, baseline_ms: float | None) -> None:
    if run.stats is None:
        click.echo(f"{run.label:<32} ERROR  {run.error}")
        return

    ratio = ""
    if baseline_ms is not None and baseline_ms > 0:
        ratio = f"  {run.stats.median_ms / baseline_ms:>6.2f}x vs loop"

    click.echo(
        f"{run.label:<32} "
        f"min={run.stats.min_ms:>8.3f} ms  "
        f"median={run.stats.median_ms:>8.3f} ms  "
        f"mean={run.stats.mean_ms:>8.3f} ms{ratio}"
    )


@click.command()
@click.option("-n", "--workload-n", "n", default=20000, show_default=True, help="Borne superieure pour sum(1..n).")
@click.option("-i", "--iterations", default=10, show_default=True, help="Iterations de mesure.")
@click.option("-w", "--warmup", default=3, show_default=True, help="Iterations de chauffe.")
@click.option("-o", "--optimize", default=1, show_default=True, help="Niveau d'optimisation Catnip.")
@click.option("--fast", is_flag=True, help="Preset rapide: n<=5000, iterations=6, warmup=2.")
def main(n: int, iterations: int, warmup: int, optimize: int, fast: bool) -> None:
    """Compare boucle impérative vs tail recursion avec TCO on/off."""
    if fast:
        n = min(n, 5000)
        iterations = 6
        warmup = 2

    loop_code = make_loop_code(n)
    tail_code = make_tail_code(n)

    click.echo("Catnip performance: TCO vs iteration")
    click.echo("=" * 80)
    click.echo(
        f"workload=sum(1..{n}), vm_mode=on, optimize={optimize}, "
        f"iterations={iterations}, warmup={warmup}"
    )

    py_loop_stats, py_loop_result = benchmark_callable(make_python_loop(n), iterations, warmup)
    cat_loop = run_catnip_case(
        label="Catnip loop",
        code=loop_code,
        optimize=optimize,
        iterations=iterations,
        warmup=warmup,
        tco_enabled=None,
    )
    cat_tail_on = run_catnip_case(
        label="Catnip tail recursion (TCO on)",
        code=tail_code,
        optimize=optimize,
        iterations=iterations,
        warmup=warmup,
        tco_enabled=True,
    )
    cat_tail_off = run_catnip_case(
        label="Catnip tail recursion (TCO off)",
        code=tail_code,
        optimize=optimize,
        iterations=iterations,
        warmup=warmup,
        tco_enabled=False,
    )

    expected = py_loop_result
    for run in (cat_loop, cat_tail_on):
        if run.error is None and run.result != expected:
            raise click.ClickException(f"{run.label} returned {run.result!r}, expected {expected!r}")
    if cat_tail_off.error is None and cat_tail_off.result != expected:
        raise click.ClickException(f"{cat_tail_off.label} returned {cat_tail_off.result!r}, expected {expected!r}")

    click.echo("\nResults")
    click.echo("-" * 80)
    click.echo(
        f"{'Python loop':<32} "
        f"min={py_loop_stats.min_ms:>8.3f} ms  "
        f"median={py_loop_stats.median_ms:>8.3f} ms  "
        f"mean={py_loop_stats.mean_ms:>8.3f} ms"
    )
    loop_baseline = cat_loop.stats.median_ms if cat_loop.stats is not None else None
    print_result(cat_loop, baseline_ms=loop_baseline)
    print_result(cat_tail_on, baseline_ms=loop_baseline)
    print_result(cat_tail_off, baseline_ms=loop_baseline)

    click.echo("\nInterpretation")
    click.echo("-" * 80)
    click.echo("- Compare loop and tail recursion at equal result first, timings second.")
    click.echo("- TCO on should matter more as recursion depth grows.")
    click.echo("- The loop often remains the best baseline for simple linear workloads.")
    click.echo("- TCO off is included mainly to expose the extra call overhead or failure mode.")


if __name__ == "__main__":
    main()
