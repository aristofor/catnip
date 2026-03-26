# Functions

## Fonctions

### Définition de fonctions

En Catnip, les fonctions sont des valeurs. On les crée avec la syntaxe `(params) => { body }` et on les assigne à un nom
:

```catnip
# Fonction sans paramètres
saluer = () => {
    print("BORN TO SEGFAULT")
}

# Fonction avec paramètres
additionner_tacos = (a, b) => {
    a + b
}

# Fonction avec valeurs par défaut
saluer_passager = (nom="Monde") => {
    print("BORN TO SEGFAULT,", nom, "!")
}

# Fonction avec plusieurs paramètres et défauts
configurer_navette = (host="localhost", port=8080, debug_mode=False) => {
    print("Serveur intergalactique:", host, ":", port)
    print("Mode debug:", debug_mode)
}
```

### Appel de fonctions

<!-- check: no-check -->

```catnip
# Appel simple
saluer()

# Avec arguments positionnels
resultat = additionner_tacos(10, 20)

# Avec arguments nommés
saluer_passager(nom="Alice")

# Mixte
configurer_navette("192.168.1.1", port=3000, debug_mode=True)
```

### Valeur de retour

La dernière expression d'une fonction est automatiquement retournée :

```catnip
calculer_moyenne = (a, b, c) => {
    somme = a + b + c
    somme / 3          # Valeur retournée
}

moyenne = calculer_moyenne(10, 20, 30)  # 20.0
```

______________________________________________________________________

## Lambdas et fonctions anonymes

Toute fonction non assignée est anonyme. La syntaxe est la même, seul le contexte d'utilisation change :

### Syntaxe

```catnip
# Assignée (nommée)
doubler = (x) => { x * 2 }
additionner = (a, b) => { a + b }

# Multiligne
calculer = (x, y) => {
    intermediaire = x * 2
    resultat = intermediaire + y
    resultat
}

# Avec valeurs par défaut
saluer = (nom="Monde") => {
    print("BORN TO SEGFAULT,", nom, "!")
}
```

### Nommer or not nommer

**(Nommer ou ne pas nommer)**

Quand une lambda est retournée depuis un bloc, deux formes sont possibles :

```catnip
# Forme condensée : la lambda est retournée directement
banana = (tree) => {
    (f) => { tree.map(f) }
}

# Forme nommée : la lambda a un nom avant d'être retournée
banana = (tree) => {
    morphism = (f) => {
        tree.map(f)
    }
    morphism
}
```

Les deux sont équivalentes. Mais la seconde dit ce qu'elle retourne : un morphisme. Nommer les choses, c'est de la doc
gratuite.

> La première forme est techniquement plus courte de deux lignes, ce qui représente un gain d'espace disque de l'ordre
> du femtogramme.

### Fonctions variadiques

Les fonctions supportent les paramètres variadiques avec `*args` :

```catnip
# Variadique simple
collect = (*items) => { items }
collect(1, 2, 3)  # list(1, 2, 3)

# Somme variadique
somme = (*nums) => {
    total = 0
    for n in nums {
        total = total + n
    }
    total
}
somme(1, 2, 3, 4, 5)  # 15

# Paramètres mixtes : réguliers + variadiques
prefix_list = (prefix, *items) => {
    result = list(prefix)
    for item in items {
        result = result + list(item)
    }
    result
}
prefix_list(0, 1, 2, 3)  # list(0, 1, 2, 3)

# Avec valeurs par défaut et variadiques
make_list = (prefix=0, *items) => {
    list(prefix) + list(items)
}
make_list(100, 1, 2, 3)  # list(100, 1, 2, 3)
make_list(1, 2, 3)       # list(1, 2, 3)
```

### Utilisation

<!-- check: no-check -->

```catnip
# Appel direct
valeur = action()           # Exécute la lambda, retourne 42

# Passage en argument
resultat = doubler(21)      # 42

# Avec plusieurs paramètres
somme_result = additionner_tacos(10, 32) # 42

# Avec valeur par défaut
saluer()                   # "BORN TO SEGFAULT"
saluer_passager("Alice")   # "BORN TO SEGFAULT, Alice!"
```

### Arité des fonctions et lambdas

