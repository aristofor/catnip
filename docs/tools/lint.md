# Linter

Analyse statique du code Catnip : syntaxe, style et sémantique.

## Vue d'ensemble

Le linter `catnip lint` effectue une analyse en trois phases pour repérer les soucis avant exécution :

1. **Syntaxe** : le code respecte-t-il la grammaire ?
1. **Style** : le formatage suit-il les conventions ?
1. **Sémantique** : les variables sont-elles définies ? Les types cohérents ?

Chaque phase peut être exécutée indépendamment ou combinée.

> Le linter ne se contente pas de valider que le code peut s'exécuter. Il vérifie qu'il *mérite* de s'exécuter.

## Utilisation CLI

### Analyse complète

```bash
# Toutes les vérifications (syntaxe + style + sémantique)
catnip lint script.cat

# Multiple fichiers
catnip lint *.cat
catnip lint src/*.cat tests/*.cat
```

### Niveaux de vérification

```bash
# Syntaxe seulement (rapide)
catnip lint -l syntax script.cat

# Style seulement
catnip lint -l style script.cat

# Sémantique seulement
catnip lint -l semantic script.cat

# Tout (défaut)
catnip lint -l all script.cat
```

### Depuis stdin

```bash
# Pipe
echo 'x = y + 1' | catnip lint --stdin

# Fichier via cat
cat script.cat | catnip lint --stdin
```

### Mode verbose

```bash
# Affiche "OK" pour les fichiers sans problème
catnip -v lint script.cat
```

## Codes de diagnostic

Les diagnostics suivent une convention de nommage :

