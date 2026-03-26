# Performance

Guide pour mesurer les performances de Catnip.

## Hot paths VM

- [`vm_optimizations_benchmark.py`](vm_optimizations_benchmark.py)

Script de référence. Benchmarke des chemins VM d'actualité et compare Catnip VM à Python natif :

- `ForRangeInt` (boucle numérique range)
- transformation tail recursion -> loop
- arithmétique BigInt (croissance et div/mod)

```bash
python docs/examples/performance/vm_optimizations_benchmark.py
python docs/examples/performance/vm_optimizations_benchmark.py --fast
python docs/examples/performance/vm_optimizations_benchmark.py -n 20 -w 5
python docs/examples/performance/vm_optimizations_benchmark.py --bigint-growth-steps 2000 --bigint-divmod-steps 4000
```

## Niveaux d'optimisation

- [`pipeline_comparison_benchmark.py`](pipeline_comparison_benchmark.py)

Compare `optimize=0..3` sur des charges avec redondances calculatoires (CSE, constant folding, LICM). Filtrable par test
et par niveau.

```bash
python docs/examples/performance/pipeline_comparison_benchmark.py
python docs/examples/performance/pipeline_comparison_benchmark.py -n 20 -l 0,3
python docs/examples/performance/pipeline_comparison_benchmark.py -t loop -t recursion
python docs/examples/performance/pipeline_comparison_benchmark.py -l 0-2
```

## TCO vs itération

- [`tco_vs_iteration_benchmark.py`](tco_vs_iteration_benchmark.py)

Compare trois approches équivalentes sur `sum(1..n)` :

- boucle impérative
- récursion terminale avec `tco:on`
- récursion terminale avec `tco:off`

```bash
python docs/examples/performance/tco_vs_iteration_benchmark.py
python docs/examples/performance/tco_vs_iteration_benchmark.py -n 50000 -i 12
python docs/examples/performance/tco_vs_iteration_benchmark.py --fast
```

## Profiling

- [`profiling_example.py`](profiling_example.py)

Point de départ minimal pour profiler un chemin Catnip représentatif avec `cProfile`. Mesure `parse + execute` sur une
charge mixte boucle + récursion terminale.

```bash
python docs/examples/performance/profiling_example.py
python docs/examples/performance/profiling_example.py -r 20 -t 25
```

## Règles de lecture

- séparer warmup et mesure
- vérifier l'égalité des résultats avant de commenter les timings
- signaler la configuration utilisée : `vm_mode`, niveau `optimize`, `tco` active ou non, cache actif ou non
- éviter d'extrapoler depuis des micro-benchmarks vers des workloads réels
