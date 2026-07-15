# ND Concurrency

Guide pratique pour choisir le bon mode d'exécution ND (`sequential`, `thread`, `process`) et comprendre l'impact du
GIL.

## Les trois modes

```catnip
pragma("nd_mode", ND.sequential)  # Défaut. Un seul thread.
pragma("nd_mode", ND.thread)      # ThreadPoolExecutor. Mémoire partagée.
pragma("nd_mode", ND.process)     # Workers Rust natifs. Processus séparés.
```

Le mode s'applique à toutes les opérations `~~` et `~>` qui suivent.

## Le GIL en 30 secondes

CPython exécute le bytecode Python sous un verrou global (GIL). Conséquence : plusieurs threads Python ne peuvent pas
exécuter du bytecode *en même temps*. Ils alternent.

Cela ne veut pas dire que les threads sont inutiles. Le GIL est relâché pendant :

- les appels système (I/O fichier, réseau, sleep)
- certaines opérations natives (numpy, compression, hashing)
- les attentes (`time.sleep`, `socket.recv`, requêtes HTTP)

Donc : **threads = utile pour I/O**, **inutile pour du calcul pur Python**.

Les processus n'ont pas ce problème. Chaque processus a son propre interpréteur, son propre GIL.

> Le GIL est un détail d'implémentation de CPython, pas une propriété du langage Python. Mais comme Catnip tourne sur
> CPython, c'est notre réalité.

## Quand utiliser quoi

### `sequential` - le défaut

```catnip
pragma("nd_mode", ND.sequential)

~~(10, (n, recur) => {
    if n <= 1 { 1 }
    else { n * recur(n - 1) }
})
```

Pas d'overhead. Pas de surprise. Le plus rapide pour les petits calculs.

**Utiliser quand** :

- Le calcul est court (< 1s)
- On debug
- On ne sait pas quel mode choisir

### `thread` - I/O et mémoïsation partagée

```catnip
pragma("nd_mode", ND.thread)
pragma("nd_workers", 8)
pragma("nd_memoize", True)

~~(30, (n, recur) => {
    if n <= 1 { n }
    else { recur(n - 1) + recur(n - 2) }
})
```

Les threads partagent la mémoire du processus. Le cache de mémoïsation est commun à tous les workers. Un résultat
calculé par un thread est immédiatement disponible pour les autres.

**Utiliser quand** :

- La lambda fait de l'I/O (lecture fichier, requêtes réseau, base de données)
- La mémoïsation est activée et les valeurs se recoupent (Fibonacci, DP)
- On veut le cache partagé sans payer le coût de la sérialisation

**Ne pas utiliser quand** :

- La lambda est du calcul pur (arithmétique, logique, manipulation de listes)
- Le GIL empêchera le parallélisme réel et l'overhead des threads sera du bruit

### `process` - vrai parallélisme CPU

```catnip
pragma("nd_mode", ND.process)
pragma("nd_workers", 8)

list(5, 10, 15, 20).[~~(n, recur) => {
    if n <= 1 { 1 }
    else { n * recur(n - 1) }
}]
```

Chaque worker est un processus séparé avec son propre interpréteur Python. Le GIL n'est plus un facteur limitant.

**Utiliser quand** :

- Le calcul est CPU-bound (arithmétique lourde, récursion profonde)
- Les items sont indépendants (broadcast sur une collection)
- Le temps de calcul par item justifie l'overhead de sérialisation

**Ne pas utiliser quand** :

- Les items sont petits ou le calcul est rapide (overhead > gain)
- La mémoïsation croisée est critique (chaque processus a son propre cache)
- Les lambdas capturent des objets non sérialisables

## Compromis résumés

| Critère              | `sequential` | `thread`  | `process`                  |
| -------------------- | ------------ | --------- | -------------------------- |
| Overhead             | Aucun        | Faible    | Faible (IPC postcard) [^1] |
| Parallélisme CPU     | Non          | Non (GIL) | Oui                        |
| Parallélisme I/O     | Non          | Oui       | Oui                        |
| Mémoïsation partagée | N/A          | Oui       | Non                        |
| Sérialisation        | Aucune       | Aucune    | Freeze postcard [^1]       |
| Debug                | Trivial      | Correct   | Difficile                  |

