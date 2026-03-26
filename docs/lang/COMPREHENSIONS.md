# Comprehensions Python vers Catnip

Guide de traduction des list/dict/set comprehensions Python vers Catnip idiomatique.

Voir aussi : [BROADCAST_GUIDE](BROADCAST_GUIDE.md) pour les patterns broadcast, [FOLD_GUIDE](FOLD_GUIDE.md) pour
l'agrégation.

## Rappel

Une comprehension combine trois opérations : itération, transformation, filtrage.

```python
[x * 2 for x in data]               # map
[x for x in data if x > 5]          # filter
[x * 2 for x in data if x > 5]      # filter + map
```

Catnip n'a pas de comprehensions. Le broadcasting et la ND-recursion couvrent ces cas avec moins de surface syntaxique
et des propriétés supplémentaires (parallélisme, descente dans les structures imbriquées).

______________________________________________________________________

## Map

```python
# Python
[x ** 2 for x in numbers]
[f(x) for x in data]
```

<!-- check: no-check -->

```catnip
# Catnip
numbers.[** 2]
data.[~> f]
```

Le broadcast descend automatiquement dans les structures imbriquées. Le même code fonctionne sur un scalaire, une liste,
un tuple ou une structure imbriquée.

______________________________________________________________________

## Filter

```python
# Python
[x for x in data if x > 5]
```

<!-- check: no-check -->

```catnip
# Catnip
data.[if > 5]
```

______________________________________________________________________

## Filter + map

```python
# Python
[x * 2 for x in data if x > 5]
```

<!-- check: no-check -->

```catnip
# Catnip : chaînage
data.[if > 5].[* 2]
```

Le chaînage est composable : chaque maillon est un filtre ou une transformation indépendante.

<!-- check: no-check -->

```catnip
data.[if > 3].[* 2].[if < 20]
```

______________________________________________________________________

## Map avec lambda

```python
# Python
[x.upper() for x in names if len(x) > 3]
```

<!-- check: no-check -->

```catnip
# Catnip
names.[if (n) => { len(n) > 3 }].[~> (n) => { upper(n) }]
```

Quand l'opération n'est pas un opérateur binaire, `~>` applique une fonction à chaque élément.

______________________________________________________________________

## Masque booléen

```python
# Python
[x for x, m in zip(data, mask) if m]
```

<!-- check: no-check -->

```catnip
# Catnip
data.[mask]
```

______________________________________________________________________

## Transformation récursive (parallèle)

```python
# Python - séquentiel obligatoire
[fib(n) for n in range(20)]
```

```catnip
# Catnip : parallélisable sans modification
pragma("nd_mode", ND.thread)
pragma("nd_memoize", True)

range(20).[~~ (n, recur) => {
    if n <= 1 { n }
    else { recur(n - 1) + recur(n - 2) }
}]
```

La ND-recursion (`~~`) passe de séquentiel à parallèle en changeant un pragma. Aucune comprehension ne permet ça.

______________________________________________________________________

## Produit cartésien

```python
# Python
[(i, j) for i in range(3) for j in range(3)]
```

```catnip
# Catnip : boucles explicites
result = list()
for i in range(3) {
    for j in range(3) {
        result = result + list(tuple(i, j))
    }
}
```

C'est le cas où Catnip est plus verbeux. Ce pattern est rare dans du code idiomatique.

______________________________________________________________________

## Flatmap

```python
# Python
[y for x in nested for y in x]
```

<!-- check: no-check -->

```catnip
# Catnip : fold + concatenation
fold(nested, list(), (acc, x) => { acc + x })
```

______________________________________________________________________

## Dict comprehension

`dict()` est un builtin Python disponible dans Catnip. `fold` construit le dict élément par élément.

```python
# Python
{k: v for k, v in pairs}
```

<!-- check: no-check -->

```catnip
# Catnip
fold(pairs, dict(), (acc, p) => { acc[p[0]] = p[1]; acc })
```

### Dict depuis une transformation

```python
# Python
{x: x ** 2 for x in range(5)}
```

```catnip
# Catnip
fold(range(5), dict(), (acc, x) => { acc[x] = x ** 2; acc })
```

### Dict avec filtre

```python
# Python
{k: v for k, v in items if v > 0}
```

<!-- check: no-check -->

```catnip
# Catnip : filtrer d'abord, puis accumuler
fold(items.[if (p) => { p[1] > 0 }], dict(), (acc, p) => {
    acc[p[0]] = p[1]; acc
})
```

### Note

Catnip ne fournit pas de syntaxe dédiée pour les comprehensions de dictionnaire.

Les opérations de transformation et de filtrage sur les dictionnaires s'expriment avec des combinateurs généraux (`map`,
`filter`, `ND`, `fold`). Le résultat peut être plus explicite qu'en Python, qui propose une syntaxe spécialisée.

Une évolution future pourrait introduire des helpers dédiés (par exemple `dict_map`, `dict_filter`) ou étendre les
opérateurs existants pour mieux prendre en charge les mappings.

______________________________________________________________________

## Récapitulatif

| Python                           | Catnip                                      |
| -------------------------------- | ------------------------------------------- |
| `[x * 2 for x in data]`          | `data.[* 2]`                                |
| `[f(x) for x in data]`           | `data.[~> f]`                               |
| `[x for x in data if x > 5]`     | `data.[if > 5]`                             |
| `[x * 2 for x in data if x > 5]` | `data.[if > 5].[* 2]`                       |
| `[x for x, m in zip(...) if m]`  | `data.[mask]`                               |
| `[fib(n) for n in range(20)]`    | `range(20).[~~ fib]`                        |
| `{x: x**2 for x in data}`        | `fold(data, dict(), (a, x) => { ... })`     |
| produit cartésien                | boucles `for` imbriquées                    |
| flatmap                          | `fold(nested, list(), (a, x) => { a + x })` |

> Les set comprehensions (`{x for ...}`) utilisent le même pattern avec `set()` à la place de `dict()`.
