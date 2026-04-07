# Directives pragma

## Philosophie des Pragmas

Les pragmas permettent de contrôler la compilation et l'exécution **sans changer la sémantique du programme**.

**Principes directeurs** :

- Un programme sans pragmas est valide et s'exécute correctement
- Les pragmas ajustent performance ou debug, pas le comportement fonctionnel
- Scope : fichier entier (pas de pragmas locaux à une fonction) ; en REPL, l'état persiste entre les évaluations
- Précédence : CLI > Fichier > Défaut
- Résolution : les pragmas sont appliqués séquentiellement ; en cas de directives contradictoires dans un même fichier,
  la dernière l'emporte

**Cas d'usage typiques** :

- Désactiver TCO pour débugger des stack traces complètes
- Activer JIT pour scripts avec boucles intensives
- Changer le scheduler ND selon les contraintes (threads vs processus)

## Vue d'ensemble

Les pragmas sont des directives d'exécution qui contrôlent :

- L'optimisation des appels terminaux (TCO)
- La compilation JIT
- Le niveau d'optimisation
- L'exécution de la ND-récursion (mode, parallélisme, optimisations)
- Le comportement du cache
- Le mode debug
- Les indications de fonctions (pures, inlining)
- Le contrôle des warnings

### Référence complète

| Pragma          | Valeurs                                  | Défaut          | Description                           |
| --------------- | ---------------------------------------- | --------------- | ------------------------------------- |
| `tco`           | `True`/`False`                           | `True`          | Optimisation des appels terminaux     |
| `jit`           | `True`/`False`, `"all"`                  | `False`         | Compilation JIT                       |
| `optimize`      | `0`-`3`                                  | `3`             | Niveau d'optimisation                 |
| `cache`         | `True`/`False`                           | `True`          | Cache de compilation                  |
| `debug`         | `True`/`False`                           | `False`         | Mode debug                            |
| `pure`          | `"func_name"`                            | -               | Marque une fonction comme pure        |
| `inline`        | `"always"`/`"never"`/`"auto"`            | `"auto"`        | Hint d'inlining pour une fonction     |
| `warning`       | `True`/`False`                           | `True`          | Contrôle les warnings du compilateur  |
| `nd_mode`       | `ND.sequential`/`ND.thread`/`ND.process` | `ND.sequential` | Mode d'exécution ND                   |
| `nd_workers`    | `0`-`N`                                  | `0` (auto)      | Nombre de workers ND                  |
| `nd_memoize`    | `True`/`False`                           | `False`         | Mémoïsation ND                        |
| `nd_batch_size` | `0`-`N`                                  | `0` (auto)      | Taille de lot pour parallélisation ND |

**Typage strict** : les pragmas booléens (`tco`, `jit`, `cache`, `debug`, `warning`, `nd_memoize`) n'acceptent que
`True`/`False`. Les pragmas numériques (`optimize`, `nd_workers`, `nd_batch_size`) n'acceptent que des entiers. Toute
autre valeur lève `CatnipPragmaError`. Les directives non reconnues sont rejetées à l'analyse sémantique.

## Syntaxe

<!-- check: no-check -->

```catnip
pragma(directive, value)
pragma(directive, value, arg1, arg2, …)
```

Les pragmas sont des instructions traitées lors de l'analyse sémantique.

## Pragmas courants

### TCO (optimisation des appels terminaux)

Contrôle l'optimisation des appels terminaux pour les fonctions récursives.

**Syntaxe :**

```catnip
# Forme booléenne
pragma("tco", True)       # Activer
pragma("tco", False)      # Désactiver
```

**Exemples :**

```catnip
pragma("tco", True)

# Factorielle récursive terminale - pas de dépassement de pile
factorial = (n, acc=1) => {
    if n <= 1 {
        acc
    } else {
        factorial(n - 1, n * acc)
    }
}

factorial(10000)  # Fonctionne avec TCO activé
```

```catnip
pragma("tco", False)

# Pour le debug - plus facile à tracer
countdown = (n) => {
    if n <= 0 {
        "done"
    } else {
        countdown(n - 1)
    }
}
```

### ND-récursion

Contrôle l'exécution et l'optimisation des opérateurs de ND-récursion (`~~`, `~>`).

**Opérateurs :**

