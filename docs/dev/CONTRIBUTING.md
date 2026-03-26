# Contribuer à Catnip

## Prérequis

- **Python** >= 3.12 avec headers (`python3-dev` / `python3-devel`)
- **Rust** (stable, édition 2021) avec `cargo` ([rustup.rs](https://rustup.rs))
- **uv** (gestionnaire de packages Python) ([docs.astral.sh/uv](https://docs.astral.sh/uv))
- **libgmp-dev** (requis par la crate `rug` pour l'arithmétique précision arbitraire)
- **tree-sitter-cli** (installé automatiquement par `make setup-dev`)

```bash
# Debian/Ubuntu
sudo apt install python3-dev libgmp-dev

# Fedora/RHEL
sudo dnf install python3-devel gmp-devel

# macOS (Homebrew)
brew install gmp
```

Optionnel :

- **Node.js + pnpm** (pour CodeMirror et l'extension VSCode)
- **fdfind** (`fd-find`) pour le tracking incremental du makefile

## Setup rapide

```bash
git clone <repo>
cd catnip

# Setup complet automatique (venv + deps + compile + install)
make setup
source .venv/bin/activate
```

C'est tout. `make setup` crée le venv, installe les dépendances, compile l'extension Rust, et installe le package
Python.

Vérifier que tout marche :

```bash
catnip -c "2 + 2"   # → 4
make test-quick      # Rust units + Python language (~10s)
```

## Setup manuel (étape par étape)

Si `make setup` ne convient pas, ou pour comprendre chaque étape :

### 1. Créer le venv

```bash
make venv
source .venv/bin/activate
```

### 2. Installer les dépendances de dev

```bash
make setup-dev
```

Installe : pytest, ruff, setuptools-rust, tree-sitter-cli, et les autres dépendances de build. Ne compile rien.

### 3. Compiler l'extension Rust

```bash
make compile
```

C'est l'étape la plus longue (~2 min la première fois). Le résultat est un `.so` dans `catnip/` que Python importe
directement.

Pour un build plus rapide en dev (incremental, thin LTO) :

```bash
CATNIP_DEV=1 make compile
```

### 4. Installer le package

```bash
make install
```

Installe le package Python en mode éditable + enregistre le serveur MCP.

## Vérifier l'installation

```bash
# Extension Rust chargée ?
python -c "import catnip._rs; print('ok')"

# CLI fonctionnelle ?
catnip -c "list(1, 2, 3).[* 2]"

# Tests rapides
make test-quick
```

## Boucle de dev quotidienne

### Après modification de code Rust

```bash
CATNIP_DEV=1 make compile   # Rebuild incremental (~10-30s)
make test-quick              # Validation rapide
```

### Après modification de la grammaire Tree-sitter

```bash
make grammar-deps            # Régénère parser.c + highlighters
make compile                 # Recompile avec le nouveau parser
make test                    # Tests complets
```

### Après modification de code Python uniquement

Rien à recompiler, le package est installé en mode éditable. Lancer les tests directement :

```bash
make test
```

## Commandes de test

| Commande              | Quoi                                | Durée  |
| --------------------- | ----------------------------------- | ------ |
| `make test-rust-fast` | Tests unitaires Rust                | ~5s    |
| `make test-quick`     | Rust units + Python language        | ~10s   |
| `make test`           | Tests Python complets (VM)          | ~25s   |
| `make test-vm`        | Tests Python mode VM (parallèle)    | ~15s   |
| `make test-ast`       | Tests Python mode AST (parallèle)   | ~15s   |
| `make test-all`       | Rust + Python VM + AST + standalone | ~2 min |

Règle pratique : `test-quick` pendant le dev, `test-all` avant de proposer un changement.

## Structure du projet

```
catnip/              # Module Python (API, CLI, context)
catnip_rs/           # Extension Rust principale (PyO3)
  src/
    parser/          # Transformateurs Tree-sitter → IR
    semantic/        # Analyseur sémantique + passes d'optimisation
    core/registry/   # Dispatch des opérations
    vm/              # VM bytecode (NaN-boxing, JIT)
    ir/              # Opcodes IR (source de vérité)
    cfg/             # CFG + SSA
    jit/             # JIT trace-based (Cranelift)
catnip_grammar/      # Grammaire Tree-sitter (grammar.js)
catnip_repl/         # REPL interactive (ratatui)
catnip_tools/        # Formatter + Linter
catnip_lsp/          # Serveur LSP (diagnostics, formatting, rename)
catnip_mcp/          # Serveur MCP utilisateur
tests/               # Tests Python d'intégration
  language/          # Tests comportement langage (parité VM/AST)
  optimization/      # Tests passes d'optimisation
  serial/            # Tests JIT (non parallélisables)
```

## Problèmes courants

### `make compile` échoue avec "can't find crate"

Le workspace Cargo a plusieurs crates. Vérifier que `Cargo.lock` est à jour :

```bash
cargo update
make compile
```

### Import error `catnip._rs`

L'extension Rust n'est pas compilée ou pas à la bonne place :

```bash
make compile
# Si ça persiste :
make reinstall-lang
```

### Tests qui échouent après modification d'opcodes

Les opcodes Python sont générés depuis Rust. Si les deux sont désynchronisés :

```bash
make check-opcodes   # Diagnostique le problème
make gen-opcodes     # Régénère les fichiers Python
make compile         # Recompile (gen-opcodes est aussi appelé automatiquement)
```