Les fonctions et lambdas sont **permissives** sur l'arité :

- Un paramètre manquant vaut `None`
- Un argument en trop est silencieusement ignoré

<!-- check: no-check -->

```catnip
doubler = (x) => { x * 2 }
doubler()       # None (x est None, None * 2 = None)
doubler(1, 2)   # 2 (le deuxième argument est ignoré)
```

Les **constructeurs de struct** (dataclasses) restent stricts : un argument manquant ou en trop produit une erreur.

<!-- check: no-check -->

```catnip
struct Point { x; y; }
Point(1)        # Error: missing 1 required positional argument: 'y'
Point(1, 2, 3)  # Error: takes 3 positional arguments but 4 were given
```

Paramètres requis après un variadique (interdit) :

<!-- check: no-check -->

```catnip
bad = (*items, last) => { last }  # Erreur de syntaxe : le paramètre variadique doit être en dernier
```

### Lambdas dans les collections

```catnip
# Liste de fonctions
operations = list(
    (x) => { x + 1 },
    (x) => { x * 2 },
    (x) => { x ** 2 }
)

# Application
nombre = 5
resultat = operations[0](nombre)  # 6
```

### Décorateurs

Les décorateurs permettent de transformer une fonction au moment de sa définition. La syntaxe `@decorator` est du sucre
syntaxique pour `f = decorator(f)`.

```catnip
# Syntaxe décorateur
@jit f = (n) => { n * 2 }

# Équivalent à :
f = jit((n) => { n * 2 })
```

#### Décorateurs multiples

Plusieurs décorateurs peuvent être empilés. Ils s'appliquent de l'intérieur vers l'extérieur :

<!-- check: no-check -->

```catnip
# @a @b f = expr équivaut à f = a(b(expr))
@outer @inner f = (x) => { x }

# Équivalent à :
f = outer(inner((x) => { x }))
```

#### Créer un décorateur

Un décorateur est simplement une fonction qui prend une fonction et retourne une fonction :

```catnip
# Décorateur qui ajoute du logging
with_log = (fn) => {
    (x) => {
        print("Appel avec:", x)
        result = fn(x)
        print("Résultat:", result)
        result
    }
}

@with_log double = (n) => { n * 2 }

double(5)
# Appel avec: 5
# Résultat: 10
# → 10
```

#### Décorateurs intégrés

