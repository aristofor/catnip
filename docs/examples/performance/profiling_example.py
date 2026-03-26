#!/usr/bin/env python3
"""
Profiling minimal d'un workload Catnip avec cProfile.

Objectif:
- donner un point de départ concret pour le futur guide de performance
- profiler le chemin Python -> parse -> execute
- sortir un top cumulé lisible sans dépendance externe
"""

from __future__ import annotations

import cProfile
import io
import pstats

import click

from catnip import Catnip

EXPECTED_RESULT = 28008000

CODE = """
sum_to = (n, acc=0) => {
    if n <= 0 { acc }
    else { sum_to(n - 1, acc + n) }
}

mix = (n) => {
    total = 0
    for i in range(1, n + 1) {
        total = total + (i * 3) - (i // 2)
    }
    total + sum_to(n)
}

mix(4000)
"""


@click.command()
@click.option("-r", "--repeat", default=12, show_default=True, help="Cycles parse+execute à profiler.")
@click.option("-t", "--top", default=20, show_default=True, help="Nombre de fonctions à afficher.")
def main(repeat: int, top: int) -> None:
    """Profile un workload représentatif Catnip (parse + execute)."""
    profiler = cProfile.Profile()

    def workload() -> None:
        for _ in range(repeat):
            cat = Catnip(vm_mode="on", optimize=1)
            cat.parse(CODE)
            result = cat.execute()
            if result != EXPECTED_RESULT:
                raise click.ClickException(f"unexpected result: {result}")

    profiler.enable()
    workload()
    profiler.disable()

    stream = io.StringIO()
    stats = pstats.Stats(profiler, stream=stream).sort_stats("cumtime")
    stats.print_stats(top)

    click.echo("Catnip profiling example")
    click.echo("=" * 80)
    click.echo(f"repeat={repeat}, optimize=1, vm_mode=on")
    click.echo("workload=parse + execute d'un mix boucle + récursion terminale")
    click.echo("\nTop cumulative time")
    click.echo("-" * 80)
    click.echo(stream.getvalue())


if __name__ == "__main__":
    main()
