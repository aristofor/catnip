# Linter

Analyse statique du code Catnip : syntaxe, style et sémantique.

## Vue d'ensemble

Le linter `catnip lint` effectue une analyse en quatre phases pour repérer les soucis avant exécution :

1. **Syntaxe** : le code respecte-t-il la grammaire ?
1. **Style** : le formatage suit-il les conventions ?
1. **Sémantique** : les variables sont-elles définies ? Les appels récursifs optimisables ?
1. **Deep** (opt-in) : analyse CFG inter-branches (variable possiblement non initialisée)

Chaque phase peut être exécutée indépendamment ou combinée.

> Le linter ne se contente pas de valider que le code peut s'exécuter. Il vérifie qu'il *mérite* de s'exécuter.

## Utilisation CLI

### Analyse complète

```bash
# Toutes les vérifications (syntaxe + style + sémantique)
catnip lint script.cat

# Dossier (récursif, tous les .cat)
catnip lint src/

# Multiple fichiers ou globs
catnip lint *.cat
catnip lint src/*.cat tests/*.cat
```

Les dossiers sont parcourus récursivement pour trouver les fichiers `.cat`. Les globs ne retiennent que les fichiers
`.cat` (les autres extensions et dossiers sont ignorés). Si aucun fichier `.cat` n'est trouvé, le message
`No .cat files found` est affiché.

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

### Seuils de métriques

```bash
# Ajuster les seuils (0 = désactiver la règle)
catnip lint --max-depth 8 script.cat       # I200: nesting depth (défaut: 5)
catnip lint --max-complexity 15 script.cat  # I201: complexité cyclomatique (défaut: 10)
catnip lint --max-length 50 script.cat      # I202: longueur de fonction (défaut: 30)
catnip lint --max-params 8 script.cat       # I203: nombre de paramètres (défaut: 6)

# Désactiver toutes les métriques
catnip lint --max-depth 0 --max-complexity 0 --max-length 0 --max-params 0 script.cat
```

### Suppression inline (`# noqa`)

Ajouter `# noqa` en fin de ligne pour supprimer les diagnostics sur cette ligne :

```catnip
x = compute()  # noqa           -- supprime tout sur cette ligne
y = value       # noqa: W200    -- supprime W200 seulement
z = other       # noqa: W200, E200  -- supprime plusieurs codes
```

Le commentaire n'affecte que la ligne où il apparaît.

### Analyse deep (CFG)

```bash
# Activer l'analyse inter-branches (W310, W311)
catnip lint --deep script.cat

# Combinable avec les autres options
catnip lint --deep --max-depth 8 script.cat
```

L'option `--deep` construit un CFG (Control Flow Graph) léger depuis le CST pour détecter des problèmes impossibles à
trouver en analyse linéaire : variables définies dans certaines branches seulement, code mort inter-branches.

Cette analyse est opt-in car plus coûteuse que les phases CST-only.

### Mode verbose

```bash
# Affiche "OK" pour les fichiers sans problème
catnip -v lint script.cat
```

## Codes de diagnostic

Les diagnostics suivent une convention de nommage :

