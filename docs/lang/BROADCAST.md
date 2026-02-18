# Notation de Broadcasting `target.[op operand]`

## Pourquoi le Broadcasting ?

Le broadcasting évite de multiplier les cas par type quand on manipule des données.

Sans broadcasting, chaque opération nécessite des branches conditionnelles :

```python
if isinstance(x, list):
    result = [op(item) for item in x]
elif isinstance(x, pd.Series):
    result = x.apply(op)
elif isinstance(x, (int, float)):
    result = op(x)
```

Avec broadcasting, une seule notation :

```catnip
result = x.[op]
```

**Propriété clé** : l'opération s'adapte à la dimension des données, sans connaître le type à l'avance. Le code fonctionne pareil sur scalaires, listes ou structures imbriquées.

Cette unification réduit le nombre de cas à traiter et supprime les branches conditionnelles basées sur le type.

Le broadcasting permet d'appliquer des opérations scalaires à des collections (listes, tuples) sans branches
conditionnelles. Trois modes sont disponibles : **map** (transformation), **filter** (filtrage conditionnel), et
**masques booléens** (indexation).

```catnip
data = list(3, 8, 2, 9, 5)

data.[* 2]       # Map: [6, 16, 4, 18, 10]
data.[> 5]       # Map: [False, True, False, True, False]
data.[if > 5]    # Filter: [8, 9]
data.[list(True, False, True, False, False)]  # Masque: [3, 2]

# Opérateurs ND
data.[@> abs]    # ND-map: applique abs à chaque élément
data.[@@ (n, recur) => { if n <= 1 { 1 } else { n * recur(n - 1) } }]  # ND-recursion: factorielle
```

> Achievement unlocked: Compréhension partielle du broadcasting.

______________________________________________________________________

## Objectif

Définir une notation qui applique des opérations scalaires à des objets de dimension indéterminée (scalaires,
vecteurs, matrices, DataFrames) **sans branches conditionnelles**.

Le broadcasting permet d'écrire une seule expression qui fonctionne aussi bien sur un scalaire que sur une collection
(liste, tuple, ou tout iterable), sans branches conditionnelles. Pour les objets multi-dimensionnels (arrays,
DataFrames…), la dimension exacte est gérée soit par la bibliothèque sous-jacente, soit par la fonction appliquée.

En pratique, cela remplace plusieurs blocs `if isinstance(…)` par une seule expression `A.[op M]`, ce qui réduit le
volume de code et le nombre de chemins d'exécution à maintenir.

## Le Concept

### Notation et Opérations

Le broadcasting applique des opérations sur des collections (listes, tuples, etc.) de manière uniforme, sans branches
conditionnelles.

**Cinq modes d'opération** :

| Syntaxe              | Mode               | Résultat                                      | Exemple                                         |
| -------------------- | ------------------ | --------------------------------------------- | ----------------------------------------------- |
| `data.[op value]`    | **Map**            | Collection de même taille avec transformation | `list(1,2,3).[* 2]` → `[2,4,6]`                 |
| `data.[if op value]` | **Filter**         | Collection filtrée (taille ≤ originale)       | `list(1,2,3).[if > 1]` → `[2,3]`                |
| `data.[mask]`        | **Masque booléen** | Collection filtrée par masque                 | `list(1,2,3).[list(True,False,True)]` → `[1,3]` |
| `data.[@> f]`        | **ND-map**         | Applique `@>` à chaque élément                | `list(-1,-2,3).[@> abs]` → `[1,2,3]`            |
| `data.[@@ lambda]`   | **ND-recursion**   | Applique `@@` à chaque élément                | `list(3,5).[@@ factorial]` → `[6,120]`          |

### Principe

Le broadcasting permet d'écrire des opérations qui portent **indifféremment** sur des scalaires ou des collections, sans
embranchements logiques (`if`/`elif`/`else`).

Une seule expression fonctionne que les données soient :

