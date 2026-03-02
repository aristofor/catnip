# Mécaniques débloquées ~ 5 minutes

> Objectif : absorber 100% des primitives Catnip en 5 minutes. Après ça, tu as littéralement **tout** : fonctions,
> blocs, match avancé, pragmas, broadcasting, modules Python. Le reste n'est que confort.

______________________________________________________________________

# 1. Opérateurs Avancés - arsenal complet

Catnip reprend **tout** l'arsenal Python, sans surprise inutile.

```catnip
jetpack = 2 ** 8         # puissance → 256
restes_de_cafe = 17 % 5  # modulo → 2

dragon_reveille = 5 > 3  # True
tasse_pleine = 10 <= 10  # True

interrupteur_and = True and False
interrupteur_or = True or False
bouton_stop = not True

# bitwise
masque = 5 & 3      # 1
fusion = 5 | 3      # 7
exclusive = 5 ^ 3   # 6
teleport = 5 << 1   # 10
```

Des opérateurs **explicites et prévisibles**. Pas de surprise patch-notes.

> Les opérateurs bitwise manipulent des représentations qui n'existent pas vraiment en surface, mais qui sont néanmoins
> matérialisées en mémoire. Une forme d'abstraction concrète.

______________________________________________________________________

# 2. Lambdas Avancées - récursion, variadique, composition

### Retour implicite

```catnip
aire = (largeur, longueur) => { largeur * longueur }
```

### Récursion

```catnip
factorielle = (n) => {
    if n <= 1 { 1 } else { n * factorielle(n - 1) }
}
print("5! =", factorielle(5))  # 120

```

> Cette fonction s'appelle elle-même jusqu'à ce qu'elle ne le fasse plus.

> Le critère d'arrêt est la seule chose qui empêche une récursion infinie, ce qui en fait paradoxalement la ligne la
> plus courte et la plus importante du programme.

> La récursion est une structure qui n'existe que conceptuellement, mais qui se matérialise pourtant dans la pile : une
> abstraction concrète, encore.

### Variadique

```catnip
sum_all = (*args) => {
    total = 0
    for x in args { total = total + x }
    total
}
```

### Lambdas d'ordre supérieur

```catnip
appliquer = (f, v) => { f(v) }
res = appliquer((x) => { x ** 2 }, 5)
```

> Les fonctions d'ordre supérieur manipulent d'autres fonctions comme valeurs. Le code écrit du code.

______________________________________________________________________

# 3. Blocs comme Expressions - scopes, retour implicite

Les blocs `{ … }` sont des **expressions**. Ils retournent la dernière valeur.

```catnip
resultat = {
    a = 10
    b = 20
    a + b
}
```

Les blocs créent aussi un **scope local** :

```catnip
boussole = "global"

test = () => {
    boussole = "local"
    boussole
}

test()        # local
boussole      # global
```

> À la fin du bloc, la variable locale cesse simplement d'exister, et la globale redevient visible.

______________________________________________________________________

# 4. Pattern Matching Avancé - OR patterns, guard, structure

### Guards + captures

<!-- check: no-check -->

```catnip
match note_du_jury {
    n if n >= 16 => { "Très bien" }
    n if n >= 14 => { "Bien" }
    n if n >= 12 => { "Assez bien" }
    n if n >= 10 => { "Passable" }
    _            => { "Insuffisant" }
}
```

### OR patterns

<!-- check: no-check -->

```catnip
match jour_de_vacances {
    1 | 2 | 3 | 4 | 5 => { "Semaine" }
    6 | 7             => { "Weekend" }
}
```

### Deep patterns

```catnip
match tuple(1, tuple(2, 3)) {
    (a, (b, c)) => { a + b + c }
}
```

### Struct patterns

```catnip
struct Point { x, y }

match Point(0, 5) {
    Point{x, y} if x == 0 => { "axe Y" }
    Point{x, y} => { x + y }
    _ => { "fallback" }
}
```

> Le match est ton commutateur logique universel.

______________________________________________________________________

# 5. Charger des Modules Python - Catnip contrôle l'écosystème

Tu peux exposer du Python dans Catnip en 1 ligne.

### Module Python

```python
# tools.py
def double(x):
    return x * 2
```

### Chargement dans Catnip

<!-- check: no-check -->

```catnip
# Dans le script :
tools = import("./tools.py")
result = tools.double(21)
```

> Catnip ne remplace pas Python. Il s'y **branche** comme un exosquelette syntaxique, plug-and-play.

______________________________________________________________________

# 6. Mode Verbeux - introspection totale

```bash
catnip -v script.cat
```

Tu vois :

- parsing
- transformation
- analyse sémantique
- exécution

Le mode verbeux expose chaque étape du pipeline. Le runtime commente sa propre mise en scène, sans gêne, en full debug.

______________________________________________________________________

# 7. Broadcasting - la forme la plus évoluée de transformation

### Rappel :

`.[op]` applique une opération **quelle que soit la dimension de la donnée**.

```catnip
data = list(1, 2, 3, 4)

boosted   = data.[* 2].[+ 3]
filtered  = data.[if > 2]
cleaned   = data.[* -1].[abs]
```

> Une seule forme de code, applicable à toutes les dimensions. Pas de branches. Pas de complexité supplémentaire. *C'est
> la signature Catnip.*

______________________________________________________________________

# 8. Exemple complet - pipeline dimension-agnostique

```catnip
temperatures = list(22, 84, -3, 48, 91)

critical   = temperatures.[if > 80]
stabilized = temperatures.[+ 5].[* 0.9]
```

Même expression fonctionne :

- sur un scalaire
- sur une liste
- sur une colonne DataFrame
- sur un vector NumPy
- sur un tensor PyTorch
- sur d'autres objets que ton application décidera de fournir

> C'est du polymorphisme sans théorie - juste une notation qui marche, en mode production.

______________________________________________________________________

# 9. Exemple complet - FizzBuzz version Catnip

```catnip
fizzbuzz = (max) => {
    for i in range(1, max + 1) {
        match i {
            n if n % 15 == 0 => { print(i, "FizzBuzz") }
            n if n % 3  == 0 => { print(i, "Fizz") }
            n if n % 5  == 0 => { print(i, "Buzz") }
            n                => { print(i) }
        }
    }
}

fizzbuzz(30)
```

> Le match rend FizzBuzz enfin lisible. Oui, il fallait un langage entier pour ça.

______________________________________________________________________

# Ce que tu maîtrises après 5 minutes

- opérateurs complets
- lambdas avancées (récursion, variadique, HOF)
- blocs-expressions
- pattern matching avancé
- modules Python
- mode verbeux
- broadcasting dimension-agnostique

Tu es officiellement **autonome en Catnip**.

## Références

- [Langage (référence)](../lang/index.md)
- [Scopes et variables](../lang/SCOPES_AND_VARIABLES.md)
- [Pragmas](../lang/PRAGMAS.md)
- [Broadcasting](../lang/BROADCAST.md)
- [Module loading](../user/MODULE_LOADING.md)
- [CLI](../user/CLI.md)
