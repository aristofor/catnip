# Syntax

- [Syntax](SYNTAX.md)
- [Expressions](EXPRESSIONS.md)
- [Control Flow](CONTROL_FLOW.md)
- [Functions](FUNCTIONS.md)
- [Pattern Matching](PATTERN_MATCHING.md)

## Introduction

Catnip est un langage interprété pensé pour être simple, expressif et performant. Il combine une syntaxe claire avec
des fonctionnalités modernes comme le pattern matching et les lambdas.

### Caractéristiques principales

- **Syntaxe claire et concise** : inspirée par des langages modernes
- **Typage dynamique** : les types sont déterminés à l'exécution
- **Pattern matching** : pour un code plus expressif et sûr
- **Fonctions de première classe** : les fonctions sont des valeurs comme les autres
- **Performance** : VM et JIT pour les workloads intensifs
- **REPL interactif** : pour expérimenter et apprendre rapidement

______________________________________________________________________

## Premiers pas

### Premier programme

Catnip n'a jamais eu vocation à parler au monde.

Seulement à l'exécuter.

```catnip
print("BORN TO SEGFAULT")
```

______________________________________________________________________

## Types de données

Catnip supporte les types de données suivants :

### Nombres

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
```

### Chaînes de caractères

```catnip
# Chaînes simples ou doubles guillemets
message = "BORN TO SEGFAULT"
name = 'Capitaine Whiskers'

# Chaînes multilignes
text = """
    Ceci est une chaîne
    sur plusieurs lignes
"""
```

#### Pas de concaténation implicite

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

#### F-strings (chaînes interpolées)

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

#### Spécificateurs de format

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
f"{pi:e}"      # → "3.14159e+00" (notation scientifique)
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

#### Conversion flags

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

#### Debug syntax

La syntaxe `=` après une expression affiche à la fois le code source et le résultat, ce qui facilite le débogage sans dupliquer le nom de la variable :

```catnip
x = 42
f"{x=}"            # → "x=42"
f"{x=:.2f}"        # → "x=42.00" (debug + format)
f"{x=!r}"          # → "x=42" (debug + conversion)

a = 5
b = 3
f"{a + b=}"        # → "a + b=8" (fonctionne avec les expressions)
```

#### Limitations

Les f-strings imbriquées (nested f-strings) ne sont pas supportées. En Python 3.12+, les f-strings peuvent contenir d'autres f-strings grâce à la réécriture du parser en PEG récursif ([PEP 701](https://peps.python.org/pep-0701/)). Catnip utilise un parser Tree-sitter dont le tokenizer ne supporte pas la récursion à l'intérieur des interpolations.

```catnip
# OK
f"{x:.2f}"

# Pas supporté
# f"{x:{'.2f' if precise else '.0f'}}"
# f"{f'{x}'}"
```

> L'impossibilité technique d'imbriquer des f-strings dans des f-strings empêche aussi d'imbriquer
> cette note dans elle-même, ce qui est probablement une bonne chose.

#### Chaînes de bytes

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

# Utile pour orjson et lecture binaire
orjson = import("orjson")
json_bytes = orjson.dumps(dict(key="value"))  # Retourne bytes
```

### Booléens

```catnip
vrai = True
faux = False
rien = None
```

### Listes

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

# Accès aux éléments (via __getitem__)
first = scores_de_licorne[0]  # 1, comme scores_de_licorne.__getitem__(0)
last = scores_de_licorne[2]   # 3
last = scores_de_licorne[-1]  # 5

# Itération
for n in scores_de_licorne {
    print(n)
}

# Avec fonctions Python
total = sum(list(1, 2, 3, 4, 5))   # 15
length = len(crew)                 # 3
```

**Note** : La syntaxe `list(…)` évite la confusion avec la notation de broadcast `.[…]`.

### Sets

Les sets sont des collections non ordonnées sans répétition, ils utilisent la syntaxe `set(…)` :

```catnip
# Set vide
empty = set()

# Set avec valeurs
numbers = set(1, 2, 3, 4, 5)