- **Exxx** : Erreurs (empêchent l'exécution)
- **Wxxx** : Warnings (problèmes potentiels)

### Syntaxe (E1xx)

| Code | Description                  |
| ---- | ---------------------------- |
| E100 | Erreur de syntaxe générique  |
| E101 | Token inattendu              |
| E102 | Bracket/parenthèse non fermé |
| E103 | String non terminée          |

```bash
$ echo 'x = 1 +' | catnip lint --
<stdin>:1:1: error [E100]: Unexpected end-of-input
```

### Style (W2xx)

| Code | Description                         |
| ---- | ----------------------------------- |
| W200 | Ligne diffère du format attendu     |
| W201 | Espaces en fin de ligne             |
| W202 | Newline manquante en fin de fichier |
| W203 | Indentation incohérente             |

```bash
$ echo 'x=1+2  ' | catnip lint -l style --
<stdin>:1:1: warning [W200]: Line differs from formatted version
<stdin>:1:6: warning [W201]: Trailing whitespace
```

### Sémantique - Noms (E3xx/W3xx)

| Code | Description                        |
| ---- | ---------------------------------- |
| E300 | Nom non défini                     |
| E301 | Fonction non définie               |
| W310 | Variable définie mais non utilisée |
| W311 | Variable masquée (shadowing)       |
| W312 | Redéfinition d'un builtin          |

```bash
$ echo 'y = x * 2' | catnip lint --
<stdin>:1:1: error [E300]: Name 'x' is not defined
```

```bash
$ echo 'x = 42' | catnip lint --
<stdin>:1:1: warning [W310]: Variable 'x' is defined but never used
```

### Sémantique - Types (W4xx)

| Code | Description                         |
| ---- | ----------------------------------- |
| W400 | Incompatibilité de type potentielle |
| W401 | Code inatteignable                  |
| W402 | Condition toujours vraie            |
| W403 | Condition toujours fausse           |

### Sémantique - Patterns (E5xx/W5xx)

| Code | Description           |
| ---- | --------------------- |
| E500 | Match non exhaustif   |
| W501 | Pattern inatteignable |
| W502 | Guard redondant       |

## Exemples pratiques

### Code avec erreur de syntaxe

```bash
$ cat broken.cat
factorial = (n) => {
    if n <= 1 { 1 }
    else { n * factorial(n - 1)  # missing }
}

$ catnip lint broken.cat
broken.cat:4:1: error [E101]: Unexpected token: '}'
```

### Code avec problèmes sémantiques

```bash
$ cat issues.cat
x = 42
y = z + 1
result = x * 2

$ catnip lint issues.cat
issues.cat:1:1: error [E300]: Name 'z' is not defined
issues.cat:1:1: warning [W310]: Variable 'y' is defined but never used
```

### Code propre

```bash
$ cat clean.cat
factorial = (n) => {
    if n <= 1 { 1 }
    else { n * factorial(n - 1) }
}
print(factorial(5))

$ catnip -v lint clean.cat
clean.cat: OK

No issues found
```

## Intégration CI/CD

### GitHub Actions

```yaml
name: Lint
on: [push, pull_request]

jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with:
          python-version: '3.12'
      - run: pip install catnip
      - run: catnip lint src/*.cat
```

### GitLab CI

```yaml
lint:
  image: python:3.12
  script:
    - pip install catnip
    - catnip lint **/*.cat
```

### Pre-commit hook

`.git/hooks/pre-commit` :

```bash
#!/bin/bash
for file in $(git diff --cached --name-only --diff-filter=ACM | grep '\.cat$'); do
    if ! catnip lint -l syntax "$file"; then
        echo "Lint failed for $file"
        exit 1
    fi
done
```

## Utilisation programmatique

### API simple

```python
from catnip.tools import lint_code, lint_file

# Depuis une string
result = lint_code('x = y + 1')
print(result.has_errors)  # True
print(result.summary())   # "1 error"

for diag in result.diagnostics:
    print(f"{diag.line}:{diag.column} [{diag.code}] {diag.message}")

# Depuis un fichier
from pathlib import Path
result = lint_file(Path('script.cat'))
```

### Configuration fine

```python
from catnip.tools import lint_code

# Syntaxe seulement
result = lint_code(source, check_syntax=True, check_style=False, check_semantic=False)

# Style seulement
result = lint_code(source, check_syntax=False, check_style=True, check_semantic=False)

# Tout (défaut)
result = lint_code(source)
```

### Accès aux diagnostics

```python
from catnip._rs import Severity

result = lint_code(source)

# Filtrer par sévérité
errors = result.errors      # Severity.Error
warnings = result.warnings  # Severity.Warning

# Filtrer par code (string)
undefined = [d for d in result.diagnostics if d.code == "E300"]

# Chaque diagnostic expose : code, message, severity, line, column,
# source_line (optionnel), suggestion (optionnel)
for diag in result.diagnostics:
    print(diag)  # "1:5: error [E300]: Name 'x' is not defined"
```

## Architecture interne

Le linter est implémenté en Rust (`catnip_rs/src/tools/linter.rs`) avec un wrapper Python léger (`catnip/tools/linter.py`). Les trois phases s'exécutent séquentiellement dans le même appel Rust.

### Phase 1 : Analyse syntaxique

```mermaid
flowchart LR
    A["Source"] --> B["Tree-sitter Parser"]
    B --> C["Parse Tree (ou erreur)"]
```

Utilise le parser Tree-sitter avec la grammaire compilée. Les nœuds `ERROR` dans l'arbre sont convertis en diagnostics
E1xx.

Si cette phase échoue (erreurs critiques), les phases suivantes sont ignorées.

### Phase 2 : Analyse stylistique

```mermaid
flowchart LR
    A["Source"] --> B["Formatter Rust"]
    B --> C["Formatted Source"]
    A --> D["Source vs Formatted"]
    C --> D
    D --> E["Différences ligne par ligne"]
```

Compare le code source avec sa version formatée par le formatter Rust. Les différences génèrent des warnings W2xx.

Détection additionnelle :

- Trailing whitespace (scan par ligne)
- Newlines en fin de fichier

### Phase 3 : Analyse sémantique (CST walk)

```mermaid
flowchart LR
    A["Parse Tree"] --> B["CST Walk"]
    B --> C["Diagnostics"]
```

Parcourt le CST (Concrete Syntax Tree) directement en Rust avec un `ScopeTracker` qui :

- Trace les scopes (push/pop sur pile de `HashSet`)
- Enregistre les définitions de variables
- Détecte les références à des noms non définis
- Collecte les variables non utilisées
- Gère les lambdas, for loops, match/case avec scopes imbriqués

> L'analyseur sémantique connaît les builtins Python (`print`, `len`, `range`, etc.) et les builtins Catnip (`pragma`).
> Ils ne déclenchent pas d'erreur E300.

## Différences avec l'exécution

| Aspect                      | Linter                   | Runtime                             |
| --------------------------- | ------------------------ | ----------------------------------- |
| **Variables non définies**  | Détecté statiquement     | `CatnipNameError` à l'exécution     |
| **Variables non utilisées** | Warning W310             | Ignoré                              |
| **Types**                   | Analyse limitée          | Erreur si incompatible              |
| **Pattern matching**        | Analyse exhaustivité     | `CatnipPatternError` si aucun match |
| **Performance**             | Rapide (pas d'exécution) | Dépend du code                      |

Le linter détecte les problèmes *certains* (variables non définies) et les problèmes *probables* (variables non
utilisées). Il ne peut pas détecter les erreurs qui dépendent des valeurs runtime.

## Limitations

1. **Analyse de flux limitée** : Le linter ne suit pas les branches conditionnelles pour déterminer si une variable est
   toujours définie

1. **Pas d'inférence de types** : L'analyse de types est minimale, pas de système de types complet

1. **Modules externes** : Les modules chargés via `import('feature')` ne sont pas analysés

> Ces limitations reflètent un choix : mieux vaut un linter rapide avec quelques faux négatifs qu'un analyseur complet
> qui prend 10 secondes sur chaque fichier.
