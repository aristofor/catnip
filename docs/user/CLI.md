# CLI

Guide pratique de la ligne de commande Catnip.

> Console basse friction : des flags comme vecteurs de trajectoire.

## Quand utiliser la CLI ?

La CLI Catnip sert surtout à deux usages :

### 1. Développement et Exploration (REPL)

- **Tester la syntaxe** Catnip interactivement
- **Débugger** des expressions et scripts
- **Explorer** les fonctionnalités du langage
- **Prototyper** rapidement des transformations

```bash
catnip  # Lance REPL
```

### 2. Scripts de Traitement de Données

- **Scripts one-off** de transformation de données
- **Configuration DSL** chargée depuis fichiers
- **Automatisation** de tâches simples

```bash
catnip transform_data.cat
```

### Note importante : DSL vs Standalone

**Si tu écris beaucoup de scripts Catnip standalone** → c'est souvent un signe que **Python sera plus adapté**.

Catnip est avant tout un **moteur DSL** :

| Use Case                                      | Recommandation                                        |
| --------------------------------------------- | ----------------------------------------------------- |
| Règles métier modifiables par administrateurs | **DSL** - Stockez scripts en DB, exécutez dans app    |
| Sandbox pour scripts utilisateur              | **DSL** - Isolation + APIs exposées                   |
| Pipelines ETL configurables                   | **DSL** - Workflows définis par utilisateurs          |
| Script de traitement ponctuel                 | **Standalone possible**, mais Python souvent meilleur |
| Application complète en Catnip                | **Pas le bon outil** - Utilisez Python                |

**Règle empirique** : si ton script Catnip dépasse ~200 lignes ou demande des imports compliqués, il vaut mieux basculer
sur Python et garder Catnip pour les parties configurables.

Voir [`docs/examples/embedding/`](../examples/embedding/) pour patterns d'intégration.

______________________________________________________________________

## Modes d'Exécution

### Runtime Rust (catnip)

Le binaire `catnip` est un runtime Rust avec Python embarqué (PyO3). Il sert de point d'entrée unique : l'exécution de
scripts est traitée directement par la VM Rust, et les sous-commandes outils (format, lint, cache, etc.) sont déléguées
au CLI Python.

```bash
# Exécution de scripts (Rust VM + JIT)
catnip script.cat
catnip -c "2 + 3"
echo "x = 10; x * 2" | catnip --stdin

# Sous-commandes outils (déléguées au CLI Python)
catnip format script.cat
catnip lint src/
catnip cache stats
catnip config show
catnip debug script.cat
```

**Délégation** : au lancement, le binaire inspecte les arguments avant le parsing Clap. Si le premier argument
positionnel est une sous-commande Python connue (`format`, `lint`, `cache`, `config`, `debug`, `repl`, `lsp`,
`commands`, `plugins`, `module`, `completion`, `extensions`), l'invocation est déléguée au CLI Python
(`catnip.cli:main`) via PyO3 embarqué. Sinon, Clap parse normalement pour l'exécution de scripts.

**Caractéristiques** :

- VM avec JIT
- Startup rapide pour scripts
- Accès à toutes les sous-commandes Python sans installation séparée

**Note** : les binaires ne sont pas inclus dans les wheels PyPI (manylinux) à cause des limites du Python statique. Ils
sont disponibles via :

- Installation locale : `make install-bins`
- GitHub releases : binaires précompilés par plateforme
- Cargo : `cargo install --path catnip_rs --bin catnip --features embedded`

Voir `docs/user/RUN.md` pour détails.

### REPL Interactif

Lance une session interactive (mode par défaut) :

```bash
catnip
```

La REPL maintient un contexte persistant entre les commandes :

```bash
$ catnip
```

<!-- check: no-check -->

```catnip
▸ x = 10
▸ x * 2
20
▸ factorial = (n) => { if (n <= 1) { 1 } else { n * factorial(n-1) } }
▸ factorial(5)
120
```

### Exécution de Script

#### Forme courte (fallback automatique)

```bash
catnip script.cat
```

Si l'argument n'est pas une sous-commande reconnue (`format`, `lint`), Catnip l'interprète comme un fichier à exécuter.

#### Forme explicite (avec `--`)

```bash
catnip -- script.cat
```

Le séparateur `--` force l'interprétation comme fichier, levant toute ambiguïté. Utile si un fichier s'appelle `format`
ou `lint`.

#### Avec options

```bash
# TCO activé
catnip -o tco:on script.cat

# Mode verbeux
catnip -v script.cat

# Multiple options
catnip -o tco:on -v --no-color script.cat
```

### Évaluation de Commande

Évalue une expression et affiche le résultat :

```bash
catnip -c "2 + 3 * 4"
# Output: 14

catnip -c "debug(42)"
# Output: 42

catnip -c "x = 10; x * 2"
# Output: 20
```

### Mode Pipe (stdin)

Lit depuis l'entrée standard :