- Un scalaire (int, float, str, bool)
- Une liste Python
- Un tuple
- Tout autre itérable

______________________________________________________________________

## Motivation

### Problème Actuel (Python/Pandas)

Quand on travaille avec des DataFrames pandas, le résultat d'une opération peut être :

- Un scalaire
- Une Series
- Une DataFrame

Cela force à écrire du code avec branches :

```python
# Code verbeux et répétitif
result = df.query("age > 30")["salary"]

if isinstance(result, pd.Series):
    doubled = result * 2
elif isinstance(result, pd.DataFrame):
    doubled = result * 2
elif isinstance(result, (int, float)):
    doubled = result * 2
else:
    raise TypeError("Type non supporté")
```

### Solution avec M.[A]

```catnip
# Code linéaire, sans branches
result = df.query("age > 30")["salary"]
doubled = 2.[result]  # Fonctionne peu importe le type
```

______________________________________________________________________

## Syntaxe Implémentée : `A.[op M]`

La syntaxe **Option 2 : Opération à Droite** a été choisie et implémentée.

### Syntaxe

```catnip
target.[operator operand]  # Opération binaire (map)
target.[operator]           # Opération unaire (map)
target.[lambda]             # Lambda/fonction (map)
target.[if operator operand]  # Filtrage conditionnel
target.[if lambda]          # Filtrage par lambda
target.[boolean_mask]       # Indexation par masque booléen
```

### Exemples Fonctionnels

```catnip
# Multiplication
doubled = data.[* 2]

# Addition
shifted = values.[+ 10]

# Puissance
squares = data.[** 2]

# Comparaison (retourne booléens)
masks = data.[> 3]

# Filtrage conditionnel (retourne éléments)
filtered = data.[if > 3]

# Lambda
tripled = data.[(x) => { x * 3 }]
```

### Avantages de Cette Syntaxe

- **Chaînage explicite** : `data.[* 2].[+ 10]`
- **Style orienté données** : Lecture gauche à droite
- **Familier** : Rappelle `.apply()` de pandas
- **Lisible** : L'objet traité vient en premier

______________________________________________________________________

### Broadcast sur scalaires littéraux

La notation de broadcast `.[…]` s'applique aussi aux **scalaires littéraux**. Un littéral (ex: `5`, `3.14`, `"foo"`) est
un `primary` et peut donc chaîner des membres, y compris un broadcast.

```catnip
5.[+ 10]        # → 15
(2 + 3).[* 2]   # → 10
5.[abs]         # → 5

# Filtrage sur un scalaire : retourne une liste
5.[if > 0]      # → list(5)
5.[if < 0]      # → list()  (liste vide)
```

## Map, Filter et Masques Booléens

### Distinction Map vs Filter

Le broadcasting supporte deux modes distincts :

**Map (transformation)** : Transforme chaque élément et retourne une collection de même taille

```catnip
data = list(3, 8, 2, 9, 5)
masks = data.[> 5]      # [False, True, False, True, False]
```

**Filter (filtrage)** : Ne garde que les éléments qui satisfont une condition

```catnip
data = list(3, 8, 2, 9, 5)
filtered = data.[if > 5]  # [8, 9]
```

La différence est critique pour le chaînage d'opérations :

```catnip
# PIÈGE : map puis multiply donne 0 et 2
data.[> 5].[* 2]       # [0, 2, 0, 2, 0]  (False*2=0, True*2=2)

# CORRECT : filter puis multiply
data.[if > 5].[* 2]    # [16, 18]  (garde 8 et 9, puis multiplie)
```

### Indexation par Masque Booléen

Un masque booléen (liste/tuple de bool) peut être utilisé pour filtrer :

```catnip
data = list(10, 20, 30, 40)
mask = list(True, False, True, False)
result = data.[mask]    # [10, 30]
```

Workflow suggéré : générer un masque puis le réutiliser

```catnip
data = list(3, 8, 2, 9, 5)
mask = data.[> 5]       # [False, True, False, True, False]
result = data.[mask]    # [8, 9] - équivaut à data.[if > 5]
```