# Les doublons sont automatiquement supprimés
unique = set(1, 2, 2, 3, 3, 3)  # {1, 2, 3}

# Opérations sur les sets (via Python)
a = set(1, 2, 3, 4)
b = set(3, 4, 5, 6)
union = a.union(b)           # {1, 2, 3, 4, 5, 6}
intersection = a.intersection(b)  # {3, 4}
difference = a.difference(b)      # {1, 2}
```

### Dictionnaires

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

# Accès aux valeurs (via __getitem__)
nom_capitaine = pirate.__getitem__("name")  # "Capitaine Whiskers"

# Avec virgule finale (optionnel)
config = dict(debug=True, port=8080,)
```

**Note** : La syntaxe `dict(...)` utilise des paires ou des kwargs car `{…}` est réservé pour les blocs de code. Les kwargs convertissent l'identifiant en clé string au parse time.

### Autres collections (via Python)

```catnip
# Tuples
coords_lune = tuple(10, 20)

# Sets
unique = set(1, 2, 2, 3, 3, 3)

# Ranges
numbers = list(range(1, 10))
```

______________________________________________________________________

### Topos ND (`@[]`)

`@[]` est un singleton vide utilisé par les opérateurs ND. Il est falsy, itérable vide, et sa longueur vaut 0.

```catnip
empty = @[]
len(empty)         # 0
list(empty)        # list()
if empty { 1 } else { 2 }  # 2
```

______________________________________________________________________

## Variables et assignation

### Assignation simple

```catnip
x = 10
nom = "Alice"
actif = True
```

### Assignation en chaîne

```catnip
# Assigner la même valeur à plusieurs variables
a = b = c = 42
```

### Référence à la dernière valeur

```catnip
x = 10
y = 20
x + y        # Résultat: 30
print(_)     # _ contient la dernière valeur calculée: 30
```

### Affectation d'attributs

```catnip
types = import("types")

obj = types.SimpleNamespace()
obj.x = 10
obj.y = 20

# Chaînes d'attributs
obj.inner = types.SimpleNamespace()
obj.inner.value = 100
```

### Affectation par index

```catnip
# Dictionnaires
d = dict()
d["name"] = "Alice"
d["age"] = 30

# Listes
items = list(0, 0, 0)
items[0] = 1
items[2] = 3
```

> L'affectation par index et par attribut transforme `obj.attr = v` en `setattr(obj, "attr", v)` et `obj[k] = v` en
> `obj.__setitem__(k, v)`. Ce qui signifie que tout objet exposant ces méthodes devient automatiquement mutable depuis
> Catnip.

______________________________________________________________________

## Séparateurs de Statements

Catnip supporte deux types de séparateurs pour délimiter les statements :

### Newlines (retours à la ligne)

Les newlines sont **significatifs** et séparent automatiquement les statements :

```catnip
# ✓ Chaque ligne est un statement séparé
x = 1
y = 2
z = x + y
```

**Cas spéciaux** : Les newlines ne sont PAS significatifs dans :

```catnip
# Arguments de fonction
result = max(10,
    20,
    30)  # ✓ OK - newlines ignorés

# Listes et collections
values = list(1,
    2,
    3)  # ✓ OK - newlines ignorés

# Blocs
x = {
    a = 1
    b = 2
    a + b
}  # ✓ OK - newlines significatifs DANS le bloc, ignorés autour des {}

# if/else multilignes
result = if condition {
    42
}
else {
    0
}  # ✓ OK - newline avant 'else' non significatif
```

### Semicolons (`;`)

Les semicolons permettent de séparer explicitement les statements sur une même ligne :

```catnip
# ✓ Plusieurs statements sur une ligne
x = 1; y = 2; z = x + y
```

**Combinaison** : On peut mélanger semicolons et newlines :

```catnip
# ✓ Mix semicolons et newlines
x = 1; y = 2
z = x + y
result = z * 2; print(result)
```

**Séparateurs multiples** : Les séparateurs consécutifs sont autorisés :