| Opérateur          | Forme       | Description                                        | Exemple                                                                 |
| ------------------ | ----------- | -------------------------------------------------- | ----------------------------------------------------------------------- |
| `~~(seed, lambda)` | Combinator  | Exécute la ND-récursion avec une graine            | `~~(10, (n, r) => { if n <= 1 { 1 } else { n * r(n-1) } })` → `3628800` |
| `~~ lambda`        | Déclaration | Crée une fonction ND-récursive réutilisable        | `countdown = ~~(n, r) => { if n > 0 { r(n-1) } else { "done" } }`       |
| `~>(data, f)`      | Applicatif  | Applique une fonction à des données en contexte ND | `~>(list(-1, -2, 3), abs)` → `[1, 2, 3]`                                |
| `~> f`             | Lift        | Enveloppe une fonction dans le contexte ND         | `f = ~> abs; f(-5)` → `5`                                               |
| `data.[~~ lambda]` | Diffusion   | Applique la ND-récursion à chaque élément          | `list(5, 3, 7).[~~ factorial]` → `[120, 6, 5040]`                       |
| `data.[~> f]`      | Diffusion   | Applique la ND-map à chaque élément                | `list(-1, -2, 3).[~> abs]` → `[1, 2, 3]`                                |

**Découpage des formes :**

- **Formes combinator** `~~(seed, lambda)` et `~>(data, f)` : exécution immédiate avec des données
- **Formes déclaration/lift** `~~ lambda` et `~> f` : retourne une fonction encapsulée pour un usage ultérieur
- **Formes de diffusion** `data.[~~ lambda]` et `data.[~> f]` : applique à chaque élément d'une collection

#### nd_mode

Contrôle le mode d'exécution pour la ND-récursion. Trois modes sont disponibles :

| Mode         | Backend                    | Mémoïsation   | Cas d'usage                  |
| ------------ | -------------------------- | ------------- | ---------------------------- |
| `sequential` | Aucun                      | Locale        | Debug, petits calculs        |
| `thread`     | rayon (par_iter)           | Partagée      | I/O bound, mémoire partagée  |
| `process`    | WorkerPool Rust (IPC) [^1] | Par processus | CPU bound, vrai parallélisme |

\[^1\]: Pool de workers `catnip worker` avec IPC bincode sur pipes. Si la lambda ou ses captures ne sont pas freezables
(struct instance, callback Python), fallback automatique vers `ProcessPoolExecutor` Python.

**Syntaxe :**

```catnip
pragma("nd_mode", ND.sequential)  # Exécution séquentielle (par défaut)
pragma("nd_mode", ND.thread)      # Basé threads (mémoire partagée, limité par le GIL)
pragma("nd_mode", ND.process)     # Basé processus (vrai parallélisme)
```

Les constantes `ND.*` sont des builtins disponibles sans import.

**Exemples :**

```catnip
# Mode séquentiel (par défaut)
pragma("nd_mode", ND.sequential)

~~(10, (n, recur) => {
    if n <= 1 { 1 }
    else { n * recur(n - 1) }
})
# → 3628800 (factorielle)
```

```catnip
# Mode thread - cache de mémoïsation partagé
pragma("nd_mode", ND.thread)
pragma("nd_memoize", True)

# Fibonacci bénéficie du cache partagé entre threads
~~(30, (n, recur) => {
    if n <= 1 { n }
    else { recur(n - 1) + recur(n - 2) }
})
# → 832040
```

```catnip
# Mode process - vrai parallélisme pour les tâches CPU-bound
pragma("nd_mode", ND.process)
pragma("nd_workers", 8)

# Chaque worker exécute indépendamment (mémoïsation NON partagée)
list(5, 10, 15, 20).[~~(n, recur) => {
    if n <= 1 { 1 }
    else { n * recur(n - 1) }
}]
# → [120, 3628800, …] (factorielles calculées en parallèle)
```

**Compromis :**

- `sequential` : pas d'overhead, prévisible, le plus facile à déboguer
- `thread` : mémoïsation partagée, mais le GIL limite le parallélisme CPU
- `process` : vrai parallélisme, mais le cache de mémoïsation n'est PAS partagé entre workers

#### nd_workers

Configure le nombre de workers pour l'exécution parallèle.

**Syntaxe :**

```catnip
pragma("nd_workers", 0)   # Détection automatique (utilise le nombre de CPU)
pragma("nd_workers", 4)   # Nombre de workers explicite
pragma("nd_workers", 8)   # 8 workers
```

