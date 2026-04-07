# Benchmarking

Guide simple pour mesurer et comparer les performances des différentes configurations Catnip.

## Méthodologie Générale

### Principe de base

Un benchmark valide nécessite :

1. **Warmup** - Éliminer l'impact du premier run (3-5 itérations)
1. **Itérations multiples** - Calculer moyenne et écart-type (10-100 runs)
1. **Isolation** - Un seul paramètre change entre les mesures
1. **Reproductibilité** - Configuration explicite, seed fixe si aléatoire
1. **Granularité** - Séparer parsing, compilation, exécution

### Types de mesures

**Performance d'exécution** :

```python
import time
from catnip import Catnip

code = """
sum = 0
for i in range(1, 10000) {
    sum = sum + i
}
sum
"""

cat = Catnip(vm_mode='on')
cat.parse(code)

# Warmup
for _ in range(3):
    cat.execute()

# Benchmark
times = []
for _ in range(10):
    start = time.perf_counter()
    result = cat.execute()
    elapsed = (time.perf_counter() - start) * 1000  # ms
    times.append(elapsed)

avg = sum(times) / len(times)
std = (sum((t - avg)**2 for t in times) / len(times))**0.5

print(f"Execution: {avg:.2f}ms ± {std:.2f}ms")
```

**Performance de compilation** (parsing + semantic) :

```python
import time
from catnip import Catnip

code = "..." # code à benchmarker

times = []
for _ in range(10):
    cat = Catnip(vm_mode='on')
    start = time.perf_counter()
    cat.parse(code)
    elapsed = (time.perf_counter() - start) * 1000
    times.append(elapsed)

avg = sum(times) / len(times)
print(f"Compilation: {avg:.2f}ms")
```

**Performance end-to-end** :

```python
import time
from catnip import Catnip

code = "..."

times = []
for _ in range(10):
    cat = Catnip(vm_mode='on')
    start = time.perf_counter()
    result = cat(code)  # parse + execute
    elapsed = (time.perf_counter() - start) * 1000
    times.append(elapsed)

avg = sum(times) / len(times)
print(f"End-to-end: {avg:.2f}ms")
```

## Comparaison de Configurations

### VM vs AST

Comparer les deux modes d'exécution :

```python
import time
from catnip import Catnip

code = """
factorial = (n, acc=1) => {
    if n <= 1 { acc }
    else { factorial(n - 1, n * acc) }
}
factorial(100)
"""

# VM mode
cat_vm = Catnip(vm_mode='on')
cat_vm.parse(code)
# ... benchmark execution

# AST mode
cat_ast = Catnip(vm_mode='off')
cat_ast.parse(code)
# ... benchmark execution

print(f"VM:  {avg_vm:.2f}ms")
print(f"AST: {avg_ast:.2f}ms")
print(f"Speedup: {avg_ast/avg_vm:.2f}x")
```

### Niveaux d'optimisation

Comparer différents niveaux via `optimize` :

```python
from catnip import Catnip

code = "..."

for opt_level in [0, 1, 2, 3]:
    cat = Catnip(optimize=opt_level)
    cat.parse(code)
    # ... benchmark execution
    print(f"Opt level {opt_level}: {avg:.2f}ms")
```

Les niveaux d'optimisation contrôlent quelles passes sont actives :

- `0` : Aucune optimisation
- `1` : Optimisations basiques (constant folding, strength reduction)
- `2` : Optimisations standard (+ CSE, dead code elimination)
- `3` : Optimisations agressives (toutes les passes)

### JIT on/off

Comparer l'impact du JIT sur les boucles chaudes :

```python
from catnip import Catnip

code = """
sum = 0
for i in range(1, 1000000) {
    sum = sum + i
}
sum
"""

# JIT activé
cat_jit = Catnip(vm_mode='on', jit=True)
cat_jit.parse(code)
# ... benchmark

# JIT désactivé
cat_no_jit = Catnip(vm_mode='on', jit=False)
cat_no_jit.parse(code)
# ... benchmark

print(f"JIT on:  {avg_jit:.2f}ms")
print(f"JIT off: {avg_no_jit:.2f}ms")
print(f"Speedup: {avg_no_jit/avg_jit:.2f}x")
```

### TCO on/off

Comparer tail-call optimization :

