# Control Flow Graph - Exemples de base

Construction et analyse de Control Flow Graphs (CFG) à partir de code Catnip.

## Importation

```python
from catnip import Catnip
import catnip._rs as rs

c = Catnip()
```

## CFG simple

⇒ Assignment linéaire

```python
ir = c.parse('x = 1; y = 2; z = 3')
cfg = rs.cfg.build_cfg_from_ir(ir, 'linear')

print(cfg)
# CFG: linear
# Entry: Some(Some("entry"))
# Exit: Some(Some("exit"))
# Blocks: 2
#
# entry:
#   0: Op { ident: 13, args: ..., ... }
#   1: Op { ident: 13, args: ..., ... }
#   2: Op { ident: 13, args: ..., ... }
#   -> exit (fallthrough)
#
# exit:

print(f'Blocks: {cfg.num_blocks}')  # 2
print(f'Edges: {cfg.num_edges}')    # 1
```

## CFG conditionnel

⇒ If/else (structure en diamant)

```python
ir = c.parse('''
if x > 0 {
    y = 1
} else {
    y = 2
}
''')
cfg = rs.cfg.build_cfg_from_ir(ir, 'if_else')

print(cfg.num_blocks)  # 5 : entry, if_then, if_else, if_merge, exit
print(cfg.num_edges)   # 5 : entry→then, entry→else, then→merge, else→merge, merge→exit
```

Le CFG crée une structure en diamant :

```
       entry
      /     \
   then     else
      \     /
       merge
         |
        exit
```

## CFG avec boucles

⇒ While loop

```python
ir = c.parse('''
while x < 10 {
    x = x + 1
}
''')
cfg = rs.cfg.build_cfg_from_ir(ir, 'while')

print(cfg.num_blocks)  # 5 : entry, while_header, while_body, while_exit, exit
print(cfg.num_edges)   # 5 : dont un back edge

# Structure :
#   entry → header ⇄ body
#              ↓
#            exit
```

⇒ For loop

```python
ir = c.parse('''
for x in range(10) {
    y = y + x
}
''')
cfg = rs.cfg.build_cfg_from_ir(ir, 'for')

# Structure similaire au while
print(cfg.num_blocks)  # 5
```

## CFG avec contrôle de flux

⇒ Break dans une boucle

```python
ir = c.parse('''
while x < 10 {
    if x == 5 {
        break
    }
    x = x + 1
}
''')
cfg = rs.cfg.build_cfg_from_ir(ir, 'break_loop')

print(cfg.num_blocks)  # 8
print(cfg.num_edges)   # 9

# Le break crée un edge direct vers while_exit
```

⇒ Continue dans une boucle

```python
ir = c.parse('''
while x < 10 {
    if x % 2 == 0 {
        continue
    }
    process(x)
    x = x + 1
}
''')
cfg = rs.cfg.build_cfg_from_ir(ir, 'continue_loop')

# Continue crée un edge direct vers while_header
```

## Blocs atteignables

```python
ir = c.parse('x = 1; y = 2')
cfg = rs.cfg.build_cfg_from_ir(ir, 'test')

reachable = cfg.get_reachable_blocks()
print(f'Blocs atteignables: {reachable}')

unreachable = cfg.get_unreachable_blocks()
print(f'Blocs non atteignables: {unreachable}')

# Nettoyage
if unreachable:
    cfg.remove_unreachable_blocks()
```

## Visualisation

⇒ Génération DOT

```python
ir = c.parse('''
if x > 0 {
    y = 1
} else {
    y = 2
}
''')
cfg = rs.cfg.build_cfg_from_ir(ir, 'example')

# Obtenir le DOT
dot = cfg.to_dot()
print(dot[:100])  # Affiche le début
```

⇒ Export vers fichier

```python
# Exporter et afficher les commandes
cfg.visualize('output.dot')
# CFG exported to output.dot
# Visualize with: dot -Tpng output.dot -o output.png

# Puis avec graphviz :
# dot -Tpng output.dot -o output.png
```

Le format DOT inclut :

- Styling : entry (vert), exit (rouge)
- Couleurs : edges true (vert), false (rouge)
- Style dashed pour break/continue
- Affichage des instructions de chaque bloc

## Représentation

```python
ir = c.parse('x = 1')
cfg = rs.cfg.build_cfg_from_ir(ir, 'test')

# repr : format court
print(repr(cfg))
# <CFG test blocks=2 edges=1>

# str : format détaillé avec structure
print(str(cfg))
# CFG: test
# Entry: Some(Some("entry"))
# ...
```

> Les blocs basiques sont comme des wagons de train : ils vont toujours dans le même sens, sauf quand ils bifurquent. À ce moment-là ce ne sont plus des wagons, ce sont des aiguillages. Mais on continue de les appeler wagons par cohérence administrative.
