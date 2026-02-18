# Performance

Exemples d'optimisation Catnip : mode VM, compilation JIT et cache.

## VM Mode

Le mode VM compile le code en bytecode et l'exécute sur une VM, avec jusqu'à **192x** de speedup vs mode AST.

- [`vm_mode_benchmark.py`](vm_mode_benchmark.py) - benchmark VM vs AST modes

```bash
python docs/examples/performance/vm_mode_benchmark.py
```

Utilisation en CLI :

```bash
catnip --vm on script.cat  # ~190x plus rapide sur boucles
```

## JIT Compilation

Le JIT trace-based utilise Cranelift pour compiler les **hot loops** et **hot functions** en code natif x86-64.

**Support** :

- **Loops** : ints, floats, bools, loops imbriquées, branches conditionnelles
  - Speedup typique : 100-200x sur boucles numériques intensives
- **Functions** : fonctions récursives et non récursives appelées souvent
  - Speedup typique : 1.1x sur fonctions simples (overhead boxing/unboxing)
  - Support récursion : appels récursifs compilés via CallSelf avec re-boxing NaN automatique
  - Protection overflow : depth counter avec MAX_RECURSION_DEPTH = 10000
  - Fallback gracieux vers interpréteur si profondeur > 10000 (évite crashes)
  - **Memoization automatique (Phase 4.3)** : cache thread_local pour fonctions à 1 paramètre
    - Fibonacci : O(2^n) → O(n) - gain exponentiel
    - Factorial : pas d'amélioration (pas de sous-problèmes qui se chevauchent)
  - Meilleur speedup pour fonctions qui contiennent des loops (loop compilée séparément)

**Exemple** :

- [`jit_benchmark.py`](jit_benchmark.py) - benchmark JIT loops + functions vs interpreter

```bash
python docs/examples/performance/jit_benchmark.py
```

**Utilisation en CLI** :

```bash
catnip -o jit script.cat  # Active le JIT (loops + functions)
```

## Optimisations

Benchmarks d'analyse et comparaison des différents niveaux et types d'optimisation.

**Tail-Call Optimization** :

- [`tail_recursion_benchmark.py`](tail_recursion_benchmark.py) - impact TCO et transformation tail→loop
- [`vm_optimizations_benchmark.py`](vm_optimizations_benchmark.py) - ForRangeInt et TailRecursionToLoopPass

```bash
python docs/examples/performance/tail_recursion_benchmark.py
python docs/examples/performance/vm_optimizations_benchmark.py
```

**Pipeline Comparison** :

- [`pipeline_comparison_benchmark.py`](pipeline_comparison_benchmark.py) - comparaison niveaux d'optimisation et modes d'exécution

```bash
python docs/examples/performance/pipeline_comparison_benchmark.py
```

Résultats typiques :

- VM vs AST : 6-19x speedup (moyenne 6x)
- Niveaux d'optimisation : impact 1-4% (overhead négligeable)

**Control Flow Graph (CFG)** :

- [`cfg_benchmark.py`](cfg_benchmark.py) - gains des optimisations CFG structurelles

```bash
python docs/examples/performance/cfg_benchmark.py
```

Gains observés :

- Empty block removal : 20-36% réduction blocs
- Loop detection : analyse pour optimisations futures
- Simplification structurelle du flot de contrôle

## Cache

- [`cache_demo.py`](cache_demo.py) - tour rapide du cache
- [`cache_memory_example.py`](cache_memory_example.py) - backend memoire
- [`cache_disk_example.py`](cache_disk_example.py) - backend disque
- [`cache_redis_example.py`](cache_redis_example.py) - backend Redis
- [`cache_comparison.py`](cache_comparison.py) - comparaison des backends
- [`memoization_build_script.py`](memoization_build_script.py) - build avec cache
- [`memoization_dependencies.py`](memoization_dependencies.py) - dependances cache
- [`memoization_python_hooks.py`](memoization_python_hooks.py) - hooks Python

## Prerequis

- Python avec Catnip installé en editable
- Redis requis pour `cache_redis_example.py`

## Lancer

```bash
python docs/examples/performance/cache_demo.py
python docs/examples/performance/cache_memory_example.py
python docs/examples/performance/cache_disk_example.py
python docs/examples/performance/cache_redis_example.py
python docs/examples/performance/cache_comparison.py
```