```bash
echo "10 * 2" | catnip
# Output: 20

cat script.cat | catnip

# Avec options
echo "factorial(10)" | catnip -o tco:on
```

## Options Globales

### Configuration

#### `--config FILE`

Utilise un fichier de configuration alternatif :

```bash
# Utiliser une config custom
catnip --config my-catnip.toml script.cat

# Afficher la config utilisée
catnip --config my-catnip.toml config show

# Formatter avec config custom
catnip --config my-catnip.toml format code.cat
```

Par défaut, Catnip charge `~/.config/catnip/catnip.toml`. L'option `--config` permet de spécifier un fichier alternatif,
utile pour :

- Configs par projet (versionner `catnip.toml` dans git)
- Environnements différents (dev, staging, prod)
- Tests avec différentes configurations

Voir [Configuration](#config) pour le format du fichier.

### Options de Parsing

#### `--parsing LEVEL`

Niveau de parsing (0-3, défaut : 3) :

- `0` : Parse tree Tree-sitter (arbre brut)
- `1` : IR après transformer
- `2` : IR exécutable après analyse sémantique
- `3` : Exécute et affiche le résultat (défaut)

```bash
# Afficher l'IR
catnip --parsing 1 -c "2 + 3"

# Afficher l'IR optimisé
catnip --parsing 2 script.cat
```

**Note** : Cette option est principalement destinée au développement du langage et à l'inspection des résultats des
optimiseurs. Les utilisateurs finaux n'ont généralement pas besoin de modifier cette valeur (utiliser la valeur par
défaut `3`).

### Options d'Optimisation

#### `-o, --optimize OPT`

Active des optimisations (peut être utilisé plusieurs fois). Les options non reconnues ou les valeurs invalides lèvent
une erreur.

**TCO (Tail-Call Optimization)** :

```bash
# Active TCO
catnip -o tco script.cat
catnip -o tco:on script.cat

# Désactive TCO
catnip -o tco:off script.cat
```

**JIT (Just-In-Time Compilation)** :

```bash
catnip -o jit script.cat          # Active JIT (hot-loop detection)
catnip -o jit:off script.cat      # Désactive JIT
```

Les valeurs booléennes acceptées pour `tco` et `jit` : `on`/`off`, `true`/`false`, `1`/`0`, `yes`/`no`.

**Niveau d'optimisation** (défaut: `3` - optimisations complètes) :

```bash
catnip -o level:0 script.cat      # Aucune optimisation
catnip -o level:3 script.cat      # Toutes (défaut)
```

Niveaux, alias et détails : voir [Pragmas](../lang/PRAGMAS.md).

**Memory guard** (défaut: `2048` MB) :

```bash
catnip -o memory:4096 script.cat  # Limite 4 Go
catnip -o memory:0 script.cat     # Désactive le guard
```

La VM vérifie périodiquement le RSS du processus et lève `MemoryError` si la limite est dépassée. Actif par défaut à
2048 MB, Linux uniquement (no-op sur autres plateformes). Voir [VM](../dev/VM.md#memory-guard).

### Options de Chargement de Modules

#### `-m, --module MODULE`

Charge un module Python comme namespace global :

```bash
# Module installé
catnip -m math script.cat
# Dans le script : math.sqrt(16)

# Plusieurs modules
catnip -m math -m random script.cat
# Dans le script : math.sqrt(16), random.random()

# Alias : namespace renommé
catnip -m math:m script.cat
# Dans le script : m.sqrt(16)

# Injection directe dans les globals (pas de namespace)
catnip -m io:! script.cat
# Dans le script : print("BORN TO SEGFAULT") (directement accessible)
```

Par défaut, le module `io` est auto-chargé en mode wild en CLI et REPL (`print()` est disponible sans import). Les
modules `-m` s'ajoutent aux modules auto-importés (définis dans `[modules].auto` du `catnip.toml` ou dans le profil de
policy). La liste combinée est dédupliquée.

Voir `docs/user/MODULE_LOADING.md` pour les détails (suffixes, `import()`, policies).

#### `--policy PROFILE`

Sélectionne une policy de modules nommée depuis `[modules.policies.<name>]` dans `catnip.toml` :

```bash
catnip --policy sandbox script.cat
catnip --policy sandbox -c 'import('os')'
# => CatnipRuntimeError: module 'os' blocked by policy
```

La policy CLI prend priorité sur la section `[modules]` de `catnip.toml`.

Voir `docs/user/MODULE_LOADING.md` pour la configuration des policies.

### Options de Debug

#### `-v, --verbose`

Affiche les étapes détaillées du pipeline :

```bash
catnip -v script.cat
```

#### `--format FORMAT`

Format de sortie pour les niveaux de parsing 1-2 :

- `text` : JSON compact lisible (défaut) - primitives aplaties, métadonnées optionnelles omises
- `json` : JSON serde complet - chaque valeur wrappée dans un tagged enum, tous les champs présents
- `repr` : `repr()` Python - ancien format pformat, utile pour inspecter les objets Python bruts

