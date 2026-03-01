# Changelog

## 0.0.6 (unreleased)

Changements depuis v0.0.5 (2026-02-18).

### BREAKING CHANGES

- Le préfixe non-déterministe passe de `@` à `~` (`~choose(xs)`, `~amb(1, 2, 3)`)

### Langage

- **Surcharge d'opérateurs** : `op +`, `op *`, etc. dans les structs
- **Héritage multiple** : linéarisation C3
- **Méthodes abstraites et statiques** : `@abstract`, `@static`
- **Imports relatifs** : résolution `./` et `../`, inférence d'extension `.cat`/`.py`
- **Wild import** : `import(spec, wild=True)`
- **Spread collections** : `list(*xs, 3, *ys)`, `dict(**base, key=val)`
- **Assignation chaînée** : `a = b = c = 42`
- **Deep broadcast** : traverse les structures imbriquées par défaut
- **Séparateur `;`** : sépare champs et méthodes dans les structs
- **Decimal exact** : type base-10 exact via `decimal.Decimal` (suffixe `d`/`D`), `0.1d + 0.2d == 0.3d`
- **Nombres complexes** : littéraux imaginaires `j`/`J` (`2j`, `1.5j`), arithmétique et attributs Python
- **Reverse operators** : `5 + S(10)` dispatche vers `S.op_add(self, 5)` quand le scalaire ne gère pas l'opération

### Performance

- Cache JIT persistant (traces + stencils natifs Cranelift)
- Nombreuses optimisations VM internes

### REPL

- Historique migré vers XDG (`$XDG_STATE_HOME/catnip/repl_history`), migration automatique depuis `~/.catnip_history`
- Coloration des résultats par type

### Bug fixes

- Crash ND recursion avec structs dans les closures
- `continue`/`break` dans un bras `match` en boucle
- Broadcast sur structs avec surcharge d'opérateurs
- Variables de boucle `for` ne fuitent plus dans le scope parent
- Littéraux numériques non-décimaux (`0xFF`, `0b1010`, `0o755`)
- Valeurs par défaut `None` dans les champs struct
- Match non exhaustif : erreur correcte au lieu de crash silencieux
- Divers fixes CLI, parser, linter
