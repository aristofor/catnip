# Stratégie de Tests (Rust, Python, VM/AST)

Objectif: garder une couverture forte sans tester deux fois la même chose au même niveau.

## Principe directeur

Catnip a 2 chemins d'exécution:

- `VM` (bytecode + VM Rust)
- `AST` (exécution directe via Registry)

Ces 2 chemins justifient la duplication des **tests de comportement langage** (parité VM/AST), mais pas la duplication systématique des **tests internes de passes**.

## Qui teste quoi

### 1. Tests unitaires Rust

À placer côté Rust quand on valide:

- logique interne d'algorithmes
- transformations IR/Op (passes d'optimisation)
- structures internes (CFG, SSA, sérialisation pure, helpers bas niveau)

Exemples:

- `catnip_rs/src/semantic/tests/test_constant_folding.rs`
- `catnip_rs/src/semantic/tests/test_cse.rs`
- `catnip_rs/src/semantic/tests/test_blunt_code.rs`
- `catnip_rs/src/cfg/tests.rs`

### 2. Tests d'intégration Python

À placer côté Python quand on valide:

- pipeline complet `parse -> semantic -> execute`
- API publique (`Catnip`, CLI, wrappers PyO3)
- sémantique observable utilisateur

Exemples:

- `tests/language/`
- `tests/cli/`
- `tests/apps/`

### 3. Tests de parité VM/AST (obligatoires pour le langage)

Pour les comportements langage (pas les détails internes), le même test doit passer en `vm` et `ast`.

Utiliser `--executor` ou `CATNIP_EXECUTOR`:

- `pytest tests/language/ --executor vm`
- `pytest tests/language/ --executor ast`

## Anti-doublons: règle pratique

Avant d'ajouter un test Python dans `tests/optimization/`, vérifier:

1. Le comportement est-il déjà validé en unitaire Rust de la pass?
1. Si oui, le test Python apporte-t-il une valeur d'intégration réelle?

Valeur d'intégration réelle = au moins un des points suivants:

- interaction entre plusieurs passes
- effet visible via `Catnip.parse/execute`
- différence potentielle VM vs AST
- contrat API Python/CLI
- bug de régression historique côté pipeline complet

Si aucun point n'est vrai, garder le test côté Rust uniquement.

## Pattern recommandé pour la parité VM/AST

Préférer un seul test paramétré plutôt que 2 fichiers quasi identiques.

```python
import pytest
from catnip import Catnip

@pytest.mark.parametrize("executor", ["vm", "ast"])
def test_language_behavior_parity(executor):
    vm_mode = "on" if executor == "vm" else "off"
    c = Catnip(vm_mode=vm_mode)
    c.parse("x = 2 + 3; x * 4")
    assert c.execute() == 20
```

## Ce qui est volontairement dupliqué

- Scénarios langage critiques exécutés en `vm` et `ast`
- Régressions critiques sur chemin VM et/ou AST

## Ce qui doit être évité

- Refaire en Python (niveau API) les mêmes cas atomiques déjà couverts en Rust unitaire sur une pass
- Répliquer des assertions structurelles IR identiques dans 2 couches sans gain d'intégration

## Plan de migration conseillé

1. Garder en priorité les tests Rust unitaires pour `constant_folding`, `cse`, `blunt_code`, `optimizer`.
1. Réduire dans `tests/optimization/` les cas redondants, conserver:
   - smoke tests d'exposition API
   - tests d'interaction entre passes
   - tests de sémantique observable
1. Renforcer la parité VM/AST sur `tests/language/` avec paramétrisation plutôt que duplication manuelle.