```python
from catnip import Catnip

code = """
factorial = (n, acc=1) => {
    if n <= 1 { acc }
    else { factorial(n - 1, n * acc) }
}
factorial(10000)
"""

# TCO activé (défaut)
cat_tco = Catnip(tco=True)
cat_tco.parse(code)
# ... benchmark

# TCO désactivé
cat_no_tco = Catnip(tco=False)
cat_no_tco.parse(code)
# ... benchmark
```

## Mesure de l'Impact des Optimisations

### Réduction du bytecode

Mesurer la taille du bytecode avant/après optimisations :

```python
from catnip import Catnip
import catnip._rs as rs

code = """
sum = 0
for i in range(1, 100) {
    sum = sum + i
}
sum
"""

# Sans optimisations
cat_no_opt = Catnip(optimize=0, vm_mode='on')
ast_no_opt = cat_no_opt.parse(code, semantic=False)
bc_no_opt = rs.compile_to_bytecode(ast_no_opt)

# Avec optimisations
cat_opt = Catnip(optimize=3, vm_mode='on')
ast_opt = cat_opt.parse(code, semantic=False)
bc_opt = rs.compile_to_bytecode(ast_opt)

print(f"Bytecode sans opt: {len(bc_no_opt)} instructions")
print(f"Bytecode avec opt: {len(bc_opt)} instructions")
print(f"Réduction: {(1 - len(bc_opt)/len(bc_no_opt))*100:.1f}%")
```

### Comptage des passes appliquées

Activer le mode verbose pour voir les passes :

```python
from catnip import Catnip

code = "..."

cat = Catnip(optimize=3, vm_mode='on')

# Parser avec verbose pour voir les passes
import sys
from io import StringIO

# Capturer stdout
old_stdout = sys.stdout
sys.stdout = buffer = StringIO()

cat.parse(code, verbose=True)

output = buffer.getvalue()
sys.stdout = old_stdout

# Analyser les passes appliquées
passes = [line for line in output.split('\n') if 'Pass:' in line]
print(f"Passes appliquées: {len(passes)}")
for p in passes:
    print(f"  - {p}")
```

### Réduction du nombre de nœuds

Compter les nœuds IR avant/après semantic :

```python
from catnip import Catnip

code = """
x = 2 + 3
y = x * 2
z = y + 1
z
"""

cat = Catnip()

# Avant semantic (IR seulement)
ir = cat.parse(code, semantic=False)

def count_nodes(node):
    """Compte récursivement les nœuds."""
    count = 1
    if hasattr(node, 'args'):
        for arg in node.args:
            if hasattr(arg, 'ident'):  # C'est un Op/IR
                count += count_nodes(arg)
    return count

nodes_before = count_nodes(ir)

# Après semantic
optimized = cat.parse(code, semantic=True)
nodes_after = count_nodes(optimized)

print(f"Nœuds avant: {nodes_before}")
print(f"Nœuds après: {nodes_after}")
print(f"Réduction: {(1 - nodes_after/nodes_before)*100:.1f}%")
```

## Profiling du Pipeline

### Profiling Python standard

Utiliser `cProfile` pour profiler l'ensemble :

```python
import cProfile
import pstats
from io import StringIO
from catnip import Catnip

code = "..."

profiler = cProfile.Profile()
profiler.enable()

cat = Catnip(vm_mode='on', optimize=3)
for _ in range(100):
    result = cat(code)

profiler.disable()

# Afficher les statistiques
s = StringIO()
ps = pstats.Stats(profiler, stream=s).sort_stats('cumtime')
ps.print_stats(20)  # Top 20 fonctions
print(s.getvalue())
```

### Profiling par étape

Mesurer chaque étape du pipeline :