### Erreurs possibles

Masque de longueur incompatible :

```catnip
data = list(10, 20, 30)
mask = list(True, False)
data.[mask]  # Error: Mask size mismatch: target has 3 elements, mask has 2
```

Masque non booléen :

```catnip
data = list(10, 20, 30)
mask = list(1, 0, 1)
data.[mask]  # Error: Mask must be a list or tuple of booleans, got list with non-boolean elements
```

Réutilisation du même masque sur plusieurs collections

```catnip
data1 = list(10, 20, 30, 40)
data2 = list("a", "b", "c", "d")
mask = data1.[> 20]     # [False, False, True, True]
result1 = data1.[mask]  # [30, 40]
result2 = data2.[mask]  # ["c", "d"]
```

### Filtrage avec Lambdas

Le filtrage conditionnel supporte les lambdas arbitraires :

```catnip
# Nombres pairs
data = list(1, 2, 3, 4, 5, 6)
pairs = data.[if (x) => { x % 2 == 0 }]  # [2, 4, 6]

# Conditions complexes
data = list(-5, 3, -2, 8, 0, -1)
result = data.[if (x) => { x > 0 and x < 5 }]  # [3]
```

### Préservation du Type

Le filtrage préserve le type de la collection d'origine :

```catnip
# Liste → Liste
data_list = list(1, 2, 3, 4)
result = data_list.[if > 2]  # [3, 4] (type: list)

# Tuple → Tuple
data_tuple = tuple(1, 2, 3, 4)
result = data_tuple.[if > 2]  # (3, 4) (type: tuple)
```

> **Note sur l'invariance des collections** : Les opérations de broadcasting préservent toujours le type de la
> collection d'origine. Une liste reste une liste, un tuple reste un tuple - bref, rien ne se transforme subitement en
> autre chose pendant le trajet. Aucune démarche n'est nécessaire : un changement de type n'est autorisé que lorsqu'il
> est explicitement demandé. Cette règle s'applique récursivement, sauf si elle s'applique déjà, auquel cas elle
> s'applique quand même.

## Opérations Supportées

### Opérateurs Arithmétiques

#### Broadcasting Scalaire

```catnip
# Multiplication
2.[x]          # x * 2
x.[* 2]        # x * 2

# Addition
10.[x]         # x + 10
x.[+ 10]       # x + 10

# Soustraction
100.[- x]      # 100 - x
x.[- 50]       # x - 50

# Division
1.[/ x]        # 1 / x
x.[/ 2]        # x / 2

# Puissance
2.[** x]       # 2 ** x
x.[** 2]       # x ** 2

# Modulo
x.[% 10]       # x % 10
```

#### Broadcasting Liste-à-Liste

Opérations élément-par-élément entre deux collections :

```catnip
a = list(1, 2, 3)
b = list(10, 20, 30)

# Addition élément par élément
a.[+ b]        # [11, 22, 33]

# Multiplication élément par élément
a.[* b]        # [10, 40, 90]

# Division élément par élément
b.[/ a]        # [10.0, 10.0, 10.0]

# Puissance élément par élément
a.[** b]       # [1**10, 2**20, 3**30]
```

Les deux collections doivent avoir la même taille, sinon le résultat s'arrête à la plus courte (comportement `zip`).

### Opérateurs de Comparaison

#### Map (retourne booléens)

```catnip
# Supérieur
x.[> 0]        # retourne [True/False pour chaque x]

# Inférieur
x.[< 100]      # retourne masque booléen

# Égalité
x.[== 42]      # retourne masque booléen

# Différent
x.[!= 0]       # retourne masque booléen
```

#### Filter (retourne éléments)

```catnip
# Filtrer éléments supérieurs à 0
x.[if > 0]     # retourne seulement les éléments > 0

# Filtrer éléments inférieurs à 100
x.[if < 100]   # retourne seulement les éléments < 100

# Filtrer éléments égaux à 42
x.[if == 42]   # retourne seulement les 42

# Filtrer éléments différents de 0
x.[if != 0]    # retourne seulement les non-zéros
```

