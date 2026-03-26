# ND-Recursion

Exemples d'utilisation de la ND-récursion (`~~`, `~>`, `~[]`).

> La ND-récursion permet d'exprimer des computations récursives qui peuvent être exécutées en parallèle sans syntaxe
> `async`/`await`. Le runtime choisit le mode d'exécution.

## `~~` ND-Recursion

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

```catnip
countdown = ~~(n, recur) => {
    if n > 0 { recur(n - 1) }
    else { "done" }
}

countdown(10)
# → "done"
```

> À ce stade, la lambda wrappée s'exécute comme une fonction normale. Une fois appelée avec une seed, elle déclenche la
> récursion.

## `~>` ND-Map

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

## `~[]` Empty Topos

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

### `data.[~> f]` Broadcast ND-map

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

### `data.[~~ lambda]` Broadcast ND-recursion

Applique ND-recursion à chaque élément:

```catnip
# Factorielle sur liste
list(3, 5).[~~(n, recur) => {
    if n <= 1 { 1 }
    else { n * recur(n - 1) }
}]
# → [6, 120]

# Countdown sur chaque élément
list(3, 5, 2).[~~(v, recur) => {
    if v > 0 { recur(v - 1) }
    else { v }
}]
# → [0, 0, 0]

# Préserve le type tuple
tuple(3, 2, 4).[~~(n, recur) => {
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

Par défaut, exécution **séquentielle**. Le mode `"thread"` utilise rayon (Rust) pour distribuer les éléments sur un pool
de threads natifs, en relâchant le GIL pendant l'exécution.

### Pragmas ND

Les pragmas permettent de contrôler le mode d'exécution :

<!-- check: no-check -->

```catnip
# Mode parallèle (rayon)
pragma("nd_mode", ND.thread)

~~(huge_dataset, (data, recur) => { … })
```

**Modes disponibles :**

- `"sequential"` (défaut) : exécution séquentielle, pas de overhead
- `"thread"` : distribution rayon avec `py.detach()` / `Python::attach()` par thread

> Le parallélisme repose sur rayon (Rust), pas sur des threads Python. Le GIL est relâché pendant le dispatch et
> réacquis par chaque worker pour les callbacks Python. La memoization reste thread-safe via `Arc<Mutex>`.

### Memoization

Le pragma `nd_memoize` active un cache automatique des résultats :

```catnip
# Activer la memoization
pragma("nd_memoize", True)

# Fibonacci avec memoization - 11x speedup sur fib(25)
~~(25, (n, recur) => {
    if n <= 1 { n }
    else { recur(n - 1) + recur(n - 2) }
})
# → 75025

# Désactiver la memoization (défaut)
pragma("nd_memoize", False)
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

```catnip
# Configuration explicite
pragma("nd_mode", ND.process)
pragma("nd_workers", 8)
pragma("nd_batch_size", 10)  # 10 items par batch

# Large collection - batching réduit l'overhead
range(1, 101).[~~(n, recur) => {
    if n <= 1 { 1 }
    else { n * recur(n - 1) }
}]

# Auto-calcul (défaut avec 0)
pragma("nd_batch_size", 0)   # batch_size = ceil(len / (workers * 4))
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

```catnip
pragma("nd_mode", ND.process)
pragma("nd_memoize", True)
pragma("nd_batch_size", 5)

# Broadcast sur collection avec doublons
# Batching: réduit overhead ThreadPoolExecutor
# Memoization: évite recalculs des valeurs déjà vues
list(10, 12, 10, 15, 12, 20).[~~(n, recur) => {
    if n <= 1 { n }
    else { recur(n - 1) + recur(n - 2) }
}]
```

### Limites de récursion

La récursion ND est limitée à 200 appels imbriqués via `recur()`. Au-delà, une `RecursionError` est levée :

<!-- check: expect-error -->

```catnip
# Récursion infinie : déclenche RecursionError
~~(0, (v, recur) => { recur(v + 1) })
# RecursionError: maximum ND recursion depth exceeded
```

Cette limite protège contre les stack overflows Rust. Chaque appel récursif crée une nouvelle instance VM sur la stack
(~16KB par frame), et la stack thread (~8MB) déborde autour de ~494 niveaux.

Pour les cas légitimes nécessitant plus de profondeur, préférer la récursion classique (fonctions `=>`) qui utilise le
frame stacking de la VM sans limite de stack Rust.

______________________________________________________________________

> **Principe** : La sémantique du code reste identique, seul le mode d'exécution change. Le déterminisme est préservé -
> résultat identique en séquentiel et parallèle.
>
> Les optimisations (memoization, batching) réduisent le temps d'exécution sans changer le résultat.