```catnip
# ✓ OK - semicolon suivi de newline
x = { 42 };
y = 1

# ✓ OK - newlines multiples
x = 1

y = 2
```

> Les semicolons sont des points de suture syntaxique. On peut en mettre plusieurs d'affilée si on aime vraiment
> la redondance, un peu comme mettre deux pansements sur la même coupure. Ça ne fait pas de mal, c'est juste
> une preuve de prudence excessive.

______________________________________________________________________

## Structures

Le mot-clé `struct` permet de déclarer une structure nommée avec des champs :

```catnip
struct Point { x, y }
```

Les structures créent des types de données personnalisés avec des champs nommés. Une fois déclarées, elles peuvent être instanciées comme des fonctions :

```catnip
# Déclaration
struct Point { x, y }

# Instanciation avec arguments positionnels
p1 = Point(10, 20)

# Instanciation avec arguments nommés
p2 = Point(x=5, y=15)

# Accès aux attributs
print(p1.x)  # 10
print(p2.y)  # 15
```

### Caractéristiques

Les structures sont implémentées en utilisant les dataclasses Python, ce qui leur confère plusieurs propriétés :

- **Attributs mutables** : les champs peuvent être modifiés après création
- **Représentation automatique** : `str()` et `repr()` affichent la structure avec ses valeurs
- **Égalité structurelle** : deux instances avec les mêmes valeurs sont considérées égales
- **Validation des arguments** : erreurs claires si arguments manquants ou en trop

```catnip
struct Color { r, g, b }

# Mutation
c = Color(255, 0, 0)
c.g = 128
print(c)  # Color(r=255, g=128, b=0)

# Égalité
c1 = Color(100, 100, 100)
c2 = Color(100, 100, 100)
print(c1 == c2)  # True
```

### Valeurs par défaut

Les champs de structure supportent des valeurs par défaut, avec la même syntaxe que les paramètres de fonctions :

```catnip
struct Point { x, y = 0 }

Point(5)        # ⇒ Point(x=5, y=0)
Point(1, 2)     # ⇒ Point(x=1, y=2)
Point(x=3)      # ⇒ Point(x=3, y=0)
```

Les champs sans défaut doivent précéder ceux avec défaut :

```catnip
struct Config { host, port = 8080, debug = False }

Config("localhost")              # ⇒ Config(host="localhost", port=8080, debug=False)
Config("0.0.0.0", 3000, True)   # ⇒ Config(host="0.0.0.0", port=3000, debug=True)
```

Si tous les champs ont un défaut, l'instanciation sans argument est possible :

```catnip
struct Opts { verbose = False, retries = 3 }
Opts()  # ⇒ Opts(verbose=False, retries=3)
```

> Si un champ requis arrive après un champ optionnel, le parseur refuse. Même dans le futur, l'ordre des paramètres reste une loi locale.

### Structures complexes

Les champs peuvent contenir n'importe quel type de valeur :

```catnip
struct Container { data, metadata }

c = Container(
    list(1, 2, 3),
    dict(name="test", version=1)
)

print(c.data[0])           # 1
print(c.metadata["name"])  # "test"
```

### Structures multiples

On peut définir plusieurs structures dans le même programme :

```catnip
struct Vector2D { x, y }
struct Particle { position, velocity, mass }

v = Vector2D(10, 20)
p = Particle(
    Vector2D(0, 0),
    Vector2D(5, 10),
    1.5
)

print(p.velocity.x)  # 5
```

### Méthodes

Les structures peuvent définir des méthodes inline avec un paramètre `self` explicite :

```catnip
struct Point {
    x, y

    distance(self, other) => {
        sqrt((self.x - other.x) ** 2 + (self.y - other.y) ** 2)
    }

    translate(self, dx, dy) => {
        Point(self.x + dx, self.y + dy)
    }
}

a = Point(0, 0)
b = Point(3, 4)
print(a.distance(b))       # 5.0
print(a.translate(1, 2))   # Point(x=1, y=2)
```

