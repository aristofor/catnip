# Catnip Stdlib Extensions

Guide pour ecrire un module stdlib compile (Rust/PyO3) pour Catnip.

## Structure d'un module

```
catnip_libs/{name}/
    spec.toml              # Manifeste (source de verite)
    {backends.rust}/       # Chemin declare dans spec.toml (defaut: rust/)
        Cargo.toml         # Crate PyO3
        src/lib.rs         # Implementation
    tests/
        test_*.cat         # Tests d'integration
```

Le chemin du backend Rust est lu depuis `[backends] rust = "rust/"` dans spec.toml. Le generateur, le Makefile et
setup.py utilisent tous cette valeur.

## spec.toml

Manifeste du module. Le generateur `gen_stdlib_registry.py` le lit pour emettre les registres.

```toml
[module]
name = "mylib"                    # Nom d'import dans Catnip : import('mylib')
version = "0.1.0"
description = "What this module does"
needs_configure = false           # true si le module a besoin d'injection runtime (ex: sys.argv)

[exports]
symbols = ["PROTOCOL", "VERSION", "my_func"]

[exports.my_func]
signature = "(x: int) -> int"
description = "What it does"

[backends]
rust = "rust/"

[errors]                          # Optionnel
my_error = "Description of the error condition"

[compat]                          # Optionnel
notes = "Design rationale or compatibility notes"
```

### Champs `[module]`

| Champ             | Requis | Description                                                                   |
| ----------------- | ------ | ----------------------------------------------------------------------------- |
| `name`            | oui    | Nom d'import Catnip. Derive `catnip_{name}` pour le crate                     |
| `version`         | oui    | Version semver                                                                |
| `description`     | oui    | Description courte                                                            |
| `needs_configure` | non    | `true` si le module necessite une configuration post-import (defaut: `false`) |

## Contrat Rust

Chaque module doit exposer 3 fonctions dans `lib.rs` :

```rust
use pyo3::prelude::*;

/// Pour l'usage embarque (build_module est appele par le loader Rust).
pub fn build_module(py: Python<'_>) -> PyResult<Py<PyModule>> {
    let m = PyModule::new(py, "mylib")?;
    register_items(&m)?;
    Ok(m.unbind())
}

/// Logique partagee d'enregistrement.
fn register_items(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("PROTOCOL", "rust")?;
    m.add("VERSION", "0.1.0")?;
    m.add_function(wrap_pyfunction!(my_func, m)?)?;
    Ok(())
}

/// Point d'entree pour le chargement dynamique (.so).
#[pymodule]
fn catnip_mylib(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_items(m)
}
```

- `PROTOCOL = "rust"` : convention, permet de distinguer les modules Rust des modules Python
- `VERSION` : doit correspondre a `spec.toml`
- Le nom du `#[pymodule]` doit etre `catnip_{name}`

## Cargo.toml

```toml
[package]
name = "catnip-{name}"
version.workspace = true
edition.workspace = true

[lib]
name = "catnip_{name}"
crate-type = ["cdylib", "rlib"]     # cdylib pour .so, rlib pour embedded

[features]
default = ["extension-module"]
extension-module = ["pyo3/extension-module"]   # Pour maturin/pip install
embedded = ["pyo3/auto-initialize"]            # Pour cargo test

[dependencies]
pyo3 = { workspace = true }
```

Le dual-feature `extension-module`/`embedded` est le meme pattern que `catnip_rs`. Pour les tests Rust :

```bash
cargo test --lib --no-default-features --features embedded
```

## Tests

### Tests unitaires Rust

Dans `lib.rs`, section `#[cfg(test)]` :

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::Python;

    #[test]
    fn test_build_module() {
        Python::attach(|py| {
            let m = build_module(py).unwrap();
            let m = m.bind(py);
            assert_eq!(m.getattr("PROTOCOL").unwrap().extract::<String>().unwrap(), "rust");
        });
    }
}
```

### Tests d'integration `.cat`

Scripts dans `tests/` executes par `make test` :

```
# tests/test_constants.cat
import('mylib')

if mylib.PROTOCOL != "rust" { raise("PROTOCOL should be 'rust'") }
if mylib.VERSION != "0.1.0" { raise("VERSION should be '0.1.0'") }
```

### Lancer les tests

```bash
cd catnip_libs && make test           # Tout (Rust + .cat)
cd catnip_libs && make test-rust      # Rust uniquement
cd catnip_libs && make test-cat       # .cat uniquement
```

## Workflow : ajouter un module

```bash
# 1. Scaffolding
catnip new-lib mylib

# 2. Regenerer les registres (loader.py, resolve.rs, setup.py, Cargo.toml)
make gen-stdlib-registry

# 3. Implementer dans catnip_libs/mylib/rust/src/lib.rs

# 4. Compiler et installer
pip install -e .

# 5. Tester
cd catnip_libs && make test
```

Le generateur lit les `spec.toml` et met a jour 4 fichiers automatiquement. Il n'y a rien a modifier a la main dans le
core.

Note : le PureVM (`catnip_vm/src/stdlib.rs`) necessite une implementation manuelle separee si le module doit fonctionner
en mode standalone (sans Python).

## Extensions tierces

Les modules externes (non-stdlib) utilisent le hook `__catnip_extension__` :

```python
# my_ext/__init__.py
__catnip_extension__ = {
    'name': 'my-ext',
    'version': '1.0.0',
    'description': 'What it does',
    'register': lambda ctx: None,       # Hook d'initialisation (optionnel)
    'exports': {'my_var': 42},          # Injectes dans globals (optionnel)
}
```

Enregistrement via entry point dans `pyproject.toml` :

```toml
[project.entry-points."catnip.extensions"]
my-ext = "my_ext"
```

Decouverte et inspection :

```bash
catnip extensions list
catnip extensions info my-ext
```
