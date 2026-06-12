# Syntaxe

## Introduction

Catnip est un langage interprété pensé pour être simple, expressif et performant. Il combine une syntaxe claire avec des
fonctionnalités modernes comme le pattern matching et les lambdas.

Conventions de lecture des exemples : voir [META](../META.md).

### Caractéristiques principales

- **Syntaxe claire et concise** : inspirée par des langages modernes
- **Typage dynamique** : les types sont déterminés à l'exécution
- **Pattern matching** : pour un code plus expressif et sûr
- **Fonctions de première classe** : les fonctions sont des valeurs comme les autres
- **Performance** : VM et JIT pour les workloads intensifs
- **REPL interactive** : pour expérimenter et apprendre rapidement

______________________________________________________________________

## Premiers pas

### Premier programme

Catnip n'a jamais eu vocation à parler au monde.

Seulement à l'exécuter.

```catnip
print("BORN TO SEGFAULT")
```

______________________________________________________________________

## Séparateurs de Statements

Catnip supporte deux types de séparateurs pour délimiter les statements :

### Newlines (retours à la ligne)

Les newlines sont **significatifs** et séparent automatiquement les statements :

```catnip
# Chaque ligne est un statement séparé
x = 1
y = 2
z = x + y
```

**Cas spéciaux** : Les newlines ne sont PAS significatifs dans :

<!-- check: no-check -->

```catnip
# Arguments de fonction
result = max(10,
    20,
    30)  # OK - newlines ignorés

# Listes et collections
values = list(1,
    2,
    3)  # OK - newlines ignorés

# Blocs
x = {
    a = 1
    b = 2
    a + b
}  # OK - newlines significatifs DANS le bloc, ignorés autour des {}

# if/else multilignes
result = if condition {
    42
}
else {
    0
}  # OK - newline avant 'else' non significatif

# Chaînage (dot-continuation)
result = data
    .[(x) => { x * 2 }]
    .method()
    .attribute  # OK - newline avant '.' non significatif
```

### Semicolons (`;`)

Les semicolons permettent de séparer explicitement les statements sur une même ligne :

```catnip
# Plusieurs statements sur une ligne
x = 1; y = 2; z = x + y
```

**Combinaison** : On peut mélanger semicolons et newlines :

```catnip
# mix semicolons et newlines
x = 1; y = 2
z = x + y
result = z * 2; print(result)
```

**Séparateurs multiples** : Les séparateurs consécutifs sont autorisés :

```catnip
# OK - semicolon suivi de newline
x = { 42 };
y = 1

# OK - newlines multiples
x = 1

y = 2
```

> Les semicolons sont des points de suture syntaxique. On peut en mettre plusieurs d'affilée si on aime vraiment la
> redondance, un peu comme mettre deux pansements sur la même coupure. Ça ne fait pas de mal, c'est juste une preuve de
> prudence excessive.

______________________________________________________________________

## Conventions et bonnes pratiques

### Nommage

```catnip
# Variables et fonctions : snake_case
ma_variable = 42
ma_fonction = (x) => { x * 2 }

# Constantes : MAJUSCULES (convention, pas imposé)
PI = 3.14159
MAX_VALEUR = 100
```

### Commentaires

```catnip
# Commentaire sur une ligne

# Commentaires
# sur plusieurs
# lignes
```

### Organisation du code

```catnip
# 1. Constantes en haut
MAX_ITERATIONS = 1000
SEUIL = 0.001

# 2. Définitions de fonctions
fonction_helper = () => {
    # …
}

fonction_principale = () => {
    # …
}

# 3. Code principal
resultat = fonction_principale()
print(resultat)
```

______________________________________________________________________

## Enums

Le mot-clé `enum` déclare un type à variantes finies :

```catnip
enum Direction { up; down; left; right }
Direction.up
# → Direction.up
```

Les variantes sont qualifiées (`Direction.up`, pas `up`). Dans un pattern `match`, un identifiant nu est une capture de
variable -- utiliser la forme qualifiée pour tester une variante :

<!-- check: no-check -->

```catnip
match d {
    Direction.up => { "haut" }
    Direction.down => { "bas" }
    _ => { "autre" }
}
```

Voir [ENUMS](ENUMS.md) pour la référence complète.

______________________________________________________________________

## Unions taggées

Le mot-clé `union` déclare un type somme : variantes nommées avec ou sans charge utile.

```catnip
union Option { Some(value); None; }

x = Option.Some(42)
y = Option.None
```