Les méthodes sont déclarées après les champs, avec la syntaxe `nom(self, ...) => { corps }`. Le premier paramètre (`self`) est lié automatiquement à l'instance lors de l'appel, via le protocole descripteur Python (`__get__`).

Les méthodes respectent la portée lexicale: elles peuvent capturer des variables locales du scope englobant.

```catnip
make_point_type = () => {
    offset = 10
    struct Point {
        x
        shifted(self) => { self.x + offset }
    }
    Point
}

P = make_point_type()
P(3).shifted()   # 13
```

> Une méthode est une fonction attachée à la `struct`. `self` désigne l'instance courante: protagoniste local, budget infini en parenthèses.

### Constructeur `init`

Une méthode `init(self)` est appelée automatiquement après l'assignation des champs. Elle sert de post-constructeur pour valider ou transformer les valeurs initiales :

```catnip
struct Counter {
    x
    init(self) => { self.x = self.x + 1 }
}

Counter(10).x   # 11
```

La valeur de retour de `init` est ignorée -- l'instance est toujours renvoyée :

```catnip
struct S {
    x
    init(self) => { self.x = self.x * 2; 999 }
}

S(5).x   # 10 (pas 999)
```

`init` fonctionne avec les valeurs par défaut et les arguments nommés :

```catnip
struct Config {
    host, port = 8080
    init(self) => { self.host = self.host + ":auto" }
}

Config("localhost").host   # "localhost:auto"
Config("localhost").port   # 8080
```

> `init` s'exécute automatiquement après l'initialisation des champs. Elle commence avant même que vous pensiez à l'appeler.

### Héritage

Les structures supportent l'héritage simple via `extends(Base)`. L'enfant hérite des champs et méthodes du parent :

```catnip
struct Point {
    x, y
    sum(self) => { self.x + self.y }
}

struct Point3D extends(Point) {
    z
    volume(self) => { self.x * self.y * self.z }
}

p = Point3D(1, 2, 3)
p.x         # 1 (hérité de Point)
p.z         # 3 (défini dans Point3D)
p.sum()     # 3 (méthode héritée de Point)
p.volume()  # 6 (méthode de Point3D)
```

**Règles d'héritage** :

- Les champs de l'enfant sont ajoutés après ceux du parent
- Redéfinir un champ hérité provoque une erreur
- Les méthodes de l'enfant peuvent remplacer (override) celles du parent
- L'ordre des paramètres au constructeur suit l'ordre des champs : parent puis enfant

```catnip
struct Base {
    x
    value(self) => { self.x }
}

struct Child extends(Base) {
    value(self) => { self.x * 10 }  # override
}

Base(5).value()   # 5
Child(5).value()  # 50
```

L'héritage fonctionne avec les valeurs par défaut. Les champs avec défaut du parent sont conservés :

```catnip
struct Config {
    host, port = 8080
}

struct SecureConfig extends(Config) {
    ssl = True
}

SecureConfig("localhost")  # host="localhost", port=8080, ssl=True
```

Tenter d'hériter d'une structure inexistante provoque une erreur à l'exécution :

```catnip
struct Child extends(Unknown) { x }  # RuntimeError: unknown base struct 'Unknown'
```

> L'héritage reprend les champs du parent, permet d'en ajouter, et autorise l'override des méthodes. Même logique, nouvelle couche de peinture.

### Accès au parent (`super`)

Dans une méthode redéfinie, `super` donne accès aux méthodes du parent :

```catnip
struct Base {
    x
    value(self) => { self.x }
}

struct Child extends(Base) {
    value(self) => { super.value() + 10 }
}

Child(5).value()   # 15
```

`super` fonctionne sur toute la chaîne d'héritage. Chaque niveau résout vers son propre parent :

```catnip
struct A {
    x
    value(self) => { self.x }
}

struct B extends(A) {
    value(self) => { super.value() + 10 }
}

struct C extends(B) {
    value(self) => { super.value() + 100 }
}

C(1).value()   # 111
```

`super.init()` appelle le constructeur du parent :

