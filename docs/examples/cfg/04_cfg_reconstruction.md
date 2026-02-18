# Control Flow Graph - Reconstruction

Reconstruction de code structuré depuis un CFG optimisé.

## Concept

Après optimisation d'un CFG (fusion de blocs, suppression de dead code), on peut reconstruire du code structuré en analysant :

- La topologie du graphe (successeurs, prédécesseurs)
- Les types d'edges (conditional, unconditional, back-edge)
- La dominance (pour détecter les loops)

## API Python

```python
import catnip._rs as rs
from catnip import Catnip

c = Catnip()

# Code source
code = """
x = 0
while x < 10 {
    if x % 2 == 0 {
        y = x * 2
    }
    x = x + 1
}
"""

# Parser → IR
ir = c.parse(code, semantic=False)

# Construire CFG
cfg = rs.cfg.build_cfg_from_ir(ir, 'example')

# Calculer dominance (nécessaire pour détecter les loops)
cfg.compute_dominators()

# Reconstruire le code structuré
reconstructed = rs.cfg.py_reconstruct_from_cfg(cfg)

print(f"Reconstructed {len(reconstructed)} operations")
for op in reconstructed:
    print(f"  Op {op.ident}")
```

## Structures détectées

### While loops

**Critère** : Bloc avec 2 successeurs (ConditionalTrue/False) où le successeur true a un chemin retour vers le header.

```python
code = "while x < 10 { x = x + 1 }"
ir = c.parse(code, semantic=False)
cfg = rs.cfg.build_cfg_from_ir(ir, 'while')
cfg.compute_dominators()

reconstructed = rs.cfg.py_reconstruct_from_cfg(cfg)

# Find while operation
from catnip.semantic.opcode import OpCode
while_ops = [op for op in reconstructed if op.ident == OpCode.OP_WHILE]
assert len(while_ops) == 1
```

### If/else

**Critère** : Bloc avec 2 successeurs conditionnels qui convergent vers un merge point.

```python
code = """
if x > 0 {
    y = 1
} else {
    y = 2
}
"""
ir = c.parse(code, semantic=False)
cfg = rs.cfg.build_cfg_from_ir(ir, 'if')
cfg.compute_dominators()

reconstructed = rs.cfg.py_reconstruct_from_cfg(cfg)

# Find if operation
if_ops = [op for op in reconstructed if op.ident == OpCode.OP_IF]
assert len(if_ops) == 1
```

### Séquences linéaires

**Critère** : Blocs avec 0 ou 1 successeur (pas de branchement).

```python
code = "x = 1; y = 2; z = 3"
ir = c.parse(code, semantic=False)
cfg = rs.cfg.build_cfg_from_ir(ir, 'linear')
cfg.compute_dominators()

reconstructed = rs.cfg.py_reconstruct_from_cfg(cfg)

# Devrait avoir 3 assignments
assert len(reconstructed) >= 3
```

## Algorithme de reconstruction

La reconstruction utilise une détection de régions :

1. **Traverser depuis entry** : DFS depuis le bloc d'entrée
1. **Analyser les successeurs** :
   - 0 successeurs → fin de région
   - 1 successeur → bloc linéaire ou back-edge
   - 2 successeurs → if/else ou while header
   - 3+ successeurs → match (non pris en charge)
1. **Détecter while headers** :
   - Vérifier si successeur true a un chemin retour
   - Extraire condition (dernière instruction du header)
   - Reconstruire récursivement le corps
1. **Reconstruire if/else** :
   - Identifier branches true/false
   - Trouver merge point (post-dominateur)
   - Reconstruire chaque branche
1. **Construire les Op nodes** :
   - Créer les nodes OP_WHILE, OP_IF, OP_BLOCK
   - Imbriquer récursivement les corps

## Limitations actuelles

- **Match statements** : Non gérés (3+ branches)
- **For loops** : Traités comme while
- **Goto arbitraires** : Reconstruction linéaire
- **Break/continue** : Partiellement supportés

> La reconstruction n'est pas utilisée dans le pipeline normal de compilation.
> C'est une feature expérimentale pour l'analyse et le debugging de CFG optimisés.

## Cas d'usage

- **Debugging d'optimisations** : Vérifier que le CFG optimisé représente toujours le code original
- **Decompilation** : Reconstruire du code lisible depuis bytecode
- **Analyse statique** : Extraire des patterns depuis un CFG normalisé
- **Code generation** : Produire du code dans un autre langage depuis le CFG

## Exemple complet

```python
import catnip._rs as rs
from catnip import Catnip
from catnip.semantic.opcode import OpCode

c = Catnip()

# Code avec loop et conditions
code = """
x = 0
while x < 10 {
    if x == 5 {
        break
    }
    x = x + 1
}
"""

# Parse → CFG
ir = c.parse(code, semantic=False)
cfg = rs.cfg.build_cfg_from_ir(ir, 'complex')

print(f"Original CFG: {cfg.num_blocks} blocks, {cfg.num_edges} edges")

# Optimiser le CFG
cfg.compute_dominators()
dead, merged, empty, branches = cfg.optimize()
print(f"After optimization: {cfg.num_blocks} blocks")
print(f"  Removed: {dead} dead, {merged} merged, {empty} empty")

# Reconstruire
reconstructed = rs.cfg.py_reconstruct_from_cfg(cfg)

print(f"\nReconstructed {len(reconstructed)} operations:")
for op in reconstructed:
    opcode_name = OpCode(op.ident).name
    print(f"  {opcode_name}")

# Vérifier structure
while_count = sum(1 for op in reconstructed if op.ident == OpCode.OP_WHILE)
if_count = sum(1 for op in reconstructed if op.ident == OpCode.OP_IF)
print(f"\nStructure: {while_count} while, {if_count} if")
```

**Sortie attendue** :

```
Original CFG: 8 blocks, 9 edges
After optimization: 8 blocks
  Removed: 0 dead, 0 merged, 1 empty

Reconstructed 2 operations:
  SET_LOCALS
  OP_WHILE

Structure: 1 while, 0 if
```

> Le if avec break est intégré dans le corps du while reconstruit.
> La reconstruction préserve la sémantique mais peut réorganiser les structures imbriquées.