```bash
# Défaut : compact JSON (lisible, parseable)
catnip --parsing 1 -c "2 + 3"

# JSON serde complet (pour analyse programmatique)
catnip --parsing 1 --format json -c "2 + 3"

# Ancien repr Python
catnip --parsing 1 --format repr -c "2 + 3"
```

Le format compact (défaut) produit du JSON lisible et parseable :

```json
[
  {
    "op": "Add",
    "args": [2, 3],
    "kwargs": {}
  }
]
```

Les champs `args` et `kwargs` sont toujours présents (même vides) pour diagnostiquer les bugs du parser. Les champs
`tail` et `pos` sont omis quand ils valent respectivement `false` et `[0, 0]`.

Le format `json` expose la structure serde complète (tagged enums, tous les champs) :

```json
[{
  "Op": {
    "opcode": "Add",
    "args": [{"Int": 2}, {"Int": 3}],
    "kwargs": {},
    "tail": false,
    "start_byte": 0,
    "end_byte": 5
  }
}]
```

La sortie est du JSON valide dans les deux cas, utilisable par pipe :

```bash
catnip --parsing 1 -c "2 + 3" | python -c "import sys,json; print(json.load(sys.stdin))"
```

**Note** : `--format` n'affecte que les niveaux de parsing 1 et 2. Le niveau 0 (parse tree) utilise toujours le format
texte Tree-sitter, et le niveau 3 (exécution) affiche le résultat.

#### `--theme THEME`

Sélectionne le thème de couleurs :

- `auto` : détecte le fond du terminal (défaut)
- `dark` : palette pour fond sombre
- `light` : palette pour fond clair

```bash
# Forcer le thème clair
catnip --theme light script.cat

# Forcer le thème sombre
catnip --theme dark script.cat
```

La détection automatique utilise `COLORFGBG` (xterm, rxvt, Konsole). Si la variable n'est pas définie, le thème sombre
est utilisé par défaut.

#### `--no-color`

Désactive la sortie colorée :

```bash
catnip --no-color script.cat
```

#### `-q, --quiet`

Supprime l'affichage du résultat final :

```bash
catnip -q script.cat
catnip -q -c "2 + 3"          # rien affiché
CATNIP_QUIET=1 catnip -c "42" # idem via env var
```

Les effets de bord (print, I/O) sont toujours exécutés, seul le résultat final est supprimé.

#### `--no-cache`

Désactive le cache disque de compilation (parsing et bytecode). Par défaut, le cache est **activé** et stocke les
résultats de parsing dans `~/.cache/catnip/` pour accélérer les exécutions suivantes.

```bash
# Exécuter sans utiliser le cache
catnip --no-cache script.cat

# Utile pour forcer une recompilation
catnip --no-cache -c "2 + 2"
```

**Par défaut** : Le cache est activé. Chaque script parsé est mis en cache avec sa configuration (niveau d'optimisation,
TCO, etc.).

**Désactivation persistante** : Via variable d'environnement (`CATNIP_CACHE=off`) ou config (fichier `catnip.toml`).

## Variables d'environnement

### Configuration

| Variable          | Description                                         | Valeurs                                               |
| ----------------- | --------------------------------------------------- | ----------------------------------------------------- |
| `CATNIP_CONFIG`   | Chemin vers un fichier de configuration alternatif  | chemin vers un `catnip.toml`                          |
| `CATNIP_CACHE`    | Active/désactive le cache disque                    | `off`, `false`, `0`, `no` pour désactiver             |
| `CATNIP_OPTIMIZE` | Options d'optimisation (même syntaxe que `-o`)      | `jit`, `tco:off`, `level:2`, combinables avec virgule |
| `CATNIP_EXECUTOR` | Mode d'exécution                                    | `vm` (défaut)                                         |
| `CATNIP_PATH`     | Répertoires de recherche pour `import()` (noms nus) | chemins séparés par `:` (ajoutés après CWD)           |
| `CATNIP_THEME`    | Thème de couleurs                                   | `auto` (défaut), `dark`, `light`                      |
| `CATNIP_QUIET`    | Supprime l'affichage du résultat (comme `-q`)       | `1`, `true`, `on` pour activer                        |
| `CATNIP_DEV`      | Compilation rapide (profil `fastdev`)               | `1` pour activer                                      |
| `NO_COLOR`        | Désactive les couleurs (standard freedesktop.org)   | toute valeur non vide                                 |

**Hiérarchie de priorité** (croissante) :

```
défauts → catnip.toml (ou CATNIP_CONFIG) → variables d'environnement → options CLI
```

```bash
# Activer JIT et réduire le niveau d'optimisation
CATNIP_OPTIMIZE=jit,level:2 catnip script.cat

# CLI override l'environnement
CATNIP_OPTIMIZE=jit catnip -o jit:off script.cat  # JIT désactivé

# Voir les sources des valeurs
catnip config show --debug
```