```catnip
struct Base {
    x
    init(self) => { self.x = self.x + 1 }
}

struct Child extends(Base) {
    init(self) => {
        super.init()
        self.x = self.x * 10
    }
}

Child(5).x   # 60  (5+1=6, 6*10=60)
```

Accéder à `super` sans héritage provoque une erreur :

```catnip
struct S {
    x
    value(self) => { super.value() }  # Erreur : super has no method 'value'
}
```

> `super` appelle le parent, puis la méthode enfant reprend le clavier.

### Traits

Les traits définissent des contrats comportementaux (méthodes) qu'une structure peut implémenter. Ils permettent la composition de comportements sans héritage simple.

#### Définition d'un trait

```catnip
trait Printable {
    repr(self) => { "printable" }
}
```

Un trait peut contenir une ou plusieurs méthodes avec `self` explicite.

#### Implémentation de traits

Une structure implémente un ou plusieurs traits via `implements(T1, T2, ...)` :

```catnip
trait Printable {
    repr(self) => { f"({self.x}, {self.y})" }
}

struct Point implements(Printable) {
    x, y
}

Point(3, 4).repr()  # "(3, 4)"
```

Les méthodes du trait sont ajoutées à la structure. La structure peut les remplacer (override) :

```catnip
trait Greetable {
    greet(self) => { "hello" }
}

struct Bot implements(Greetable) {
    name
    greet(self) => { f"I am {self.name}" }
}

Bot("R2").greet()  # "I am R2"
```

#### Héritage de traits

Un trait peut étendre un ou plusieurs autres traits via `extends(T1, T2, ...)` :

```catnip
trait Named {
    name(self) => { "anonymous" }
}

trait Greeter extends(Named) {
    greet(self) => { f"hello, {self.name()}" }
}

struct User implements(Greeter) {
    label
    name(self) => { self.label }
}

User("Alice").greet()  # "hello, Alice"
```

La structure qui implémente `Greeter` hérite aussi des méthodes de `Named`.

#### Composition multiple et conflits

Quand une structure implémente plusieurs traits, les méthodes sont fusionnées. Si deux traits définissent la même méthode, c'est une erreur -- sauf si la structure fournit un override :

```catnip
trait X { f(self) => { 1 } }
trait Y { f(self) => { 2 } }

# struct S implements(X, Y) { }     # Erreur : f en conflit entre X et Y

struct S implements(X, Y) {
    f(self) => { 3 }                 # Override qui résout le conflit
}

S().f()  # 3
```

#### Diamonds

Quand deux traits héritent d'un même trait ancêtre (diamond), l'ancêtre n'est compté qu'une seule fois (première occurrence). Pas d'erreur tant qu'il n'y a pas de conflit de méthodes :

```catnip
trait Base { m(self) => { 0 } }
trait Left extends(Base) { }
trait Right extends(Base) { }

struct S implements(Left, Right) { }
S().m()  # 0 (Base.m hérité une seule fois)
```

> En diamond, l'ancêtre commun n'est intégré qu'une seule fois. Pas de doublon, pas de boucle temporelle.

#### Combiner héritage et traits

Une structure peut combiner `extends` et `implements` dans n'importe quel ordre :

```catnip
trait Loggable { log(self) => { "logged" } }
struct Base { x }

struct Child extends(Base) implements(Loggable) { y }
# ou
struct Child2 implements(Loggable) extends(Base) { y }

c = Child(1, 2)
c.x       # 1 (hérité de Base)
c.y       # 2 (propre à Child)
c.log()   # "logged" (de Loggable)
```

______________________________________________________________________

## Exemples complets

### Calcul de factorielle

```catnip
fn factorielle(n) {
    match n {
        0 | 1 => { 1 }
        n => {
            resultat = 1
            i = 2
            while i <= n {
                resultat = resultat * i
                i = i + 1
            }
            resultat
        }
    }
}

print("10! =", factorielle(10))
```

### Nombres de Fibonacci

