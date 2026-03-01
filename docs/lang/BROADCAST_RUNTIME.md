# BROADCAST_RUNTIME

Voir aussi : [BROADCAST_SPEC.md](./BROADCAST_SPEC.md) pour la sémantique, et [BROADCAST_GUIDE.md](./BROADCAST_GUIDE.md)
pour les exemples d'usage.

## Implémentation

### Détection Automatique de Type et d'Opération

Le système détecte automatiquement :

**Type de target** :

1. **Scalaire Python** (`int`, `float`, `str`, `bool`, `None`)
   - Application directe de l'opération
1. **Liste/Tuple Python**
   - Itération optimisée avec préservation du type
1. **Autres itérables**
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

    # Autre itérable
    return [map_func(x) for x in target]
```

______________________________________________________________________

## Performance : Fast Path SIMD

Pour les listes numériques homogènes (tous ints ou tous floats), le broadcasting contourne entièrement le protocole
d'appel Python. Les valeurs sont extraites dans un `Vec` Rust contigu, l'opération est appliquée en boucle serrée
auto-vectorisée par LLVM (AVX2/SSE2), puis le résultat est reconstruit en une seule passe.

**Opérations accélérées** (map) :

- Arithmétique : `+`, `-`, `*`, `/`, `//`, `%`, `**`
- Comparaisons : `>`, `<`, `>=`, `<=`, `==`, `!=`

**Filtres accélérés** :

- Toutes les comparaisons dans `.[if op operand]`

**Conditions d'activation** :

- La cible est une `list` (pas tuple)
- Tous les éléments sont du même type numérique (int ou float, pas bool)
- L'opérande est un scalaire du type compatible

Si l'une de ces conditions n'est pas remplie, le chemin Python standard est utilisé, sans changement de sémantique. La
division par zéro, le modulo par zéro et la puissance négative sur entiers déclenchent aussi un fallback vers Python
pour produire les erreurs appropriées.

<!-- check: no-check -->

```catnip
# Fast path : liste homogène d'ints + opérateur arithmétique
list(1, 2, 3, 4, 5).[* 2]     # -> boucle Rust SIMD, pas d'appel Python par élément

# Fast path : filtre numérique
list(3, 8, 2, 9, 5).[if > 5]  # -> comparaison + collecte en Rust pur

# Fallback : liste hétérogène
list(1, "a", 3).[+ 1]         # -> chemin Python standard (TypeError sur "a")
```

> Le fast path extrait N valeurs, applique N opérations, et reconstruit N résultats, le tout sans franchir la frontière
> PyO3 une seule fois par élément. Le nombre d'appels Python passe de O(N) à O(1). La constante multiplicative restante
> est celle de LLVM essayant de décider s'il doit auto-vectoriser en AVX-512 ou simplement en AVX2, ce qui constitue
> techniquement une perte de temps mais à une échelle que seul un compilateur peut percevoir.

## Fonctions non-pures dans le broadcasting

Le broadcasting accepte **tout callable**, y compris les fonctions à effets de bord. Les fonctions suivantes du contexte
fonctionnent techniquement dans un broadcast mais n'ont pas de sémantique de transformation :

- `import` -- charge des modules, modifie le contexte
- `jit` -- wrapper de compilation JIT
- `pure` -- décorateur de marquage
- `cached` -- wrapper de memoization
- `debug` -- introspection

```catnip
# Fonctionne, mais ce n'est pas du data flow
list("math", "json").[import]   # charge deux modules
```

Ces wrappers ne sont pas interdits dans le broadcasting, mais leur usage n'a pas de sens dans un pipeline de données.
Préférer un appel direct :

```catnip
math = import("math")
json = import("json")
```

> Le broadcasting ne juge pas. Il applique la séquence. Trois modules à charger en parallèle, et la matrice
> administrative s'aligne d'elle-même. Champ confirmé, cachet quantifié, chargement effectif.

## Améliorations Futures

### 1. Support Pandas/NumPy

Détecter automatiquement pandas/numpy et utiliser leurs opérations optimisées.

### 3. Erreurs Intelligentes

Options possibles :

- Filtrer les valeurs incompatibles
- Retourner None/NaN pour les échecs
- Mode strict vs permissif

______________________________________________________________________

### Contraintes d'Implémentation

Le ND implicite repose sur :

**Critère d'arrêt** : Quand considère-t-on une valeur comme "scalaire" ?

- Types de base : int, float, str, bool, None
- Pas de méthode `__iter__` (ou explicitement marqué non-itérable)
- Objets pandas/numpy : traités comme des scalaires de haut niveau (la récursion s'arrête là)

**Unification** : Même notation ND pour tous les niveaux

- `nums.[~> (x) => { x * 2 }]` fonctionne que nums soit scalaire, liste, ou tensor ND
- Pas de syntaxe spéciale pour le cas multi-dimensionnel
- Réduction du nombre de cas à traiter dans le code utilisateur

**Performance** : Optimisations possibles grâce à la naturalité

- Détection des fonctions pures (pas d'effet de bord)
- Parallélisation automatique (pas de dépendances)
- Fusion d'opérations successives (`.[f].[g]` → `.[g∘f]`)

> **Paradoxe d'optimisation** : La fusion d'opérations successives `.[f].[g]` en `.[g∘f]` nécessite qu'on détecte deux
