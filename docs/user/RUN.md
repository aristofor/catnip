# Mode standalone

Binaire standalone avec VM embarquée (support JIT compilé).

## Installation

Le binaire `catnip` est compilé et installé via :

```bash
make install-run
# Le binaire est installé dans .venv/bin/catnip
```

### Build manuel

```bash
cd catnip_rs
cargo build --bin catnip --no-default-features --features embedded --release
# Binary : target/release/catnip
```

## Usage

### Modes d'exécution

```bash
# Exécuter script
catnip script.cat

# Évaluer expression
catnip -c "x = 10; x * 2"

# Lire depuis stdin
echo "2 + 3" | catnip --stdin

# Version
catnip --version

# Mode verbose avec stats
catnip script.cat -v
# Affiche : parse time, compile time, execute time, stats VM
```

<!-- doc-snapshot: standalone/version -->

```console
$ catnip --version
catnip 0.0.7
```

### Options JIT

```bash
# Désactiver le JIT (flag réservé)
catnip script.cat --no-jit

# Changer le seuil JIT (flag réservé)
catnip script.cat --jit-threshold 50
```

**Note** : dans la pipeline standalone actuelle, les options JIT sont acceptées mais ne sont pas encore prises en compte
par l'exécution.

### Mode Benchmark

```bash
# Benchmarker un script (10 iterations par défaut)
catnip bench script.cat

# Spécifier nombre d'iterations
catnip bench 20 script.cat

# Benchmark (options JIT acceptées, non appliquées pour l'instant)
catnip bench 10 script.cat --no-jit
```

### Info Runtime

```bash
catnip info
```

Output :

<!-- doc-snapshot: standalone/info -->

```text
$ catnip info
Catnip Runtime
Version: 0.0.7

Features:
  - Rust VM: Yes (NaN-boxing)
  - JIT Compiler: Available (Cranelift x86-64)
  - Python Support: Embedded (PyO3)

Build Profile:
  - Mode: Release (LTO, optimized)

Usage:
  catnip script.cat
  catnip -c "x = 10; x * 2"
  echo "2 + 3" | catnip --stdin
  catnip bench script.cat  # Benchmark mode
```

## Cas d'Usage

### Scripts Production

```bash
#!/usr/bin/env catnip
# data_pipeline.cat

# Traitement batch
data = load_data()
processed = process(data)
save(processed)
```

### Benchmarking

```bash
# Benchmark (options JIT acceptées, non appliquées pour l'instant)
catnip bench 50 script.cat
catnip bench 50 script.cat --no-jit

# Profiler avec verbose
catnip script.cat -v > stats.txt
```

### CI/CD

```yaml
# .gitlab-ci.yml
script:
  - catnip validate.cat
  - if [ $? -eq 0 ]; then deploy; fi
```

## REPL Standalone

La REPL Rust vit dans le crate `catnip_repl` (`_repl.run_repl()`). Elle est disponible automatiquement avec
`pip install catnip-lang` ou `make compile`. Aucun binaire séparé n'est nécessaire.

Le binaire standalone `catnip-repl` reste disponible comme option alternative :

### Build binaire (optionnel)

```bash
make install-repl
# Binary installé : .venv/bin/catnip-repl
```

```bash
cd catnip_repl
cargo build --bin catnip-repl --features repl-standalone --release
# Binary : target/release/catnip-repl
```

### Commandes REPL

Toutes les commandes commencent par `/` :

| Commande         | Description                                |
| ---------------- | ------------------------------------------ |
| `/help`          | Afficher l'aide complète                   |
| `/exit`, `/quit` | Quitter la REPL                            |
| `/clear`         | Effacer l'écran                            |
| `/version`       | Informations version et build              |
| `/history`       | Afficher historique (20 dernières entrées) |
| `/load <file>`   | Charger et exécuter un fichier .cat        |
| `/stats`         | Statistiques d'exécution (variables, JIT)  |
| `/jit`           | Toggle compilateur JIT                     |
| `/verbose`       | Toggle mode verbose (timings)              |
| `/debug`         | Toggle mode debug (IR + bytecode)          |
| `/time <expr>`   | Benchmarker une expression                 |

