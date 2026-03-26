# Fold et Reduce : guide pratique

Voir aussi : [FUNCTIONS](FUNCTIONS.md) pour la référence des signatures, [BROADCAST_GUIDE](BROADCAST_GUIDE.md) pour les
patterns de transformation.

## Le principe

Broadcast **distribue** une opération sur une structure et en préserve la forme. Fold **agrège** une structure en une
seule valeur.

```
broadcast : [a, b, c] → [f(a), f(b), f(c)]     # forme conservée
fold      : [a, b, c] → résultat               # forme réduite
```

Les deux opèrent sur des collections, mais dans des directions opposées :

- broadcast descend dans les structures (jusqu'aux feuilles)
- fold consomme un niveau de structure (une seule profondeur)

## Signatures

<!-- check: no-check -->

```catnip
fold(iterable, init, f)
# f(acc, x) -> new_acc
# Retourne init si la collection est vide

reduce(iterable, f)
# f(acc, x) -> new_acc
# Le premier élément sert d'accumulateur
# Erreur si la collection est vide
```

`fold` est la primitive (totale). `reduce` est le raccourci (partiel).

______________________________________________________________________

## Exemples de base

### Accumuler des valeurs

```catnip
# Somme
fold(list(1, 2, 3, 4), 0, (acc, x) => { acc + x })  # → 10

# Produit
fold(list(1, 2, 3, 4), 1, (acc, x) => { acc * x })  # → 24

# Concaténation
fold(list("a", "b", "c"), "", (acc, x) => { acc + x })  # → 'abc'
```

Le choix de `init` est crucial : c'est l'**élément neutre** de l'opération.

- Somme : `0` (car `0 + x = x`)
- Produit : `1` (car `1 * x = x`)
- Concaténation : `""` (car `"" + s = s`)
- Intersection logique : `True` (car `True and x = x`)

### Cas vide

```catnip
fold(list(), 0, (acc, x) => { acc + x })     # → 0
fold(list(), 1, (acc, x) => { acc * x })     # → 1
fold(list(), "", (acc, x) => { acc + x })    # → ''
```

Quand la collection est vide, fold retourne `init`. C'est ce qui rend l'opération totale : pas de cas d'erreur.

### reduce : quand init est inutile

```catnip
reduce(list(3, 1, 4, 1, 5), (a, b) => { if a > b { a } else { b } })  # → 5
reduce(list(10, 20, 30), (acc, x) => { acc + x })                     # → 60
```

`reduce` prend le premier élément comme accumulateur initial. Utile quand l'élément neutre est évident ou quand il
n'existe pas (ex: max, min).

______________________________________________________________________

## Patterns courants

### Compter des éléments

```catnip
# Compter les éléments positifs
positifs = fold(list(3, -1, 4, -1, 5), 0, (acc, x) => {
    if x > 0 { acc + 1 } else { acc }
})
print(positifs)  # → 3
```

### Construire une structure

```catnip
# Inverser une liste (fold construit une nouvelle structure)
inversee = fold(list(1, 2, 3, 4), list(), (acc, x) => {
    list(x) + acc
})
print(inversee)  # → [4, 3, 2, 1]
```

### Aplatir un niveau

```catnip
# Aplatir une liste de listes (un seul niveau)
plat = fold(list(list(1, 2), list(3, 4), list(5)), list(), (acc, row) => {
    acc + row
})
print(plat)  # → [1, 2, 3, 4, 5]
```

> fold ne descend qu'un niveau. C'est une propriété, pas une limitation : chaque appel de fold consomme exactement un
> niveau de structure. Pour deux niveaux, deux folds.

### Réduire avec logique

```catnip
# Tous vrais ?
tous = fold(list(True, True, False), True, (acc, x) => { acc and x })
print(tous)  # → False

# Au moins un vrai ?
un = fold(list(False, False, True), False, (acc, x) => { acc or x })
print(un)  # → True
```

______________________________________________________________________

## Composition broadcast + fold

C'est le pattern central : broadcast transforme, fold agrège. Ensemble, ils forment un pipeline complet.

### Transformer puis agréger

```catnip
# Carrés puis somme
data = list(1, 2, 3, 4)
somme_carres = fold(data.[** 2], 0, (acc, x) => { acc + x })
print(somme_carres)  # → 30

# Doubler puis produit
produit_doubles = fold(data.[* 2], 1, (acc, x) => { acc * x })
print(produit_doubles)  # → 384
```

### Filtrer puis agréger

```catnip
# Somme des éléments > 3
data = list(1, 5, 2, 8, 3, 7)
somme_grands = fold(data.[if > 3], 0, (acc, x) => { acc + x })
print(somme_grands)  # → 20
```

### Pipeline complet : filter -> map -> fold

```catnip
# Prendre les positifs, les doubler, les sommer
data = list(-3, 1, -1, 4, -2, 5)
resultat = fold(data.[if > 0].[* 2], 0, (acc, x) => { acc + x })
print(resultat)  # → 20
```

Lecture : prendre `data`, garder les positifs (`[if > 0]`), doubler (`[* 2]`), puis fold-sommer.

> Le pipeline broadcast -> fold est l'aller-retour complet d'une donnée à travers une structure : broadcast l'ouvre,
> fold la referme.

### Comparaison avec l'approche impérative

Le pipeline broadcast + fold remplace des boucles explicites :

```catnip
# Impératif
data = list(1, 2, 3, 4, 5)
total = 0
for x in data {
    if x > 2 {
        total = total + (x * 10)
    }
}
print(total)  # → 120

# Déclaratif (broadcast + fold)
resultat = fold(data.[if > 2].[* 10], 0, (acc, x) => { acc + x })
print(resultat)  # → 120
```

Les deux produisent le même résultat. La version déclarative sépare les trois opérations (filtrer, transformer, agréger)
au lieu de les entremêler dans une boucle.

______________________________________________________________________

## Fold sur structures imbriquées

fold opère sur un seul niveau. Pour des structures imbriquées, on compose les niveaux explicitement.

### Un niveau : agréger les sous-structures

```catnip
# Longueur totale de sous-listes
matrice = list(list(1, 2), list(3, 4, 5), list(6))
total_elements = fold(matrice, 0, (acc, row) => { acc + len(row) })
print(total_elements)  # → 6
```

### Deux niveaux : fold imbriqué

```catnip
# Somme de tous les éléments d'une matrice
matrice = list(list(1, 2, 3), list(4, 5, 6), list(7, 8, 9))
somme_totale = fold(matrice, 0, (acc, row) => {
    acc + fold(row, 0, (a, x) => { a + x })
})
print(somme_totale)  # → 45
```

### Fold pour mapper sur les lignes

```catnip
# Somme des carrés de chaque ligne
matrice = list(list(1, 2, 3), list(4, 5, 6))
sommes = fold(matrice, list(), (acc, row) => {
    acc + list(fold(row.[** 2], 0, (a, x) => { a + x }))
})
print(sommes)  # → [14, 77]
```

Lecture : pour chaque ligne, broadcast `[** 2]` élève au carré, puis un fold interne somme. Le fold externe accumule les
résultats dans une liste.

______________________________________________________________________

## fold vs builtins spécialisés

Les builtins `sum`, `min`, `max` sont des spécialisations de fold :

```catnip
# Équivalences conceptuelles
data = list(1, 2, 3, 4, 5)

# sum(data)  ≡  fold(data, 0, (acc, x) => { acc + x })
print(sum(data))                                          # → 15
print(fold(data, 0, (acc, x) => { acc + x }))             # → 15

# min(data)  ≡  reduce(data, (a, b) => { if a < b { a } else { b } })
print(min(data))                                              # → 1
print(reduce(data, (a, b) => { if a < b { a } else { b } }))  # → 1
```

Les builtins sont plus concis pour les cas standards. `fold` prend le relais quand la logique d'agrégation est custom.

______________________________________________________________________

## Quand utiliser quoi

| Besoin                                         | Outil                                 |
| ---------------------------------------------- | ------------------------------------- |
| Transformer chaque élément                     | `data.[op]` (broadcast)               |
| Filtrer des éléments                           | `data.[if cond]` (broadcast filter)   |
| Agréger en une valeur (somme, produit, concat) | `fold(data, init, f)`                 |
| Agréger sans valeur initiale                   | `reduce(data, f)`                     |
| Somme / min / max simples                      | `sum(data)`, `min(data)`, `max(data)` |
| Transformer puis agréger                       | `fold(data.[op], init, f)`            |
| Filtrer, transformer, agréger                  | `fold(data.[if cond].[op], init, f)`  |
| Agréger des structures imbriquées              | fold dans fold                        |

> Le choix entre fold et un builtin spécialisé est un choix de lisibilité, pas de sémantique. Les deux font la même
> chose. fold le fait en montrant le mécanisme.
