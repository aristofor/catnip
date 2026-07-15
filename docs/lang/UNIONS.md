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

## Champs de payload typés

Un champ de payload peut porter une annotation, comme un champ de struct. Le type est un **contrat vérifié à la
construction** de la variante : un mismatch prouvable est une erreur statique (E300), une valeur non prouvable (issue
d'un appel, d'une frontière Python) échoue en `TypeError` au moment de la construction.

<!-- check: expect-error -->

```catnip
union Shape {
    Circle(radius: int);
    Rect(w: int, h: int);
}

Shape.Circle(3)     # accepté
Shape.Rect(4, 5)    # accepté
Shape.Circle(1.5)   # E300 : le champ 'radius' attend 'int', reçoit 'float'
```

La tour numérique s'applique comme pour les struct : un `int` fourni à un champ `: float` est coercé (`Circle(3)` avec
`radius: float` stocke `3.0`).

Un champ dont le type **est** un paramètre générique (`Some(value: T)`) fait exception : `T` n'est pas fixé à la
construction, donc le contrat s'applique plus loin, au franchissement d'une frontière typée (voir ci-dessous).

> Un champ annoté mais non vérifié n'est pas typé, il est décoré.

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

Une annotation générique est un contrat vérifié. `x: Option[int]` exige que la valeur soit une `Option` **et**, si c'est
un variant à payload dont le champ est le paramètre `T`, que ce payload satisfasse `int` (substitution `T := int`) :

<!-- check: expect-error -->

```catnip
union Option[T] { Some(value: T); None }

f = (x: Option[int]) => { 1 }

f(Option.Some(42))     # accepté
f(Option.None)         # accepté (pas de payload à vérifier)
f(Option.Some("nope")) # TypeError : le payload est str, pas int
```

La vérification s'applique aux positions annotées : paramètres, champs de struct, champs de payload, type de retour.
L'arité fait partie du contrat -- `Option[int, str]` (deux arguments pour un seul paramètre) est une erreur statique. Un
champ **paramètre** (`value: T`) reste non typé à la construction (`Option.Some("x")` construit sans erreur : il vaut
`Option[str]`) ; `T` n'étant pas fixé, c'est au franchissement d'une frontière typée que son contrat s'applique. Un
champ **concret** (`x: int`), lui, est vérifié dès la construction (voir « Champs de payload typés »).

> La substitution v1 couvre les champs qui **sont** un paramètre direct (`value: T`). Un paramètre imbriqué dans un
> composite (`items: list[T]`) vérifie le conteneur, pas encore l'élément.

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

## Méthodes

Une union peut déclarer des méthodes, au niveau de l'union -- jamais par variante. `self` reçoit la variante sur
laquelle la méthode est appelée, quelle qu'elle soit, et le corps discrimine par `match` :

```catnip
union Option {
    Some(value);
    None;

    map(self, f) => {
        match self {
            Option.Some{value} => { Option.Some(f(value)) }
            Option.None => { Option.None }
        }
    }

    unwrap_or(self, default) => {
        match self {
            Option.Some{value} => { value }
            Option.None => { default }
        }
    }
}

Option.Some(21).map((x) => { x * 2 }).unwrap_or(0)   # → 42
Option.None.unwrap_or(-1)                            # → -1
```

Cette forme donne :

- une seule forme de code pour toutes les variantes, payload ou nullaire ;
- pas de dispatch caché : la logique par variante est lisible dans le corps, au même endroit ;
- l'exhaustivité du linter s'applique aux corps -- dans une méthode, le type de `self` est connu, donc un `match self`
  incomplet émet `I103` avec les variantes manquantes.

Les méthodes ne mutent jamais `self` : les unions sont immutables, une méthode retourne une valeur. `map` ci-dessus ne
modifie pas l'`Option`, elle en construit une nouvelle.

> `self` est la seule variable du langage qui ne sait pas qui elle est avant de se matcher elle-même.

Différences avec les méthodes de struct :

- **Pas d'`init`** : les constructeurs d'une union sont ses variantes.
- **Pas d'`@abstract`** : une union n'a pas d'héritage, le corps est toujours obligatoire.
- **Pas de décorateurs ni d'`op`** : une méthode statique ou un opérateur surchargé introduirait un second espace de
  noms sur `Union.X` et un second mécanisme de dispatch ; le MVP garde une seule forme.

Méthodes et variantes partagent l'espace de noms `Union.x` : un nom de méthode identique à un nom de variante est rejeté
à la compilation, comme un nom dupliqué.

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

Le runtime des `union` est câblé sur les trois exécuteurs :

- **Mode VM (défaut)** : opcode `MakeUnion` qui matérialise le namespace au démarrage du module.
- **Mode AST** (`catnip -x ast`) : `op_union` dans le registry, même logique partagée.
- **PureVM (MCP, runtime pur sans Python)** : même opcode `MakeUnion`, décodé par `handle_make_union` -- variantes à
  payload en struct types qualifiés, nullaires en symboles internés.

## Différences avec `enum` et `struct`

| Propriété        | `enum`               | `struct`               | `union`                                |
| ---------------- | -------------------- | ---------------------- | -------------------------------------- |
| Contenu          | Variantes nullaires  | Champs typés           | Variantes avec/sans payload            |
| Instanciation    | `Color.red`          | `Point(1, 2)`          | `Option.Some(42)`, `Option.None`       |
| Pattern matching | `Color.red => {...}` | `Point{x, y} => {...}` | `Option.Some{value} => {...}`          |
| Égalité          | Même variante = égal | Même champs = égal     | Variante + champs structurels          |
| Hash             | Structurel           | Structurel             | Structurel                             |
| Truthiness       | Toujours truthy      | Toujours truthy        | Toujours truthy                        |
| Mutation         | Non (valeur fixe)    | Oui (`p.x = 5`)        | Non (variante figée, payload local)    |
| Méthodes         | Non                  | Oui                    | Oui (niveau union, `match self`)       |
| Génériques       | Non                  | Pas encore             | Oui (`Option[T]`, vérifié au boundary) |

## Exhaustivité (linter)

Le linter vérifie statiquement qu'un `match` sur une union couvre toutes les variantes. La règle (`I103`) s'applique dès
que le type du scrutinee peut être inféré -- assignation directe à une variante (`x = Option.Some(...)`), accès à
l'union (`Option.None`), ou variable typée dans le scope courant.

```catnip
union Option { Some(value); None; }

x = Option.Some(1)

# Manque Option.None -- émet I103
match x {
    Option.Some{value} => { value }
}
```

Les patterns OR comptent normalement : `Event.Quit | Event.Reset => { ... }` couvre les deux variantes. Un wildcard
final (`_ => { ... }`) supprime l'avertissement.

## Limitations (MVP)

- **Champ paramètre non fixé à la construction** : un champ dont le type **est** un paramètre générique
  (`Some(value: T)`) n'est pas vérifié au constructeur ; son contrat s'applique au franchissement d'une frontière typée
  (voir « Champs de payload typés » et « Paramètres génériques »). Un champ **concret** (`x: int`) est vérifié dès la
  construction.
- **Paramètre imbriqué dans un composite** : `items: list[T]` vérifie le conteneur, pas encore l'élément.
- **Traits / `implements`** : non supportés.