### Chemins XDG

| Variable          | Défaut           | Usage                                   |
| ----------------- | ---------------- | --------------------------------------- |
| `XDG_CONFIG_HOME` | `~/.config`      | Fichier config (`catnip/catnip.toml`)   |
| `XDG_STATE_HOME`  | `~/.local/state` | Historique REPL (`catnip/repl_history`) |
| `XDG_CACHE_HOME`  | `~/.cache`       | Cache de compilation (`catnip/`)        |
| `XDG_DATA_HOME`   | `~/.local/share` | Données persistantes (`catnip/`)        |

### Autres Options

#### `-V, --version`

Affiche la version de Catnip. `-V` donne la version courte, `--version` inclut le commit et la date de build :

<!-- doc-snapshot: cli/version -->

```console
$ catnip -V
catnip 0.0.8

$ catnip --version
catnip 0.0.8
  commit  6e0fe146
  build   2026-03-27-17:55:58
```

#### `--help`

Affiche l'aide :

<!-- doc-snapshot: cli/help -->

```console
$ catnip --help
Catnip runtime with embedded Python

Usage: catnip [OPTIONS] [FILE] [COMMAND]

Commands:
  info   Show runtime information
  bench  Benchmark a script (run N times)
  help   Print this message or the help of the given subcommand(s)

Arguments:
  [FILE]  Script file to execute

Options:
  -c, --command <CODE>                 Evaluate expression directly
      --stdin                          Read from stdin
  -v, --verbose                        Verbose output with execution statistics
      --no-jit                         Disable JIT compiler (enabled by default)
      --jit-threshold <JIT_THRESHOLD>  JIT threshold (number of iterations before compilation) [default: 100]
  -q, --quiet                          Suppress result display
  -b, --bench <N>                      Benchmark mode (run multiple times and show stats)
  -h, --help                           Print help
  -V, --version                        Print version

Python subcommands: cache, commands, config, debug, format, lint, lsp, module, plugins, repl
Run 'catnip <command> --help' for subcommand help.
```

## Sous-Commandes

Les sous-commandes sont extensibles via le système de plugins (entry points Python).

### `format`

Formate le code source Catnip selon les conventions du projet.

**Implémentation** : Rust

```bash
# Formater un fichier (affiche sur stdout)
catnip format script.cat

# Formater en place
catnip format -i script.cat

# Formater un dossier (récursif, tous les .cat)
catnip format -i src/

# Formater depuis stdin
echo "x=1+2" | catnip format --stdin

# Vérifier si formaté (exit 1 si pas formaté - utile en CI)
catnip format --check src/

# Afficher le diff
catnip format --diff script.cat

# Options de style
catnip format -l 80 script.cat
catnip format --indent-size 2 src/

# Avec config custom
catnip --config my-catnip.toml format script.cat
```

**Options :**

- `--stdin` : Lire depuis stdin
- `--in-place`, `-i` : Modifier fichiers en place
- `--check` : Vérifier formatage (exit 1 si non formaté)
- `--diff` : Afficher unified diff au lieu du code formaté
- `--indent-size N` : Taille indentation (défaut: 4 ou depuis config)
- `--line-length`, `-l N` : Longueur ligne max (défaut: 120 ou depuis config)
- `--align` : Aligner les `=` (assignments), `=>` (match arms) et `#` (trailing comments) sur les groupes de lignes
  consécutives

**Configuration** : Section `[format]` dans `catnip.toml` :

```toml
[format]
indent_size = 4
line_length = 120
align = false
```

**Variables d'environnement :**

- `CATNIP_FORMAT_INDENT_SIZE=2`
- `CATNIP_FORMAT_LINE_LENGTH=100`

**Priorité** : `défaut < catnip.toml < ENV < CLI flags`

**Règles de style** :

- Espaces autour des opérateurs binaires (`x + y`, `a == b`)
- Pas d'espace pour opérateurs unaires (`-x`, `not y`)
- Indentation 4 espaces par défaut (configurable)
- Espace avant `{`, après `,`
- Préserve commentaires et shebangs
- Max 2 newlines consécutifs

### `lint`

Analyse statique du code (syntaxe, style, sémantique) :

```bash
# Analyse complète
catnip lint script.cat

# Syntaxe seulement (rapide)
catnip lint -l syntax script.cat

# Style seulement
catnip lint -l style script.cat

# Sémantique seulement
catnip lint -l semantic script.cat

# Depuis stdin
echo "x = y + 1" | catnip lint --stdin

# Seuils de métriques (0 = désactiver la règle)
catnip lint --max-depth 8 script.cat       # I200: nesting (défaut: 5)
catnip lint --max-complexity 15 script.cat  # I201: cyclomatique (défaut: 10)
catnip lint --max-length 50 script.cat      # I202: longueur fn (défaut: 30)
catnip lint --max-params 8 script.cat       # I203: paramètres (défaut: 6)
```

