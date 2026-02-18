# Configuration

Catnip utilise une configuration hiérarchique avec des **overrides par mode d'exécution**.

## Fichier de Configuration

Emplacement : `~/.config/catnip/catnip.toml`

Voir [`catnip.toml.example`](../../catnip.toml.example) à la racine du projet pour un fichier complet commenté.

## Structure de Base

```toml
# Config globale
enable_cache = true

[optimize]
executor = "vm"
jit = false
tco = true
optimize = 3

[repl]
no_color = false
theme = "auto"      # auto, dark, light

[format]
indent_size = 4
line_length = 120

[cache]
cache_max_size_mb = 100     # Limite 100 Mo ("unlimited" pour illimité)
cache_ttl_seconds = 86400   # TTL 24 heures ("unlimited" pour pas d'expiration)
```

## Configuration Par Mode

Catnip charge des overrides selon le mode d'exécution détecté.

### Fonctionnement

Les sections `[mode.X]` dans `catnip.toml` surchargent les valeurs globales selon le contexte d'exécution :

- **`standalone`** : Scripts `.cat` exécutés via CLI (fichier fourni en argument)
- **`repl`** : Session interactive (pas de fichier, stdin est un TTY)
- **`embedded`** : Catnip importé comme bibliothèque Python

**Ordre de priorité** :

```
DEFAULT < FILE (global) < FILE ([mode.X]) < ENV < CLI
```

Les overrides par mode s'appliquent **après** la config globale du fichier mais **avant** les variables d'environnement.

## Modes d'Exécution

Catnip détecte automatiquement 3 modes d'exécution et applique des overrides spécifiques :

### Mode `standalone` (Scripts)

**Déclenchement** : `catnip script.cat`

**Objectif** : Startup rapide, scripts batch, automation

**Config recommandée** :

```toml
[mode.standalone]
jit = false      # Pas de JIT : startup instant
optimize = 1     # Compilation rapide
tco = true       # TCO utile pour récursion
```

**Use cases** :

- Scripts système (config validation, data transform)
- Cron jobs, automation
- CLI tools en Catnip

### Mode `repl` (Interactif)

**Déclenchement** : `catnip` (sans arguments, stdin = tty)

**Objectif** : Performance interactive, exploration

**Config recommandée** :

```toml
[mode.repl]
jit = true       # JIT pour perf runtime
optimize = 2     # Équilibre perf/compilation
tco = true       # TCO pour expérimentation
```

**Use cases** :

- Exploration interactive
- Debugging
- Calculs quick

### Mode `embedded` (DSL Python)

**Déclenchement** : `from catnip import Catnip` (usage library)

**Objectif** : Prévisibilité, debugging facile, intégration simple

**Config recommandée** :

```toml
[mode.embedded]
jit = false      # Pas de JIT : intégration simple
tco = false      # Pas de TCO : call stacks clairs
optimize = 0     # Comportement prédictible
```

**Use cases** :

- DSL dans applications Python
- Pandas/data wrangling DSL
- Config DSL, rule engines

> **Note** : le mode embedded n'est pas encore auto-détecté. Pour l'instant, utilisez les sections `[optimize]` et `[repl]` classiques pour embedded.

## Précédence des Configurations

Ordre de priorité (du plus faible au plus fort) :

1. **Defaults** (hardcodés)
1. **Fichier config** (`~/.config/catnip/catnip.toml`)
1. **Mode overrides** (`[mode.standalone]`, `[mode.repl]`, `[mode.embedded]`)
1. **Variables d'environnement** (`CATNIP_EXECUTOR`, `CATNIP_OPTIMIZE`, `CATNIP_THEME`, `NO_COLOR`)
1. **Arguments CLI** (`-x`, `-o`, `--theme`, `--no-color`)

### Exemple de Précédence

```toml
# catnip.toml
[optimize]
jit = false
optimize = 3

[mode.standalone]
optimize = 1
```

```bash
# Base
$ catnip -c "42"              # jit=false, optimize=3 (base config)

# Mode override
$ catnip script.cat           # jit=false, optimize=1 (mode.standalone)

# Env var override
$ CATNIP_OPTIMIZE=jit catnip script.cat
                              # jit=true, optimize=1 (env > mode)

# CLI override (highest)
$ catnip -o jit:off -o level:2 script.cat
                              # jit=false, optimize=2 (cli > all)
```

## Options de Cache

Le cache de compilation stocke les résultats de parsing/compilation pour accélérer les exécutions répétées.

**Emplacement** : `~/.cache/catnip/` (XDG_CACHE_HOME)

**Configuration** :

```toml
[cache]
cache_max_size_mb = 50      # Taille maximale en Mo
cache_ttl_seconds = 7200    # Durée de vie des entrées (secondes)
```

Valeurs spéciales :

- `"unlimited"` : pas de limite de taille ou de TTL
- Défauts : 100 Mo, 24 heures

**Gestion via CLI** :

```bash
catnip cache stats         # Statistiques (taille, hits/misses)
catnip cache prune         # Nettoyer entrées expirées
catnip cache clear         # Vider complètement le cache
```

Voir [CLI](CLI.md#cache) pour plus de détails.

## Variables d'Environnement

- `CATNIP_EXECUTOR` : `vm` (défaut), `ast`, `shadow`
- `CATNIP_OPTIMIZE` : syntaxe identique à `-o` (ex: `jit,tco:off,level:2`)
- `CATNIP_THEME` : `auto` (défaut), `dark`, `light`
- `NO_COLOR` : `1` pour désactiver les couleurs (standard freedesktop.org)
- `CATNIP_FORMAT_INDENT_SIZE` : taille indentation (défaut: 4)
- `CATNIP_FORMAT_LINE_LENGTH` : longueur ligne max (défaut: 120)

## Inspecter la Configuration

```bash
# Voir config actuelle
$ catnip config show

# Avec sources (defaults, file, env, cli)
$ catnip config show --debug
```

<!-- doc-snapshot: config/show -->

```console
$ catnip config show
executor = "vm"
...
```

<!-- doc-snapshot: config/show-debug -->

```console
$ catnip config show --debug
Configuration (with sources):
...
```

## Exemples Pratiques

### Dev : debugging avec AST interprète

```bash
export CATNIP_EXECUTOR=ast
export CATNIP_OPTIMIZE=tco:off,level:0
catnip myscript.cat
```

### Prod : max performance

```bash
export CATNIP_OPTIMIZE=jit,level:3
catnip -o tco batch_processing.cat
```

### CI : reproducibilité

```toml
[mode.standalone]
jit = false
tco = false
optimize = 0
```

## Notes

- Les overrides par mode sont **optionnels** : si absents, la config de base s'applique
- Les 3 modes peuvent coexister dans le même fichier
- La détection de mode est **automatique** et transparente