### Fonctions Unaires

```catnip
abs.[x]        # abs(x)
sqrt.[x]       # sqrt(x)
log.[x]        # log(x)
exp.[x]        # exp(x)
round.[x]      # round(x)
```

### Lambdas

```catnip
# Lambda simple
((x) => { x * 2 }).[data]

# Lambda complexe
((x) => {
    if x > 0 {
        x * 2
    } else {
        0
    }
}).[data]
```

______________________________________________________________________

## Cas d'Usage

### 1. Traitement de Données Pandas

```catnip
# Charger module pandas
# import("./pandas_helper.py") as pd

# Obtenir des données (peut être scalaire, Series, ou DataFrame)
data = pd.query(df, "age > 30")["salary"]

# Doubler les valeurs - fonctionne dans tous les cas
doubled = 2.[data]

# Normalisation
mean_val = pd.mean(data)
std_val = pd.std(data)
normalized = (data.[- mean_val]).[/ std_val]
```

### 2. Composition d'Opérations

```python
# Sans broadcasting - code verbeux
threshold = 100
if isinstance(data, pd.Series):
    temp = abs(data - threshold)
    result = temp.mean()
elif isinstance(data, (int, float)):
    temp = abs(data - threshold)
    result = temp
else:
    # gérer DataFrame…

# Avec broadcasting - une seule expression, sans duplication
threshold = 100
result = mean.[abs.[data.[- threshold]]]
```

### 3. Traitement Conditionnel

```catnip
# Filtrer les valeurs positives
positive = data.[if > 0]

# Remplacer les valeurs négatives par 0 (nécessite lambda car transformation)
positive = data.[(x) => { if x > 0 { x } else { 0 } }]

# Ou avec pattern matching
positive = data.[(x) => {
    match x {
        n if n > 0 => { n }
        _ => { 0 }
    }
}]
```

### 4. Agrégations avec Broadcasting

```catnip
# Centrage des données
centered = data.[- mean.[data]]

# Normalisation min-max
min_val = min.[data]
max_val = max.[data]
normalized = (data.[- min_val]).[/ (max_val - min_val)]
```

### 5. Workflows avec Masques

```catnip
# Générer un masque et le réutiliser
data1 = list(10, 20, 30, 40, 50)
data2 = list("a", "b", "c", "d", "e")

# Créer masque pour valeurs > 25
high_mask = data1.[> 25]  # [False, False, True, True, True]

# Appliquer le même masque aux deux listes
high_values = data1.[high_mask]  # [30, 40, 50]
high_labels = data2.[high_mask]  # ["c", "d", "e"]
```

______________________________________________________________________

## Implémentation

### Détection Automatique de Type et d'Opération

Le système détecte automatiquement :

**Type de target** :

1. **Scalaire Python** (`int`, `float`, `str`, `bool`, `None`)
   - Application directe de l'opération
1. **Liste/Tuple Python**
   - Itération optimisée avec préservation du type
1. **Autres iterables**
   - Tentative d'itération, sinon traité comme scalaire

**Type d'opération** :

1. **Masque booléen** (`target.[bool_list]`)
   - Détecté si l'opérande est une liste/tuple de booléens
   - Applique filtrage par masque via `filter_by_mask()`
1. **Filtrage conditionnel** (`target.[if condition]`)
   - Détecté par le flag `is_filter=True` dans le nœud `Broadcast`
   - Applique filtrage via `filter_conditional()`
1. **Map standard** (`target.[op value]` ou `target.[lambda]`)
   - Applique transformation via `broadcast_map()`

### Pseudo-code