Suppression inline par commentaire `# noqa` (bare ou avec codes spécifiques). Voir [docs/tools/lint](../tools/lint.md)
pour les codes de diagnostic et la syntaxe noqa.

### `commands`

Liste les commandes disponibles (built-ins + plugins) :

<!-- doc-snapshot: cli/commands -->

```console
$ catnip commands
command     source   status  help
----------  -------  ------  ------------------------------------------
cache       builtin  ok      Manage Catnip cache.
commands    builtin  ok      List available commands (built-ins +...
completion  builtin  ok      Generate shell completion script.
config      builtin  ok      Manage Catnip configuration.
debug       builtin  ok      Start an interactive debug session.
extensions  builtin  ok      Manage compiled extensions.
format      builtin  ok      Format Catnip code files.
lint        builtin  ok      Full code analysis (syntax + style +...
lsp         builtin  ok      Start Catnip LSP server.
module      builtin  ok      Inspect module access policies.
plugins     builtin  ok      List registered plugins and their status.
repl        builtin  ok      Start the interactive REPL (default mode).
```

```bash
# Liste sans résolution (plus rapide)
catnip commands --no-resolve
```

### `plugins`

Inspecte les plugins CLI enregistrés :

<!-- doc-snapshot: cli/plugins-entrypoints -->

```console
$ catnip plugins --entrypoints
plugin      source   status  entry_point
----------  -------  ------  ---------------------------------------------
cache       builtin  ok      catnip.cli.commands.cache:cmd_cache
commands    builtin  ok      catnip.cli.commands.commands:cmd_commands
completion  builtin  ok      catnip.cli.commands.completion:cmd_completion
config      builtin  ok      catnip.cli.commands.config:cmd_config
debug       builtin  ok      catnip.cli.commands.debug:cmd_debug
extensions  builtin  ok      catnip.cli.commands.extensions:cmd_extensions
format      builtin  ok      catnip.cli.commands.format:cmd_format
lint        builtin  ok      catnip.cli.commands.lint:cmd_lint
lsp         builtin  ok      catnip.cli.commands.lsp:cmd_lsp
module      builtin  ok      catnip.cli.commands.module:cmd_module
plugins     builtin  ok      catnip.cli.commands.plugins:cmd_plugins
repl        builtin  ok      catnip.cli.commands.repl:cmd_repl
```

```bash
# Liste tous les plugins
catnip plugins

# Valide le chargement de chaque plugin
catnip plugins --check

# Exclut les commandes built-in
catnip plugins --no-builtins
```

### `repl`

Démarre explicitement la REPL (équivalent au mode par défaut) :

```bash
catnip repl
```

### `config`

Gère la configuration persistante. Fichier par défaut : `~/.config/catnip/catnip.toml`

Aussi disponible en REPL via `/config` : sans argument, ouvre un éditeur interactif TUI. Les sous-commandes `show`,
`get`, `set`, `path` restent accessibles.

```bash
# Afficher toutes les valeurs
catnip config show

# Afficher avec les sources (default/file/env/cli)
catnip config show --debug

# Obtenir une valeur
catnip config get jit

# Définir une valeur
catnip config set jit true
catnip config set optimize 2
catnip config set executor ast

# Afficher le chemin du fichier
catnip config path

# Utiliser un fichier alternatif
catnip --config my-catnip.toml config show
catnip --config my-catnip.toml config set jit false
```

<!-- doc-snapshot: cli/config-show -->

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

Si `[modules]` est configuré dans le `catnip.toml`, `config show` affiche les modules auto-importés et la policy active.

<!-- doc-snapshot: cli/config-show-debug -->

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

**Format du fichier** : TOML avec sections

```toml
# ~/.config/catnip/catnip.toml

[repl]
no_color = false
theme = "auto"      # auto, dark, light

# Cache de compilation (parsing/bytecode)
enable_cache = true         # Activé par défaut

[optimize]
jit = false
tco = true
optimize = 3        # 0=none, 1=low, 2=medium, 3=high
executor = "vm"     # vm, ast
memory_limit = 2048 # RSS limit in MB (0 = disabled, Linux only)

[cache]
cache_max_size_mb = 100     # Limite 100 Mo (ou "unlimited")
cache_ttl_seconds = 86400   # TTL 24 heures (ou "unlimited")

[format]
indent_size = 4
line_length = 120

[diagnostics]
log_weird_errors = true     # Crash reports sur disque (défaut: true)
max_weird_logs = 50         # Nombre max de rapports (défaut: 50)
```

Voir `catnip.toml.example` dans le dépôt pour un fichier exemple commenté.

**Clés disponibles :**

