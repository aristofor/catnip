# Types de données

Catnip supporte les types de données suivants :

## Nombres

<!-- check: no-check -->

```catnip
# Entiers
chaussette_gauche = 42
chaussette_droite = -17

# Flottants
pi = 3.14159
constante_de_cafe = 2.71828

# Notation scientifique
espace_infiniment_loin = 1.5e10

# Hexadécimal, binaire, octal
hex_val = 0xFF
bin_val = 0b1010
oct_val = 0o755

# Grands entiers (promotion automatique)
fact50 = 30414093201713378043612608166064768844377641568960512000000000000
big = 2 ** 100
# → 1267650600228229401496703205376
```

### Grands entiers

L'arithmétique sur les entiers est en précision arbitraire. Les petits entiers (47-bit signés, de -2^46 à 2^46-1) sont
stockés inline sans allocation. Au-delà, la VM promeut automatiquement en BigInt (`Arc<GmpInt>` via `rug`/GMP). La
demotion inverse se fait si le résultat retombe dans la plage SmallInt.

```catnip
# Promotion transparente
2 ** 100
# → 1267650600228229401496703205376

# Factorielle de grands nombres
fact = (n) => { if n <= 1 { 1 } else { n * fact(n - 1) } }
fact(50)
# → 30414093201713378043612608166064768844377641568960512000000000000

# Arithmétique mixte SmallInt/BigInt
(2 ** 100) + 1
# → 1267650600228229401496703205377
```

Toutes les opérations arithmétiques (`+`, `-`, `*`, `//`, `%`, `**`) et de comparaison (`<`, `>`, `==`, etc.)
fonctionnent de manière uniforme sur SmallInt et BigInt. La division `/` promeut en float.

> Les entiers Catnip ne débordent jamais. Ils grandissent jusqu'à ce que la RAM s'ennuie.

### Décimales exactes

Le suffixe `d` (ou `D`) crée un nombre décimal base-10 exact (Python `decimal.Decimal`, 28 chiffres significatifs).
Résout le problème classique IEEE 754 : `0.1 + 0.2 != 0.3` en float.

```catnip
# Littéraux décimaux
prix = 99.99d
taxe = 0.08d
total = prix + prix * taxe
# → 107.9892d

# Le test canonique
0.1d + 0.2d == 0.3d
# → True (faux en float)

# Promotion entier → Decimal
2 + 0.5d
# → 2.5d
```

Catnip possède sa propre implémentation des littéraux décimaux via le suffixe `d` - pas besoin d'import. L'exemple
ci-dessous est purement démonstratif : il montre qu'on peut aussi utiliser Python comme DSL.

```catnip
# python Decimal (démonstratif, préférer le suffixe d)
import("decimal", "Decimal")
Decimal("3.14")
# → 3.14d
```

Règles de mélange :

- `Decimal op Decimal` → `Decimal`
- `Int op Decimal` → `Decimal` (promotion exacte)
- `Decimal op Float` → **TypeError** (pas de coercion implicite)

La division arrondit au contexte décimal Python (28 chiffres significatifs). `10d / 2d` donne `5d`, `1d / 3d` donne
`0.3333...d` (28 chiffres).

> Les décimales ne mentent pas. Elles arrondissent poliment à 28 chiffres, ce qui suffit pour la plupart des réalités.

### Nombres complexes

Le suffixe `j` (ou `J`) crée un nombre imaginaire pur. La construction d'un complexe complet passe par l'addition
standard.

```catnip
# Imaginaire pur
2j
# → 2j

# Complexe via addition
1 + 2j
# → (1+2j)

# Arithmétique
(1+2j) * (3+4j)
# → (-5+10j)

# Attributs
(1+2j).real
# → 1.0
(1+2j).imag
# → 2.0
(1+2j).conjugate()
# → (1-2j)
abs(3+4j)
# → 5.0

# Builtin complex()
complex(1, 2)
# → (1+2j)
```

Règles de mélange :

