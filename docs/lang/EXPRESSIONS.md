# Expressions

- [Syntax](SYNTAX.md)
- [Types](TYPES.md)
- [Expressions](EXPRESSIONS.md)
- [Control Flow](CONTROL_FLOW.md)
- [Functions](FUNCTIONS.md)
- [Structures](STRUCTURES.md)
- [Pattern Matching](PATTERN_MATCHING.md)

## Expressions multilignes

Les expressions peuvent s'étendre sur plusieurs lignes en utilisant des parenthèses. Les newlines sont ignorés à
l'intérieur des parenthèses, permettant de formater le code de manière lisible.

> Les parenthèses créent une zone de micro-apesanteur syntaxique où les newlines flottent librement sans jamais toucher
> le sol du parser. Un espace où les expressions longues se détendent naturellement.

### Expressions arithmétiques

<!-- check: no-check -->

```catnip
# Expression multiligne avec parenthèses
resultat = (
    10 + 20 +
    30 + 40 +
    50
)  # 150

# Calcul complexe
moyenne = (
    valeur1 + valeur2 + valeur3
    + valeur4 + valeur5
) / 5

# Avec indentation libre
total = (
    prix_ht
    * quantite
    * (1 + tva)
)
```

### Appels de fonction

Les arguments de fonction peuvent être répartis sur plusieurs lignes. **Note** : le premier argument doit être sur la
même ligne que `(`, les arguments suivants peuvent être sur des lignes séparées.

<!-- check: no-check -->

```catnip
# Appel multiligne
resultat = fonction_complexe(argument1,
    argument2,
    argument3,
    argument4)

# Avec arguments nommés
configurer(host="localhost",
    port=8080,
    debug=True,
    timeout=30)

# Mélange positionnel et nommé
creer_serveur("192.168.1.1",
    3000,
    ssl=True,
    workers=4)
```

### Lambdas multilignes

Les paramètres de lambda peuvent aussi s'étendre sur plusieurs lignes. **Note** : le premier paramètre doit être sur la
même ligne que `(`.

```catnip
# Paramètres multiligne
transformer = (x,
    y,
    z,
    scale=1.0) => {
    x * scale + y * scale + z * scale
}

# Lambda variadique
combiner = (prefix,
    *items) => {
    result = list(prefix)
    for item in items {
        result = result + list(item)
    }
    result
}
```

### Expressions conditionnelles

Les conditions peuvent être formatées sur plusieurs lignes :

<!-- check: no-check -->

```catnip
# Condition complexe
if (
    age >= 18
    and permis == True
    and experience > 2
) {
    print("Peut conduire")
}

# Expression booléenne longue
valide = (
    x > 0
    and x < 100
    and y > 0
    and y < 100
    and z > 0
)
```

### Cas d'usage

**Sans parenthèses** (erreur de parsing) :

```catnip
# ✗ ERREUR : newline termine l'expression
x = 1 + 2 +
    3 + 4  # Parsing error!
```

**Avec parenthèses** (valide) :

```catnip
# OK : newlines ignorés dans les parenthèses
x = (1 + 2 +
     3 + 4)  # 10
```

**Note** : Les collections (`list()`, `dict()`, etc.) et les blocs `{}` supportent déjà les newlines naturellement sans
nécessiter de parenthèses supplémentaires.

______________________________________________________________________

## Opérateurs

### Opérateurs arithmétiques

```catnip
# Addition, soustraction, multiplication
resultat = 10 + 5 - 2 * 3

# Division flottante (true division) : toujours un float
10 / 3                 # ⇒ 3.3333333333333335
6 / 2                  # ⇒ 3.0 (float, pas int)

# Division entière (floor division) : arrondi vers le bas
17 // 5                # ⇒ 3
-17 // 5               # ⇒ -4 (floor, pas troncature)

# Modulo
reste = 17 % 5         # 2

# Exponentiation
puissance = 2 ** 10    # 1024

# Opérateurs unaires
negatif = -42
positif = +42

# Promotion automatique en grands entiers
2 ** 100               # ⇒ 1267650600228229401496703205376
```