```python
def execute_broadcast(broadcast_node, context):
    """Exécute une opération de broadcasting."""

    # Évaluer la cible
    target = execute(broadcast_node.target, context)

    # Cas 1 : Masque booléen (détection automatique)
    if is_boolean_mask(broadcast_node.operand):
        mask = execute(broadcast_node.operand, context)
        return filter_by_mask(target, mask)

    # Cas 2 : Filtrage conditionnel (.[if condition])
    elif broadcast_node.is_filter:
        # Créer fonction de condition
        condition_func = make_condition_func(
            broadcast_node.operator,
            broadcast_node.operand,
            context
        )
        return filter_conditional(target, condition_func)

    # Cas 3 : Map standard (.[op value] ou .[lambda])
    else:
        # Créer fonction de transformation
        map_func = make_map_func(
            broadcast_node.operator,
            broadcast_node.operand,
            context
        )
        return broadcast_map(target, map_func)

def is_boolean_mask(operand):
    """Vérifie si operand est un masque booléen."""
    return (isinstance(operand, (list, tuple)) and
            all(isinstance(x, bool) for x in operand))

def filter_by_mask(target, mask):
    """Filtre par masque booléen."""
    if len(target) != len(mask):
        raise ValueError("Mask size mismatch")

    result = [t for t, m in zip(target, mask) if m]
    return tuple(result) if isinstance(target, tuple) else result

def filter_conditional(target, condition_func):
    """Filtre conditionnel."""
    # Scalaire
    if isinstance(target, (int, float, str, bool, None)):
        return [target] if condition_func(target) else []

    # Iterable
    result = [x for x in target if condition_func(x)]
    return tuple(result) if isinstance(target, tuple) else result

def broadcast_map(target, map_func):
    """Map standard."""
    # Scalaire
    if isinstance(target, (int, float, str, bool, None)):
        return map_func(target)

    # Liste
    if isinstance(target, list):
        return [map_func(x) for x in target]

    # Tuple
    if isinstance(target, tuple):
        return tuple(map_func(x) for x in target)

    # Autre iterable
    return [map_func(x) for x in target]
```

______________________________________________________________________

## Décisions Prises

### 1. Syntaxe Finale

**Choix** : `A.[op M]` (opération à droite)

**Raisons** :

- Chaînage naturel
- Style orienté données
- Familier pour utilisateurs pandas

### 2. Priorité des Opérateurs

Le broadcasting a la priorité d'accès membre (`.`)

```catnip
2 + data.[* 3]  # = 2 + (data.[* 3])
```

Équivalent à :

```catnip
2 + data.method()  # Priorité de . avant +
```

### 3. Type Safety et Erreurs

Les erreurs Python sont propagées

Si l'opération échoue sur un élément, une exception Python est levée.

```catnip
data = list("a", "b", "c")
result = data.[+ 2]  # TypeError: can only concatenate str (not "int") to str
```

## Performance : Fast Path SIMD

Pour les listes numériques homogènes (tous ints ou tous floats), le broadcasting contourne entièrement le protocole d'appel Python. Les valeurs sont extraites dans un `Vec` Rust contigu, l'opération est appliquée en boucle serrée auto-vectorisée par LLVM (AVX2/SSE2), puis le résultat est reconstruit en une seule passe.

**Opérations accélérées** (map) :

- Arithmétique : `+`, `-`, `*`, `/`, `//`, `%`, `**`
- Comparaisons : `>`, `<`, `>=`, `<=`, `==`, `!=`

**Filtres accélérés** :

- Toutes les comparaisons dans `.[if op operand]`

**Conditions d'activation** :

- La cible est une `list` (pas tuple)
- Tous les éléments sont du même type numérique (int ou float, pas bool)
- L'opérande est un scalaire du type compatible

Si l'une de ces conditions n'est pas remplie, le chemin Python standard est utilisé, sans changement de sémantique. La division par zéro, le modulo par zéro et la puissance négative sur entiers déclenchent aussi un fallback vers Python pour produire les erreurs appropriées.

