# Early Game - 2 Minutes

> Objectif : absorber 80% de Catnip avant que ton café ne refroidisse. Deux minutes. Pas plus. Pas moins. Juste la dose.

______________________________________________________________________

## 1. Valeurs & Opérations - la base dimensionnelle

```catnip
voltage = 42
entropy = voltage * 2
print(entropy)  # → 84
```

Catnip applique exactement ce que tu écris : valeur, opération, résultat. Pas d'esthétique superflue.

> La beauté vient après, jamais avant.

______________________________________________________________________

## 2. Abstraction & Application - forme compacte, impact maximal

```catnip
amp = (signal) => { signal * 2 }
amp(21)  # 42
```

Une lambda Catnip :

- se définit vite
- retourne la dernière expression
- capture son scope
- se fiche totalement de la dimension du signal

> Une lambda nommée perd son anonymat, mais pas sa nature. Dans Catnip, l'identité ne vient pas du nom mais de la forme.

Tu viens de voir deux primitives d'un coup : **abstraction** (définir) et **application** (appeler).

______________________________________________________________________

## 3. Boucles - direct, sans décor

### For loop

```catnip
for i in range(1, 6) {
    print(i)
}
```

### While loop

```catnip
tic = 0
while tic < 3 {
    print("Tick:", tic)
    tic = tic + 1
}
```

Boucle minimale, attitude maximale : **bloc + condition + répétition**.

## 4. Conditions - le tri logique le plus sobre possible

```catnip
age = 25
if age < 18 {
    print("Mineur")
} else {
    print("Adulte")
}
```

Aucune ambiguïté, aucune indentation capricieuse. Les blocs sont explicites et retournent leur dernière valeur si
utilisés comme expressions. Un `if` est un bloc qui choisit.

______________________________________________________________________

## 5. Pattern Matching - la machine de tri universelle

<!-- check: no-check -->

```catnip
pulse = read_sensor()

match pulse {
    0 => { print("No signal - sector offline") }
    n if n < 0 => { print("Reverse flux:", n) }
    n if n > 100 => { print("Overload detected:", n) }
    _ => { print("Nominal:", pulse) }
}
```

Le wildcard `_` capture les cas non couverts explicitement, y compris ceux sortis d'un univers parallèle.

> `match` remplace les cascades de `if/elif`. Moins de branches mentales, plus de signal utile.

______________________________________________________________________

## Premier Script Complet

Crée `born_to_segfault.cat` :

```catnip
# Fonction lambda
hello = (nom) => {
    print("BORN TO SEGFAULT,", nom, "!")
}

hello("Monde")

for robot in range(1, 4) {
    print("Compteur:", robot)
}
```

Exécute :

```bash
catnip born_to_segfault.cat
```

Tu viens d'écrire ton premier programme Catnip. Dimension-agnostique, simple, propre, sans bruit.

______________________________________________________________________

## Et Après ?

> Tu connais maintenant cinq primitives qui permettent de construire n'importe quel programme, y compris un programme
> qui génère ce guide, qui pourrait lui-même générer un guide expliquant comment générer des guides. À partir d'ici, il
> s'agit seulement de les combiner, de les imbriquer, et de les simplifier, dans des configurations de moins en moins
> absurdes jusqu'à obtenir quelque chose d'utile.

## Teaser : Broadcasting Dimension-Agnostique

*(Le vrai superpouvoir, en 5 secondes seulement.)*

```catnip
result = list(1, 2, 3).[* 2]
# → list(2, 4, 6)
```

Même expression pour :

- un scalaire
- une liste
- un tensor
- une colonne entière de DataFrame
- et probablement des choses encore non inventées

> Une seule forme de code pour tous les univers parallèles. Schrödinger aurait adoré.

______________________________________________________________________

## Prochaine étape

Tu maîtrises maintenant :

- variables
- lambdas
- boucles
- conditions
- pattern matching

Tu es à 60% de Catnip. Les 40% restants tiennent dans le guide suivant :

**[Guide 5 minutes](QUICKSTART_5MIN.md)**

## Références

- [Langage (référence)](../lang/index.md)
- [Scopes et variables](../lang/SCOPES_AND_VARIABLES.md)
- [Broadcasting](../lang/BROADCAST.md)