**Division** : `/` produit toujours un `float` (true division, comme Python 3). `//` produit un `int` arrondi vers le
bas (floor division). Pour les entiers, `/` promeut en float même si le résultat est entier (`6 / 2` → `3.0`).

L'arithmétique entière est en précision arbitraire : quand le résultat dépasse 48 bits, la VM promeut automatiquement en
BigInt natif. La démotion se fait si le résultat retombe dans la plage SmallInt. Ce mécanisme est transparent pour le
code utilisateur.

### Opérateurs de comparaison

<!-- check: no-check -->

```catnip
# Égalité et inégalité
egal = 5 == 5          # True
different = 5 != 3     # True

# Comparaisons
plus_petit = 3 < 5     # True
plus_grand = 10 > 2    # True
inf_egal = 5 <= 5      # True
sup_egal = 10 >= 5     # True

# Comparaisons en chaîne
dans_intervalle = 1 < x < 10
```

### Opérateurs logiques

<!-- check: no-check -->

```catnip
# AND, OR, NOT
condition1 = True and False    # False
condition2 = True or False     # True
condition3 = not True          # False

# Court-circuit
resultat = x > 0 and y / x > 2
```

### Opérateurs binaires

<!-- check: no-check -->

```catnip
# AND, OR, XOR binaire
masque = 0xFF & 0x0F           # 0x0F
union = 0xF0 | 0x0F            # 0xFF
exclusif = 0xFF ^ 0x0F         # 0xF0

# Inversion binaire
inverse = ~0xFF
```

______________________________________________________________________

## Accès aux attributs et chaînage

Catnip supporte l'accès aux attributs et le chaînage de méthodes :

<!-- check: no-check -->

```catnip
# Accès à un attribut
valeur = objet.attribut

# Appel de méthode
resultat = objet.methode()

# Appel avec arguments
resultat = objet.methode(arg1, arg2)

# Chaînage
resultat = objet.methode1().methode2().attribut

# Chaînage complexe
valeur = objet.get_data().process(param).to_string()
```

______________________________________________________________________

## Indexation et Slicing

Catnip supporte l'indexation et le slicing avec la syntaxe `[…]`, compatible avec Python.

### Indexation simple

```catnip
# Accès par index (listes)
nombres = list(10, 20, 30, 40, 50)
premier = nombres[0]      # 10
troisieme = nombres[2]    # 30
dernier = nombres[4]      # 50

# Avec variable ou expression
idx = 3
valeur = nombres[idx]           # 40
autre = nombres[1 + 1]          # 30

# Dictionnaires
personne = dict(nom="Alice", age=30)
nom = personne["nom"]           # "Alice"
age = personne["age"]           # 30

# Chaînes de caractères
texte = "bonjour"
premiere_lettre = texte[0]      # "b"
```

### Slicing (extraction de sous-séquences)

Le slicing utilise la syntaxe `[start:stop:step]` où tous les paramètres sont optionnels.

#### Slicing basique

```catnip
# start:stop - du start (inclus) au stop (exclu)
nombres = list(10, 20, 30, 40, 50)
sous_liste = nombres[1:4]       # [20, 30, 40]

# start: - du start jusqu'à la fin
fin = nombres[2:]               # [30, 40, 50]

# :stop - du début jusqu'à stop
debut = nombres[:3]             # [10, 20, 30]

# : - copie complète
copie = nombres[:]              # [10, 20, 30, 40, 50]
```

#### Slicing avec pas (step)

```catnip
# ::step - tous les step éléments
chiffres = list(0, 1, 2, 3, 4, 5, 6, 7, 8, 9)
pairs = chiffres[::2]           # [0, 2, 4, 6, 8]
impairs = chiffres[1::2]        # [1, 3, 5, 7, 9]

# start:stop:step - syntaxe complète
nombres = list(0, 10, 20, 30, 40, 50, 60, 70, 80, 90)
extrait = nombres[1:8:2]        # [10, 30, 50, 70]
```