- `no_color` (bool) : Désactive sortie colorée
- `theme` (str) : Thème de couleurs (`auto`/`dark`/`light`, défaut: `auto`)
- `jit` (bool) : Active JIT (expérimental)
- `tco` (bool) : Active tail-call optimization
- `optimize` (int) : Niveau optimisation (0-3)
- `executor` (str) : Mode exécution (vm/ast)
- `enable_cache` (bool) : Active le cache disque (défaut: `true`, via fichier)
- `cache_max_size_mb` (int|"unlimited") : Taille max cache en Mo
- `cache_ttl_seconds` (int|"unlimited") : TTL des entrées en secondes
- `memory_limit` (int) : Limite mémoire RSS en MB (défaut: `2048`, `0` = désactivé, Linux uniquement)
- `log_weird_errors` (bool) : Enregistre les erreurs internes sur disque (défaut: `true`)
- `max_weird_logs` (int) : Nombre max de crash reports conservés (défaut: `50`)

**Option `--debug`** : Montre d'où vient chaque valeur :

<!-- doc-snapshot: cli/config-show-debug-example -->

```bash
$ CATNIP_OPTIMIZE=jit catnip -o jit:off config show --debug
Configuration from: /home/ari/.config/catnip/catnip.toml

  cache_max_size_mb: 100  [default]
  cache_ttl_seconds: 86400  [default]
  enable_cache: True  [default]
  executor: 'vm'  [default]
  jit: False  [cli (-o jit:off)]
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

Sources possibles :

- `default` : valeur par défaut hardcodée
- `file` : fichier `catnip.toml`
- `env` : variable d'environnement (`CATNIP_CACHE`, `CATNIP_OPTIMIZE`, `CATNIP_THEME`, `NO_COLOR`)
- `cli` : option ligne de commande (`-o`, `--theme`, `--no-color`)

### `lsp`

Lance le serveur LSP (Language Server Protocol) pour l'intégration éditeur.

```bash
catnip lsp
```

Le serveur communique sur stdio (JSON-RPC) et expose :

- **Diagnostics** : lint automatique à l'ouverture et à chaque modification
- **Formatting** : formatage du document (`textDocument/formatting`)
- **Rename** : renommage scope-aware via tree-sitter (`textDocument/rename`, `textDocument/prepareRename`)

**Prérequis** : le binaire `catnip-lsp` doit être installé (`make install-bins`).

**VSCode** : l'extension Catnip détecte automatiquement le binaire `catnip-lsp` dans le PATH. Si nécessaire, configurer
le chemin vers `catnip` dans les settings :

```json
{
  "catnip.path": "catnip"
}
```

### `module`

Inspecte les policies d'accès aux modules.

```bash
# Lister les profils nommés depuis catnip.toml
catnip module list-profiles

# Vérifier quels modules sont autorisés sous un profil
catnip module check sandbox os math json subprocess
```

```console
$ catnip module list-profiles
  admin  (a69a9813)
  sandbox  (fd10df1f)

$ catnip module check sandbox os math json
  - os
  + math
  + json
```

Voir `docs/user/MODULE_LOADING.md` pour la configuration des policies.

### `cache`

Gère le cache de compilation. Le cache est **activé par défaut** et stocke le parsing et bytecode dans
`~/.cache/catnip/` (XDG_CACHE_HOME).

**Comportement par défaut** : Tous les scripts exécutés sont automatiquement mis en cache.

```bash
# Afficher statistiques du cache
catnip cache stats

# Nettoyer les entrées expirées (selon TTL et max_size)
catnip cache prune

# Simulation (dry-run)
catnip cache prune --dry-run

# Supprimer tout le cache
catnip cache clear
```

<!-- doc-snapshot: cli/cache-stats -->

```console
$ catnip cache stats
Disk Cache Statistics
==================================================
Directory:      /home/ari/.cache/catnip
Entries:        213
Volume:         1.78 MB (1871226 bytes)
Max size:       100.00 MB
TTL:            86400 seconds

Debug: CATNIP_CACHE_DEBUG=1 catnip ...
```

**Configuration du cache** :

```bash
# Définir taille maximale (en Mo)
catnip config set cache_max_size_mb 100

# Définir TTL (en secondes)
catnip config set cache_ttl_seconds 7200  # 2 heures

# Désactiver les limites
catnip config set cache_max_size_mb unlimited
catnip config set cache_ttl_seconds unlimited
```

**Format dans catnip.toml** :

```toml
# Cache activé par défaut
enable_cache = true           # Activer le cache disque (défaut)

