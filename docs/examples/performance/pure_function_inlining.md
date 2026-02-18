# Inline de Fonctions Pures dans le JIT

Le JIT Catnip inline automatiquement les petites fonctions pures dans les hot loops pour réduire l'overhead des appels de fonction.

## Marquage de Fonctions Pures

```python
@pure
square = (x) => { x * x }

@pure
add_one = (x) => { x + 1 }

# Utilisation dans hot loop
sum = 0
for i in range(100000) {
    sum = sum + square(i)  # Inliné après 100 iterations
}
```

## Builtins Purs

Les fonctions builtins suivantes sont automatiquement reconnues comme pures et inlinées :

```python
# Mathématiques
abs(-5)
min(10, 20)
max(10, 20)
round(3.7)

# Collections
len([1, 2, 3])
sum([1, 2, 3])
sorted([3, 1, 2])

# Types
int("42")
float("3.14")
str(123)
bool(1)

# Utilisation dans hot loop
for i in range(100000) {
    value = abs(i - 50000) + min(i, 1000)  # Les 2 fonctions inlinées
}
```

## Exemple Complet

```python
from catnip import Catnip
from catnip._rs import VM, Compiler

# Code avec fonctions pures
code = """
{
    # Marquer fonctions comme pures
    @pure
    square = (x) => { x * x }

    @pure
    double = (x) => { x * 2 }

    # Hot loop avec appels de fonctions
    sum = 0
    for i in range(100000) {
        # Après 100 iterations:
        # - square(i) inliné : i * i
        # - double(i) inliné : i * 2
        # - abs(i) inliné : bytecode direct
        sum = sum + square(i) + double(i) + abs(i - 50000)
    }
    sum
}
"""

# Compiler et exécuter avec JIT
cat = Catnip(vm_mode='on')
vm = VM()
vm.set_context(cat.context)
vm.enable_jit()

compiler = Compiler()
ast = cat.parse(code)
bytecode = compiler.compile(ast)

result = vm.execute(bytecode, (), {}, None)
print(f"Résultat: {result}")
```

## Critères d'Inline

Une fonction est inlinée si :

1. **Pureté** : marquée `@pure` ou builtin pur
1. **Taille** : ≤ 20 opcodes (petites fonctions)
1. **Profondeur** : ≤ 2 niveaux d'inline (évite explosion de code)

```python
# ✓ Inlinée (3 opcodes)
@pure
square = (x) => { x * x }

# ✓ Inlinée (8 opcodes)
@pure
clamp = (x, min, max) => {
    if x < min {
        min
    } elif x > max {
        max
    } else {
        x
    }
}

# ✗ PAS inlinée (>20 opcodes)
@pure
huge = (x) => {
    a = x + 1
    b = a * 2
    c = b - 3
    # ... 20+ operations ...
    result
}
```

## Gains de Performance

Speedup attendu sur hot loops :

| Type de fonction | Opcodes | Speedup attendu         |
| ---------------- | ------- | ----------------------- |
| Small pure       | 3       | **1.25-1.35x**          |
| Builtin          | 5       | **1.20-1.30x**          |
| Medium           | 8       | **1.15-1.25x**          |
| Large (>20 ops)  | 25+     | **1.0x** (pas d'inline) |

## Inline Transitive

L'inline fonctionne sur plusieurs niveaux :

```python
@pure
double = (x) => { x * 2 }

@pure
quad = (x) => { double(double(x)) }  # double() inliné 2 fois

# Dans hot loop
for i in range(100000) {
    # quad(i) → double(double(i)) → (i*2)*2 → i*4
    result = quad(i)
}
```

## Limitations

**Closures** : les fonctions qui capturent des variables externes ne sont pas encore inlinées :

```python
outer = 100

use_outer = (x) => { x + outer }  # Closure

# Pas d'inline (limitation actuelle)
for i in range(100000) {
    result = use_outer(i)
}
```

**Workaround** : passer la variable en paramètre :

```python
@pure
add_value = (x, value) => { x + value }

# Maintenant inlinable
outer = 100
for i in range(100000) {
    result = add_value(i, outer)
}
```

## Vérification de Pureté

Une fonction pure doit :

- Ne pas avoir d'effets de bord (pas de print, pas de mutations globales)
- Toujours retourner le même résultat pour les mêmes arguments
- Ne pas dépendre de l'état externe (sauf constantes)

```python
# ✓ Pure
@pure
add = (a, b) => { a + b }

# ✓ Pure
@pure
factorial = (n) => {
    if n <= 1 { 1 } else { n * factorial(n - 1) }
}

# ✗ PAS pure (effet de bord)
impure = (x) => {
    print(x)  # Effet de bord !
    x + 1
}

# ✗ PAS pure (dépend de l'état externe mutable)
counter = 0
impure2 = (x) => {
    counter = counter + 1  # Mutation globale !
    x + counter
}
```

## Voir Aussi

- `docs/dev/JIT_INLINING.md` - Documentation technique complète
- `tests/serial/jit/test_inlining.py` - Tests d'intégration
