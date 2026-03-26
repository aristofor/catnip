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

[modules]
auto = []                    # Modules chargés automatiquement au démarrage
```

## Configuration Par Mode

Catnip charge des overrides selon le mode d'exécution détecté.

### Fonctionnement

Les sections `[mode.X]` dans `catnip.toml` surchargent les valeurs globales selon le contexte d'exécution :

- **`standalone`** : Scripts `.cat` exécutés via CLI (fichier fourni en argument)
- **`repl`** : Session interactive (pas de fichier, stdin est un TTY)
- **`dsl`** : Catnip utilisé comme DSL (importé comme bibliothèque Python)

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

**Cas d'usage** :

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

**Cas d'usage** :

- Exploration interactive
- Debugging
- Calculs quick

### Mode DSL

**Déclenchement** : `from catnip import Catnip` (usage library)

**Objectif** : Prévisibilité, debugging facile, intégration simple

**Config recommandée** :

```toml
[mode.dsl]
jit = false      # Pas de JIT : intégration simple
tco = false      # Pas de TCO : call stacks clairs
optimize = 0     # Comportement prédictible
```

**Cas d'usage** :

- DSL dans applications Python
- Pandas/data wrangling DSL
- Config DSL, rule engines

> **Note** : le mode DSL n'est pas encore auto-détecté. Pour l'instant, utilisez les sections `[optimize]` et `[repl]`
> classiques.

## Précédence des Configurations

Ordre de priorité (du plus faible au plus fort) :

1. **Defaults** (hardcodés)
1. **Fichier config** (`~/.config/catnip/catnip.toml`)
1. **Mode overrides** (`[mode.standalone]`, `[mode.repl]`, `[mode.dsl]`)
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

## Modules

La section `[modules]` configure le chargement automatique de modules et les policies d'accès.

```toml
[modules]
auto = ["io", "math"]            # Modules chargés au démarrage
policy = "allow"                 # Fallback : allow ou deny
allow = ["math", "json"]         # Patterns autorisés
deny = ["os", "subprocess"]      # Patterns bloqués
```

`auto` charge les modules listés comme namespaces globaux avant l'exécution, équivalent à `-m` pour chaque module.
Chaque mode (REPL, CLI, DSL) peut définir sa propre liste via `[modules.repl]`, `[modules.cli]`, `[modules.dsl]` avec
fallback sur `[modules].auto`. Les modules passés via `-m` sont additifs (dédupliqués).

Un module introuvable ou bloqué par la policy est ignoré avec un message d'erreur.

Voir [MODULE_LOADING](MODULE_LOADING.md) pour la policy et l'auto-import en détail.

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

## Diagnostics

Les erreurs internes (WeirdErrors) sont automatiquement loggées sur disque sous forme de rapports JSON. Ces rapports
servent au diagnostic post-mortem : un utilisateur peut les partager pour signaler un bug.

**Emplacement** : `~/.local/state/catnip/weird/` (XDG_STATE_HOME)

**Configuration** :

```toml
[diagnostics]
log_weird_errors = true     # Actif par défaut
max_weird_logs = 50         # Rotation automatique
```

**Env var** : `CATNIP_WEIRD_LOG=off` désactive le logging (priorité sur le TOML).

Le logging est silencieux : une erreur I/O lors de l'écriture du rapport ne masque jamais l'erreur originale. Fonctionne
dans les deux contextes : Python (via PyO3) et standalone Rust.

## Variables d'Environnement

- `CATNIP_EXECUTOR` : `vm` (défaut)
- `CATNIP_OPTIMIZE` : syntaxe identique à `-o` (ex: `jit,tco:off,level:2`)
- `CATNIP_THEME` : `auto` (défaut), `dark`, `light`
- `NO_COLOR` : `1` pour désactiver les couleurs (standard freedesktop.org)
- `CATNIP_FORMAT_INDENT_SIZE` : taille indentation (défaut: 4)
- `CATNIP_FORMAT_LINE_LENGTH` : longueur ligne max (défaut: 120)
- `CATNIP_WEIRD_LOG` : `on`/`off` pour activer/désactiver le logging des erreurs internes

## Inspecter la Configuration

```bash
# Voir config actuelle
$ catnip config show

# Avec sources (defaults, file, env, cli)
$ catnip config show --debug
```

En REPL, `/config` (sans argument) ouvre un éditeur interactif avec navigation clavier, toggle/cycle/édition inline et
sauvegarde immédiate. Voir [REPL](REPL.md#editeur-de-configuration-config) pour les détails.

<!-- doc-snapshot: config/show -->

```console
$ catnip config show
# /home/ari/.config/catnip/catnip.toml
cache_max_size_mb = 100
cache_ttl_seconds = 86400
enable_cache = true
executor = vm
jit = false
log_weird_errors = true
max_weird_logs = 50
memory_limit = 2048
no_color = false
optimize = 3
tco = true
theme = auto
```

<!-- doc-snapshot: config/show-debug -->

```console
$ catnip config show --debug
Configuration from: /home/ari/.config/catnip/catnip.toml

  cache_max_size_mb: 100  [default]
  cache_ttl_seconds: 86400  [default]
  enable_cache: True  [default]
  executor: 'vm'  [default]
  jit: False  [default]
  log_weird_errors: True  [default]
  max_weird_logs: 50  [default]
  memory_limit: 2048  [default]
  no_color: False  [default]
  optimize: 3  [default]
  tco: True  [default]
  theme: 'auto'  [default]
  --- format config ---
  format.align: True  [default]
  format.indent_size: 4  [default]
  format.line_length: 120  [default]
```

## Exemples Pratiques

### Dev : debugging avec optimisations désactivées

```bash
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
