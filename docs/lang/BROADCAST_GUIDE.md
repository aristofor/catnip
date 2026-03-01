# BROADCAST_GUIDE

Voir aussi : [BROADCAST_SPEC.md](./BROADCAST_SPEC.md) pour la référence normative et
[BROADCAST_RUNTIME.md](./BROADCAST_RUNTIME.md) pour les détails d'implémentation.

## Cas d'Usage

### 1. Traitement de Données Pandas

<!-- check: no-check -->

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

<!-- check: no-check -->

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

<!-- check: no-check -->

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

## Exemples

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

<!-- check: no-check -->

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

### Exemple Concret

<!-- check: no-check -->

```catnip
# Tensor 3D : données OHLCV multi-actifs
# Structure : [actif][jour][ohlcv]
data = list(
    list(list(100, 105, 95, 102), list(102, 108, 101, 107)),  # actif 1
    list(list(50, 52, 48, 51), list(51, 53, 50, 52))          # actif 2
)

# Normaliser tous les prix (ND implicite)
normalized = data.[~> (x) => { x / 100 }]

# Garantie de naturalité :
# Résultat identique que tu parcoures :
# - actif → jour → prix
# - jour → actif → prix
# - prix → jour → actif
```