```catnip
# Fast path : liste homogène d'ints + opérateur arithmétique
list(1, 2, 3, 4, 5).[* 2]     # -> boucle Rust SIMD, pas d'appel Python par élément

# Fast path : filtre numérique
list(3, 8, 2, 9, 5).[if > 5]  # -> comparaison + collecte en Rust pur

# Fallback : liste hétérogène
list(1, "a", 3).[+ 1]         # -> chemin Python standard (TypeError sur "a")
```

> Le fast path extrait N valeurs, applique N opérations, et reconstruit N résultats, le tout sans franchir la frontière PyO3 une seule fois par élément. Le nombre d'appels Python passe de O(N) à O(1). La constante multiplicative restante est celle de LLVM essayant de décider s'il doit auto-vectoriser en AVX-512 ou simplement en AVX2, ce qui constitue techniquement une perte de temps mais à une échelle que seul un compilateur peut percevoir.

## Fonctions non-pures dans le broadcasting

Le broadcasting accepte **tout callable**, y compris les fonctions à effets de bord. Les fonctions suivantes du contexte fonctionnent techniquement dans un broadcast mais n'ont pas de sémantique de transformation :

- `import` -- charge des modules, modifie le contexte
- `jit` -- wrapper de compilation JIT
- `pure` -- décorateur de marquage
- `cached` -- wrapper de memoization
- `debug` -- introspection

```catnip
# Fonctionne, mais ce n'est pas du data flow
list("math", "json").[import]   # charge deux modules
```

Ces wrappers ne sont pas interdits dans le broadcasting, mais leur usage n'a pas de sens dans un pipeline de données. Préférer un appel direct :

```catnip
math = import("math")
json = import("json")
```

> Le broadcasting ne juge pas. Il applique la séquence.
> Trois modules à charger en parallèle, et la matrice administrative
> s'aligne d'elle-même.
> Champ confirmé, cachet quantifié, chargement effectif.

## Améliorations Futures

### 1. Support Pandas/NumPy

Détecter automatiquement pandas/numpy et utiliser leurs opérations optimisées.

### 3. Erreurs Intelligentes

Options possibles :

- Filtrer les valeurs incompatibles
- Retourner None/NaN pour les échecs
- Mode strict vs permissif

______________________________________________________________________

## Comparaison avec Autres Langages

### Python (Pandas)

```python
# Pandas
result = df['column'].apply(lambda x: x * 2)

# Ou vectorisé
result = df['column'] * 2
```

**Problème** : Il faut savoir si c'est un scalaire ou une Series.

### R

```r
# R a un broadcasting automatique
x <- c(1, 2, 3, 4, 5)
y <- x * 2  # Vectorisation automatique
```

**Avantage** : Broadcasting natif, mais syntaxe moins explicite.

### Julia

```julia
# Julia utilise .operator pour broadcasting
x = [1, 2, 3, 4, 5]
y = x .* 2  # Broadcasting explicite avec .
```

**Inspiration** : Julia est très proche de notre notation !

### APL/J

```apl
⍝ APL - tout est array par défaut
x ← 1 2 3 4 5
y ← x × 2  ⍝ Broadcasting implicite
```

**Avantage** : Concis, mais moins lisible.

______________________________________________________________________

## Statut d'Implémentation

### Fonctionnalités Futures

**Intégration Pandas/NumPy**

- [ ] Détecter pandas Series/DataFrame
- [ ] Détecter numpy arrays
- [ ] Utiliser opérations vectorisées pandas
- [ ] Utiliser broadcasting numpy natif

**Optimisations avancées**

- [ ] Fusion d'opérations : `.[op1].[op2]` → `.[op2∘op1]`
- [ ] Détection de fonctions pures pour parallélisation
- [ ] Broadcasting ND récursif automatique

**Extensions**

- [ ] Support sets et generators
- [ ] Gestion d'erreurs personnalisable (skip, NaN, strict)
- [ ] Modes de padding pour listes de tailles différentes

______________________________________________________________________

## Exemples Testés et Fonctionnels

### Exemples de Base

