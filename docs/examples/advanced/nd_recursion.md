# ND-Recursion

Exemples d'utilisation de la ND-récursion (`~~`, `~>`, `~[]`).

> La ND-récursion permet d'exprimer des computations récursives qui peuvent être exécutées en parallèle sans syntaxe
> `async`/`await`. Le runtime choisit le mode d'exécution.

## `~~` - ND-Recursion

### Forme combinateur

La forme combinateur applique la lambda avec une seed initiale:

```catnip
# Countdown récursif
~~(5, (v, recur) => {
    if v > 0 {
        recur(v - 1)
    } else {
        v
    }
})
# → 0
```

**Explication**: La lambda reçoit `(v, recur)` où:

- `v` = valeur courante
- `recur` = fonction pour continuer la récursion

### Factorielle

```catnip
~~(5, (n, recur) => {
    if n <= 1 { 1 }
    else { n * recur(n - 1) }
})
# → 120
```

### Forme déclaration

Créer une fonction ND-récursive réutilisable:

<!-- check: no-check -->

```catnip
countdown = ~~ (n, recur) => {
    if n > 0 { recur(n - 1) }
    else { "done" }
}

countdown(10)
# → "done"
```

> À ce stade, la lambda wrappée s'exécute comme une fonction normale. Une fois appelée avec une seed, elle déclenche la
> récursion.

## `~>` - ND-Map

### Forme lift

Lifter une fonction dans le contexte ND:

```catnip
f = ~> abs
f(-5)
# → 5
```

### Forme applicative

Appliquer une fonction en contexte ND:

```catnip
~>(list(-1, -2, -3), abs)
# → [1, 2, 3]
```

### Broadcast ND-map

Mapper une fonction sur chaque élément:

```catnip
list(-1, -2, 3).[~> abs]
# → [1, 2, 3]
```

Avec lambda:

```catnip
list(1, 2, 3).[~> (x) => { x * 2 }]
# → [2, 4, 6]
```

## `~[]` - Empty Topos

Littéral représentant le topos vide (élément neutre):

```catnip
empty = ~[]
```

**Propriétés**:

```catnip
# Falsy en contexte booléen
if ~[] { 1 } else { 2 }
# → 2

# Égalité
~[] == ~[]
# → True

# Longueur
len(~[])
# → 0
```

> Le topos vide sert d'élément identité pour les opérations ND. Il marque la terminaison dans les graphes de calcul.

## Broadcast ND

Les opérateurs ND fonctionnent avec le broadcast sur collections.

### data.[~> f] - Broadcast ND-map

Applique une fonction à chaque élément en contexte ND:

```catnip
# Map abs sur liste
list(-1, -2, 3).[~> abs]
# → [1, 2, 3]

# Map lambda inline
list(1, 2, 3).[~> (x) => { x * 2 }]
# → [2, 4, 6]

# Préserve le type tuple
tuple(-1, -2, 3).[~> abs]
# → (1, 2, 3)
```

### data.[\~~ lambda] - Broadcast ND-recursion

Applique ND-recursion à chaque élément:

```catnip
# Factorielle sur liste
list(3, 5).[~~ (n, recur) => {
    if n <= 1 { 1 }
    else { n * recur(n - 1) }
}]
# → [6, 120]

# Countdown sur chaque élément
list(3, 5, 2).[~~ (v, recur) => {
    if v > 0 { recur(v - 1) }
    else { v }
}]
# → [0, 0, 0]

# Préserve le type tuple
tuple(3, 2, 4).[~~ (n, recur) => {
    if n <= 1 { 1 }
    else { n * recur(n - 1) }
}]
# → (6, 2, 24)
```

> **Note**: Le broadcast ND préserve automatiquement le type de la collection (list, tuple, set).

## Chaînage

Les opérateurs ND se chaînent naturellement avec le broadcasting standard:

```catnip
data = list(1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
result = data.[if > 3].[* 2].[if < 15].[+ 1]
# > 3: [4,5,6,7,8,9,10]
# * 2: [8,10,12,14,16,18,20]
# < 15: [8,10,12,14]
# + 1: [9,11,13,15]
```

## Mode d'exécution

Par défaut, exécution **séquentielle** via `NDScheduler.execute_sync()`.

### Pragmas ND

Les pragmas permettent de contrôler le mode d'exécution :

<!-- check: no-check -->