- **`jit`** : Force la compilation JIT immédiate (voir [Pragmas](PRAGMAS.md))
- **`abstract`** : Déclare une méthode sans corps dans un struct ou trait (voir
  [Structures](STRUCTURES.md#m%C3%A9thodes-abstraites))
- **`static`** : Déclare une méthode sans `self`, appelable sur le type (voir
  [Structures](STRUCTURES.md#m%C3%A9thodes-statiques))

> Les décorateurs sont évalués une seule fois, au moment de la définition. Ce qui signifie qu'un décorateur ne peut pas
> décider de ne pas être appliqué. Une fois appliqué, c'est trop tard pour changer d'avis.

______________________________________________________________________

## Tail Calls (Appels en position terminale)

Les **tail calls** (appels en position terminale) sont des appels de fonction qui sont la dernière opération avant le
retour d'une fonction. Catnip détecte automatiquement ces appels durant l'analyse sémantique et peut les optimiser pour
éviter la croissance de la pile d'appels.

### Qu'est-ce qu'un tail call ?

Un appel est en **position terminale** si :

1. C'est la dernière expression dans une fonction
1. Son résultat est directement retourné (sans opération supplémentaire)
1. C'est dans la dernière branche d'un `if`/`match`
1. C'est la dernière expression d'un bloc

### Exemples de tail calls

```catnip
# Factorielle tail-recursive (avec accumulateur)
factorial = (n, acc=1) => {
    if n <= 1 { acc } else { factorial(n-1, n*acc) }  # Tail call
}

# Compteur à rebours
countdown = (n) => {
    if n == 0 {
        "Terminé!"
    } else {
        countdown(n - 1)  # Tail call
    }
}

# Avec return explicite
chercher = (liste, valeur) => {
    if len(liste) == 0 {
        return False
    }
    if liste[0] == valeur {
        return True
    }
    return chercher(liste[1:], valeur)  # Tail call
}
```

### ✗ Exemples de NON-tail calls

```catnip
# Factorielle classique (opération après l'appel)
factorial_bad = (n) => {
    if n <= 1 { 1 } else { n * factorial_bad(n-1) }  # ✗ NOT tail: multiplication après
}

# Fibonacci (deux appels + addition)
fib = (n) => {
    if n <= 1 { n } else { fib(n-1) + fib(n-2) }  # ✗ NOT tail: addition après
}

# Opération après l'appel
double_sum = (n) => {
    2 * sum_to(n)  # ✗ NOT tail: multiplication après
}
```

### Détection automatique

Catnip détecte automatiquement les tail calls pendant l'analyse sémantique :

- Les nœuds `Op` avec `ident='call'` reçoivent un attribut `tail=True` s'ils sont en position terminale
- Cette annotation permet à l'exécuteur d'optimiser l'appel (réutilisation du cadre d'exécution)
- Seuls les appels **récursifs** (à la même fonction) sont annotés comme tail calls

### Positions terminales

| Structure           | Position terminale                                             |
| ------------------- | -------------------------------------------------------------- |
| **Fonction/Lambda** | Dernière expression du corps                                   |
| **Block**           | Dernière expression du bloc                                    |
| **If/Elif/Else**    | Dernière expression de chaque branche                          |
| **Match**           | Dernière expression de chaque case                             |
| **Return**          | Expression retournée                                           |
| **Opérations**      | ✗ Les arguments d'opérations ne sont PAS en position terminale |

### Conversion non-tail → tail

Pour profiter de l'optimisation, transformez vos fonctions récursives en tail-recursive avec un accumulateur :

**Avant (non-tail) :**

```catnip
sum_list = (liste) => {
    if len(liste) == 0 {
        0
    } else {
        liste[0] + sum_list(liste[1:])  # ✗ Addition après
    }
}
```

**Après (tail-recursive) :**

```catnip
sum_list = (liste, acc=0) => {
    if len(liste) == 0 {
        acc
    } else {
        sum_list(liste[1:], acc + liste[0])  # Tail call
    }
}
```

______________________________________________________________________

## Fonctions intégrées

Catnip donne accès à plusieurs fonctions Python intégrées :

### Fonctions de base

<!-- check: no-check -->

```catnip
# write : écrire sur stdout (bas niveau, sans séparateur ni newline)
write("BORN")                             # Écrit "BORN" sans newline
write("TO", " ", "SEGFAULT")              # Écrit "TO SEGFAULT"

# write_err : écrire sur stderr (bas niveau, sans séparateur ni newline)
write_err("Error: ", code)                # Écrit sur stderr

# print : fourni par le module io (auto-importé en CLI/REPL, auto-import configurable)
# Joint les arguments avec des espaces et ajoute un newline
print("Message:", valeur)                 # Écrit "Message: <valeur>\n"
print("A", "B", 42)                       # Écrit "A B 42\n"
print()                                   # Écrit juste "\n"

# range : générer une séquence de nombres
for i in range(10) { print(i) }           # 0 à 9
for i in range(5, 10) { print(i) }        # 5 à 9
for i in range(0, 10, 2) { print(i) }     # 0, 2, 4, 6, 8

# len : longueur d'une séquence
taille = len(list(1, 2, 3, 4, 5))         # 5
```

### Conversion de types

```catnip
# Conversions
entier = int("42")                         # 42
flottant = float("3.14")                   # 3.14
texte = str(42)                            # "42"

# Collections (littéraux natifs, pas des appels Python)
ma_liste = list(1, 2, 3, 4)               # list(1, 2, 3, 4)
mon_tuple = tuple(1, 2, 3)                 # tuple(1, 2, 3)
mon_dict = dict(a=1, b=2)                  # {"a": 1, "b": 2}
mon_set = set(1, 2, 2, 3)                  # {1, 2, 3}
```

### Fonctions mathématiques

```catnip
# Valeur absolue
valeur = abs(-42)                          # 42

# Minimum et maximum
petit = min(3, 7, 2, 9)                    # 2
grand = max(3, 7, 2, 9)                    # 9

# Arrondi
arrondi = round(3.14159, 2)                # 3.14

# Somme
total = sum(list(1, 2, 3, 4, 5))              # 15
```

### Fonctions sur les séquences

`sorted`, `reversed`, `enumerate`, `zip`, `map` et `filter` sont des builtins Python. Elles retournent des
**itérateurs** (pas des listes). On les consomme avec `for...in` :

```catnip
# Tri (sorted retourne une liste, exception parmi les itérateurs)
trie = sorted(list(3, 1, 4, 1, 5, 9))         # [1, 1, 3, 4, 5, 9]

# Inversion (reversed retourne un itérateur)
for x in reversed(list(1, 2, 3)) {
    print(x)  # 3, 2, 1
}

# Énumération
for pair in enumerate(list("a", "b", "c")) {
    print(pair)  # (0, "a"), (1, "b"), (2, "c")
}

# Zip (combiner des séquences)
for pair in zip(list(1, 2, 3), list("a", "b", "c")) {
    print(pair)  # (1, "a"), (2, "b"), (3, "c")
}
```

**Attention** : `list()` en Catnip suit une règle d'arité déterministe :

- `list()` -> liste vide
- `list(x)` -> encapsule `x` comme élément unique
- `list(a, b, c)` -> littéral à N éléments (un argument = un élément)

Exemples :

```catnip
list(range(5))       # [range(0, 5)]
list(list(1, 2, 3))  # [[1, 2, 3]]
list(42)             # [42]
```

### Fonctions d'ordre supérieur

`map` et `filter` retournent des itérateurs, consommables avec `for...in` :

```catnip
# Map (appliquer une fonction à chaque élément)
doubler = (x) => { x * 2 }
for x in map(doubler, list(1, 2, 3, 4, 5)) {
    print(x)  # 2, 4, 6, 8, 10
}

# Filter (filtrer selon une condition)
est_pair = (x) => { x % 2 == 0 }
for x in filter(est_pair, list(1, 2, 3, 4, 5, 6)) {
    print(x)  # 2, 4, 6
}
```

### Agrégation : fold et reduce

Voir [FOLD_GUIDE](FOLD_GUIDE.md) pour le guide complet avec patterns de composition broadcast + fold.

`fold` et `reduce` sont les primitives d'agrégation de Catnip. Là où broadcast distribue une opération sur une structure
(en préservant la forme), fold la consomme en une seule valeur.

#### fold

```catnip
# fold(iterable, init, f) -> valeur
# Applique f(acc, x) de gauche à droite, en partant de init
fold(list(1, 2, 3, 4), 0, (acc, x) => { acc + x })  # 10

# Concaténation de chaînes
fold(list("a", "b", "c"), "", (acc, x) => { acc + x })  # "abc"

# Produit
fold(list(1, 2, 3, 4), 1, (acc, x) => { acc * x })  # 24
```

Sur une collection vide, `fold` retourne `init` -- l'opération est totale :

```catnip
fold(list(), 0, (acc, x) => { acc + x })  # 0
```

`fold` agrège **un seul niveau** de structure. Il n'effectue pas de descente récursive (broadcast s'en charge) :

```catnip
# Compter les éléments des sous-listes (un niveau)
fold(list(list(1, 2), list(3, 4)), 0, (acc, row) => { acc + len(row) })  # 4
```

#### reduce

`reduce` est la variante sans valeur initiale. Le premier élément sert d'accumulateur :

```catnip
# reduce(iterable, f) -> valeur
reduce(list(1, 2, 3), (acc, x) => { acc + x })  # 6

# Maximum artisanal
reduce(list(3, 1, 4, 1, 5, 9), (acc, x) => {
    if x > acc { x } else { acc }
})  # 9
```

Sur une collection vide, `reduce` lève une erreur (`fold` est préférable quand le cas vide est possible).

#### Composition avec broadcast

Les deux primitives se composent : broadcast transforme, fold agrège.

```catnip
# Multiplier par 10, puis sommer
fold(list(1, 2, 3).[* 10], 0, (acc, x) => { acc + x })  # 60
```

> `broadcast` distribue. `fold` rassemble. La pipeline `broadcast -> fold` est le chemin aller-retour complet d'une
> valeur à travers une structure.
