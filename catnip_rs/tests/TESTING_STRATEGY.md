# Stratégie de Tests Rust vs Python

## Principe

**Tests Rust = Source de vérité pour les features testables**

Les tests Rust sont prioritaires car :

- ⚡ Plus rapides (~0.2s vs ~1.5s Python)
- 🎯 Plus proches du code (pas de couche PyO3)
- 🔒 Testent directement le binaire standalone
- 📦 Pas de dépendances Python runtime

**Règle d'or** : Si testé en Rust → ne pas dupliquer en Python (sauf smoke test minimal)

## Organisation

### Tests de Régression Rust (`regression_*.rs`)

Tests ciblés pour bugs historiques spécifiques avec documentation inline.

#### `regression_chained_calls.rs` (10 tests)

```rust
// Bug: f(a)(b) causait ambiguïté avec unpacking
// Solution: External scanner pour newlines significatifs
```

- Appels chaînés 2-3 niveaux
- Avec/sans assignations intermédiaires
- Paramètres multiples, nested, edge cases

**Remplace en Python** : Anciens tests d'appels chaînés dans `test_lambda_correctness.py`

- Avant : ~5 tests Python détaillés
- Après : 1 smoke test Python + 10 tests Rust complets

#### `regression_newlines.rs` (26 tests)

```rust
// Bug: Newlines ambiguës (séparateurs vs whitespace)
// Solution: External scanner + extras
```

- Newlines significatives vs whitespace
- Séparateurs mixtes (`;` + `\n`)
- Multilignes dans arguments/listes/blocks
- Unpacking vs appels chaînés
- Nesting profond

**Remplace en Python** : Majorité de `test_statement_separators.py`

- Tests multilignes redondants supprimés

#### `regression_vmfunction.rs` (6 tests)

```rust
// Bug: Paramètres lambda → LoadConst au lieu de LoadLocal
// Symptôme: double(5) retournait "xx" au lieu de 10
```

- Paramètres vs constantes
- Closures
- Expressions multiples

**Conservé en Python** : `test_bug_vmfunction_returns_string` (1 smoke test bytecode)

### Tests Standalone (`standalone_*.rs`)

Tests du binaire compilé sans Python :

- `standalone_basic.rs` (15 tests)
- `standalone_broadcast.rs` (8 tests)
- `standalone_control_flow.rs` (15 tests)
- `standalone_functions.rs` (15 tests)
- `standalone_cli.rs` (8 tests)

**Total Rust** : 87 tests (61 standalone + 26 régression)

## Tests Python Conservés

Tests spécifiques à l'intégration Python et aspects non testables en Rust.

### `tests/bytecode/` (24 tests conservés)

**`test_lambda_correctness.py`** (15 tests, -25% après nettoyage)

- ✅ Structure bytecode (LoadLocal vs LoadConst)
- ✅ Closures (1 test au lieu de 3)
- ✅ Edge cases (defaults, pas de params)
- ✅ Smoke test appels chaînés (1 test référence Rust)

**`test_lambda_ir.py`** (9 tests)

- Structure IR (Ref vs Identifier)
- Transformation parser → IR
- Spécifique transformer Python/Rust

### `tests/language/` (~700 tests)

Tests d'intégration complète du langage :

- Features complexes multi-aspects
- Intégration Python (modules, builtins)
- Scénarios réels end-to-end

## Métriques

| Métrique         | Avant | Après | Δ     |
| ---------------- | ----- | ----- | ----- |
| **Tests Python** | 800   | 720   | -10%  |
| **Tests Rust**   | 43    | 87    | +102% |
| **Total**        | 843   | 807   | -4%   |
| **Temps Python** | ~8s   | ~7s   | -12%  |
| **Temps Rust**   | ~0.2s | ~0.5s | +150% |
| **Temps Total**  | ~8.2s | ~7.5s | -9%   |

**Gains qualitatifs** :