```catnip
# Configurer le nombre de workers (0 = auto-detect)
pragma("nd_workers", "8")

# Mode d'exécution : "sequential" (défaut) ou "parallel"
pragma("nd_mode", "parallel")

# Cette computation utilisera 8 workers en parallèle
~~(huge_dataset, (data, recur) => { ... })
```

**Modes disponibles :**

- `"sequential"` (défaut) : exécution séquentielle, pas de overhead de threads
- `"parallel"` : exécution sur N workers via ThreadPoolExecutor

**Nombre de workers :**

- `"0"` : détection automatique (nombre de cœurs CPU)
- `"N"` : nombre explicite de workers

> **Note sur le parallélisme** : Le mode parallèle utilise `ThreadPoolExecutor` (threads Python) par défaut. Le GIL
> limite les gains pour du code CPU-bound pur, mais permet de profiter du parallélisme pour les I/O ou les opérations
> natives.
>
> Pour du vrai parallélisme CPU-bound, utiliser `ProcessPoolExecutor` via `pragma("nd", "process")`. Les lambdas Catnip
> sont picklables (sérializables), donc les workers peuvent les exécuter dans leurs propres processus. Chaque worker
> initialise son registry Catnip automatiquement.

### Memoization

Le pragma `nd_memoize` active un cache automatique des résultats :

```catnip
# Activer la memoization
pragma("nd_memoize", "on")

# Fibonacci avec memoization - 11x speedup sur fib(25)
~~(25, (n, recur) => {
    if n <= 1 { n }
    else { recur(n - 1) + recur(n - 2) }
})
# → 75025

# Désactiver la memoization (défaut)
pragma("nd_memoize", "off")
```

**Principe** : Le scheduler cache les résultats par valeur seed. Si `recur(n)` est rappelé avec la même valeur `n`, le
résultat en cache est retourné au lieu de recalculer.

**Cas d'usage optimal** :

- Algorithmes avec redondance (Fibonacci, DP)
- Calculs coûteux sur mêmes valeurs
- Broadcast sur collections avec valeurs dupliquées

**Performance** :

- Fibonacci sans memoization : O(2^n) appels
- Fibonacci avec memoization : O(n) appels
- Speedup mesuré : 11x sur fib(25), plus élevé pour n plus grand

### Batching (Phase 6)

Le pragma `nd_batch_size` contrôle la granularité du parallélisme en regroupant plusieurs items avant de les soumettre à
ThreadPoolExecutor :

<!-- check: no-check -->

```catnip
# Configuration explicite
pragma("nd_mode", "parallel")
pragma("nd_workers", "8")
pragma("nd_batch_size", "10")  # 10 items par batch

# Large collection - batching réduit l'overhead
range(1, 101).[~~ (n, recur) => {
    if n <= 1 { 1 }
    else { n * recur(n - 1) }
}]

# Auto-calcul (défaut avec 0)
pragma("nd_batch_size", "0")   # batch_size = ceil(len / (workers * 4))
pragma("batch_size", "0")      # Shorthand
```

**Principe** : Au lieu de soumettre chaque item individuellement à l'executor, on groupe `batch_size` items ensemble.
Cela réduit le nombre de submits et l'overhead de synchronisation.

**Auto-calcul** : Si `batch_size = 0` (défaut), le scheduler calcule automatiquement pour obtenir ~4 batches par worker.

**Détection intelligente** : Pour les petites collections (< workers\*2 items), le batching est automatiquement
désactivé pour éviter l'overhead.

**Cas d'usage optimal** :

- Collections grandes (100+ items)
- Mode parallel avec plusieurs workers
- Items avec temps de calcul variable

**Combinaison avec memoization** :

<!-- check: no-check -->

```catnip
pragma("nd_mode", "parallel")
pragma("nd_memoize", "on")
pragma("nd_batch_size", "5")

# Broadcast sur collection avec doublons
# Batching: réduit overhead ThreadPoolExecutor
# Memoization: évite recalculs des valeurs déjà vues
list(10, 12, 10, 15, 12, 20).[~~ (n, recur) => {
    if n <= 1 { n }
    else { recur(n - 1) + recur(n - 2) }
}]
```

______________________________________________________________________

> **Principe** : La sémantique du code reste identique, seul le mode d'exécution change. Le déterminisme est préservé -
> résultat identique en séquentiel et parallèle.
>
> Les optimisations (memoization, batching) réduisent le temps d'exécution sans changer le résultat.