#### Indices négatifs

Les indices négatifs comptent depuis la fin (-1 = dernier élément).

```catnip
liste = list(1, 2, 3, 4, 5, 6, 7, 8, 9)

# Accès négatif
dernier = liste[-1]             # 9
avant_dernier = liste[-2]       # 8

# Slicing avec indices négatifs
fin = liste[-3:]                # [7, 8, 9]
milieu = liste[-5:-2]           # [5, 6, 7]
sans_dernier = liste[:-1]       # [1, 2, 3, 4, 5, 6, 7, 8]
```

#### Inversion et pas négatif

Un pas négatif parcourt la séquence en sens inverse.

```catnip
nombres = list(1, 2, 3, 4, 5)

# Inversion complète
inverse = nombres[::-1]         # [5, 4, 3, 2, 1]

# Un élément sur deux en partant de la fin
extrait = nombres[::-2]         # [5, 3, 1]

# Chaînes inversées
texte = "bonjour"
reverse = texte[::-1]           # "ruojnob"
```

#### Slicing sur chaînes

Le slicing fonctionne également sur les chaînes de caractères.

```catnip
message = "hello world"

# Extraction
debut = message[0:5]            # "hello"
fin = message[6:]               # "world"
un_sur_deux = message[::2]      # "hlowrd"

# Palindrome check
mot = "kayak"
est_palindrome = mot == mot[::-1]  # True
```

#### Notation `.[:]` (fullslice)

La syntaxe `.[start:stop:step]` permet le slicing avec notation explicite à point, utile après des expressions ou pour
chaîner avec d'autres opérations.

```catnip
# Équivalent à [:]
data = list(1, 2, 3, 4, 5, 6)
data.[:]                        # [1, 2, 3, 4, 5, 6]

# Sur expressions
(list(1, 2, 3) + list(4, 5, 6)).[1:5]  # [2, 3, 4, 5]

# Chaînage avec broadcast
list(10, 20, 30, 40, 50).[:3].[* 2]    # [20, 40, 60]
```

> La notation `.[:]` fonctionne exactement comme `[:]` mais avec la syntaxe membre. Elle s'avère particulièrement
> pratique quand l'objet source est une expression complexe ou quand on souhaite chaîner plusieurs opérations de manière
> fluide.

#### Cas particuliers

```catnip
# Slice hors bornes (pas d'erreur, liste vide)
liste = list(1, 2, 3)
vide = liste[10:20]             # []
aussi_vide = liste[5:]          # []

# Step de 0 lève une erreur
# liste[::0]  # ValueError!

# Slicing d'une liste imbriquée
matrix = list(list(1, 2, 3), list(4, 5, 6), list(7, 8, 9))
premiere_ligne = matrix[0]      # [1, 2, 3]
sous_matrice = matrix[0:2]      # [[1, 2, 3], [4, 5, 6]]
```

### Indexation et slicing imbriqués

```catnip
# Accès dans une liste de listes
matrix = list(list(1, 2, 3), list(4, 5, 6), list(7, 8, 9))
element = matrix[1][2]          # 6 (2ème ligne, 3ème colonne)

# Slicing puis indexation
liste = list(10, 20, 30, 40, 50)
sous = liste[1:4]               # [20, 30, 40]
element = sous[1]               # 30

# Équivalent en une ligne
element = liste[1:4][1]         # 30
```

### Exemple pratique : recherche dichotomique

```catnip
binary_search = (liste, cible, gauche=0, droite=None) => {
    if droite == None { droite = len(liste) - 1 }

    if gauche > droite {
        -1
    } else {
        milieu = (gauche + droite) // 2
        valeur = liste[milieu]      # Indexation

        if valeur == cible {
            milieu
        } elif valeur < cible {
            binary_search(liste, cible, milieu + 1, droite)
        } else {
            binary_search(liste, cible, gauche, milieu - 1)
        }
    }
}

liste_triee = list(1, 3, 5, 7, 9, 11, 13, 15)
index = binary_search(liste_triee, 7)  # 3
```