```catnip
fn fibonacci(n) {
    match n {
        0 => { 0 }
        1 => { 1 }
        n => {
            a = 0
            b = 1
            i = 2
            while i <= n {
                temp = a + b
                a = b
                b = temp
                i = i + 1
            }
            b
        }
    }
}

# Afficher les 10 premiers nombres de Fibonacci
for i in range(10) {
    print("fib(", i, ") =", fibonacci(i))
}
```

### Tri à bulles

```catnip
fn trier_bulles(liste) {
    n = len(liste)
    i = 0
    while i < n - 1 {
        j = 0
        while j < n - i - 1 {
            if liste.get(j) > liste.get(j + 1) {
                # Échanger
                temp = liste.get(j)
                liste.set(j, liste.get(j + 1))
                liste.set(j + 1, temp)
            }
            j = j + 1
        }
        i = i + 1
    }
    liste
}

nombres = list([64, 34, 25, 12, 22, 11, 90])
trie = trier_bulles(nombres)
print("Liste triée:", trie)
```

### Calculatrice simple

```catnip
fn calculer(operation, a, b) {
    match operation {
        "+" => { a + b }
        "-" => { a - b }
        "*" => { a * b }
        "/" => {
            match b {
                0 => { print("Erreur: division par zéro"); None }
                _ => { a / b }
            }
        }
        op => {
            print("Opération inconnue:", op)
            None
        }
    }
}

resultat = calculer("+", 10, 5)   # 15
resultat = calculer("*", 7, 6)    # 42
resultat = calculer("/", 10, 0)   # Erreur: division par zéro
```

### FizzBuzz

```catnip
fn fizzbuzz(n) {
    for i in range(1, n + 1) {
        match i {
            i if i % 15 == 0 => { print("FizzBuzz") }
            i if i % 3 == 0 => { print("Fizz") }
            i if i % 5 == 0 => { print("Buzz") }
            i => { print(i) }
        }
    }
}

fizzbuzz(20)
```

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
fn fonction_helper() {
    # …
}

fn fonction_principale() {
    # …
}

# 3. Code principal
resultat = fonction_principale()
print(resultat)
```

______________________________________________________________________

## Astuces et pièges à éviter

### Portée des variables

Les variables dans les blocs ne sont pas accessibles à l'extérieur.

```catnip
{
    x = 10
}
print(x)
✗ Error: Unknown identifier 'x'
```

Si tu veux qu'une variable survive, déclare-la avant:

```catnip
x = 0
{
    x = 10  # réassigne la variable du scope parent
}
print(x)  # → 10
```

**Toutes les structures de contrôle créent un scope local** (`if`, `while`, `for`, `{}`):

```catnip
for i in range(5) {
    # i appartient au scope interne créé par le for
    …
}
# i n'existe plus ici
print(i)
✗ Error: Unknown identifier 'i'
```

Note technique: Les blocs et boucles appellent `ctx.push_scope()` avant d'exécuter le corps, ce qui isole les variables
locales. Contrairement à Python où les variables "fuient" dans le scope parent (comportement historique controversé),
Catnip applique le principe de moindre surprise: une variable locale reste locale.

> Checkpoint atteint. Les variables locales ne te suivront pas.

### Évaluation court-circuit

```catnip
# AND s'arrête au premier False
resultat = False and fonction_couteuse()  # fonction_couteuse() n'est PAS appelée

# OR s'arrête au premier True
resultat = True or fonction_couteuse()    # fonction_couteuse() n'est PAS appelée
```

### Match exhaustif

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

| Opérateur                        | Description              |
| -------------------------------- | ------------------------ |
| `()`                             | Parenthèses              |
| `**`                             | Exponentiation           |
| `+x`, `-x`, `~x`                 | Unaires                  |
| `*`, `/`, `//`, `%`              | Multiplication, division |
| `+`, `-`                         | Addition, soustraction   |
| `&`                              | AND binaire              |
| `^`                              | XOR binaire              |
| \`                               | \`                       |
| `<`, `<=`, `>`, `>=`, `==`, `!=` | Comparaisons             |
| `not`                            | NOT logique              |
| `and`                            | AND logique              |
| `or`                             | OR logique               |
