# Catnip Cheat-Sheet

Reference rapide du langage. Pour le detail, voir les pages liees.

## Syntaxe de base

```catnip
# Commentaire ligne
x = 42                      # affectation
x = y = 0                   # affectation chainee
{ a = 1; a + 2 }            # bloc (expression, retourne 3)
```

Separateurs : retour a la ligne ou `;`. Parentheses pour multilignes : `(expr\n+ expr)`. Dot-continuation : `.` en debut
de ligne continue l'expression precedente.

Voir [SYNTAX](../lang/SYNTAX.md).

## Types

| Type     | Syntaxe                         | Notes                                  |
| -------- | ------------------------------- | -------------------------------------- |
| Int      | `42`, `-7`                      | SmallInt 47-bit, promotion BigInt auto |
| Float    | `3.14`, `1e-5`                  | IEEE 754 double                        |
| Decimal  | `99.99d`                        | base-10 exact, 28 digits significatifs |
| Complex  | `2j`, `1 + 3j`                  | `.real`, `.imag`, `.conjugate()`       |
| Bool     | `True`, `False`                 |                                        |
| None     | `None`                          | falsy                                  |
| String   | `"hello"`, `'world'`            | `"""multiline"""`                      |
| f-string | `f"x = {x}"`                    | `{x:fmt}`, `{x!r}`, `{x=}` debug       |
| Bytes    | `b"data"`                       |                                        |
| List     | `list(1, 2, 3)`                 | mutable                                |
| Tuple    | `tuple(1, 2)`                   | immutable                              |
| Dict     | `dict(a=1, b=2)`                | `dict((k, v), ...)`                    |
| Set      | `set(1, 2, 3)`                  |                                        |
| Range    | `range(n)`, `range(a, b, step)` | iterable                               |
| ND empty | `~[]`                           | falsy                                  |

Builtins : `typeof(x)` retourne le nom du type. `RUNTIME.smallint_max/min` pour les bornes.

Voir [TYPES](../lang/TYPES.md).

## Operateurs

| Priorite (haute -> basse) | Operateurs                                                       |
| ------------------------- | ---------------------------------------------------------------- |
| Puissance                 | `**`                                                             |
| Unaire                    | `-x`, `+x`, `~x`                                                 |
| Multiplicatif             | `*`, `/` (float), `//` (floor), `%`                              |
| Additif                   | `+`, `-`                                                         |
| Bitwise shift             | `<<`, `>>`                                                       |
| Bitwise AND/XOR/OR        | `&`, `^`, `\|`                                                   |
| Comparaison               | `==`, `!=`, `<`, `<=`, `>`, `>=`, `in`, `not in`, `is`, `is not` |
| Logique                   | `not`, `and`, `or`                                               |
| Nil-coalescing            | `??` (None-only, preserve falsy)                                 |

Comparaisons chainees : `1 < x < 10`. Logiques short-circuit, retournent `bool`.

Indexation : `obj[i]`, `obj[start:stop:step]`. Dot-slice : `obj.[1:3]`.

Voir [EXPRESSIONS](../lang/EXPRESSIONS.md).

## Controle de flux

<!-- check: no-check -->

```catnip
if x > 0 { "pos" } elif x == 0 { "zero" } else { "neg" }    # expression

while cond { body }
for item in iterable { body }
break; continue; return val
```

### Pattern matching

<!-- check: no-check -->

```catnip
match value {
    1 | 2 | 3    => { "petit" }
    n if n > 100 => { "grand" }
    Point{x, y}  => { x + y }       # destructuration struct
    _            => { "autre" }
}
```

Voir [CONTROL_FLOW](../lang/CONTROL_FLOW.md), [PATTERN_MATCHING](../lang/PATTERN_MATCHING.md).

## Fonctions

```catnip
add = (a, b) => { a + b }                # definition
square = (x) => { x ** 2 }               # retour implicite (derniere expr)
greet = (name, sep="") => { sep + name } # defaut
variadic = (*args) => { args }           # variadique
```

- Args manquants -> `None`, args en trop ignores
- Closures : capture lexicale auto (pas de `nonlocal`)
- TCO : auto-detecte sur appels recursifs terminaux
- Decorateurs : `@dec f = ...` -> `f = dec(f)`, empilables

