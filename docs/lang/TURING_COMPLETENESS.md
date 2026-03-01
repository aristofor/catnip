# Complétude de Turing

## Définition

Un langage de programmation est **Turing-complet** s'il peut simuler une machine de Turing universelle, c'est-à-dire
s'il peut calculer **toute fonction calculable**.

Pour être Turing-complet, un langage doit posséder :

1. **Stockage arbitraire de données** - Mémoire illimitée (théoriquement)
1. **Structures conditionnelles** - Branchement basé sur des conditions
1. **Boucles arbitraires** - Itération sans limite connue à l'avance
1. **Opérations arithmétiques et logiques** - Manipulation de données
1. *Optionnel mais utile* : Récursion ou fonctions de première classe

______________________________________________________________________

## Catnip satisfait tous les critères

### 1. Stockage arbitraire

Catnip permet de stocker des données de manière illimitée via :

- **Variables dynamiques** : `x = 10`
- **Collections Python** : accès à `list()`, `dict()`, `tuple()`, `set()`
- **BigInt natif** : entiers de taille arbitraire (promotion automatique)
- **Mémoire théoriquement illimitée** (limitée uniquement par le système)

```catnip
# Variables
compteur = 0
nom = "Alice"
valeurs = list(1, 2, 3, 4, 5)

# BigInt automatique
grand = 2 ** 1000
```

### 2. Structures conditionnelles

Catnip offre plusieurs mécanismes de branchement :

#### If/elif/else

<!-- check: no-check -->

```catnip
if x > 0 {
    print("positif")
} elif x < 0 {
    print("négatif")
} else {
    print("zéro")
}
```

#### Pattern matching avec guards

<!-- check: no-check -->

```catnip
match age {
    n if n < 18 => { print("Mineur") }
    n if n < 65 => { print("Adulte") }
    n => { print("Senior") }
}
```

### 3. Boucles arbitraires

Catnip supporte deux types de boucles :

#### While - Itération conditionnelle

```catnip
# Boucle potentiellement infinie
i = 0
while i < 100 {
    i = i + 1
}

# Conjecture de Collatz (nombre d'itérations inconnu)
n = 27
while n != 1 {
    if n % 2 == 0 {
        n = n / 2
    } else {
        n = 3 * n + 1
    }
}
```

#### For - Itération sur séquences

<!-- check: no-check -->

```catnip
for i in range(100) {
    print(i)
}

for element in collection {
    print(element)
}
```

### 4. Opérations arithmétiques et logiques

Catnip possède un ensemble complet d'opérateurs :

#### Arithmétiques

<!-- check: no-check -->

```catnip
addition = a + b
soustraction = a - b
multiplication = a * b
division = a / b
division_entiere = a // b
modulo = a % b
puissance = a ** b
```

#### Comparaisons

<!-- check: no-check -->

```catnip
egal = a == b
different = a != b
inferieur = a < b
superieur = a > b
inf_egal = a <= b
sup_egal = a >= b
```

#### Logiques

<!-- check: no-check -->

```catnip
et = a and b
ou = a or b
non = not a
```

#### Binaires

<!-- check: no-check -->

```catnip
et_binaire = a & b
ou_binaire = a | b
xor_binaire = a ^ b
inversion = ~a
decalage_gauche = a << n
decalage_droite = a >> n
```

### 5. Récursion et fonctions de première classe

Catnip supporte les **lambdas récursives** et les **fonctions de première classe** :

> Note quantique : cette section densifie la récursion. Plus tu l'observes, plus elle se replie sur elle-même.

```catnip
# Lambda récursive - Factorielle
factorielle = (n) => {
    match n {
        0 | 1 => { 1 }
        n => { n * factorielle(n - 1) }
    }
}

print(factorielle(10))  # 3628800

# Fonction d'Ackermann (récursion complexe non primitive)
ackermann = (m, n) => {
    match m {
        0 => { n + 1 }
        m => {
            match n {
                0 => { ackermann(m - 1, 1) }
                n => { ackermann(m - 1, ackermann(m, n - 1)) }
            }
        }
    }
}

print(ackermann(3, 2))  # 29
```

______________________________________________________________________

## Exemples de preuves

### Exemple 1 : Conjecture de Collatz

La conjecture de Collatz démontre les boucles arbitraires (nombre d'itérations inconnu à l'avance).

```catnip
collatz = (n) => {
    steps = 0
    valeur = n
    while valeur != 1 {
        if valeur % 2 == 0 {
            valeur = valeur / 2
        } else {
            valeur = 3 * valeur + 1
        }
        steps = steps + 1
    }
    steps
}

print("collatz(27) =", collatz(27))  # 111 étapes
```

### Exemple 2 : Suite de Fibonacci

Fibonacci démontre la manipulation d'état multiple et les boucles conditionnelles.

```catnip
fibonacci = (n) => {
    match n {
        0 => { 0 }
        1 => { 1 }
        n => {
            a = 0
            b = 1
            i = 2
            while i <= n {
                temp = a + b
                a = b
                b = temp
                i = i + 1
            }
            b
        }
    }
}

print("fibonacci(20) =", fibonacci(20))  # 6765
```