[cache]
cache_max_size_mb = 100       # Limite 100 Mo (défaut)
cache_ttl_seconds = 86400     # TTL 24 heures (défaut)
# cache_max_size_mb = unlimited  # Pas de limite de taille
# cache_ttl_seconds = unlimited  # Pas d'expiration
```

**Statistiques affichées** :

- Nombre d'entrées
- Volume total (Mo)
- Limite de taille (si configurée)
- TTL (si configuré)

**Debug du cache** :

Pour voir les hits/misses en temps réel, activer le mode debug :

```bash
CATNIP_CACHE_DEBUG=1 catnip script.cat
```

Cela affiche sur stderr les événements cache (`[cache] HIT ...`, `[cache] MISS ...`).

**Comportement de `prune`** :

1. Supprime les entrées dont l'âge dépasse `cache_ttl_seconds`
1. Si la taille totale > `cache_max_size_mb`, supprime les entrées les moins récemment accédées (LRU)

### `debug`

Debugger interactif avec breakpoints et stepping. Pause l'exécution aux points d'arrêt et permet d'inspecter l'état de
la VM.

```bash
# Debugger un script avec breakpoints
catnip debug -b 5 -b 12 script.cat

# Debugger du code inline
catnip debug -c "x = 10; y = x * 2; y + 1" -b 1

# Plusieurs breakpoints
catnip debug -b 3 -b 7 -b 15 script.cat
```

**Options** :

- `-b, --break LINE` : Ajouter un breakpoint (répétable)
- `-c, --command CODE` : Code à débugger (au lieu d'un fichier)

**Commandes interactives** (au point d'arrêt) :

| Commande     | Alias    | Description                                  |
| ------------ | -------- | -------------------------------------------- |
| `continue`   | `c`      | Reprendre jusqu'au prochain breakpoint       |
| `step`       | `s`      | Pas à pas (entre dans les appels)            |
| `next`       | `n`      | Pas à pas (saute les appels)                 |
| `out`        | `o`      | Sort de la fonction courante                 |
| `break N`    | `b N`    | Ajouter un breakpoint à la ligne N           |
| `rbreak N`   | `rb N`   | Retirer un breakpoint                        |
| `print EXPR` | `p EXPR` | Évaluer une expression dans le scope courant |
| `vars`       | `v`      | Afficher les variables locales               |
| `list`       | `l`      | Afficher le source autour de la position     |
| `backtrace`  | `bt`     | Afficher la pile d'appels                    |
| `repl`       |          | Sous-mode REPL dans le scope courant         |
| `quit`       | `q`      | Arrêter l'exécution                          |
| `help`       | `h`      | Aide                                         |

Appuyer sur Entrée sans commande répète le dernier `step`. La commande `repl` ouvre une session interactive persistante
dans le scope du point d'arrêt (voir [Debugger](../tools/debug.md#sous-mode-repl)).

### `completion`

Genere un script de completion pour le shell courant. Les completions sont synchronisees automatiquement avec les
sous-commandes, options et valeurs -- rien a maintenir manuellement.

```bash
# Bash
eval "$(catnip completion bash)"

# Zsh
eval "$(catnip completion zsh)"

# Fish
catnip completion fish | source
```

**Installation persistante** :

```bash
# Bash
catnip completion bash >> ~/.bashrc

# Zsh
catnip completion zsh >> ~/.zshrc

# Fish
catnip completion fish > ~/.config/fish/completions/catnip.fish
```

La completion couvre :

- Sous-commandes (`format`, `lint`, `config`, etc.)
- Options globales (`-o`, `-v`, `--theme`, etc.)
- Valeurs d'options (`-o tco:on`, `--theme dark`, etc.)
- Niveaux de parsing (`--parsing 0..3`)
- Fichiers et dossiers (pour l'argument script positional)

## Plugins CLI

La CLI supporte l'ajout de sous-commandes via des packages Python externes.

### Créer un plugin

```python
# mon_plugin/__init__.py
import click

@click.command("mycommand")
@click.argument("file")
@click.pass_context
def mycommand(ctx, file):
    """Ma commande custom."""
    verbose = ctx.obj.get("verbose", False)
    # ...
```

```toml
# pyproject.toml du plugin
[project.entry-points."catnip.commands"]
mycommand = "mon_plugin:mycommand"
```

### Utiliser un plugin

```bash
pip install mon-plugin
catnip mycommand file.cat  # Automatiquement disponible
```

Les options globales (`-v`, `--no-color`, `--theme`, etc.) sont accessibles via `ctx.obj`.

## Exemples Complets

### Exécution Simple

```bash
# REPL
catnip

# Script
catnip script.cat

# Commande
catnip -c "print('Hello, Catnip!')"

# Pipe
echo "2 + 3" | catnip
```

### Avec Optimisations

```bash
# TCO pour récursion profonde
catnip -o tco:on recursive_fibonacci.cat

# Mode verbeux + TCO
catnip -v -o tco:on script.cat
```

### Avec Modules Python

```bash
# Module installé
catnip -m math script.cat
# Utilisation : math.sqrt(16)

# Plusieurs modules
catnip -m math -m random script.cat
# Utilisation : math.sqrt(16), random.random()
```

Pour alias et chargement par nom, utiliser `import()` dans le code :

<!-- check: no-check -->

```catnip
m = import('math')
host = import('host', protocol='py')
```

### Debugging

```bash
# Afficher l'IR
catnip --parsing 2 -c "x = 10; x * 2"

# Mode verbeux
catnip -v script.cat

