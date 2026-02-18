# Control Flow Graph - Analyse avancée

Analyse de dominance et détection de boucles dans les CFG.

## Analyse de dominance

Un bloc X **domine** Y si tout chemin de l'entry vers Y passe par X.

### Calcul des dominateurs

```python
from catnip import Catnip
import catnip._rs as rs

c = Catnip()
ir = c.parse('x = 1; y = 2; z = 3')
cfg = rs.cfg.build_cfg_from_ir(ir, 'linear')

# Calculer les dominateurs
cfg.compute_dominators()

# Obtenir tous les dominateurs d'un bloc
entry_id = 0
exit_id = 1

entry_doms = cfg.get_dominators(entry_id)
exit_doms = cfg.get_dominators(exit_id)

print(f'Entry dominators: {entry_doms}')  # [0]
print(f'Exit dominators: {exit_doms}')    # [0, 1]

# Entry se domine lui-même
# Exit est dominé par entry et lui-même
```

### Dominateur immédiat

Le **dominateur immédiat** (idom) de X est l'unique bloc qui :

1. Domine X
1. Est dominé par tous les autres dominateurs de X

```python
ir = c.parse('if x > 0 { y = 1 } else { y = 2 }')
cfg = rs.cfg.build_cfg_from_ir(ir, 'if_else')
cfg.compute_dominators()

# Structure des blocs :
# 0 : entry
# 1 : exit
# 2 : if_then
# 3 : if_else
# 4 : if_merge

for block_id in sorted(cfg.get_reachable_blocks()):
    idom = cfg.get_immediate_dominator(block_id)
    print(f'Block {block_id}: idom = {idom}')

# Block 0: idom = None       (entry n'a pas d'idom)
# Block 1: idom = 0          (exit dominé par entry)
# Block 2: idom = 0          (if_then dominé par entry)
# Block 3: idom = 0          (if_else dominé par entry)
# Block 4: idom = 0          (if_merge dominé par entry)
```

### Blocs dominés

Obtenir tous les blocs dominés par un bloc :

```python
ir = c.parse('''
while x < 10 {
    x = x + 1
}
''')
cfg = rs.cfg.build_cfg_from_ir(ir, 'while')
cfg.compute_dominators()

# Entry domine tout
entry_id = 0
dominated = cfg.get_dominated(entry_id)
print(f'Entry dominates: {dominated}')
# [1, 2, 3, 4] : tous les autres blocs
```

## Détection de boucles

Une **boucle naturelle** a :

- Un bloc **header** qui domine tous les blocs de la boucle
- Un **back edge** d'un bloc vers le header

### Boucle simple

```python
ir = c.parse('''
while x < 10 {
    x = x + 1
}
''')
cfg = rs.cfg.build_cfg_from_ir(ir, 'while')
cfg.compute_dominators()

loops = cfg.detect_loops()
print(f'Loops detected: {len(loops)}')  # 1

for header, loop_blocks in loops:
    print(f'Loop header: {header}')
    print(f'Loop blocks: {sorted(loop_blocks)}')
# Loop header: 2
# Loop blocks: [2, 3]  (while_header, while_body)
```

### Boucles imbriquées

```python
ir = c.parse('''
while x < 10 {
    while y < 5 {
        y = y + 1
    }
    x = x + 1
}
''')
cfg = rs.cfg.build_cfg_from_ir(ir, 'nested')
cfg.compute_dominators()

loops = cfg.detect_loops()
print(f'Loops detected: {len(loops)}')  # 2

for i, (header, loop_blocks) in enumerate(loops):
    print(f'Loop {i}: header={header}, blocks={sorted(loop_blocks)}')
# Loop 0: header=2, blocks=[2, 3, 4, 5, 6, 7]  (outer)
# Loop 1: header=4, blocks=[4, 5]              (inner)
```

### Boucle avec break

```python
ir = c.parse('''
while x < 10 {
    if x == 5 {
        break
    }
    x = x + 1
}
''')
cfg = rs.cfg.build_cfg_from_ir(ir, 'break')
cfg.compute_dominators()

loops = cfg.detect_loops()
print(f'Loops detected: {len(loops)}')  # 1

header, loop_blocks = loops[0]
print(f'Loop blocks: {sorted(loop_blocks)}')
# Inclut les blocs du if imbriqué même avec break
```

## Analyse complète

Combiner dominance et boucles pour une analyse complète :

```python
def analyze_cfg(code, name):
    """Analyse complète d'un CFG."""
    c = Catnip()
    ir = c.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, name)

    print(f'⇒ CFG: {name}' )
    print(f'Blocks: {cfg.num_blocks}')
    print(f'Edges: {cfg.num_edges}')
    print()

    # Dominance
    cfg.compute_dominators()

    print('Dominance:')
    for block_id in sorted(cfg.get_reachable_blocks()):
        doms = cfg.get_dominators(block_id)
        idom = cfg.get_immediate_dominator(block_id)
        dominated = cfg.get_dominated(block_id)
        print(f'  Block {block_id}:')
        print(f'    Dominators: {sorted(doms)}')
        print(f'    Immediate dominator: {idom}')
        print(f'    Dominates: {sorted(dominated)}')
    print()

    # Boucles
    loops = cfg.detect_loops()
    print(f'Loops: {len(loops)}')
    for i, (header, blocks) in enumerate(loops):
        print(f'  Loop {i}: header={header}, blocks={sorted(blocks)}')
    print()

    return cfg

# Utilisation
code = '''
while x < 10 {
    if x % 2 == 0 {
        continue
    }
    x = x + 1
}
'''
cfg = analyze_cfg(code, 'example')
```

## Propriétés de dominance

Propriétés vérifiables :

```python
ir = c.parse('if x { y = 1 } else { y = 2 }')
cfg = rs.cfg.build_cfg_from_ir(ir, 'test')
cfg.compute_dominators()

# Entry domine tous les blocs
entry = 0
for block_id in cfg.get_reachable_blocks():
    if block_id != entry:
        assert entry in cfg.get_dominators(block_id)

# Chaque bloc se domine lui-même
for block_id in cfg.get_reachable_blocks():
    assert block_id in cfg.get_dominators(block_id)

# idom est unique (sauf pour entry)
for block_id in cfg.get_reachable_blocks():
    if block_id != entry:
        idom = cfg.get_immediate_dominator(block_id)
        assert idom is not None
```

## Applications

### Optimisation : élimination de code mort

```python
ir = c.parse('x = 1; y = 2')
cfg = rs.cfg.build_cfg_from_ir(ir, 'test')

# Trouver les blocs non atteignables
unreachable = cfg.get_unreachable_blocks()
if unreachable:
    print(f'Dead code blocks: {unreachable}')
    cfg.remove_unreachable_blocks()
```

### Analyse statique : invariants de boucle

```python
def find_loop_invariants(cfg):
    """Trouver les blocs invariants dans les boucles."""
    cfg.compute_dominators()
    loops = cfg.detect_loops()

    for header, loop_blocks in loops:
        # Les blocs dominés par le header mais pas dans la boucle
        # sont potentiellement des invariants
        dominated = cfg.get_dominated(header)
        invariants = [b for b in dominated if b not in loop_blocks]

        print(f'Loop {header}: potential invariants = {invariants}')

# Exemple
ir = c.parse('''
c = 10
while x < c {
    x = x + 1
}
''')
cfg = rs.cfg.build_cfg_from_ir(ir, 'invariant')
find_loop_invariants(cfg)
```

> En dominance, un bloc domine un autre si tous les chemins vers ce dernier passent par le premier. C'est une loi topologique, pas un vote démocratique.