- 🎯 Meilleure couverture bugs historiques (régression)
- 📚 Auto-documentation via tests Rust
- ⚡ CI fail-fast (Rust first)
- 🔧 Facilite debugging (tests Rust plus directs)

## Workflow de Test

### 1. Nouveau Bug Découvert

```bash
# 1. Créer test Rust de régression
vim catnip_rs/tests/regression_mybug.rs

# 2. Vérifier qu'il échoue
cargo test --test regression_mybug

# 3. Fixer le bug
vim catnip_rs/src/...

# 4. Vérifier que le test passe
cargo test --test regression_mybug
```

### 2. Nouvelle Feature

```bash
# 1. Tests Rust d'abord (TDD)
vim catnip_rs/tests/standalone_myfeature.rs
cargo test --test standalone_myfeature --features embedded

# 2. Implémenter
vim catnip_rs/src/...

# 3. Smoke test Python si intégration PyO3 critique
vim tests/language/test_myfeature.py
pytest tests/language/test_myfeature.py
```

### 3. Intégration Python Spécifique

```bash
# Uniquement si feature non testable en Rust
pytest tests/bytecode/test_specific_integration.py
```

## Commandes

### Tests Rust

```bash
# Tous les tests (~0.5s)
make rust-test

# Unit tests uniquement (~0.3s)
make rust-test-fast

# Tests standalone (~0.2s)
make test-standalone
make test-standalone-fast  # dev build

# Test spécifique
cargo test --test regression_newlines
cargo test --test regression_newlines test_semicolon_then_newline
```

### Tests Python

```bash
# Tous les tests (~7s)
make test
pytest tests/

# Suite spécifique
pytest tests/bytecode/      # ~1.5s
pytest tests/language/      # ~5s

# Fichier spécifique
pytest tests/bytecode/test_lambda_correctness.py -v
```

### CI Pipeline (recommandé)

```yaml
test:
  script:
    # Fail fast sur Rust (rapide)
    - cargo test --all --no-default-features --features embedded
    - make test-standalone

    # Tests Python si Rust passe
    - pytest tests/ -v
```

## Règles de Contribution

### ✅ Créer un Test Rust Si...

- Bug de parsing/exécution reproductible en Rust
- Feature testable via `catnip-standalone -c "code"`
- Test d'intégration simple input/output
- Régression d'un bug historique

### ✅ Créer un Test Python Si...

- Test spécifique bytecode/IR
- Intégration PyO3 (marshalling Python↔Rust)
- Feature nécessitant modules Python
- Scénario complexe multi-features

### ❌ Ne PAS Dupliquer

- Tests d'exécution déjà couverts en Rust
- Tests de régression déjà documentés en Rust
- Tests multilignes/newlines (→ `regression_newlines.rs`)
- Tests appels chaînés (→ `regression_chained_calls.rs`)

## Exemples

### ✅ Bon : Test Rust de Régression

```rust
// catnip_rs/tests/regression_mybug.rs
/// Bug: Description du problème
/// Solution: Comment c'est fixé
#[test]
fn test_specific_case() {
    assert_output("code", "expected");
}
```

### ❌ Mauvais : Duplication Python

```python
# tests/language/test_mybug.py
def test_specific_case():
    """Teste la même chose que le test Rust ci-dessus"""
    assert execute("code") == "expected"
```

### ✅ Bon : Smoke Test Python

```python
def test_mybug_smoke_test():
    """
    Smoke test intégration Python.
    Tests complets dans catnip_rs/tests/regression_mybug.rs (10 tests)
    """
    assert execute("code") == "expected"
```

## Notes

- Les tests Rust utilisent `catnip-standalone` (feature `embedded`)
- Les tests Python utilisent extension PyO3 `catnip._rs`
- Pas de duplication sauf smoke tests explicitement documentés
- README.md dans `tests/` documente les tests standalone uniquement