# Sans couleurs (pour logs)
catnip --no-color script.cat > output.log
```

#### Messages d'Erreur et Stack Traces

Les erreurs runtime affichent la position source complète avec pile d'appels :

<!-- doc-snapshot: cli/error-not-callable -->

```bash
$ catnip -c 'x = 1; x()'
TypeError: 'int' object is not callable
  1 | x = 1; x()
    |        ^
```

**Format** : `fichier:ligne:colonne: message` avec snippet source et caret pointant sur l'erreur.

**Suggestions "Did you mean?"** : quand une variable ou un attribut struct est mal orthographié, Catnip suggère le nom
le plus proche :

```bash
$ catnip -c 'factorial = 1; factoral'
Name 'factoral' is not defined
  Did you mean 'factorial'?
```

```bash
$ catnip -c 'struct Config { name; value; }; c = Config("a", 1); c.naem'
'Config' has no attribute 'naem'. Did you mean 'name'?
```

Les mots-clés d'autres langages sont aussi détectés :

```bash
$ catnip -c 'class Foo { x }'
Unexpected token 'class Foo' at line 1, column 1. Did you mean 'struct'?
```

**Stack traces imbriquees** :

<!-- doc-snapshot: cli/error-division-by-zero -->

```bash
$ catnip -c 'f = (x) => { x / 0 }; g = (y) => { f(y) }; g(42)'
ZeroDivisionError: division by zero
  1 | f = (x) => { x / 0 }; g = (y) => { f(y) }; g(42)
    |              ^
```

Le message inclut le snippet source avec un caret (`^`) pointant sur l'instruction fautive.

### Combinaisons Avancées

```bash
# Tout ensemble
catnip -v -o tco:on -m math --no-color script.cat

# REPL avec module chargé
catnip -m math
# Puis dans la REPL : math.sqrt(42)

# Pipe avec options
cat data.cat | catnip -o tco:on -v
```

## Ordre des Arguments

```mermaid
flowchart TD
    A["Invocation catnip ..."] --> B{"Pré-parse :<br/>sous-commande Python ?<br/>(format/lint/cache/...)"}
    B -->|Oui| C["Délégation CLI Python<br/>(PyO3 embarqué)"]
    B -->|Non| D{"Sous-commande Rust ?<br/>(info/bench)"}
    D -->|Oui| E["Exécution Rust directe"]
    D -->|Non| F{"Option -c présente ?"}
    F -->|Oui| G["Évaluer expression inline"]
    F -->|Non| H{"Entrée stdin pipée ?"}
    H -->|Oui| I["Lire depuis stdin"]
    H -->|Non| J{"Argument fichier présent ?"}
    J -->|Oui| K["Exécuter script (VM Rust)"]
    J -->|Non| L["Erreur : pas d'input"]
```

L'ordre recommandé pour la clarté :

```bash
catnip [OPTIONS] [--] [SCRIPT|SUBCOMMAND]
```

**Exemples** :

```bash
# Options avant le fichier
catnip -o tco:on -v script.cat        # Recommandé

# Séparateur pour lever l'ambiguïté
catnip -o tco:on -- format.cat        # 'format.cat' est un fichier, pas une commande

# Fallback automatique
catnip format.cat                     # Ambigu (fichier ou commande ?)
catnip -- format.cat                  # Explicite : c'est un fichier
```

## Codes de Sortie

| Code | Signification                                                       |
| ---- | ------------------------------------------------------------------- |
| `0`  | Succès (exécution, formatage, lint sans erreur)                     |
| `1`  | Erreur (fichier introuvable, erreur de syntaxe, erreur d'exécution) |

Convention Unix standard. `format --check` retourne `1` si le code n'est pas formaté. `lint` retourne `1` si des
diagnostics sont trouvés.

```bash
# Utilisable dans des scripts shell
catnip script.cat && echo "OK" || echo "ERREUR"

# CI : vérifier le formatage
catnip format --check src/ || exit 1
```

## Notes Techniques

### Fallback Script

Si l'argument n'est pas une option (`-x`) ni une sous-commande reconnue, Catnip le traite comme un fichier script.

```bash
catnip script.cat        # Fallback → exécute le fichier
catnip format script.cat # Sous-commande format
catnip lint script.cat   # Sous-commande lint
catnip -- format.cat     # Force fichier (même si "format" existe)
```

### Séparateur `--`

Le séparateur `--` garantit qu'aucune confusion ne peut survenir :

```bash
catnip format.cat       # Ambigu : fichier "format.cat" ou commande "format" ?
catnip -- format.cat    # Explicite : c'est le fichier "format.cat"
catnip format file.cat  # Explicite : commande "format" sur fichier "file.cat"
```

### Contexte REPL

La REPL maintient un contexte persistant :

```bash
$ catnip
▸ x = 10          # Définit x
▸ y = x * 2       # Utilise x
▸ y
20
▸ quit()
```
