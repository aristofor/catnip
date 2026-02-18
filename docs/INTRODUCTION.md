# Introduction à Catnip

## Ambition et Motivation

Catnip est un **langage de script sandboxé**, conçu pour vivre **à l'intérieur** d'applications Python : un moteur
discret, compact, dimension-agnostique, pensé pour manipuler des données sans réintroduire de complexité accidentelle.

> Un locataire silencieux, mais qui améliore systématiquement la plomberie interne.

> Dans le moteur de Catkin, je modelais une grammaire pour les DataFrames et les build scripts.
> J'ai glissé sur une backref. Et Catnip était là, avec sa propre syntaxe.

Objectif :

1. **Scripter en sécurité** - sans exposer l'hôte
1. **Simplifier pandas** - en supprimant `.loc[…].apply(lambda)` comme routine quotidienne
1. **Unifier les dimensions** - scalaires, listes, DataFrames, tensors : une seule forme
1. **S'intégrer dans Python** - pas d'écosystème séparé, pas de rupture cognitive

## Modes d'utilisation

Catnip propose trois modes d'utilisation, **avec l'embedding comme cas d'usage principal** :

### Embedded (Mode DSL)

Catnip est avant tout un **moteur embedded**. Intégrez-le dans votre application Python pour :

- Permettre aux utilisateurs de définir des règles métier sans recompilation
- Créer des DSL personnalisés (validation, ETL, workflows)
- Exécuter des scripts utilisateur dans un sandbox sécurisé
- Configurer des pipelines de transformation de données

**Exemples** : moteur de règles pricing, validation de configuration, workflows Flask, DSL pandas.

Voir [`docs/examples/embedding/`](examples/embedding/) pour 9 exemples complets.

### Standalone (Scripts)

Catnip peut aussi s'utiliser comme langage de script autonome via CLI :

- Scripts de traitement de données
- Configuration DSL pour applications
- Automatisation de tâches

**Note** : ce mode est secondaire. Si vous écrivez beaucoup de scripts Catnip standalone, vous utilisez probablement Catnip pour un cas où Python serait plus adapté.

### REPL (Exploration)

REPL interactif pour :

- Explorer la syntaxe Catnip
- Débugger des scripts
- Calculs rapides

**Note** : le REPL est un outil de développement, pas un mode d'usage principal.

______________________________________________________________________

# Héritage direct de Python

Catnip sait accueillir ce que connaissent déjà les développeurs Python :

```catnip
reponse = 42
pi = 3.14
user = "Capitaine Whiskers"
msg = f"Hello, {user}!"
coords = tuple(10, 20)
config = dict(mode="debug")
```

Pas de rééducation, pas de dialecte exotique. La familiarité est conservée, la complexité réduite.

______________________________________________________________________

# Note importante sur les littéraux structurés

Catnip reprend les *constructeurs* Python (`list()`, `tuple()`, `dict()`, `set()`), mais **pas** les syntaxes littérales
inline (`[ ]`, `{ }`, `( )` pour tuples littéraux).

C'est un choix technique :

> Moins de grammaire. Moins de conflits. Moins de branches. Une surface plus stable.

| Type  | Python inline | Constructeur Python | Catnip         |
| ----- | ------------- | ------------------- | -------------- |
| Liste | `[1,2,3]`     | `list((1,2,3))`     | `list(1,2,3)`  |
| Tuple | `(1,2,3)`     | `tuple((1,2,3))`    | `tuple(1,2,3)` |
| Set   | `{1,2,3}`     | `set((1,2,3))`      | `set(1,2,3)`   |
| Dict  | `{"k":v}`     | `dict([("k",v)])`   | `dict(k=v)`    |

Pourquoi ?

- la notation `.[…]` (broadcasting) est prioritaire
- les scripts Catnip reçoivent surtout des structures venant du Python hôte
- éviter les collisions syntaxiques
- préserver un langage minimal, donc robuste

Ce n'est pas une perte : c'est un renforcement d'invariants.

______________________________________________________________________

# Les compréhensions existent via broadcasting

Catnip n'a pas de list comprehensions. Il a **mieux** : une notation unifiée applicable à toutes les dimensions.

```catnip
doubled  = numbers.[* 2]
filtered = numbers.[if > 5]
result   = numbers.[if > 0].[* 2].[+ 10]
```

Cette forme unique remplace simultanément :

- les list comprehensions
- les filtrages
- les `map` / `filter`
- les pipelines
- les transformations DataFrame
- les opérations tensor/array