**Exemples :**

```catnip
pragma("nd_mode", ND.process)
pragma("nd_workers", 4)   # Utilise 4 workers

range(1, 101).[~> (x) => { x * 2 }]
# → Répartit les items sur 4 workers
```

#### nd_memoize

Active la mise en cache automatique des résultats de ND-récursion pour éviter les calculs redondants.

**Syntaxe :**

```catnip
pragma("nd_memoize", True)    # Active la mémoïsation
pragma("nd_memoize", False)   # Désactive (par défaut)
```

**Exemples :**

```catnip
# Sans mémoïsation - lent (O(2^n))
pragma("nd_memoize", False)

~~(25, (n, recur) => {
    if n <= 1 { n }
    else { recur(n - 1) + recur(n - 2) }
})
# → ~1,7 s (242 785 appels récursifs)
```

```catnip
# Avec mémoïsation - rapide (O(n))
pragma("nd_memoize", True)

~~(25, (n, recur) => {
    if n <= 1 { n }
    else { recur(n - 1) + recur(n - 2) }
})
# → ~0,15 s (25 appels uniques, le reste depuis le cache)
# → 11x plus rapide !
```

**Cas d'usage :**

- Programmation dynamique (Fibonacci, etc.)
- Diffusion sur des collections avec des valeurs dupliquées
- Calculs purs coûteux

#### nd_batch_size

Contrôle la granularité des lots pour l'exécution parallèle.

**Syntaxe :**

```catnip
pragma("nd_batch_size", 0)   # Calcul automatique (par défaut)
pragma("nd_batch_size", 10)  # Taille de lot explicite
```

**Calcul automatique :**

- Formule : `batch_size = ceil(collection_length / (workers * 4))`
- Objectif : ~4 lots par worker pour l'équilibrage de charge

**Exemples :**

<!-- check: no-check -->

```catnip
pragma("nd_mode", ND.process)
pragma("nd_workers", 8)
pragma("nd_batch_size", 0)  # Auto : ~4 lots/worker

# 100 éléments → batch_size = ceil(100/32) = 4
# Crée ~25 lots pour un équilibrage optimal
range(1, 101).[~~(n, recur) => { ... }]
```

<!-- check: no-check -->

```catnip
# Taille de lot explicite pour un contrôle fin
pragma("nd_batch_size", 10)

# Traite 10 éléments par lot
large_collection.[~> expensive_function]
```

**Compromis :**

- Petits lots : meilleur équilibrage, plus d'overhead
- Gros lots : moins d'overhead, risque de déséquilibre

#### Exemple combiné

```catnip
# Pile d'optimisations ND complète
pragma("nd_mode", ND.process)
pragma("nd_workers", 8)
pragma("nd_memoize", True)
pragma("nd_batch_size", 10)

# Fibonacci sur une collection avec des doublons
# - Parallèle : utilise 8 workers
# - Mémoïsation : met en cache les résultats (gain sur les doublons)
# - Batching : 10 éléments/lot pour l'équilibrage
list(10, 12, 10, 15, 12, 20, 10).[~~(n, recur) => {
    if n <= 1 { n }
    else { recur(n - 1) + recur(n - 2) }
}]
# → Exécution rapide avec toutes les optimisations
```

### JIT (compilation Just-In-Time)

Contrôle le comportement de compilation JIT.

**Syntaxe :**

```catnip
pragma("jit", True)    # Active le JIT (détection de boucles chaudes)
pragma("jit", False)   # Désactive le JIT
pragma("jit", "all")   # Active le JIT ET compile TOUTES les fonctions immédiatement
```

**Valeurs :**

| Valeur  | Effet                                                               |
| ------- | ------------------------------------------------------------------- |
| `True`  | Active le JIT avec détection de boucles chaudes (seuil ~100 appels) |
| `False` | Désactive la compilation JIT                                        |
| `"all"` | Force la compilation JIT immédiate pour TOUTES les fonctions        |

**Exemple :**

```catnip
pragma("jit", "all")

# Toutes les fonctions sont compilées en JIT au moment de la définition
factorial = (n, acc=1) => {
    if n <= 1 { acc }
    else { factorial(n - 1, n * acc) }
}

factorial(1000)  # Exécute avec du code JIT compilé
```