- `int op complex` → `complex`
- `float op complex` → `complex`
- `complex op complex` → `complex`

L'égalité (`==`, `!=`) fonctionne. Les comparaisons d'ordre (`<`, `<=`, `>`, `>=`) lèvent un `TypeError` - un nombre
complexe n'a pas d'ordre total.

> Les complexes vivent dans un plan, pas sur une droite. Leur demander qui est le plus grand, c'est demander à un point
> de la carte de se justifier.

## Chaînes de caractères

```catnip
# Chaînes simples ou doubles guillemets
message = "BORN TO SEGFAULT"
name = 'Capitaine Whiskers'

# Chaînes multilignes (doubles ou simples guillemets)
text = """
    Ceci est une chaîne
    sur plusieurs lignes
"""

text2 = '''
    Même chose avec
    des guillemets simples
'''
```

### Pas de concaténation implicite

Contrairement à Python, Catnip **ne concatène pas** automatiquement les chaînes adjacentes :

```catnip
# Python : "hello" "world" → "helloworld"
# Catnip : erreur de syntaxe

# Utiliser l'opérateur + explicitement
message = "hello" + "world"
```

**Pourquoi ce choix ?**

La concaténation implicite est une source de bugs silencieux, notamment dans les listes :

```python
# En Python, une virgule oubliée passe inaperçue :
items = [
    "foo",
    "bar"   # virgule oubliée
    "baz"
]
# items == ["foo", "barbaz"] - aucune erreur, bug silencieux
```

Forcer un opérateur explicite élimine cette catégorie de bugs. Une erreur de syntaxe visible vaut mieux qu'un
comportement incorrect silencieux.

> La concaténation implicite applique le principe du moindre effort au mauvais endroit : elle économise un caractère
> (`+`) mais coûte potentiellement des heures de débogage. Le ratio effort/bénéfice est défavorable d'un facteur
> d'environ 10⁴.

### F-strings (chaînes interpolées)

Les f-strings permettent d'interpoler des expressions directement dans les chaînes, sans multiplier les concaténations.

**Syntaxe** : préfixer la chaîne avec `f` ou `F` (insensible à la casse) et utiliser `{expression}` pour insérer des
valeurs.

```catnip
# Variables simples
astronaute = "Léonie"
age = 30
f"Je m'appelle {astronaute} et j'ai {age} ans"
# → "Je m'appelle Léonie et j'ai 30 ans"

# Expressions arithmétiques
x = 10
y = 20
f"La somme de {x} et {y} est {x + y}"
# → "La somme de 10 et 20 est 30"

# Appels de fonction
double_burrito = (n) => { n * 2 }
n = 5
f"Le double de {n} est {double_burrito(n)}"
# → "Le double de 5 est 10"

# Échappements standards
f"Ligne 1\nLigne 2"  # Retour à la ligne
f"Colonne 1\tColonne 2"  # Tabulation

# Majuscules ou minuscules
F"Valeur: {42}"  # Identique à f"Valeur: {42}"
```

**Note** : les expressions dans les f-strings sont évaluées dans le scope courant. Les variables non définies ou les
erreurs de syntaxe produisent des messages d'erreur clairs.

### Spécificateurs de format

Les f-strings supportent les spécificateurs de format Python standard via la syntaxe `{expression:format_spec}`.

**Nombres entiers** :

```catnip
n = 42
f"{n:05}"      # → "00042" (zero-padding sur 5 caractères)
f"{n:>10}"     # → "        42" (aligné à droite sur 10 caractères)
f"{n:<10}"     # → "42        " (aligné à gauche sur 10 caractères)
f"{n:^10}"     # → "    42    " (centré sur 10 caractères)
f"{n:x}"       # → "2a" (hexadécimal)
f"{n:b}"       # → "101010" (binaire)
```

**Nombres flottants** :

