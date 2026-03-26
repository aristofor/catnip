# How-to Pattern Matching

> Objectif : maîtriser le pattern matching de zéro à avancé. Chaque section introduit un concept, l'illustre, et montre
> quand l'utiliser.

______________________________________________________________________

## 1. Match basique - le switch qui renvoie une valeur

`match` est une expression : il produit un résultat.

```catnip
status = 404

message = match status {
    200 => { "OK" }
    404 => { "Not Found" }
    500 => { "Server Error" }
    _ => { "Unknown" }
}

print(message)
# → "Not Found"
```

Le `_` (wildcard) attrape tout ce qui ne matche pas. Sans lui, une valeur non couverte déclenche une erreur.

______________________________________________________________________

## 2. Capture de variable

Si un pattern est un identifiant (pas un littéral), il capture la valeur :

```catnip
x = 42

match x {
    0 => { "zero" }
    n => { f"got {n}" }
}
# → "got 42"
```

`n` est lié dans le scope du bras. La capture n'affecte pas le scope extérieur.

______________________________________________________________________

## 3. Guards - filtrer avec une condition

Un guard (`if condition`) affine un pattern. Le pattern matche d'abord, puis le guard est évalué :

```catnip
classify = (score) => {
    match score {
        n if n >= 90 => { "excellent" }
        n if n >= 70 => { "good" }
        n if n >= 50 => { "pass" }
        _ => { "fail" }
    }
}

classify(85)
# → "good"

classify(30)
# → "fail"
```

Le guard a accès aux variables capturées par le pattern. Si le guard échoue, le match passe au bras suivant.

______________________________________________________________________

## 4. OR patterns - regrouper les cas

Le `|` combine plusieurs patterns en un seul bras :

```catnip
day = 6

match day {
    1 | 2 | 3 | 4 | 5 => { "weekday" }
    6 | 7 => { "weekend" }
    _ => { "invalid" }
}
# → "weekend"
```

Un seul bras au lieu de cinq. Le premier sous-pattern qui matche gagne.

______________________________________________________________________

## 5. Tuples - destructuration positionnelle

```catnip
point = tuple(0, 5)

match point {
    (0, 0) => { "origin" }
    (0, y) => { f"y-axis at {y}" }
    (x, 0) => { f"x-axis at {x}" }
    (x, y) => { f"({x}, {y})" }
}
# → "y-axis at 5"
```

Les positions comptent. Un littéral vérifie la valeur, un identifiant la capture.

______________________________________________________________________

## 6. Star patterns - capturer le reste

L'opérateur `*` dans un pattern collecte les éléments restants :

```catnip
values = list(1, 2, 3, 4, 5)

# Tête et reste
match values {
    (first, *rest) => { print("first:", first, "rest:", rest) }
}
# → first: 1 rest: [2, 3, 4, 5]

# Dernier élément
match values {
    (*init, last) => { print("last:", last) }
}
# → last: 5

# Premier, milieu, dernier
match values {
    (a, *mid, z) => { print(a, mid, z) }
}
# → 1 [2, 3, 4] 5
```

______________________________________________________________________

## 7. Struct patterns - dispatch par type

Le pattern matching sur structures vérifie le type puis lie les champs :

```catnip
struct Circle { radius }
struct Rect { width; height; }

area = (shape) => {
    match shape {
        Circle{radius} => { 3.14159 * radius ** 2 }
        Rect{width, height} => { width * height }
        _ => { 0 }
    }
}

area(Circle(5))
# → 78.53975

area(Rect(3, 4))
# → 12
```

C'est du dispatch par type sans hiérarchie de classes. Chaque bras teste le type et destructure les champs en une
opération.

### Struct + guard

```catnip
struct Point { x; y; }

classify_point = (p) => {
    match p {
        Point{x, y} if x == 0 and y == 0 => { "origin" }
        Point{x, y} if y == 0 => { "x-axis" }
        Point{x, y} if x == 0 => { "y-axis" }
        Point{x, y} => { "general" }
    }
}

classify_point(Point(0, 0))
# → "origin"

classify_point(Point(3, 0))
# → "x-axis"
```

______________________________________________________________________

## 8. Patterns imbriqués

Les patterns se composent en profondeur :

```catnip
data = list(1, tuple(2, 3))

match data {
    (a, (b, c)) => { a + b + c }
}
# → 6

# Profondeur arbitraire
deep = list(1, list(2, list(3, 4)))

match deep {
    (a, (b, (c, d))) => { a + b + c + d }
}
# → 10
```

______________________________________________________________________

## 9. Match comme expression

`match` renvoie la valeur du bras exécuté. On peut l'assigner ou le passer directement :

```catnip
grade = (score) => {
    match score {
        n if n >= 90 => { "A" }
        n if n >= 80 => { "B" }
        n if n >= 70 => { "C" }
        _ => { "F" }
    }
}

print(grade(92))
# → "A"
```

______________________________________________________________________

## 10. Pièges courants

### Ordre des bras

Le premier pattern qui matche gagne. Un wildcard en premier masque tout le reste :

```catnip
match 42 {
    _ => { "catch-all" }
    42 => { "exact" }
}
# → "catch-all"  (le 42 n'est jamais testé)
```

### Guard trop large

Un guard permissif masque les bras suivants :

```catnip
match 10 {
    n if n > 0 => { "positive" }
    10 => { "ten" }
}
# → "positive"  ("ten" est inatteignable)
```

### Match non exhaustif

Sans wildcard ni capture finale, une valeur non couverte est une erreur :

<!-- check: expect-error -->

```catnip
match 99 {
    1 => { "one" }
    2 => { "two" }
}
# → Error: No matching pattern
```

> Le compilateur ne vérifie pas l'exhaustivité statiquement. Le `_` est ton filet de sécurité.

______________________________________________________________________

## Récapitulatif

| Pattern  | Syntaxe                  | Rôle                          |
| -------- | ------------------------ | ----------------------------- |
| Littéral | `42`, `"hello"`, `True`  | Vérifie une valeur exacte     |
| Variable | `n`, `x`                 | Capture la valeur             |
| Wildcard | `_`                      | Matche tout, ne capture rien  |
| Guard    | `pattern if cond`        | Filtre conditionnel           |
| OR       | <code>a \| b \| c</code> | Regroupe plusieurs patterns   |
| Tuple    | `(a, b, c)`              | Destructure par position      |
| Star     | `(first, *rest)`         | Capture le reste              |
| Struct   | `Point{x, y}`            | Vérifie le type + destructure |

> Le pattern matching est un outil de décision unique : une entrée, un chemin, un résultat. Pas de backtracking, pas
> d'ambiguïté. Le premier bras qui matche s'exécute, les autres n'existent pas.