```catnip
# Sur scalaire
x = 5
doubled = x.[* 2]
print(doubled)  # 10

# Sur liste
data = list(1, 2, 3, 4, 5)
doubled = data.[* 2]
print(doubled)  # [2, 4, 6, 8, 10]

# Addition
plus_ten = data.[+ 10]
print(plus_ten)  # [11, 12, 13, 14, 15]

# Puissance
squares = data.[** 2]
print(squares)  # [1, 4, 9, 16, 25]

# Comparaison (retourne masque booléen)
gt_three = data.[> 3]
print(gt_three)  # [False, False, False, True, True]

# Filtrage (retourne éléments)
filtered = data.[if > 3]
print(filtered)  # [4, 5]
```

### Filtrage et Masques

```catnip
# Filtrage conditionnel
data = list(3, 8, 2, 9, 5)
high = data.[if > 5]
print(high)  # [8, 9]

# Génération et application de masque
mask = data.[> 5]
print(mask)  # [False, True, False, True, False]
filtered = data.[mask]
print(filtered)  # [8, 9]

# Réutilisation du masque
labels = list("a", "b", "c", "d", "e")
filtered_labels = labels.[mask]
print(filtered_labels)  # ["b", "d"]
```

### Lambdas

```catnip
# Lambda simple
data = list(1, 2, 3, 4, 5)
tripled = data.[(x) => { x * 3 }]
print(tripled)  # [3, 6, 9, 12, 15]

# Filtrage avec lambda
pairs = data.[if (x) => { x % 2 == 0 }]
print(pairs)  # [2, 4]

# Lambda avec transformation conditionnelle
clamped = data.[(x) => {
    if x > 3 {
        3
    } else {
        x
    }
}]
print(clamped)  # [1, 2, 3, 3, 3]
```

### Chaînage

```catnip
# Chaîner map et filter
data = list(1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
result = data.[if > 3].[* 2].[if < 15].[+ 1]
print(result)  # [9, 11, 13, 15]
# > 3: [4,5,6,7,8,9,10]
# * 2: [8,10,12,14,16,18,20]
# < 15: [8,10,12,14]
# + 1: [9,11,13,15]
```

### Broadcasting Liste-à-Liste

```catnip
# Opérations élément-par-élément
a = list(1, 2, 3)
b = list(10, 20, 30)

sum_lists = a.[+ b]
print(sum_lists)  # [11, 22, 33]

prod_lists = a.[* b]
print(prod_lists)  # [10, 40, 90]
```

### Avec Pandas

```catnip
# catnip -m pandas script.cat

df = pd.read_csv("data.csv")
prices = df["price"]

# Doubler les prix
doubled = prices.[* 2]

# Calculer la TVA (20%)
with_tax = prices.[* 1.20]

# Filtrer et normaliser
high_prices = prices.[> 100]
mean_price = pd.mean(high_prices)
normalized = high_prices.[- mean_price]
```

______________________________________________________________________

## Extension Future : ND-Broadcast Récursif

### Comportement Actuel

Le broadcast opère sur le niveau externe uniquement :

```catnip
# Liste simple - fonctionne directement
nums = list(1, 2, 3)
nums.[* 2]  # [2, 4, 6]

# Liste imbriquée - nécessite composition manuelle
matrix = list(list(1, 2), list(3, 4))
matrix.[(row) => { row.[* 2] }]  # [[2, 4], [6, 8]]
```

### Objectif : Récursion Automatique

Permettre une récursion naturelle sur les structures imbriquées :

```catnip
# Futur : broadcast récursif automatique
matrix = list(list(1, 2), list(3, 4))
matrix.[* 2]  # [[2, 4], [6, 8]] - récursion automatique

# Tensor 3D
cube = list(
    list(list(1, 2), list(3, 4)),
    list(list(5, 6), list(7, 8))
)
cube.[+ 10]  # Ajoute 10 à tous les scalaires, quelle que soit la profondeur
```