<!-- doc-snapshot: standalone/repl-version -->

```console
▸ /version
Catnip REPL v0.0.7
Build: release mode
Features: JIT (Cranelift), NaN-boxing VM, Rust builtins
```

### Mode Debug

Le mode debug affiche l'IR optimisé et le bytecode sans exécuter :

<!-- check: no-check -->

```catnip
▸ /debug
Debug mode: enabled (shows IR and bytecode)

▸ x = 10 + 20
=== IR (after semantic analysis) ===
Assign(
    target: "x",
    value: BinOp(Add, Int(10), Int(20))
)

=== Bytecode ===
Instructions:
    0: LOAD_CONST(0)    # 10
    1: LOAD_CONST(1)    # 20
    2: BINARY_OP(Add)
    3: STORE_NAME("x")

Constants: 2 values
Names: ["x"]
```

Utile pour comprendre les optimisations appliquées et le code généré.

### Auto-complétion

Tab complète :

- Commandes REPL (`/he` → `/help`)
- Keywords (`wh` → `while`)
- Builtins (`pri` → `print`)
- Variables définies
- Méthodes après `.` (string/list/dict)

<!-- check: no-check -->

```catnip
▸ x = "hello"
▸ x.<Tab>
capitalize  casefold  center  count  encode  endswith  find  format
index  isalnum  isalpha  join  lower  replace  split  strip  upper
```

### Messages d'erreur

Affichage avec contexte visuel (ligne/colonne + pointeur) :

<!-- check: no-check -->

```catnip
▸ x = 10 +
Syntax error at line 1, column 8
    1 | x = 10 +
      |        ^
```

### Raccourcis clavier

| Raccourci | Action              |
| --------- | ------------------- |
| Ctrl+D    | Quitter             |
| Ctrl+C    | Annuler saisie      |
| ↑/↓       | Naviguer historique |
| Tab       | Auto-complétion     |

Historique persistant dans `$XDG_STATE_HOME/catnip/repl_history` (1000 entrées max, défaut
`~/.local/state/catnip/repl_history`).

### Syntax Highlighting

Coloration live en temps réel :

- Keywords : cyan bold (if, while, for, match)
- Constants : magenta bold (True, False, None)
- Types : teal (dict, list, tuple, set)
- Numbers : vert pâle
- Strings : orange
- Comments : gris
- Operators : gris clair
- Builtins : jaune (print, len, range)

Couleurs RGB 24-bit configurables dans `catnip_core/src/constants.rs`.

## Couverture du langage

Le mode standalone couvre 100% des features du langage Catnip : variables, fonctions, closures, structs, traits, pattern
matching, broadcasting, pragmas, imports (y compris sélectifs et relatifs), memoization (`cached`/`_cache`), runtime
introspection (`catnip.version`, `catnip.tco`, etc.), extensions, et module policies.

Les seules features non disponibles en standalone sont des couches d'adaptation de l'API d'embedding Python :

- `@pass_context` : injection du `Context` Python dans des fonctions hôtes (feature d'embedding, aucun script `.cat` ne
  l'utilise)
- `Catnip(context=ctx)` : subclassing du Context (API Python)
- Broadcast purity tracking : optimisation interne du registry Python (le broadcasting fonctionne, juste sans
  l'optimisation sur `@pure`)

## Limitations

### 1. Dépendance Python Runtime

Malgré le nom "standalone", le binary nécessite :

- Python installé (libpython.so)
- Package `catnip` accessible (`PYTHONPATH`)

**Workaround** : Installer avec `make install-lang` pour assurer disponibilité.

### 2. Pas de Cross-Compilation Facile

Le binary est lié à :

- Architecture CPU (x86-64)
- Version Python (3.9+)
- OS (Linux, macOS, Windows)

**Solution** : Builds séparés par plateforme.
