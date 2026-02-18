# Module Loading

Catnip peut charger des modules Catnip et Python, avec des **namespaces propres**.

## CLI : `-m`

```bash
catnip -m <module> script.cat
```

Charge un module Python installé et l'expose comme namespace global.

```bash
catnip -m math -c "math.sqrt(16)"
# → 4.0

catnip -m math -m random -c "math.floor(random.random() * 100)"
```

> Le namespace porte le nom du module. Pas d'alias, pas d'injection directe. Si on veut un alias, on utilise `import()` dans le code.

## Langage : `import()`

Le builtin `import()` charge un module et retourne un objet namespace :

```catnip
m = import("math")
m.sqrt(144)
# → 12.0

m.pi
# → 3.141592653589793
```

### Chemin explicite

Un spec contenant `/`, `\`, un préfixe `.` ou une extension est traité comme chemin de fichier. Le fichier est chargé directement selon son type :

```catnip
host = import("./host.py")          # Python local
tools = import("./tools.cat")       # Catnip local
tools = import("/opt/lib/tools.cat") # Chemin absolu
```

### Nom nu (sans chemin)

Un nom sans séparateur ni extension déclenche une résolution par priorité :

1. **Cache** -- si déjà chargé, retour immédiat
1. **`.cat`** dans les répertoires de recherche
1. **`.py`** dans les répertoires de recherche
1. **Extensions natives** (`.so`, `.pyd`) dans les répertoires de recherche
1. **`importlib`** -- fallback Python (stdlib, pip)

```catnip
utils = import("utils")   # cherche utils.cat, puis utils.py, puis pip
json = import("json")     # pas de .cat/.py local → stdlib
```

> Si `utils.cat` et `utils.py` coexistent dans le même répertoire, le `.cat` gagne. Pour forcer le `.py`, utiliser un chemin explicite : `import("./utils.py")`.

### Repertoires de recherche

La résolution parcourt ces répertoires dans l'ordre :

1. **CWD** -- le répertoire courant
1. **`CATNIP_PATH`** -- variable d'environnement, répertoires séparés par `:`

```bash
export CATNIP_PATH="/opt/catnip-libs:$HOME/.catnip/modules"
catnip script.cat
```

```catnip
# Si /opt/catnip-libs/mylib.cat existe :
m = import("mylib")
m.greet()
```

### Modules Catnip

Si `__all__` est défini dans le module Catnip, seuls ces symboles sont exportés. Sinon, les noms publics ajoutés par le module sont exposés.

### Cache

Un module chargé une fois est mis en cache par son spec exact. Les appels suivants retournent le même objet :

```catnip
a = import("math")
b = import("math")
# a et b sont le même namespace
```

## Écrire un Module Host

### Structure recommandée

```python
# my_module.py

def my_function(x):
    """Cette fonction sera exposée dans le namespace."""
    return x * 2

class MyClass:
    """Les classes sont aussi exposées."""
    def __init__(self):
        self.value = 0

def _private_helper():
    """Fonctions commençant par _ ne sont PAS exposées."""
    return "private"
```

### Règles d'exposition

**Exposé dans le namespace** :

- Fonctions publiques (ne commencent pas par `_`)
- Classes publiques
- Constantes publiques

**Non exposé** :

- Attributs commençant par `_`

## REPL avec modules

```bash
catnip -m math
```

> Quand des modules sont chargés via `-m`, le REPL Python minimal est utilisé à la place du REPL Rust (pour que les namespaces soient accessibles).

## Résumé

```bash
# CLI
catnip -m math script.cat              # math.sqrt()
catnip -m math -m random script.cat    # math + random
catnip -m math                         # REPL avec math

# Langage -- chemin explicite
host = import("./host.py")             # Fichier Python local
tools = import("./tools.cat")          # Fichier Catnip local

# Langage -- nom nu (résolution par priorité)
m = import("math")                     # .cat → .py → .so → stdlib
utils = import("utils")                # utils.cat si présent, sinon utils.py

# Search path
export CATNIP_PATH="/opt/libs"         # répertoires supplémentaires
```
