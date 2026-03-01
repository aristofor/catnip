# Tests d'Intégration Standalone

Tests d'intégration pour le binaire `catnip-standalone`.

## Qu'est-ce qui est testé ?

Ces tests vérifient que le binaire Rust standalone fonctionne correctement end-to-end :

- Parsing du code Catnip
- Exécution via la VM
- Sortie correcte

## Différence avec les Tests Python

| Tests Python (792)                  | Tests Rust Standalone (60+)            |
| ----------------------------------- | -------------------------------------- |
| Runtime Python + extensions Rust    | Binaire Rust pur avec Python embarqué  |
| Tests d'intégration langage complet | Tests d'intégration binaire standalone |
| API `Catnip().parse().execute()`    | CLI `catnip-standalone -c "code"`      |

**Complémentarité** : Les tests Python valident le langage, les tests standalone valident le déploiement binaire.

## Suites de Tests

### `standalone_basic.rs` (15 tests)

- Literaux (int, float, bool, string)
- Arithmétique et opérateurs
- Variables et scopes
- Listes, tuples, dicts
- Blocs
- Gestion d'erreurs

### `standalone_control_flow.rs` (15 tests)

- `if`/`elif`/`else`
- `while` et `for` loops
- `break` et `continue`
- Pattern matching avec `match`
- Guards et OR patterns

### `standalone_functions.rs` (15 tests)

- Lambdas simples et avec paramètres
- Closures
- Récursion (factorial, fibonacci)
- Tail-call optimization (TCO)
- Higher-order functions

### `standalone_broadcast.rs` (8 tests + 1 ignoré)

- Map (`.[* 2]`)
- Filter (`.[if > 5]`)
- Masques booléens
- Broadcasting de fonctions
- Chaînage

### `standalone_cli.rs` (8 tests)

- Flags CLI (`--version`, `--help`, `-v`)
- Mode stdin (`--stdin`)
- Subcommande `info`
- Multi-statements

## Lancer les Tests

### Tous les tests

```bash
make test-standalone        # Release build (optimisé)
make test-standalone-fast   # Dev build (rapide)
```

### Suite spécifique

```bash
cd catnip_rs
cargo test --test standalone_basic --features embedded --release
cargo test --test standalone_functions --features embedded --release
```

### Test individuel

```bash
cd catnip_rs
cargo test --test standalone_basic test_arithmetic --features embedded --release
```

## Prérequis

Le binaire `catnip-standalone` doit être compilé avant de lancer les tests :

```bash
cargo build --bin catnip-standalone --features embedded --release
```

Les tests lancent automatiquement `target/release/catnip-standalone`.

## Ajouter de Nouveaux Tests

1. **Créer un nouveau fichier** dans `tests/` (ex: `standalone_myfeature.rs`)
1. **Importer le module common** :
   ```rust
   mod common;
   use common::assert_output;
   ```
1. **Écrire des tests** :
   ```rust
   #[test]
   fn test_my_feature() {
       assert_output("2 + 2", "4");
   }
   ```

## Helpers Disponibles

Dans `common/mod.rs` :

- `assert_output(code, expected)` - Vérifie la sortie
- `assert_error(code)` - Vérifie qu'une erreur se produit
- `assert_error_contains(code, fragment)` - Vérifie le message d'erreur
- `run_code(code)` - Lance le code et retourne `Output`
- `run_code_stdout(code)` - Lance le code et retourne stdout

## Résultats Actuels

```
standalone_basic:       15 passed
standalone_broadcast:    8 passed, 1 ignored
standalone_cli:          8 passed
standalone_control_flow: 15 passed
standalone_functions:    15 passed
─────────────────────────────────────
Total:                   61 passed, 1 ignored
```

## CI/CD

Les tests standalone sont exécutés en CI via :

```yaml
- name: Test Standalone Binary
  run: make test-standalone
```