```python
import time
from catnip import Catnip

code = "..."

cat = Catnip(vm_mode='on', optimize=3)

# 1. Parsing
start = time.perf_counter()
ast = cat.parser.parse(code)
parse_time = (time.perf_counter() - start) * 1000

# 2. Transformation
start = time.perf_counter()
ir = cat.transformer.transform(ast)
transform_time = (time.perf_counter() - start) * 1000

# 3. Semantic analysis
start = time.perf_counter()
optimized = cat.semantic.analyze(ir)
semantic_time = (time.perf_counter() - start) * 1000

# 4. Compilation bytecode
start = time.perf_counter()
bytecode = cat.compile(optimized)
compile_time = (time.perf_counter() - start) * 1000

# 5. Exécution
start = time.perf_counter()
result = cat.execute()
exec_time = (time.perf_counter() - start) * 1000

total = parse_time + transform_time + semantic_time + compile_time + exec_time

print(f"Parsing:    {parse_time:.2f}ms ({parse_time/total*100:.1f}%)")
print(f"Transform:  {transform_time:.2f}ms ({transform_time/total*100:.1f}%)")
print(f"Semantic:   {semantic_time:.2f}ms ({semantic_time/total*100:.1f}%)")
print(f"Compile:    {compile_time:.2f}ms ({compile_time/total*100:.1f}%)")
print(f"Execute:    {exec_time:.2f}ms ({exec_time/total*100:.1f}%)")
print(f"Total:      {total:.2f}ms")
```

## Comparaison avec Python Natif

### Méthodologie équitable

Comparer des implémentations équivalentes :

```python
import time
from catnip import Catnip

# Catnip
code_catnip = """
sum = 0
for i in range(1, 100001) {
    sum = sum + i
}
sum
"""

cat = Catnip(vm_mode='on')
cat.parse(code_catnip)

# Warmup
for _ in range(3):
    cat.execute()

times_catnip = []
for _ in range(10):
    start = time.perf_counter()
    result_catnip = cat.execute()
    elapsed = (time.perf_counter() - start) * 1000
    times_catnip.append(elapsed)

avg_catnip = sum(times_catnip) / len(times_catnip)

# Python équivalent
def python_sum():
    total = 0
    for i in range(1, 100001):
        total = total + i
    return total

# Warmup
for _ in range(3):
    python_sum()

times_python = []
for _ in range(10):
    start = time.perf_counter()
    result_python = python_sum()
    elapsed = (time.perf_counter() - start) * 1000
    times_python.append(elapsed)

avg_python = sum(times_python) / len(times_python)

# Vérifier résultats identiques
assert result_catnip == result_python

print(f"Catnip: {avg_catnip:.2f}ms")
print(f"Python: {avg_python:.2f}ms")
overhead = ((avg_catnip - avg_python) / avg_python) * 100
print(f"Overhead: {overhead:+.1f}%")
```

**Important** : comparer des algorithmes équivalents, pas des APIs différentes. Par exemple, une fonction tail-recursive
transformée en loop doit être comparée à un `while` Python, pas à une fonction récursive Python.

## Benchmarks Reproductibles

### Template de benchmark

Structure recommandée pour un benchmark :

```python
#!/usr/bin/env python3
"""
Titre du Benchmark

Description de ce qui est mesuré et pourquoi.
"""

import time
import statistics
from catnip import Catnip

def benchmark_execution(cat_instance, iterations=10, warmup=3):
    """
    Benchmark l'exécution d'une instance Catnip déjà parsée.

    Returns:
        (moyenne_ms, ecart_type_ms, résultat)
    """
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
    print("=" * 70)
    print("BENCHMARK: [Nom]")
    print("=" * 70)

    # Configuration
    code = """
    # Code à benchmarker
    """

    # Test 1: Configuration A
    print("\n1. Configuration A")
    print("-" * 70)
    cat_a = Catnip(vm_mode='on', optimize=3)
    cat_a.parse(code)
    avg_a, std_a, result_a = benchmark_execution(cat_a)
    print(f"Temps: {avg_a:.2f}ms ± {std_a:.2f}ms")
    print(f"Résultat: {result_a}")

    # Test 2: Configuration B
    print("\n2. Configuration B")
    print("-" * 70)
    cat_b = Catnip(vm_mode='off', optimize=3)
    cat_b.parse(code)
    avg_b, std_b, result_b = benchmark_execution(cat_b)
    print(f"Temps: {avg_b:.2f}ms ± {std_b:.2f}ms")
    print(f"Résultat: {result_b}")

    # Comparaison
    print("\n" + "=" * 70)
    print("RÉSULTATS")
    print("=" * 70)

    assert result_a == result_b, "Résultats différents!"

    speedup = avg_b / avg_a
    print(f"\nConfig A: {avg_a:.2f}ms ± {std_a:.2f}ms")
    print(f"Config B: {avg_b:.2f}ms ± {std_b:.2f}ms")
    print(f"Speedup: {speedup:.2f}x")

    if speedup > 1.5:
        print("✓ Amélioration significative")
    elif speedup > 1.1:
        print("✓ Amélioration notable")
    elif speedup > 0.9:
        print("≈ Performance équivalente")
    else:
        print("▲  Régression de performance")

if __name__ == "__main__":
    main()
```