**Alternative : décorateur `@jit` :**

Pour une compilation JIT sélective, utilisez plutôt le décorateur `@jit` :

```catnip
# Seule cette fonction est compilée immédiatement en JIT
@jit factorial = (n, acc=1) => {
    if n <= 1 { acc }
    else { factorial(n - 1, n * acc) }
}

# Cette fonction utilise la détection normale des boucles chaudes
other_func = (x) => { x * 2 }
```

> La compilation JIT ne réussit que pour les fonctions correspondant à un motif compilable (actuellement : fonctions
> récursives terminales simples avec 1 à 3 paramètres entiers). Les fonctions non conformes s'exécutent normalement via
> l'interpréteur.

**Intégration VM :**

En mode VM (bytecode), le pragma JIT active aussi la compilation trace-based des boucles :

```catnip
pragma("jit", True)

# Cette boucle while sera compilée en code natif
i = 0
total = 0
while i < 100000 {
    total = total + i
    i = i + 1
}
```

Le JIT détecte automatiquement les boucles fréquentes (seuil ~100 itérations) et les compile en x86-64 via Cranelift.
Voir [OPTIMIZATIONS](../dev/OPTIMIZATIONS.md) pour les détails techniques.

### Optimize

Contrôle le niveau d'optimisation (effet actuellement minimal).

**Syntaxe :**

<!-- check: no-check -->

```catnip
pragma("optimize", 0)  # Pas d'optimisation
pragma("optimize", 1)  # Optimisation basique
pragma("optimize", 2)  # Optimisation modérée
pragma("optimize", 3)  # Optimisation agressive
```

### Cache

Contrôle le comportement du cache (effet actuellement minimal).

**Syntaxe :**

```catnip
pragma("cache", True)   # Active le cache
pragma("cache", False)  # Désactive le cache
```

### Debug

Contrôle le mode debug (effet actuellement minimal).

**Syntaxe :**

```catnip
pragma("debug", True)   # Active le debug
pragma("debug", False)  # Désactive le debug
```

### Pure

Marque une fonction comme pure (sans effets de bord, résultat déterministe). Les fonctions pures peuvent bénéficier
d'optimisations supplémentaires dans le broadcasting.

**Syntaxe :**

<!-- check: no-check -->

```catnip
pragma("pure", "func_name")                  # Marquer comme pure
pragma("pure", "func_name", enable=False)    # Retirer le marquage
```

Le décorateur `@pure` est la forme recommandée :

```catnip
@pure double = (x) => { x * 2 }
```

### Inline

Donne des indications d'inlining au compilateur pour une fonction.

**Syntaxe :**

<!-- check: no-check -->

```catnip
pragma("inline", "always", function="func_name")  # Toujours inliner
pragma("inline", "never", function="func_name")   # Jamais inliner
pragma("inline", "auto", function="func_name")    # Décision automatique (défaut)
```

### Warning

Contrôle les warnings émis par le compilateur.

**Syntaxe :**

```catnip
pragma("warning", False)                 # Désactiver tous les warnings
pragma("warning", True)                  # Activer tous les warnings (défaut)
```

<!-- check: no-check -->

```catnip
pragma("warning", False, name="unused")  # Désactiver un warning spécifique
```

## Exemples

### Exemple complet avec plusieurs pragmas

```catnip
pragma("tco", True)
json = import('json')
m = import('math')

# Fibonacci optimisé TCO
fib = (n, a=0, b=1) => {
    if n == 0 {
        a
    } else {
        fib(n - 1, b, a + b)
    }
}

# Utiliser les modules importés
result = dict(
    fibonacci=fib(100),
    sqrt=m.sqrt(100),
    data=json.dumps(list(1, 2, 3))
)

result
```

### TCO conditionnel

```catnip
# Activer le TCO en production
pragma("tco", True)

sum_range = (n, acc=0) => {
    if n <= 0 {
        acc
    } else {
        sum_range(n - 1, acc + n)
    }
}

sum_range(100000)  # Pas de dépassement de pile avec TCO
```

## Voir aussi

- [ND-Recursion Examples](../examples/advanced/nd_recursion.md) - Exemples d'usage de ND-récursion
- [Quickstart 2 Minutes](../tuto/QUICKSTART_2MIN.md) - Introduction à Catnip
- [Glossary](GLOSSARY.md) - Référence du langage