### Garanties Recherchées

**Naturalité** : Le résultat ne dépend pas de l'ordre de traitement des dimensions

- Parcourir par ligne puis colonne donne le même résultat que colonne puis ligne
- La profondeur de récursion est déterminée par la structure des données
- Pas de dépendance sur l'implémentation interne

**Composition** : Les opérations se composent de manière prévisible

- `A.[f].[g]` équivaut à `A.[(x) => { g(f(x)) }]` pour toute profondeur
- Pas d'effet de bord entre opérations successives
- Une seule forme de code pour les cas simples et complexes

**Préservation de structure** : Le "shape" du tensor reste identique

- La forme du tensor (dimensions, imbrication) est préservée
- Nombre de dimensions constant
- Seules les valeurs scalaires changent

### Propriétés Théoriques

**Note théorique** : Ces propriétés correspondent à la naturalité d'une transformation dans un topos de faisceaux
(Johnstone, [*Sketches of an Elephant*](https://math.jhu.edu/~eriehl/ct/sketches-of-an-elephant.pdf), vol. 2, C2.1). Un broadcast récursif est une transformation naturelle qui :

- Préserve la structure catégorique (le "shape" du tensor)
- Se comporte uniformément à chaque niveau d'imbrication
- Compose de manière associative (ordre d'application indifférent)

Cette base théorique garantit que, pour les fonctions pures, l'ordre de récursion (par ligne, par colonne, en profondeur
d'abord, etc.) produit toujours le même résultat final. C'est ce qui permet :

- D'optimiser l'exécution sans changer la sémantique
- De paralléliser le traitement (pas de dépendances entre branches)
- De prévoir le comportement sans exécuter (raisonnement équationnel)

> +1 Life. Tu as survécu à cette section sans segmentation fault.

### Exemple Concret

```catnip
# Tensor 3D : données OHLCV multi-actifs
# Structure : [actif][jour][ohlcv]
data = list(
    list(list(100, 105, 95, 102), list(102, 108, 101, 107)),  # actif 1
    list(list(50, 52, 48, 51), list(51, 53, 50, 52))          # actif 2
)

# Normaliser tous les prix
normalized = data.[/ 100]  # Récursion automatique

# Garantie de naturalité :
# Résultat identique que tu parcoures :
# - actif → jour → prix
# - jour → actif → prix
# - prix → jour → actif
```

### Décisions d'Implémentation

Le ND-broadcast respecte :

**Critère d'arrêt** : Quand considère-t-on une valeur comme "scalaire" ?

- Types de base : int, float, str, bool, None
- Pas de méthode `__iter__` (ou explicitement marqué non-itérable)
- Objets pandas/numpy : traités comme des scalaires de haut niveau (la récursion s'arrête là)

**Unification** : Même notation pour tous les niveaux

- `nums.[* 2]` fonctionne que nums soit scalaire, liste, ou tensor ND
- Pas de syntaxe spéciale pour le cas multi-dimensionnel
- Réduction du nombre de cas à traiter dans le code utilisateur

**Performance** : Optimisations possibles grâce à la naturalité

- Détection des fonctions pures (pas d'effet de bord)
- Parallélisation automatique (pas de dépendances)
- Fusion d'opérations successives (`.[f].[g]` → `.[g∘f]`)

> **Paradoxe d'optimisation** : La fusion d'opérations successives `.[f].[g]` en `.[g∘f]` nécessite qu'on détecte deux
> opérations avant de les fusionner. Mais si on les fusionne, il n'y a plus qu'une seule opération. La fusion doit donc
> s'appliquer à elle-même pour être complètement optimisée, créant ainsi une boucle de fusion infinie qui se résout en
> un point fixe où l'opération fusionne si vite qu'elle disparaît avant d'avoir existé. C'est l'optimisation ultime :
> O(0) opérations, zéro allocation mémoire, temps d'exécution négatif. En théorie. En pratique, on se contente de O(n)
> comme tout le monde.
