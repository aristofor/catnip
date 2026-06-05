# Tagged Unions

Le mot-clé `union` déclare un type somme : un ensemble fini de **variantes** nommées, chacune pouvant porter des champs
typés.

```catnip
union Option {
    Some(value);
    None;
}
```

Là où `enum` ne nomme que des constantes (variantes nullaires uniquement), `union` permet à certaines variantes de
transporter une **charge utile** (payload) qu'on récupère par destructuration dans `match`.

> Une `union` Catnip est toujours taggée et fermée. Pas d'union mémoire à la C : chaque valeur sait quelle variante elle
> est.

## Construction

Les variantes sont toujours qualifiées par le nom de l'union :

```catnip
union Option {
    Some(value);
    None;
}

x = Option.Some(42)
y = Option.None
```

- `Option.Some(42)` appelle le constructeur de la variante `Some` avec un argument `value=42`.
- `Option.None` est un singleton -- pas d'appel, juste la valeur.

Accéder à une variante inexistante lève une erreur :

<!-- check: no-check -->

```catnip
union Option { Some(value); None }
Option.Maybe  # Erreur : union 'Option' has no variant 'Maybe'
```

## Variantes avec et sans payload

Une union peut mélanger les deux dans la même déclaration :

```catnip
union Event {
    Click(x, y);
    KeyPress(code);
    Quit;
}

e = Event.Click(10, 20)
q = Event.Quit
```

Quand toutes les variantes sont nullaires, `enum` reste préférable :

```catnip
enum Color { red; green; blue }
```

Critère pratique : `enum` pour des constantes nommées, `union` pour une forme de donnée qui peut transporter une charge
utile.

## Paramètres génériques

Les unions peuvent déclarer des paramètres de type entre crochets :

```catnip
union Option[T] {
    Some(value: T);
    None;
}

union Result[T, E] {
    Ok(value: T);
    Err(error: E);
}
```

Les paramètres de type sont **parsés** (et conservés pour les diagnostics) mais ne sont **pas encore vérifiés** au
runtime. Le type checker arrivera avec les type hints (voir TODO).

Les variantes elles-mêmes ne déclarent pas de paramètres : `Some[T](value: T)` est refusé. Le paramétrage décrit l'union
complète, pas chaque constructeur.

## Pattern matching

Une variante avec payload se matche avec la syntaxe `Union.Variant{field, ...}`, identique aux struct patterns :

```catnip
union Option {
    Some(value);
    None;
}

opt = Option.Some(42)

match opt {
    Option.Some{value} => { print("got", value) }
    Option.None => { print("nothing") }
}
# → got 42
```

Une variante nullaire se matche sans accolades (forme `Union.Variant`), identique aux variantes d'enum :

<!-- check: no-check -->
```catnip
match Event.Quit {
    Event.Click{x, y} => { print("click at", x, y) }
    Event.KeyPress{code} => { print("key", code) }
    Event.Quit => { print("bye") }
}
```

> Les champs de la variante sont reliés en variables locales du bloc de match : `value` dans `Option.Some{value}`
> capture la charge utile dans une variable `value`.

## Égalité

L'égalité est **structurelle** :

- Deux variantes du même nom avec les mêmes champs sont égales.
- Deux variantes différentes ne le sont jamais, même si elles portent les mêmes données.

<!-- check: no-check -->
```catnip
Option.Some(1) == Option.Some(1)
# → True

Option.Some(1) == Option.Some(2)
# → False

union Box { A(v); B(v) }
Box.A(1) == Box.B(1)
# → False  (variantes différentes)
```

## Hash

Toutes les variantes sont hashables, payload incluse. Les nullaires passent par `CatnipEnumVariant`, les payloads par
`CatnipStructProxy` (hash structural : nom de variante + hash de chaque champ).

```catnip
union Event { Click(x, y); Quit; }

s = set()
s.add(Event.Quit)        # variante nullaire
s.add(Event.Click(1, 2)) # variante payload
Event.Click(1, 2) in s   # True
```