```catnip
pi = 3.14159
f"{pi:.2f}"    # → "3.14" (2 décimales)
f"{pi:.4f}"    # → "3.1416" (4 décimales, arrondi)
f"{pi:8.2f}"   # → "    3.14" (largeur 8, 2 décimales)
f"{pi:e}"      # → "3.141590e+00" (notation scientifique)
```

**Pourcentages** :

```catnip
ratio = 0.856
f"{ratio:.1%}" # → "85.6%" (pourcentage avec 1 décimale)
f"{ratio:.0%}" # → "86%" (pourcentage arrondi)
```

**Alignement et remplissage** :

```catnip
text = "chat"
f"{text:>10}"  # → "      chat" (aligné à droite)
f"{text:*<10}" # → "chat******" (remplissage avec *)
f"{text:_^10}" # → "___chat___" (centré avec _)
```

**Référence complète** : tous les spécificateurs de la
[Format Specification Mini-Language](https://docs.python.org/3/library/string.html#format-specification-mini-language)
Python sont supportés.

### Conversion flags

Les flags `!r`, `!s` et `!a` appliquent une conversion avant le formatage :

- `!r` appelle `repr()` sur la valeur (utile pour afficher les guillemets autour des chaînes)
- `!s` appelle `str()` (comportement par défaut)
- `!a` appelle `ascii()`

```catnip
name = "Alice"
f"{name!r}"        # → "'Alice'"
f"{name!r:>15}"    # → "        'Alice'" (repr + alignement)
f"{42!s}"          # → "42" (conversion explicite en str)
```

### Debug syntax

La syntaxe `=` après une expression affiche à la fois le code source et le résultat, ce qui facilite le débogage sans
dupliquer le nom de la variable :

```catnip
x = 42
f"{x=}"            # → "x=42"
f"{x=:.2f}"        # → "x=42.00" (debug + format)
f"{x=!r}"          # → "x=42" (debug + conversion)

a = 5
b = 3
f"{a + b=}"        # → "a + b=8" (fonctionne avec les expressions)
```

### Limitations

Les f-strings imbriquées (nested f-strings) ne sont pas supportées. En Python 3.12+, les f-strings peuvent contenir
d'autres f-strings grâce à la réécriture du parser en PEG récursif ([PEP 701](https://peps.python.org/pep-0701/)).
Catnip utilise un parser Tree-sitter dont le tokenizer ne supporte pas la récursion à l'intérieur des interpolations.

<!-- check: no-check -->

```catnip
# OK
f"{x:.2f}"

# Pas supporté
# f"{x:{'.2f' if precise else '.0f'}}"
# f"{f'{x}'}"
```

> L'impossibilité technique d'imbriquer des f-strings dans des f-strings empêche aussi d'imbriquer cette note dans
> elle-même, ce qui est probablement une bonne chose.

### Chaînes de bytes

Les chaînes de bytes utilisent le préfixe `b` et produisent un objet `bytes` Python :

```catnip
# Bytes simples
data = b"hello world"
print(data)  # b'hello world'

# Conversion en string
text = data.decode("utf-8")
print(text)  # hello world

# Avec séquences d'échappement
binary = b"\x48\x65\x6c\x6c\x6f"  # Hello en hexadécimal
newlines = b"line1\nline2"

# Bytes multilignes
raw = b"""
binary
data
"""

# Chargement de modules Python (voir docs/user/)
orjson = import("orjson")
json_bytes = orjson.dumps(dict(key="value"))  # Retourne bytes
```

## Booléens

```catnip
vrai = True
faux = False
rien = None
```

### Truthiness (valeur de vérité)

Catnip utilise les mêmes règles de truthiness que Python. Toute valeur peut être évaluée dans un contexte booléen (`if`,
`while`, `and`, `or`, `not`).

**Valeurs falsy** (évaluées à `False`) :

- `False`
- `None`
- `0` (entier zéro)
- `0.0` (flottant zéro)
- `""` (chaîne vide)
- `list()` (liste vide)
- `tuple()` (tuple vide)
- `set()` (set vide)
- `dict()` (dictionnaire vide)
- `~[]` (topos vide ND)

**Tout le reste est truthy** : nombres non nuls, chaînes non vides, collections non vides, structs, fonctions.

```catnip
x = 0
if x { "jamais" } else { "zero est falsy" }
# → "zero est falsy"

s = ""
if s { "jamais" } else { "chaine vide est falsy" }
# → "chaine vide est falsy"

data = list(1)
if data { "liste non vide est truthy" }
# → "liste non vide est truthy"

# Court-circuit : and/or retournent un booléen (pas la valeur opérande)
0 or "fallback"        # → True  (pas "fallback" comme en Python)
"ok" and 42            # → True  (pas 42 comme en Python)
False and "nope"       # → False
```

> La truthiness est déléguée au protocole Python (`__bool__` / `__len__`). Les structs Catnip sont toujours truthy, sauf
> si quelqu'un implémente un jour un struct quantique dans un état superposé vrai-faux, ce qui n'est pas prévu.

### Nil-coalescing (`??`)

`a ?? b` retourne `a` si `a` n'est pas `None`, sinon évalue et retourne `b`.

```catnip
42 ?? 0              # → 42
None ?? 0            # → 0
None ?? None ?? 3    # → 3
```

`??` teste uniquement `None`, pas la truthiness. Les valeurs falsy sont conservées :

```catnip
0 ?? 99              # → 0
False ?? 99          # → False
"" ?? 99             # → ""
```

Trois niveaux distincts de sélection de valeur :

- `and`/`or` - logique pure, retourne un booléen
- `??` - nil-check, retourne la valeur si non-`None`, sinon le RHS
- `if x { x } else { y }` - contrôle de flux, teste la truthiness

Code équivalent explosé :

<!-- check: no-check -->

```catnip
# a ?? b
{ v = a; if v is None { b } else { v } }
```

> `??` est le seul opérateur qui distingue `None` de `False`. Les autres s'en remettent à la truthiness, qui ne fait pas
> de différence entre les deux.

## Listes

Catnip supporte les littéraux de listes avec la syntaxe `list(…)` :

```catnip
# Liste vide
empty = list()

# Liste de nombres
scores_de_licorne = list(1, 2, 3, 4, 5)

# Liste de chaînes
crew = list("Alice", "Bob", "Charlie")

# Liste avec expressions
computed = list(1 + 1, 2 * 3, 10 / 2)  # list(2, 6, 5.0)

# Listes imbriquées
matrix = list(
    list(1, 2, 3),
    list(4, 5, 6),
    list(7, 8, 9)
)

# Avec virgule finale (optionnel)
items = list(1, 2, 3,)

# Accès aux éléments
first = scores_de_licorne[0]   # 1
last = scores_de_licorne[2]    # 3
last = scores_de_licorne[-1]   # 5 (indexation négative)

# Slicing
scores_de_licorne[1:3]         # → list(2, 3)
scores_de_licorne[::-1]        # → list(5, 4, 3, 2, 1)

# Itération
for n in scores_de_licorne {
    print(n)
}

# Avec fonctions Python
total = sum(list(1, 2, 3, 4, 5))   # → 15
length = len(crew)                 # → 3
```

**Note** : La syntaxe `list(…)` évite la confusion avec la notation de broadcast `.[…]`.

### Sémantique des collections

`list()`, `tuple()` et `set()` sont des littéraux purs : chaque argument devient un élément.

Règle déterministe :

- **0 argument** : collection vide
- **1+ arguments** : un argument = un élément (pas de consommation implicite d'itérable)

```catnip
list()                          # → []
list(range(5))                  # → [range(0, 5)]
list(list(1, 2, 3))             # → [[1, 2, 3]]
list("hello")                   # → ["hello"]
list(42)                        # → [42]
list(1, 2, 3)                   # → [1, 2, 3]
list("hello", "world")          # → ["hello", "world"]
```

Même principe pour `tuple()` et `set()`.

L'expansion est explicite via `*` (et `**` pour `dict`) :

```catnip
list(*list(1, 2), 3, *tuple(4, 5))     # → [1, 2, 3, 4, 5]
tuple(*list(1, 2), 3)                  # → (1, 2, 3)
set(*list(1, 2, 2), 3)                 # → {1, 2, 3}
dict(**dict(a=1), ("b", 2), c=3)       # → {"a": 1, "b": 2, "c": 3}
```

## Sets

Les sets sont des collections non ordonnées sans répétition, ils utilisent la syntaxe `set(…)` :

```catnip
# Set vide
empty = set()

# Set avec valeurs
numbers = set(1, 2, 3, 4, 5)

# Les doublons sont automatiquement supprimés
unique = set(1, 2, 2, 3, 3, 3)  # → {1, 2, 3}

# Opérations sur les sets (via Python)
a = set(1, 2, 3, 4)
b = set(3, 4, 5, 6)
union = a.union(b)                # → {1, 2, 3, 4, 5, 6}
intersection = a.intersection(b)  # → {3, 4}
difference = a.difference(b)      # → {1, 2}
```

## Dictionnaires

Les dictionnaires supportent deux notations : paires `(clé, valeur)` et kwargs `clé=valeur`.

```catnip
# Dictionnaire vide
empty = dict()

# Notation kwargs (clés string implicites)
pirate = dict(name="Capitaine Whiskers", age=7, city="Paris")

# Notation paires (clés arbitraires)
mapping = dict((1, "un"), (2, "deux"), (3, "trois"))

# Mixte : paires et kwargs dans le même appel
mixed = dict((1, "un"), name="Alice", (2, "deux"))

# Valeurs calculées
stats = dict(sum=1 + 2 + 3, product=2 * 3 * 4)

# Structures imbriquées
data = dict(
    numbers=list(1, 2, 3),
    info=dict(x=10, y=20)
)

# Accès aux valeurs
nom_capitaine = pirate["name"]  # → "Capitaine Whiskers"

# Avec virgule finale (optionnel)
config = dict(debug=True, port=8080,)
```

**Note** : La syntaxe `dict(…)` utilise des paires ou des kwargs car `{…}` est réservé pour les blocs de code. Les
kwargs convertissent l'identifiant en clé string au parse time.

## Tuples

Les tuples sont des séquences immutables, avec la syntaxe `tuple(…)` :

```catnip
# Tuple vide
empty = tuple()

# Tuple de coordonnées
coords_lune = tuple(10, 20)

# Accès par index
coords_lune[0]   # → 10
coords_lune[-1]  # → 20

# Unpacking dans for
for (x, y) in list(tuple(1, 2), tuple(3, 4)) {
    print(f"{x}, {y}")
}
```

**Note** : la syntaxe `(a, b)` est réservée aux appels de fonction et au groupement d'expressions. Les tuples utilisent
`tuple(…)` pour lever l'ambiguité.

## Ranges (via Python)

`range()` est un builtin Python disponible directement. C'est un itérable, consommable avec `for...in` :

```catnip
for i in range(1, 10) {
    print(i)  # 1 à 9
}

list(range(5))    # → [range(0, 5)] (littéral à 1 élément)
```

## Introspection de type

`typeof(expr)` retourne le nom du type comme chaîne de caractères. Contrairement au `type()` Python qui retourne un
objet classe, Catnip retourne directement une string exploitable.

```catnip
typeof(42)             # → "int"
typeof(3.14)           # → "float"
typeof(8.9d)           # → "decimal"
typeof(True)           # → "bool"
typeof(None)           # → "nil"
typeof("hello")        # → "string"
typeof(list(1, 2))     # → "list"
typeof(tuple(1, 2))    # → "tuple"
typeof(() => { 1 })    # → "function"
```

Pour les structs, `typeof()` retourne le nom du type :

```catnip
struct Point { x; y }
typeof(Point(1, 2))    # → "Point"
```

Les grands entiers retournent `"int"` (même type logique que les petits entiers) :

```catnip
typeof(2 ** 100)       # → "int"
```

| Valeur             | Retour                    |
| ------------------ | ------------------------- |
| entier             | `"int"`                   |
| flottant           | `"float"`                 |
| décimal            | `"decimal"`               |
| booléen            | `"bool"`                  |
| `None`             | `"nil"`                   |
| chaîne             | `"string"`                |
| liste              | `"list"`                  |
| tuple              | `"tuple"`                 |
| dictionnaire       | `"dict"`                  |
| set                | `"set"`                   |
| fonction / lambda  | `"function"`              |
| instance de struct | nom du type               |
| objet Python       | nom de classe (lowercase) |

`typeof()` est un intrinsic du langage, pas une fonction first-class. L'expression `f = typeof` ne fonctionne pas. Pour
accéder au `type` Python original : `import("builtins").type`.

> `typeof()` inspecte directement le tag NaN-boxed de la valeur (4 bits, O(1)). Les types natifs ne passent jamais par
> Python. Les PyObjects font un lookup par type pointer pour les cas courants, et retournent le `qualname` de la classe
> Python en lowercase pour le reste.

## Différences avec Python

Quelques types et syntaxes Python qui n'existent pas en Catnip :

- **Pas de séparateur `_` dans les nombres** : `1_000_000` n'est pas reconnu, écrire `1000000`
- **Pas de raw strings** : pas de préfixe `r"..."`, les séquences d'échappement sont toujours interprétées
- **Pas de concaténation implicite** de chaînes adjacentes (voir plus haut)

### `list()` / `tuple()` / `set()` : littéraux purs

En Python, `list()` est un constructeur qui prend un seul itérable. `[...]` est un littéral. Ce sont deux syntaxes
distinctes.

En Catnip, `list()` est uniquement un littéral (même principe pour `tuple()` et `set()`). La syntaxe `[...]` n'existe
pas (réservée au broadcast). La règle est :

| Arité | Comportement                           | Exemple                            |
| ----- | -------------------------------------- | ---------------------------------- |
| 0     | collection vide                        | `list()` → `[]`                    |
| 1+    | littéral (argument encapsulé tel quel) | `list(range(5))` → `[range(0, 5)]` |

**Ruptures avec Python** :

- `list(1, 2, 3)` fonctionne (Python lève TypeError)
- `list(42)` encapsule (Python lève TypeError)
- `list("hello")` encapsule la chaîne entière (Python itère en caractères)

Ce choix impose une sémantique unique et prévisible : un argument = un élément. Aucune branche implicite selon
`__iter__`.

______________________________________________________________________

## Topos ND (`~[]`)

`~[]` est un singleton vide utilisé par les opérateurs ND. Il est falsy, itérable vide, et sa longueur vaut 0.

```catnip
empty = ~[]
len(empty)                 # → 0
list(empty)                # → [~[]]
if empty { 1 } else { 2 }  # → 2
```

______________________________________________________________________

## Namespaces builtin

Catnip fournit des namespaces en lecture seule, accessibles sans import. Ils suivent la convention CAPS (`META`, `ND`,
`INT`).

### META

Métadonnées du module en cours d'exécution.

- `META.file` -- chemin du fichier source (ou `"<input>"`)
- `META.main` -- `True` si exécuté directement, `False` si importé

### ND

Constantes pour les modes d'exécution ND-récursion. Évite les fautes de frappe sur les strings.

```catnip
ND.sequential   # → "sequential"
ND.thread       # → "thread"
ND.process      # → "process"

pragma("nd_mode", ND.thread)
```

### INT

Bornes du type entier immédiat (SmallInt, 47 bits signés). Au-delà, promotion automatique en BigInt.

```catnip
INT.max   # → 70368744177663  (2^46 - 1)
INT.min   # → -70368744177664 (-2^46)

INT.max + 1   # → BigInt, toujours exact
```
