# Enums

Le mot-clé `enum` déclare un type avec un ensemble fini de variantes nommées :

```catnip
enum Color { red; green; blue }
```

Chaque variante est un identifiant séparé par `;`. L'accès se fait par qualification : `Color.red`, `Color.green`,
`Color.blue`.

```catnip
enum Color { red; green; blue }

c = Color.red
print(c)
# → Color.red
```

## Accès aux variantes

Les variantes sont toujours qualifiées par le nom de l'enum. `red` seul ne désigne rien -- c'est `Color.red` qui produit
la valeur.

```catnip
enum Direction { up; down; left; right }

d = Direction.left
print(d)
# → Direction.left
```

Accéder à une variante inexistante lève une erreur :

<!-- check: no-check -->

```catnip
enum Color { red; green; blue }
Color.yellow  # Erreur : 'Color' has no variant 'yellow'
```

## Enums multiples

Plusieurs enums coexistent dans le même scope. Des noms de variantes identiques dans des enums distincts ne créent aucun
conflit : la qualification lève l'ambiguité.

```catnip
enum Color { red; blue }
enum Direction { up; down }

Color.red != Direction.up
# → True
```

```catnip
enum A { x; y }
enum B { x; y }

A.x != B.x
# → True
```

> Deux variantes portant le même nom dans deux enums différents n'ont rien en commun. La cohabitation est purement
> syntaxique.

## Egalite

Deux variantes du même enum sont égales si et seulement si elles désignent la même variante. Deux variantes d'enums
différents ne sont jamais égales, même si elles portent le même nom.

```catnip
enum Color { red; green; blue }

Color.red == Color.red
# → True

Color.red == Color.blue
# → False

Color.red != Color.blue
# → True
```

```catnip
enum A { x }
enum B { x }

A.x == B.x
# → False
```

## Truthiness

Toutes les variantes d'enum sont truthy. Aucune n'est falsy.

```catnip
enum Color { red; green; blue }

if Color.red { "oui" } else { "non" }
# → "oui"
```

## Pattern matching

Les variantes d'enum s'utilisent comme patterns dans un `match`. La syntaxe est identique à l'accès : `Enum.variante`.

```catnip
enum Color { red; green; blue }

c = Color.green

match c {
    Color.red => { "rouge" }
    Color.green => { "vert" }
    Color.blue => { "bleu" }
}
# → "vert"
```

Le wildcard `_` fonctionne normalement :

```catnip
enum Color { red; green; blue }

c = Color.blue

match c {
    Color.red => { "rouge" }
    _ => { "autre" }
}
# → "autre"
```

**Attention** : un identifiant nu (sans qualification) dans un pattern `match` est une capture de variable, pas un test
de variante. `red` capture la valeur dans une variable `red` ; `Color.red` teste l'égalité avec la variante.

<!-- check: no-check -->

```catnip
enum Color { red; green; blue }

match Color.blue {
    red => { "piege" }    # capture dans 'red', matche toujours
    _ => { "jamais" }
}
# → "piege" (red est un binding, pas Color.red)
```

Pour matcher une variante, toujours qualifier : `Color.red`.

## Erreurs

Un enum vide est interdit :

<!-- check: no-check -->

```catnip
enum Empty { }  # Erreur de syntaxe
```

Les variantes dupliquées sont interdites :

<!-- check: no-check -->

```catnip
enum Bad { a; a }  # Erreur : variante dupliquée
```

## Limitations (v1)

La version actuelle des enums couvre les cas d'usage les plus courants. Les fonctionnalités suivantes ne sont pas
supportées :

- **Pas de payload** : les variantes ne portent pas de données (`Option.Some(42)` n'existe pas)
- **Pas de méthodes** : on ne peut pas définir de fonctions sur un enum
- **Pas d'héritage** : pas de `extends` pour les enums
- **Pas de traits** : pas de `implements` pour les enums

## Enum vs Struct

| Propriété        | `enum`                 | `struct`                 |
| ---------------- | ---------------------- | ------------------------ |
| Contenu          | Variantes nommées      | Champs typés             |
| Instanciation    | `Color.red`            | `Point(1, 2)`            |
| Mutation         | Non (valeur fixe)      | Oui (`p.x = 5`)          |
| Méthodes         | Non                    | Oui                      |
| Héritage         | Non                    | Oui (`extends`)          |
| Traits           | Non                    | Oui (`implements`)       |
| Pattern matching | `Color.red => { ... }` | `Point{x, y} => { ... }` |
| Egalité          | Même variante = égal   | Même champs = égal       |
| Truthiness       | Toujours truthy        | Toujours truthy          |

> Les enums sont des constantes nommées avec un type. Les structs sont des conteneurs avec des champs. Les deux se
> matchent, mais l'un bouge et l'autre pas.