> Une seule forme conceptuelle pour toutes les dimensions. Le polymorphisme sans cérémonie.

______________________________________________________________________

# Ce qui distingue Catnip

## 1. Pattern Matching - épuré, fonctionnel

```catnip
match value {
    0 => { "zero" }
    n if n < 0 => { "negatif" }
    n => { "positif" }
}
```

## 2. Blocs-expressions - comme Rust et Scala

```catnip
potions = {
    chaudron = init()
    transform(chaudron)
}
```

## 3. Lambdas modernes - syntaxe arrow

```catnip
double = (x) => { x * 2 }
```

## 4. Broadcasting : un pipeline pour tous les univers dimensionnels

> Un seul code pour tous les univers parallèles.

> L'absolu s'applique absolument partout.

Catnip applique une opération dans toutes les dimensions simultanément : scalaires, listes, matrices, tensors, colonnes
de DataFrame, structures hôtes inconnues…

La dimension cesse d'être un paramètre : elle devient un détail d'implémentation.

```catnip
result = data.[if > -10].[* 2].[abs]
```

Schrödinger serait fier : le filtrage existe dans toutes les dimensions en même temps.

Une lambda appliquée partout, c'est presque du calcul lambda - industrialisé.

Principe

- une seule forme de code pour tous les cas (scalaire, 1D, 2D, nD, ∞D).
- aucune branche conditionnelle.
- aucun test de type.
- une expression unique, stable, prévisible.

> C'est du code polymorphe qui ne connaît même pas le mot "polymorphisme".

______________________________________________________________________

# Sources d'inspiration

- **APL** : opérations vectorielles universelles
  > les hiéroglyphes aliens, très puissants
- **NumPy** : broadcasting automatique
  > l'art de faire croire que `[1,2,3] + 10` a du sens
- **MATLAB** : opérations élément par élément
  > le `. *` partout, mais ici sans surcharge mentale
- **R** : "tout est un vecteur"
  > même quand ça ne devrait pas l'être

Catnip reprend ces principes, les simplifie, les unifie et surtout :

**il les rend applicables à n'importe quelle structure fournie par l'hôte Python.**

## Ce que Catnip emprunte aux autres langages

Catnip ne sort pas de nulle part. Il assume ses influences, puis les simplifie.

**Tail-call optimization - Scheme, Scala**

Catnip applique automatiquement la TCO sur les appels récursifs terminaux : la fonction ne s'empile pas, elle réutilise
son propre cadre d'exécution.

```catnip
factorial = (n, acc = 1) => {
    if n <= 1 { acc } else { factorial(n - 1, n * acc) }
}
```

Même pour `factorial(100000, 1)`, la pile reste en O(1).

C'est de la récursion qui se comporte comme une boucle.

**Pragmas - C, Rust**

Des directives internes permettent d'ajuster le comportement du moteur sans changer le langage lui-même :

```catnip
pragma("tco", True)                 # activer/désactiver TCO
pd = import("pandas")               # exposer un module Python (pur ou binaire)
tools = import("./tools.cat")      # charger un module Catnip local
```

L'API reste stable, l'exécution reste configurable.

**Contexte implicite - Perl, Unix**

Catnip s'inspire aussi du `$_` de Perl et de la culture des pipes Unix :

certains usages favorisent un *contexte implicite* (valeur courante, flux en cours, résultat précédent), pour réduire le
bruit sans sacrifier la lisibilité.

> Héritage assumé, surface réduite, comportement prévisible.

______________________________________________________________________

# Critères de conception

Réduction du nombre de branches

- if isinstance(…) éliminé

Unification de la forme du code

- une seule syntaxe pour toutes les dimensions

Localité cognitive

- toute la logique dans une expression

Surface de code réduite

- moins de code = moins de bugs

> Si tu veux faire du *beau*, fais de l'ASCII-art. Catnip privilégie l'efficacité.

Ce sont des critères objectifs, pas esthétiques. L'esthétique rouille. La cohérence tient la soudure.

______________________________________________________________________

# Ce que Catnip promet

- des scripts *assez* familiers pour développeurs Python
- une syntaxe stricte, minimale, prévisible
- une unification dimensionnelle rare
- zéro surcharge cognitive
- interop totale avec Python
- une mécanique propre pour transformer des données

Catnip n'est pas là pour faire joli. Il est là pour **faire juste**.

Bienvenue dans Catnip.