- **Exxx** : Erreurs (empêchent l'exécution)
- **Wxxx** : Warnings (problèmes potentiels)
- **Ixxx** : Hints (suggestions d'amélioration)

### Syntaxe (E1xx)

| Code | Sévérité | Description       |
| ---- | -------- | ----------------- |
| E100 | Error    | Erreur de syntaxe |

Le message est contextuel : `Parse failed` si le parser échoue, ou un message précis avec position
(`at line N, column M`) si Tree-sitter localise un nœud `ERROR` dans l'arbre.

```bash
⇒ echo 'x = 1 +' | catnip lint --
<stdin>:1:1: error [E100]: Syntax error at line 1, column 8
```

### Style (W1xx)

| Code | Sévérité | Description                         |
| ---- | -------- | ----------------------------------- |
| W100 | Warning  | Line differs from formatted version |
| W101 | Warning  | Trailing whitespace                 |
| W102 | Info     | Expected N lines, got M             |

W100 compare chaque ligne avec la sortie du formatter Rust. W101 détecte les espaces en fin de ligne. W102 signale un
écart de nombre de lignes entre source et version formatée.

```bash
⇒ echo 'x=1+2  ' | catnip lint -l style --
<stdin>:1:1: warning [W100]: Line differs from formatted version
<stdin>:1:6: warning [W101]: Trailing whitespace
```

### Sémantique - Noms (E2xx/W2xx)

| Code | Sévérité | Description                                               |
| ---- | -------- | --------------------------------------------------------- |
| E200 | Error    | Name 'x' is not defined                                   |
| W200 | Warning  | Variable 'x' is defined but never used (local scope only) |
| W202 | Warning  | Wild import returns None; assignment is useless           |
| W203 | Warning  | Keyword used as variable name                             |

E200 détecte les références à des noms non définis dans le scope courant. W200 signale les variables assignées mais
jamais lues dans un **scope local** (lambda, for, match). Les variables au scope global sont ignorées : elles peuvent
constituer l'API publique d'un module consommée par un appelant externe. Le warning est aussi ignoré si le nom commence
par `_`. W202 avertit qu'assigner le retour d'un `import('...', wild=True)` est inutile puisque le wild import retourne
`None`. W203 avertit si un mot-clé du langage est utilisé comme nom de variable (`if = 5`).

```bash
⇒ echo 'y = x * 2' | catnip lint --
<stdin>:1:5: error [E200]: Name 'x' is not defined
```

```bash
⇒ echo 'f = () => { y = 1; 2 }' | catnip lint --
<stdin>:1:13: warning [W200]: Variable 'y' is defined but never used
```

### Sémantique - Noms suite (W2xx)

| Code | Sévérité | Description                  |
| ---- | -------- | ---------------------------- |
| W201 | Warning  | Parameter is never used      |
| W204 | Warning  | Variable shadows outer scope |

### Control flow (W3xx)

| Code | Sévérité | Description                                            |
| ---- | -------- | ------------------------------------------------------ |
| W300 | Warning  | Unreachable code after return                          |
| W301 | Warning  | Dead branch (condition always True/False)              |
| W302 | Warning  | `while True` without break (infinite loop détectable)  |
| W310 | Warning  | Variable possibly uninitialized (`--deep` requis)      |
| W311 | Warning  | Unreachable code after terminating branches (`--deep`) |

W300 détecte le code mort après un `return` dans le même bloc. W302 signale les boucles `while True` dont le corps ne
contient pas de `break` (les `break` dans des boucles ou lambdas imbriquées ne comptent pas). W201 signale les
paramètres de fonction jamais utilisés dans le corps (les paramètres prefixés par `_` et `self` sont ignorés). Les
paramètres lus dans une lambda imbriquée (capture) comptent comme utilisés. W204 détecte les affectations qui créent une
variable locale qui masque une variable du scope parent. L'heuristique distingue les mutations de capture (`x = x + 1`
dans une closure) du vrai shadowing. W301 signale les branches mortes quand la condition d'un `if` est un littéral
booléen (`if True` / `if False`).

W310 détecte les variables définies dans certaines branches d'un `if`/`elif`/`match` mais pas toutes, puis lues après le
point de jonction. Nécessite `--deep` car l'analyse construit un CFG et calcule un point fixe de dataflow (forward
definite-assignment). Les variables jamais définies nulle part ne déclenchent pas W310 (c'est le rôle de E200).

W311 détecte le code inatteignable après des branches qui terminent toutes : si chaque branche d'un `if`/`match`
contient un `return` ou `raise`, le code qui suit le bloc ne sera jamais exécuté. Complémente W300 (code mort intra-bloc
après un `return` isolé) en couvrant le cas inter-branches.

```bash
⇒ echo 'f = () => { return 1; x = 2 }' | catnip lint --
<stdin>:1:23: warning [W300]: Unreachable code after return
```

```bash
⇒ echo 'f = (x, y) => { x + 1 }' | catnip lint --
<stdin>:1:8: warning [W201]: Parameter 'y' is never used
```

```bash
⇒ printf 'if cond { x = 1 }\nprint(x)' | catnip lint --deep --
<stdin>:2:7: warning [W310]: Variable 'x' may be uninitialized (defined in some branches only)
```

### Hints (Ixxx)

| Code | Sévérité | Description                                           |
| ---- | -------- | ----------------------------------------------------- |
| I100 | Hint     | Recursive call to 'f' is not in tail position         |
| I101 | Hint     | Redundant comparison with boolean literal             |
| I102 | Hint     | Self-assignment has no effect                         |
| I103 | Hint     | Match has no wildcard/catch-all branch                |
| I200 | Hint     | Nesting depth exceeds threshold (default: 5)          |
| I201 | Hint     | Function cyclomatic complexity exceeds threshold (10) |
| I202 | Hint     | Function has too many statements (default: 30)        |
| I203 | Hint     | Function has too many parameters (default: 6)         |

I100 détecte les appels récursifs qui ne sont pas en position terminale et ne bénéficient donc pas du TCO (Tail Call
Optimization). I101 signale les comparaisons inutiles avec `True`/`False` (ex: `x == True` -> `x`). I102 repère les
auto-assignations (`x = x`).

I200 signale les structures de contrôle imbriquées au-delà du seuil (if, while, for, match, try). La profondeur se remet
à zéro aux limites de chaque fonction. I201 mesure la complexité cyclomatique par fonction : chaque branche (if, elif,
while, for, `and`, `or`) et chaque case d'un match (sauf le premier) ajoutent 1 au compteur. Les lambdas imbriquées sont
comptées séparément. I202 compte les statements directs dans le corps d'une fonction. I203 compte les paramètres d'une
fonction (`self` exclu pour les méthodes). I103 signale un `match` sans branche catch-all (`_` ou variable nue sans
guard).

```bash
⇒ echo 'f = (n) => { 1 + f(n - 1) }' | catnip lint --
<stdin>:1:14: hint [I100]: Recursive call to 'f' is not in tail position - consider restructuring for TCO
```

```bash
⇒ echo 'x = x' | catnip lint --
<stdin>:1:1: hint [I102]: Self-assignment has no effect
```

## Exemples pratiques

### Code avec erreur de syntaxe

```bash
⇒ cat broken.cat
factorial = (n) => {
    if n <= 1 { 1 }
    else { n * factorial(n - 1)  # missing }
}

⇒ catnip lint broken.cat
broken.cat:3:1: error [E100]: Syntax error at line 3, column ...
```

### Code avec problèmes sémantiques

```bash
⇒ cat issues.cat
compute = (x) => {
    y = x + 1
    temp = z * 2
    y
}

⇒ catnip lint issues.cat
issues.cat:3:12: error [E200]: Name 'z' is not defined
issues.cat:3:5: warning [W200]: Variable 'temp' is defined but never used
```

### Code propre

```bash
⇒ cat clean.cat
factorial = (n) => {
    if n <= 1 { 1 }
    else { n * factorial(n - 1) }
}
print(factorial(5))

⇒ catnip -v lint clean.cat
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

# Analyse deep (CFG)
result = lint_code(source, check_ir=True)

# Seuils personnalisés (0 = désactivé)
result = lint_code(source, max_nesting_depth=8, max_cyclomatic_complexity=15)
result = lint_code(source, max_function_length=50, max_parameters=10)
```

### Accès aux diagnostics

```python
from catnip._rs import Severity

result = lint_code(source)

# Filtrer par sévérité
errors = result.errors      # Severity.Error
warnings = result.warnings  # Severity.Warning

# Filtrer par code (string)
undefined = [d for d in result.diagnostics if d.code == "E200"]

# Chaque diagnostic expose : code, message, severity, line, column,
# source_line (optionnel), suggestion (optionnel)
for diag in result.diagnostics:
    print(diag)  # "1:5: error [E200]: Name 'x' is not defined"
```

## Architecture interne

Le linter est implémenté en Rust (`catnip_tools/src/linter.rs`) avec un wrapper Python léger (`catnip/tools/linter.py`).
Les quatre phases s'exécutent séquentiellement dans le même appel Rust.

### Phase 1 : Analyse syntaxique

```mermaid
flowchart LR
    A["Source"] --> B["Tree-sitter Parser"]
    B --> C["Parse Tree (ou erreur)"]
```

Utilise le parser Tree-sitter avec la grammaire compilée. Les nœuds `ERROR` dans l'arbre sont convertis en diagnostics
E100.

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

Compare le code source avec sa version formatée par le formatter Rust. Les différences génèrent des warnings W1xx.

Détection additionnelle :

- Trailing whitespace (scan par ligne)
- Écart de nombre de lignes (source vs formaté)

### Phase 3 : Analyse sémantique (CST walk)

```mermaid
flowchart LR
    A["Parse Tree"] --> B["CST Walk"]
    B --> C["Diagnostics"]
```

Parcourt le CST (Concrete Syntax Tree) directement en Rust avec un `ScopeTracker` qui :

- Maintient une pile de `ScopeFrame` (un frame par scope : lambda, for, match/case, except, with)
- Chaque frame contient ses propres `names`, `definitions` et `used`
- Distingue le genre de chaque définition via `DefKind` (Local, Param, VariadicParam, ForVar, MatchVar, WithVar,
  ExceptVar)
- Émet les diagnostics W200/W201 au `pop_scope()` (par frame, pas globalement)
- Résout `use_name()` en remontant la pile de scopes vers le parent
- Reconnaît les noms importés via `import('mod', 'name')` et les alias `import('mod', 'name:alias')`
- Pour les affectations : collecte les LHS d'abord, analyse le RHS, puis distingue shadowing (`W204`) vs mutation de
  capture selon que le RHS lit la variable du scope parent

La liste des builtins connus est générée automatiquement depuis `context.py` (source de vérité) par
`catnip_tools/gen_builtins.py`. Les builtins ne déclenchent pas d'erreur E200. `make check-builtins` vérifie la
synchronisation en CI.

### Phase 4 : Suggestions d'amélioration

Passes globales sur le CST, indépendantes de l'analyse sémantique :

- Appels récursifs hors position terminale (I100)
- Comparaisons redondantes avec booléens (I101)
- Auto-assignations (I102)
- Branches mortes sur conditions littérales (W301)
- Boucles infinies détectables (W302)
- Code mort après return (W300)
- Métriques : profondeur de nesting (I200), complexité cyclomatique (I201), longueur de fonction (I202), nombre de
  paramètres (I203), match sans catch-all (I103)

### Phase 5 : Analyse deep (CFG)

```mermaid
flowchart LR
    A["Parse Tree (CST)"] --> B["CFG Builder"]
    B --> C["LintCFG (blocs + edges)"]
    C --> D["Forward Dataflow"]
    D --> E["Diagnostics W310, W311"]
```

Activée par `--deep` / `check_ir=True`. Construit un CFG léger directement depuis le CST tree-sitter (pas depuis l'IR du
semantic analyzer). Chaque bloc trace les variables définies (`defs` avec byte offset pour l'ordre intra-bloc) et lues
(`reads`), les edges discriminent `CondTrue`/`CondFalse`/`LoopBack`/`LoopExit`/`Exception`.

L'analyse de dataflow calcule un point fixe : pour chaque bloc, l'ensemble des variables **définitivement définies** sur
tous les chemins entrants. Une lecture hors de cet ensemble déclenche W310.

> Ce CFG est distinct du CFG/SSA utilisé par l'optimiseur (`catnip_core/src/cfg/`). L'optimiseur travaille sur l'IR
> après semantic analysis. Le linter travaille sur le CST avant toute transformation.

## Différences avec l'exécution

| Aspect                      | Linter                          | Runtime                         |
| --------------------------- | ------------------------------- | ------------------------------- |
| **Variables non définies**  | Détecté statiquement (E200)     | `CatnipNameError` à l'exécution |
| **Variables non utilisées** | Warning W200/W201 (local scope) | Ignoré                          |
| **Code mort**               | W300, W301, W311 (--deep)       | Ignoré                          |
| **Init partielle**          | W310 (--deep, inter-branches)   | `NameError` à l'exécution       |
| **Complexité**              | Hints I200/I201/I202            | Pas de limite                   |
| **Tail position**           | Hint I100                       | TCO appliqué silencieusement    |
| **Performance**             | Rapide (pas d'exécution)        | Dépend du code                  |

Le linter détecte les problèmes *certains* (variables non définies, code mort après return) et les problèmes *probables*
(variables non utilisées, shadowing). Il ne peut pas détecter les erreurs qui dépendent des valeurs runtime.

## Limitations

1. **Analyse de flux opt-in** : L'analyse inter-branches (`--deep`) détecte les variables partiellement initialisées
   (W310) mais reste conservative : elle ne track pas les valeurs, seulement les définitions

1. **Pas d'inférence de types** : L'analyse de types est minimale, pas de système de types complet

1. **Modules externes** : Les modules chargés via `import('feature')` ne sont pas analysés

1. **Mutation de capture** : La distinction shadowing vs mutation repose sur une heuristique (le RHS lit-il la variable
   parente ?). Certains patterns indirects peuvent être mal classés

> Ces limitations reflètent un choix : mieux vaut un linter rapide avec quelques faux négatifs qu'un analyseur complet
> qui prend 10 secondes sur chaque fichier.
