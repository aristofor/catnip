# Functions

- [Syntax](SYNTAX.md)
- [Expressions](EXPRESSIONS.md)
- [Control Flow](CONTROL_FLOW.md)
- [Functions](FUNCTIONS.md)
- [Pattern Matching](PATTERN_MATCHING.md)

## Fonctions

### Définition de fonctions

```catnip
# Fonction sans paramètres
fn saluer() {
    print("BORN TO SEGFAULT!")
}

# Fonction avec paramètres
fn additionner_tacos(a, b) {
    a + b
}

# Fonction avec valeurs par défaut
fn saluer_passager(nom="Monde") {
    print("BORN TO SEGFAULT,", nom, "!")
}

# Fonction avec plusieurs paramètres et défauts
fn configurer_navette(host="localhost", port=8080, debug_mode=False) {
    print("Serveur intergalactique:", host, ":", port)
    print("Mode debug:", debug_mode)
}
```

### Appel de fonctions

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
fn calculer_moyenne(a, b, c) {
    somme = a + b + c
    somme / 3          # Valeur retournée
}

moyenne = calculer_moyenne(10, 20, 30)  # 20.0
```

______________________________________________________________________

## Lambdas et blocs anonymes

Les lambdas sont des fonctions anonymes qui peuvent être passées comme valeurs.

### Syntaxe

```catnip
# Lambda sans paramètres
action = () => {
    print("Action exécutée!")
    42
}

# Lambda avec paramètres
doubler = (x) => { x * 2 }
additionner = (a, b) => { a + b }

# Lambda multiligne
calculer = (x, y) => {
    intermediaire = x * 2
    resultat = intermediaire + y
    resultat
}

# Lambda avec valeurs par défaut
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

Les deux sont équivalentes. Mais la seconde dit ce qu'elle retourne : un morphisme. Nommer les choses, c'est de la doc gratuite.

> La première forme est techniquement plus courte de deux lignes,
> ce qui représente un gain d'espace disque de l'ordre du femtogramme.

### Fonctions variadiques

Les lambdas supportent les paramètres variadiques avec `*args` :

```catnip
# Lambda variadique simple
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

```catnip
# Appel direct
valeur = action()           # Exécute la lambda, retourne 42

# Passage en argument
resultat = doubler(21)      # 42

# Avec plusieurs paramètres
somme_result = additionner_tacos(10, 32) # 42

# Avec valeur par défaut
saluer()                   # "BORN TO SEGFAULT!"
saluer_passager("Alice")   # "BORN TO SEGFAULT, Alice !"
```

### Arité des fonctions et lambdas

Les fonctions et lambdas sont **permissives** sur l'arité :

- Un paramètre manquant vaut `None`
- Un argument en trop est silencieusement ignoré

```catnip
doubler = (x) => { x * 2 }
doubler()       # ⇒ None (x est None, None * 2 = None)
doubler(1, 2)   # ⇒ 2 (le deuxième argument est ignoré)
```

Les **constructeurs de struct** (dataclasses) restent stricts : un argument manquant ou en trop produit une erreur.

```catnip
struct Point { x, y }
Point(1)        # Error: missing 1 required positional argument: 'y'
Point(1, 2, 3)  # Error: takes 3 positional arguments but 4 were given
```

Paramètres requis après un variadique (interdit) :

```catnip
bad = (*items, last) => { last }
# Syntaxe invalide: le paramètre variadique doit être en dernier.
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
apply_first = operations.__getitem__(0)
resultat = apply_first(nombre)  # 6
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

### ✓ Exemples de tail calls

```catnip
# Factorielle tail-recursive (avec accumulateur)
factorial = (n, acc=1) => {
    if n <= 1 { acc } else { factorial(n-1, n*acc) }  # ✓ Tail call
}

# Compteur à rebours
countdown = (n) => {
    if n == 0 {
        "Terminé!"
    } else {
        countdown(n - 1)  # ✓ Tail call
    }
}

# Avec return explicite
chercher = (liste, valeur) => {
    if len(liste) == 0 {
        return False
    } else {
        if liste[0] == valeur {
            return True
        } else {
            return chercher(liste[1:], valeur)  # ✓ Tail call
        }
    }
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
    if liste == [] {
        0
    } else {
        liste.get(0) + sum_list(liste.slice(1))  # ✗ Addition après
    }
}
```

**Après (tail-recursive) :**

```catnip
sum_list = (liste, acc=0) => {
    if liste == [] {
        acc
    } else {
        sum_list(liste.slice(1), acc + liste.get(0))  # ✓ Tail call
    }
}
```

______________________________________________________________________

## Fonctions intégrées

Catnip donne accès à plusieurs fonctions Python intégrées :

### Fonctions de base

```catnip
# write : écrire sur stdout (bas niveau, sans séparateur ni newline)
write("BORN")                             # Écrit "BORN" sans newline
write("TO", " ", "SEGFAULT")              # Écrit "TO SEGFAULT"

# write_err : écrire sur stderr (bas niveau, sans séparateur ni newline)
write_err("Error: ", code)                # Écrit sur stderr

# print : fonction de haut niveau (builtin, implémentée en Python)
# Joint les arguments avec des espaces et ajoute un newline
# Implémentée en utilisant write() sous le capot
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

# Collections
ma_liste = list(range(5))                  # [0, 1, 2, 3, 4]
mon_tuple = tuple(1, 2, 3)                 # (1, 2, 3)
mon_dict = dict()                          # {}
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

```catnip
# Tri
trie = sorted(list(3, 1, 4, 1, 5, 9))         # [1, 1, 3, 4, 5, 9]

# Inversion
inverse = reversed(list(1, 2, 3))              # [3, 2, 1]

# Énumération
for pair in enumerate(list("a", "b", "c")) {
    print(pair)  # (0, "a"), (1, "b"), (2, "c")
}

# Zip (combiner des séquences)
for pair in zip(list(1, 2, 3), list("a", "b", "c")) {
    print(pair)  # (1, "a"), (2, "b"), (3, "c")
}
```

### Fonctions d'ordre supérieur

```catnip
# Map (appliquer une fonction à chaque élément)
doubler = (x) => { x * 2 }
doubles = map(doubler, list(1, 2, 3, 4, 5))

# Filter (filtrer selon une condition)
est_pair = (x) => { x % 2 == 0 }
pairs = filter(est_pair, list(1, 2, 3, 4, 5, 6))
```