### Exemple 3 : PGCD d'Euclide

L'algorithme d'Euclide démontre la récursion et les opérations arithmétiques.

```catnip
pgcd = (a, b) => {
    match b {
        0 => { a }
        b => { pgcd(b, a % b) }
    }
}

print("pgcd(48, 18) =", pgcd(48, 18))  # 6
```

### Exemple 4 : Test de primalité

Combine boucles, conditions, arithmétique et pattern matching.

```catnip
est_premier = (n) => {
    match n {
        n if n < 2 => { False }
        2 => { True }
        n if n % 2 == 0 => { False }
        n => {
            diviseur = 3
            est_premier_local = True
            while diviseur * diviseur <= n {
                if n % diviseur == 0 {
                    est_premier_local = False
                }
                diviseur = diviseur + 2
            }
            est_premier_local
        }
    }
}

print("97 est premier ?", est_premier(97))  # True
```

______________________________________________________________________

## Capacités théoriques

Étant Turing-complet, Catnip peut :

**Calculer toute fonction calculable**

- Algorithmes de tri, recherche, optimisation
- Calculs mathématiques arbitraires
- Simulations, automates cellulaires
- Interprètes et compilateurs pour d'autres langages

**Simuler n'importe quelle machine de Turing**

- Peut implémenter des automates à états finis
- Peut simuler des machines de Turing universelles
- Peut émuler d'autres langages Turing-complets

**Résoudre tout problème décidable**

- Si un problème est algorithmiquement résolvable, Catnip peut le résoudre
- Limité uniquement par les ressources (temps, mémoire) du système

______________________________________________________________________

## Limites pratiques

Bien que Turing-complet théoriquement, Catnip a des limites pratiques :

### Limites de mémoire

- Limité par la mémoire RAM du système
- Pas de mémoire virtuelle infinie

### Optimisations d'exécution

Catnip dispose de plusieurs niveaux d'optimisation qui repoussent les limites pratiques :

- **TCO (Tail-Call Optimization)** : La récursion terminale utilise O(1) d'espace pile via un trampoline Rust - pas de
  stack overflow pour les appels récursifs en position terminale
- **VM bytecode** : Exécution par défaut sur une VM stack-based Rust avec NaN-boxing
- **JIT** : Compilation Cranelift vers x86-64 natif pour les boucles et fonctions récursives chaudes

<!-- check: no-check -->

```catnip
# Récursion profonde sans stack overflow grâce au TCO
factorielle = (n, acc) => {
    match n {
        0 | 1 => { acc }
        n => { factorielle(n - 1, n * acc) }
    }
}

print(factorielle(10000, 1))  # fonctionne sans problème
```

______________________________________________________________________

## Comparaison avec d'autres langages

| Langage       | Turing-complet ? | Notes                                           |
| ------------- | ---------------- | ----------------------------------------------- |
| **Catnip**    | ✓ Oui            | TCO, VM bytecode, JIT, pattern matching         |
| Python        | ✓ Oui            | Récursion, fonctions de première classe         |
| JavaScript    | ✓ Oui            | Récursion, closures                             |
| C             | ✓ Oui            | Récursion, pointeurs                            |
| SQL           | ✗ Non (standard) | Manque de boucles arbitraires (sauf extensions) |
| HTML          | ✗ Non            | Langage de balisage, pas de logique             |
| Regex         | ✗ Non            | Automate à états finis, pas Turing-complet      |
| CSS3 (+ HTML) | ✓ Oui\*          | Avec `:checked` et combinateurs (Rule 110)      |

\* CSS3 + HTML est accidentellement Turing-complet via des règles complexes.

______________________________________________________________________

## Concepts théoriques

- [Machine de Turing (Wikipedia)](https://fr.wikipedia.org/wiki/Machine_de_Turing)
- [Complétude de Turing (Wikipedia)](https://fr.wikipedia.org/wiki/Turing-complet)
- [Thèse de Church-Turing](https://fr.wikipedia.org/wiki/Th%C3%A8se_de_Church-Turing)

______________________________________________________________________

## Démonstration complète

Pour exécuter la démonstration complète de complétude de Turing :

```bash
# Télécharger et exécuter la démo
catnip docs/examples/advanced/08_turing_completeness.cat
```

La démonstration inclut :

1. Récursion (factorielle)
1. Boucles arbitraires (Collatz)
1. Récursion complexe (Ackermann)
1. État multiple (Fibonacci)
1. Pattern matching (calculatrice)
1. Algorithmes récursifs (PGCD)
1. Algorithmes complets (primalité)

______________________________________________________________________

**Conclusion** : Catnip est un langage de programmation **Turing-complet**, capable de calculer toute fonction
calculable. Il possède tous les éléments requis par la définition théorique, ainsi que des fonctionnalités modernes
(pattern matching, lambdas, fonctions de première classe) qui facilitent l'écriture d'algorithmes complexes.