Les variantes avec payload portent des champs typés optionnels (génériques aussi optionnels) :

```catnip
union Result[T, E] {
    Ok(value: T);
    Err(error: E);
}
```

Les annotations de type et paramètres génériques sont parsés mais non encore vérifiés au runtime. Le pattern matching
utilise `Union.Variant{field}` pour les variantes avec payload, `Union.Variant` pour les nullaires :

<!-- check: no-check -->

```catnip
match opt {
    Option.Some{value} => { value }
    Option.None => { 0 }
}
```

Voir [UNIONS](UNIONS.md) pour la référence complète.

______________________________________________________________________

## Gestion d'erreurs

### try/except/finally

Le bloc `except` utilise une syntaxe match-like avec `=>` :

<!-- check: no-check -->

```catnip
try {
    resultat = operation_risquee()
} except {
    e: TypeError => { gerer_type(e) }
    e: ValueError | KeyError => { gerer_valeur(e) }
    _ => { gerer_tout() }
} finally {
    cleanup()
}
```

Règles :

- `try` + bloc obligatoire
- `except` optionnel, contient 1..N clauses `pattern => { body }`
- `finally` optionnel, exécuté dans tous les cas (succès, exception, return, break)
- au moins un `except` ou `finally` requis
- `_` (catch-all) doit être la dernière clause
- binding optionnel : `e: TypeError` bind le message d'erreur dans `e`
- union de types : `ValueError | KeyError` match l'un ou l'autre

### raise

<!-- check: expect-error -->

```catnip
raise ValueError("message")    # lever une exception typée
raise "message"                 # lever un RuntimeError
raise                           # re-lever l'exception courante (dans except)
```

### Types d'exception

Hiérarchie compatible Python :

```
Exception
├── TypeError
├── ValueError
├── NameError
├── AttributeError
├── RuntimeError
├── MemoryError
├── ArithmeticError
│   └── ZeroDivisionError
└── LookupError
    ├── IndexError
    └── KeyError
```

Le matching `except` suit la hiérarchie : `except { e: ArithmeticError => { ... } }` catch aussi `ZeroDivisionError`.
`except { e: Exception => { ... } }` catch tout.

### with (context managers)

<!-- check: no-check -->

```catnip
with f = open("data.csv") {
    f.read()
}

# Multi-binding (cleanup en ordre inverse)
with a = open("in"), b = open("out") {
    b.write(a.read())
}
```

Règles :

- `with` + au moins un binding `name = expr` + bloc obligatoire
- bindings séparés par `,`, chacun visible pour les suivants
- `__enter__()` appelé de gauche à droite, `__exit__()` de droite à gauche
- si `__exit__` retourne truthy, l'exception est supprimée
- traceback toujours `None` (écart avec Python)

______________________________________________________________________

## Astuces et pièges à éviter

### Évaluation court-circuit

```catnip
# AND s'arrête au premier False
resultat = False and fonction_couteuse()  # fonction_couteuse() n'est PAS appelée

# OR s'arrête au premier True
resultat = True or fonction_couteuse()    # fonction_couteuse() n'est PAS appelée

# ?? s'arrête si le LHS n'est pas None
resultat = 42 ?? fonction_couteuse()      # fonction_couteuse() n'est PAS appelée
```

### Match exhaustif

<!-- check: no-check -->

```catnip
# Toujours prévoir un cas par défaut
match valeur {
    1 => { "un" }
    2 => { "deux" }
    _ => { "autre" }  # IMPORTANT : évite les cas non gérés
}
```

## Annexes

### Priorité des opérateurs

*du plus fort au plus faible*

| Opérateur                                                        | Description              |
| ---------------------------------------------------------------- | ------------------------ |
| `()`                                                             | Parenthèses              |
| `**`                                                             | Exponentiation           |
| `+x`, `-x`, `~x`                                                 | Unaires                  |
| `*`, `/`, `//`, `%`                                              | Multiplication, division |
| `+`, `-`                                                         | Addition, soustraction   |
| `&`                                                              | AND binaire              |
| `^`                                                              | XOR binaire              |
| <code>\|</code>                                                  | OR binaire               |
| `<`, `<=`, `>`, `>=`, `==`, `!=`, `in`, `not in`, `is`, `is not` | Comparaisons             |
| `not`                                                            | NOT logique              |
| `and`                                                            | AND logique              |
| `or`                                                             | OR logique               |
| `??`                                                             | Nil-coalescing           |