### Facteurs à contrôler

Pour des benchmarks reproductibles :

1. **Version Python** : `python --version`
1. **Version Catnip** : `catnip --version`
1. **Configuration système** : CPU, RAM, OS
1. **Charge CPU** : Éviter autres processus lourds
1. **Mode power** : Performance mode (pas économie d'énergie)
1. **Seed aléatoire** : Si code utilise `random`, fixer seed

Exemple de header de benchmark :

<!-- doc-snapshot: bench/header -->

```python
"""
Benchmark: VM vs AST Performance

Configuration:
- Python: 3.12.1
- Catnip: 2.0.0
- CPU: AMD Ryzen 9 5900X
- RAM: 32GB DDR4
- OS: Ubuntu 24.04
- Date: 2026-01-27
"""
```

## Analyse Statistique

### Moyenne et écart-type

Toujours rapporter les deux :

```python
import statistics

times = [...]  # Liste de mesures

avg = statistics.mean(times)
std = statistics.stdev(times)
print(f"Résultat: {avg:.2f}ms ± {std:.2f}ms")
```

### Détection d'outliers

Éliminer les valeurs aberrantes :

```python
import statistics

def remove_outliers(data, threshold=2.0):
    """
    Retire les valeurs à plus de threshold écart-types.
    """
    if len(data) < 3:
        return data

    mean = statistics.mean(data)
    std = statistics.stdev(data)

    return [x for x in data if abs(x - mean) <= threshold * std]

times = [...]
cleaned = remove_outliers(times)
avg = statistics.mean(cleaned)
```

### Intervalle de confiance

Calculer intervalle de confiance à 95% :

```python
import statistics
import math

def confidence_interval_95(data):
    """
    Retourne (moyenne, marge_erreur) pour IC à 95%.
    """
    n = len(data)
    mean = statistics.mean(data)
    std = statistics.stdev(data)

    # Approximation t-distribution (t ≈ 1.96 pour n > 30)
    t_value = 1.96
    margin = t_value * (std / math.sqrt(n))

    return mean, margin

avg, margin = confidence_interval_95(times)
print(f"Résultat: {avg:.2f}ms ± {margin:.2f}ms (IC 95%)")
```

## Comparaison de Pipelines

### Scénario : IR passes vs CFG optimizations

Hypothèse : comparer pipeline actuel (6 passes IR) vs futur pipeline (CFG).

```python
import time
from catnip import Catnip
import catnip._rs as rs

code = """
x = 2 + 3
if True {
    y = x * 2
} else {
    y = 0
}
z = y + 1
z
"""

# Pipeline actuel (IR passes)
cat_ir = Catnip(optimize=3, vm_mode='on')
start = time.perf_counter()
result_ir = cat_ir(code)
time_ir = (time.perf_counter() - start) * 1000

# Pipeline CFG (intégré quand optimize>=3; analyse manuelle via catnip._rs.cfg)
cat_cfg = Catnip(optimize=0, vm_mode='on')  # Désactiver IR passes
ast = cat_cfg.parse(code, semantic=False)

start = time.perf_counter()

# Construire CFG
cfg = rs.cfg.build_cfg_from_ir(ast, 'test')

# Optimisations CFG
cfg.compute_dominators()
stats = cfg.optimize()
dead, merged, empty, branches = stats

# Reconstruction (intégrée via optimize=3; non exposée dans l'API cfg)
# optimized = cfg.reconstruct()
# time_cfg = ...

print(f"Pipeline IR:  {time_ir:.2f}ms")
print(f"IR passes:    6 (BluntCode, ConstFold, StrengthRed, BlockFlat, DeadCode, CSE)")

print(f"\nPipeline CFG: [Non intégré]")
print(f"CFG passes:   4 (dead code, merge blocks, empty blocks, const branches)")
print(f"CFG stats:    dead={dead}, merged={merged}, empty={empty}, branches={branches}")
print(f"Blocs:        {cfg.num_blocks}")
print(f"Edges:        {cfg.num_edges}")
```

### Scénario : Impact de chaque passe

Mesurer l'impact individuel de chaque passe :

```python
from catnip import Catnip

code = """
x = 1 + 2
y = x * 3
z = y + 4
z
"""

# Baseline : aucune optimisation
cat_base = Catnip(optimize=0)
cat_base.parse(code)
# ... benchmark
print(f"Aucune opt:      {avg_base:.2f}ms")

# + Constant folding
cat_cf = Catnip(optimize=1)  # Active CF
cat_cf.parse(code)
# ... benchmark
print(f"+ ConstFolding:  {avg_cf:.2f}ms (gain: {avg_base/avg_cf:.2f}x)")

# + Strength reduction
cat_sr = Catnip(optimize=2)  # Active CF + SR
cat_sr.parse(code)
# ... benchmark
print(f"+ StrengthRed:   {avg_sr:.2f}ms (gain: {avg_base/avg_sr:.2f}x)")

# Toutes les passes
cat_all = Catnip(optimize=3)
cat_all.parse(code)
# ... benchmark
print(f"Toutes passes:   {avg_all:.2f}ms (gain: {avg_base/avg_all:.2f}x)")
```

## Exemples Complets

Les benchmarks suivants ont été déplacés (besoin de les réécrire pour refléter les conditions de prod)

## Recommandations

### Pour comparer 2 pipelines

1. **Isoler la variable** : Un seul paramètre change (ex: `optimize=2` vs `optimize=3`)
1. **Warmup suffisant** : 3-5 itérations avant mesure
1. **Itérations multiples** : 10+ runs, rapporter moyenne ± écart-type
1. **Vérifier résultats** : `assert result_a == result_b` pour garantir sémantique identique
1. **Contexte complet** : Versions, hardware, configuration
1. **Benchmark varié** : Tester plusieurs types de code (loops, recursion, pattern matching)

### Pour mesurer impact optimisations

1. **Baseline clair** : Toujours mesurer sans optimisations (`optimize=0`)
1. **Passes progressives** : Activer une passe à la fois pour isoler impact
1. **Métriques multiples** :
   - Temps exécution (user-facing)
   - Taille bytecode (compilation overhead)
   - Nombre nœuds IR (complexité AST)
   - Nombre passes appliquées (overhead semantic)
1. **Code représentatif** : Patterns réels, pas juste micro-benchmarks

### Pour comparer avec Python

1. **Équivalence sémantique** : Même algorithme, pas mêmes APIs
1. **Warmup Python aussi** : JIT Python (si CPython 3.11+) nécessite warmup
1. **Mesures identiques** : Même timer, même nombre d'itérations
1. **Rapporter overhead** : Pourcentage ou ratio, pas juste temps absolu
1. **Contexte réaliste** : Python excelle sur certains patterns (comprehensions, builtins)

> La comparaison de pipelines n'a de sens que si on compare des pipelines qui font la même chose. Mesurer qu'un pipeline
> est "plus rapide" sans vérifier qu'il produit le même résultat, c'est comme mesurer qu'une voiture va plus vite en
> retirant les freins. Techniquement correct, mais l'utilisateur ne validera pas le benchmark.

## Outils Externes

### hyperfine

Pour benchmarks CLI automatisés :

```bash
# Installer hyperfine
sudo apt install hyperfine  # Ubuntu/Debian
brew install hyperfine       # macOS

# Comparer deux configs
hyperfine \
  --warmup 3 \
  --runs 10 \
  "catnip -o 'optimize:2' script.cat" \
  "catnip -o 'optimize:3' script.cat"
```

### py-spy

Pour profiling en production :

```bash
# Installer py-spy
pip install py-spy

# Profiler un script Catnip
py-spy record -o profile.svg -- catnip script.cat

# Ouvrir profile.svg dans navigateur
```

### memory_profiler

Pour mesurer consommation mémoire :

```bash
# Installer memory_profiler
pip install memory_profiler

# Profiler
python -m memory_profiler benchmark.py
```

## Références

- **Optimisations** : `docs/dev/OPTIMIZATIONS.md`
- **CFG** : `docs/dev/ARCHITECTURE.md` (section CFG/SSA)
- **VM Architecture** : `docs/dev/VM.md`