\[^1\]: Quand la lambda et ses captures/seeds sont freezables (int, float, bool, string, list, tuple, dict, set, et
structs plats -- frontière v1), le mode process utilise un pool persistant de workers Rust (`catnip worker`) avec IPC
postcard : parallélisme CPU réel, pas de pickle, pas de startup Python par worker. Sinon (callback Python, struct
`extends`/`implements`/abstract, grand entier en champ, global non freezable référencé, builtin redéfini par une
fonction), l'exécution retombe automatiquement sur le mode `thread`.

## Sérialisation et processus

En mode `process`, deux chemins sont possibles :

**Chemin natif (Rust workers)** -- utilisé quand la lambda, ses captures et ses seeds sont freezables (types primitifs,
listes, dicts, tuples, strings, sets, et **structs plats** -- frontière v1) :

- La lambda est compilée avec son IR source encodé (`encoded_ir` dans le CodeObject)
- Les captures, seeds et définitions de type struct sont converties en `FrozenValue`/`FrozenStructType` (postcard, pas
  pickle) ; le worker reconstruit les types struct par nom
- Un pool persistant de workers `catnip worker` traite les tâches via IPC stdin/stdout
- Pas de startup Python par worker, pas de pickle, pas de GIL sur l'orchestration

**Chemin thread (fallback)** -- utilisé quand quelque chose que la lambda touche ne peut pas voyager jusqu'au worker :

- captures ou seeds non freezables (callback Python, struct hors frontière v1 -- `extends`/`implements`/abstract, grand
  entier en champ) ;
- un global du parent référencé par la lambda et non freezable (fonction helper, struct hors frontière) : le worker
  vierge ne le reconstruit pas et lèverait `NameError` ;
- un builtin redéfini par une fonction (`str = (x) => {...}`) : le worker résoudrait *son* builtin pré-installé, pas la
  redéfinition -- détecté en amont, sinon la divergence serait silencieuse (aucune erreur à intercepter).

Dans tous ces cas, l'exécution retombe sur le mode `thread` (rayon, in-process) : correct pour tout, mais le GIL
sérialise le calcul CPU. Pas de sérialisation, pas de pickle.

Dans les deux cas, le cache de mémoïsation n'est **pas** partagé entre workers.

## Patterns courants

### Fibonacci avec cache partagé (thread)

```catnip
pragma("nd_mode", ND.thread)
pragma("nd_memoize", True)

fib = ~~(n, recur) => {
    if n <= 1 { n }
    else { recur(n - 1) + recur(n - 2) }
}

fib(30)
# Cache partagé : O(n) appels au lieu de O(2^n)
```

Le mode `thread` est le bon choix ici : la mémoïsation partagée transforme un algorithme exponentiel en linéaire. Le GIL
n'est pas un problème car le gain vient du cache, pas du parallélisme.

### Broadcast CPU-bound (process)

<!-- check: no-check -->

```catnip
pragma("nd_mode", ND.process)
pragma("nd_workers", 4)

list(100, 200, 300, 400).[~~(n, recur) => {
    if n <= 1 { 1 }
    else { n * recur(n - 1) }
}]
# 4 factorielles calculées en parallèle sur 4 processus
```

Chaque item est indépendant et coûteux. Le mode `process` distribue le travail sans contention GIL.

### Fallback automatique

```catnip
pragma("nd_mode", ND.process)

# Si le fork échoue (sandbox, WASM, etc.), le scheduler
# bascule automatiquement en sequential. Pas d'erreur.
~~(5, (n, recur) => { if n <= 1 { 1 } else { n * recur(n - 1) } })
```

> Le mode d'exécution est un choix d'infrastructure, pas de sémantique. Le résultat est identique dans les trois modes.
> Seul le temps change.

## Configuration CLI

```bash
catnip -o nd_mode:thread -o nd_workers:8 script.cat
catnip -o nd_mode:process -o nd_workers:4 script.cat
```

Les options CLI ont priorité sur les pragmas du fichier.

## Références

- [PRAGMAS](../lang/PRAGMAS.md) - spec complète des pragmas ND
- [nd_recursion](../examples/advanced/05_nd_recursion.cat) - exemples d'usage
- [scheduler.rs](../../catnip_rs/src/nd/scheduler.rs) - implémentation du scheduler