Le contrat hash/eq des structs s'applique : voir [STRUCTURES.md](STRUCTURES.md#hashabilit%C3%A9) pour les règles
(`op_hash`/`op ==`, freeze-on-hash).

## Truthiness

Toutes les variantes -- y compris les nullaires comme `None`, `Quit`, `Eof` -- sont **truthy**. Aucune n'est falsy par
défaut.

```catnip
union Option { Some(value); None }

if Option.None { "oui" } else { "non" }
# → "oui"
```

Pour tester l'absence de valeur, utiliser explicitement `match` ou la comparaison directe :

<!-- check: no-check -->
```catnip
opt = Option.None
match opt {
    Option.None => { "absent" }
    Option.Some{value} => { f"présent: {value}" }
}
```

## Cohabitation avec `enum`

`enum` et `union` cohabitent. Le critère est simple :

| Cas                          | Outil   |
| ---------------------------- | ------- |
| Constantes nommées           | `enum`  |
| Variantes avec payload       | `union` |
| Mix nullaires + avec payload | `union` |

Une `union` entièrement nullaire est autorisée par le parseur, mais le linter peut suggérer `enum` à la place.

## Mode d'exécution

Le runtime des `union` est câblé sur les deux exécuteurs :

- **Mode VM (défaut)** : opcode `MakeUnion` qui matérialise le namespace au démarrage du module.
- **Mode AST** (`catnip -x ast`) : `op_union` dans le registry, même logique partagée.

Le binaire standalone `catnip-run` (PureVM sans Python) lève `MakeUnion: union runtime not implemented in PureVM` quand
il rencontre une union. Utiliser le CLI Python (`catnip` / `python -m catnip`) pour exécuter du code union.

## Différences avec `enum` et `struct`

| Propriété        | `enum`               | `struct`               | `union`                             |
| ---------------- | -------------------- | ---------------------- | ----------------------------------- |
| Contenu          | Variantes nullaires  | Champs typés           | Variantes avec/sans payload         |
| Instanciation    | `Color.red`          | `Point(1, 2)`          | `Option.Some(42)`, `Option.None`    |
| Pattern matching | `Color.red => {...}` | `Point{x, y} => {...}` | `Option.Some{value} => {...}`       |
| Égalité          | Même variante = égal | Même champs = égal     | Variante + champs structurels       |
| Hash             | Structurel           | Structurel             | Structurel                          |
| Truthiness       | Toujours truthy      | Toujours truthy        | Toujours truthy                     |
| Mutation         | Non (valeur fixe)    | Oui (`p.x = 5`)        | Non (variante figée, payload local) |
| Méthodes         | Non                  | Oui                    | Pas encore                          |
| Génériques       | Non                  | Pas encore             | Oui (`Option[T]`, parsé)            |

## Exhaustivité (linter)

Le linter vérifie statiquement qu'un `match` sur une union couvre toutes les variantes. La règle (`I103`) s'applique dès
que le type du scrutinee peut être inféré -- assignation directe à une variante (`x = Option.Some(...)`), accès à
l'union (`Option.None`), ou variable typée dans le scope courant.

```catnip
union Option { Some(value); None; }

x = Option.Some(1)

# Manque Option.None -- emet I103
match x {
    Option.Some{value} => { value }
}
```

Les patterns OR comptent normalement : `Event.Quit | Event.Reset => { ... }` couvre les deux variantes. Un wildcard
final (`_ => { ... }`) supprime l'avertissement.

## Limitations (MVP)

- **Binaire standalone `catnip-run`** : non supporté (PureVM sans Python). Utiliser le CLI Python.
- **Type hints** : les annotations `value: T` sont parsées mais ignorées au runtime.
- **Méthodes sur unions** : non supportées (pas de `union Option { Some(value); None; map(f) => {...} }`).
- **Traits / `implements`** : non supportés.
