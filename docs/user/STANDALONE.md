# Mode Standalone - Catnip Runtime Optimisé

Binaire standalone avec VM embarquée (support JIT compilé).

## Installation

Le binaire `catnip-standalone` est compilé et installé via :

```bash
make install-standalone
# Le binaire est installé dans .venv/bin/catnip-standalone
```

### Build manuel

```bash
cd catnip_rs
cargo build --bin catnip-standalone --features embedded --release
# Binary : target/release/catnip-standalone
```

## Usage

### Modes d'exécution

```bash
# Exécuter script
catnip-standalone script.cat

# Évaluer expression
catnip-standalone -c "x = 10; x * 2"

# Lire depuis stdin
echo "2 + 3" | catnip-standalone --stdin

# Version
catnip-standalone --version

# Mode verbose avec stats
catnip-standalone script.cat -v
# Affiche : parse time, compile time, execute time, stats VM
```

<!-- doc-snapshot: standalone/version -->

```console
$ catnip-standalone --version
Catnip Standalone, version ...
```

### Options JIT

```bash
# Désactiver le JIT (flag réservé)
catnip-standalone script.cat --no-jit

# Changer le seuil JIT (flag réservé)
catnip-standalone script.cat --jit-threshold 50
```

**Note** : dans la pipeline standalone actuelle, les options JIT sont acceptées mais
ne sont pas encore prises en compte par l'exécution.

### Mode Benchmark

```bash
# Benchmarker un script (10 iterations par défaut)
catnip-standalone bench script.cat

# Spécifier nombre d'iterations
catnip-standalone bench 20 script.cat

# Benchmark (options JIT acceptées, non appliquées pour l'instant)
catnip-standalone bench 10 script.cat --no-jit
```

### Info Runtime

```bash
catnip-standalone info
```

Output :

<!-- doc-snapshot: standalone/info -->

```text
Catnip Standalone Runtime
Version: 0.0.4

Features:
  - Rust VM: Yes (NaN-boxing)
  - JIT Compiler: Available (Cranelift x86-64)
  - Python Support: Embedded (PyO3)

Build Profile:
  - Mode: Release (LTO, optimized)

Usage:
  catnip-standalone script.cat
  catnip-standalone -c "x = 10; x * 2"
  echo "2 + 3" | catnip-standalone --stdin
  catnip-standalone bench script.cat  # Benchmark mode
```

## Cas d'Usage

### Scripts Production

```bash
#!/usr/bin/env catnip-standalone
# data_pipeline.cat

# Traitement batch
data = load_data()
processed = process(data)
save(processed)
```

### Benchmarking

```bash
# Benchmark (options JIT acceptées, non appliquées pour l'instant)
catnip-standalone bench 50 script.cat
catnip-standalone bench 50 script.cat --no-jit

# Profiler avec verbose
catnip-standalone script.cat -v > stats.txt
```

### CI/CD

```yaml
# .gitlab-ci.yml
script:
  - catnip-standalone validate.cat
  - if [ $? -eq 0 ]; then deploy; fi
```

## REPL Standalone

La REPL Rust vit dans le crate `catnip_repl` (`_repl.run_repl()`). Elle est disponible automatiquement avec `pip install catnip-lang` ou `make compile`. Aucun binaire séparé n'est nécessaire.

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

### Mode Debug

Le mode debug affiche l'IR optimisé et le bytecode sans exécuter :

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

```catnip
▸ x = "hello"
▸ x.<Tab>
capitalize  casefold  center  count  encode  endswith  find  format
index  isalnum  isalpha  join  lower  replace  split  strip  upper
```

### Messages d'erreur

Affichage avec contexte visuel (ligne/colonne + pointeur) :

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

Historique persistant dans `~/.catnip_history` (1000 entrées max).

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

Couleurs RGB 24-bit configurables dans `catnip_rs/src/constants.rs`.

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
