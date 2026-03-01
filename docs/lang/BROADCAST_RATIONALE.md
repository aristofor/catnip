# BROADCAST_RATIONALE

Voir aussi : [BROADCAST_SPEC.md](./BROADCAST_SPEC.md) pour la syntaxe normative et la sémantique.

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

<!-- check: no-check -->

```catnip
result = x.[op]
```

**Propriété clé** : l'opération s'adapte à la dimension des données, sans connaître le type à l'avance. Le code
fonctionne pareil sur scalaires, listes ou structures imbriquées.

Cette unification réduit le nombre de cas à traiter et supprime les branches conditionnelles basées sur le type.

Le broadcasting permet d'appliquer des opérations scalaires à des collections (listes, tuples) sans branches
conditionnelles. Trois modes sont disponibles : **map** (transformation), **filter** (filtrage conditionnel), et
**masques booléens** (indexation).

> Note culturelle : oui, on connaît la trilogie map/filter/reduce des langages à "surcouche typée". Ici, c'est la même
> famille d'idées, sans inflation de plomberie.

```catnip
data = list(3, 8, 2, 9, 5)

data.[* 2]       # Map: [6, 16, 4, 18, 10]
data.[> 5]       # Map: [False, True, False, True, False]
data.[if > 5]    # Filter: [8, 9]
data.[list(True, False, True, False, False)]  # Masque: [3, 2]

# Opérateurs ND
data.[~> abs]    # ND-map: applique abs à chaque élément
data.[~~ (n, recur) => { if n <= 1 { 1 } else { n * recur(n - 1) } }]  # ND-recursion: factorielle
```

> Achievement unlocked: Compréhension partielle du broadcasting.

______________________________________________________________________

## Objectif

Définir une notation qui applique des opérations scalaires à des objets de dimension indéterminée (scalaires, vecteurs,
matrices, DataFrames) **sans branches conditionnelles**.

Le broadcasting permet d'écrire une seule expression qui fonctionne aussi bien sur un scalaire que sur une collection
(liste, tuple, ou tout itérable), sans branches conditionnelles. Pour les objets multi-dimensionnels (arrays,
DataFrames…), la dimension exacte est gérée soit par la bibliothèque sous-jacente, soit par la fonction appliquée.

En pratique, cela remplace plusieurs blocs `if isinstance(…)` par une seule expression `A.[op M]`, ce qui réduit le
volume de code et le nombre de chemins d'exécution à maintenir.

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

<!-- check: no-check -->

```catnip
# Code linéaire, sans branches
result = df.query("age > 30")["salary"]
doubled = 2.[result]  # Fonctionne peu importe le type
```

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

**Inspiration** : Julia est très proche de la notation Catnip !

### APL/J

```apl
⍝ APL - tout est array par défaut
x ← 1 2 3 4 5
y ← x × 2  ⍝ Broadcasting implicite
```

**Avantage** : Concis, mais moins lisible.

______________________________________________________________________

### Propriétés Théoriques

**Note théorique** : Ces propriétés correspondent à la naturalité d'une transformation dans un topos de faisceaux
(Johnstone, [*Sketches of an Elephant*](https://math.jhu.edu/~eriehl/ct/sketches-of-an-elephant.pdf), vol. 2, C2.1). Un
broadcast récursif est une transformation naturelle qui :

- Préserve la structure catégorique (le "shape" du tensor)
- Se comporte uniformément à chaque niveau d'imbrication
- Compose de manière associative (ordre d'application indifférent)

Cette base théorique garantit que, pour les fonctions pures, l'ordre de récursion (par ligne, par colonne, en profondeur
d'abord, etc.) produit toujours le même résultat final. C'est ce qui permet :

- D'optimiser l'exécution sans changer la sémantique
- De paralléliser le traitement (pas de dépendances entre branches)
- De prévoir le comportement sans exécuter (raisonnement équationnel)

> +1 Life. Tu as survécu à cette section sans segmentation fault.