Builtins : `print`, `len`, `abs`, `min`, `max`, `sum`, `round`, `sorted`, `reversed`, `enumerate`, `zip`, `map`,
`filter`, `range`, `int`, `float`, `str`, `bool`, `type`, `list`, `tuple`, `dict`, `set`, `fold`, `reduce`, `globals`,
`locals`.

Voir [FUNCTIONS](../lang/FUNCTIONS.md).

## Structures

```catnip
struct Point { x; y=0 }                  # champs + defaut
p = Point(1, 2)                          # instantiation positionnelle
p = Point(x=1, y=2)                      # instantiation nommee
p.x                                      # acces champ
p.x = 5                                  # mutation

struct Point {
    x; y
    dist(self) => { (self.x**2 + self.y**2)**0.5 }
    @static origin() => { Point(0, 0) }
    init(self) => { print(self.x) }      # post-constructeur
    op +(self, other) => { Point(self.x + other.x, self.y + other.y) }
}
```

### Enums

```catnip
enum Color { red; green; blue }       # declaration
c = Color.red                         # acces qualifie
c == Color.green                      # egalite : False
```

<!-- check: no-check -->

```catnip
match c {
    Color.red   => { "rouge" }
    Color.green => { "vert" }
    _           => { "autre" }
}
```

Pas de payload, pas de methodes, pas d'heritage. Toujours truthy.

Voir [ENUMS](../lang/ENUMS.md).

### Heritage et traits

<!-- check: no-check -->

```catnip
struct Child extends(Parent) { z }                # heritage simple
struct Multi extends(A, B) { }                    # heritage multiple (C3 MRO)
super.method()                                    # appel parent

trait Printable { @abstract to_str(self) }
struct Pt implements(Printable) { x; to_str(self) => { f"{self.x}" } }
```

Operateurs surchargeables : `+`, `-`, `*`, `/`, `//`, `%`, `**`, `==`, `!=`, `<`, `<=`, `>`, `>=`, `&`, `|`, `^`, `<<`,
`>>`, `in`, `not in`, unaires `-`, `+`, `~`. Dispatch inverse auto : `5 + S(10)` appelle `S.op+`.

Voir [STRUCTURES](../lang/STRUCTURES.md).

## Broadcast

<!-- check: no-check -->

```catnip
data.[+ 1]                       # map : ajouter 1 a chaque element
data.[if > 5]                    # filter : garder les > 5
data.[(x) => { x * 2 }]          # map lambda
data.[if (x) => { x % 2 == 0 }]  # filter lambda
data.[abs]                       # map fonction unaire
data.[1:3]                       # dot-slice
```

Chainable : `data.[if > 0].[* 2].[abs]`. Descente recursive dans les listes/tuples imbriques. Preservation de type :
list->list, tuple->tuple, set/dict/range->list.

### ND-recursion

<!-- check: no-check -->

```catnip
data.[~> f]                 # ND-map : appliquer f a chaque feuille
data.[~~(n, r) => { ... }]  # ND-recursion : recursion sur chaque feuille
~~(seed, lambda)            # appel direct
~> f                        # lift : wrapper ND
```

Voir [BROADCAST](../lang/BROADCAST.md), [ND_RECURSION](../lang/ND_RECURSION.md),
[COMPREHENSIONS](../lang/COMPREHENSIONS.md).

## Modules

<!-- check: no-check -->

```catnip
import('utils')                   # bind auto : utils.func()
m = import('math')                # bind explicite
import('io'); io.print("hello")   # stdlib
import('sys'); sys.exit(0)        # stdlib

import('.sibling')                # relatif (meme dossier)
import('..parent_mod')            # relatif (dossier parent)
```

Voir [MODULE_LOADING](../user/MODULE_LOADING.md).

## Pragmas

```catnip
pragma('tco', True)           # tail-call optimization (defaut: True)
pragma('jit', True)           # compilation JIT
pragma('optimize', 3)         # niveau d'optimisation (0-3)
pragma('nd_mode', ND.thread)  # backend ND : sequential, thread, process
```

Voir [PRAGMAS](../lang/PRAGMAS.md).

## Namespaces builtin

| Namespace | Attributs                                                    |
| --------- | ------------------------------------------------------------ |
| `META`    | `.file` (chemin source), `.main` (True si execution directe) |
| `ND`      | `.sequential`, `.thread`, `.process`                         |
| `RUNTIME` | `.smallint_max`, `.smallint_min`                             |
