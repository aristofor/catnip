# Changelog

## 0.0.8 (unreleased)

Changements depuis v0.0.7 (2026-03-26).

### Langage

- **Type énuméré** : `enum Name { variant1; variant2 }` avec variantes qualifiées `Name.variant` et pattern matching
  dans `match`
- **Statement `import`** : `import('math')` bind automatiquement `math` dans le scope courant. Les formes expression
  (`m = import('math')`) et sélective (`import('math', 'sqrt')`) restent inchangées
- **Context managers (`with`)** : `with a = expr { body }` garantit `__exit__` en sortie de bloc. Multi-binding, cleanup
  en ordre inverse, suppression d'exception si `__exit__` retourne truthy
- **`try`/`except`/`finally`/`raise`** : gestion d'erreurs avec syntaxe match-like. Clauses typées
  (`e: TypeError => { }`), union de types (`ValueError | KeyError`), wildcard (`_ => { }`), binding optionnel
- **Types d'exception built-in** : `TypeError`, `ValueError`, `NameError`, `IndexError`, `KeyError`, `AttributeError`,
  `ZeroDivisionError`, `RuntimeError`, `MemoryError`
- **Hiérarchie d'exceptions** : `Exception` (racine), `ArithmeticError`, `LookupError`. Matching via MRO :
  `except ArithmeticError` catch `ZeroDivisionError`
- **Structs d'exception** : `struct AppError extends(RuntimeError) { message }` fonctionne avec le matching par
  hiérarchie
- **Pattern struct dans tuple** : `(Point{x, y}, z)` avec guards et dispatch par type
- **Complex natif** : `ExtendedValue::Complex(f64, f64)` dans la PureVM, `TAG_COMPLEX` dans la VM principale.
  Arithmétique, `.real`/`.imag`/`.conjugate()`, `abs()`, `hash()`, `complex()`, interop PyComplex. Les littéraux `j`/`J`
  compilent nativement (plus de fallback Python)
- **Module `http`** : `http.serve("<h1>Hello</h1>")` lance un serveur local et ouvre le navigateur. `http.Server(addr)`
  pour le contrôle fin (recv, respond). Content-type auto-détecté

### Outils

- **Formatter source-aware** : préserve l'intention du développeur. Layout inline/multiline, if/else, method chains et
  struct bodies suivent la source. Trailing comma reste le signal pour forcer multiline
- **Formatter try/except** : support complet du formatage try/except/finally/raise
- **Linter : 11 nouvelles règles** : code mort (W300, W301, W302), paramètres inutilisés (W201), shadowing (W204),
  métriques (I200 nesting, I201 cyclomatique, I202 longueur, I203 paramètres, I103 match sans catch-all)
- **Seuils configurables** : `--max-depth`, `--max-complexity`, `--max-length`, `--max-params` via CLI et API Python
- **Suppression inline `# noqa`** : `# noqa` (tous), `# noqa: E200` (spécifique), `# noqa: E200, W200` (multiple)
- **Analyse deep** (`--deep`) : W310 détecte les variables possiblement non initialisées. W311 détecte le code mort
  après des branches qui terminent toutes

### CLI

- **Expansion de fichiers** : `catnip lint` et `catnip format` acceptent dossiers (récursif `*.cat`) et globs
- **Shell completion** : fix perte des completions `plain` quand une entrée `file` suit
- **`--version` détaillé** : affiche commit et date de build (`-V` = version courte)

### Bugfixes

- Cache d'import : modules homonymes dans des répertoires différents ne se polluent plus
- `cached()` : fonctions retournant `None` correctement mémoïsées
- `cached()` : `f(a=1, b=2)` et `f(b=2, a=1)` partagent la même entrée de cache
- `--policy sandbox` lit correctement `[modules.policies.sandbox]` dans `catnip.toml`
- Policy : `import('.secret')` relatif vérifié contre la policy (plus contournable)
- CLI : `catnip bench N script.cat` et `catnip --no-jit -- script.cat` fonctionnent correctement
- Linter I100 : commentaire trailing après un tail call ne déclenche plus le hint TCO
- Linter W310 : fix faux positifs sur `match` exhaustif avec wildcard `_`
- Linter W310 : fix faux positifs sur variables définies dans `except` ou avant `try`

## 0.0.7 (2026-03-26)

Changements depuis v0.0.6 (2026-03-02).

### Langage

- **`typeof()`** : intrinsic natif retournant le nom du type (remplace `type()`)
- **`freeze()`/`thaw()`** : sérialisation binaire
- **`globals()` et `locals()`** : introspection du scope
- **Nil-coalescing `??`** : `a ?? b` retourne `a` si non-`None`, sinon évalue `b`
- **`in`/`not in`** : membership operators (list, tuple, dict, set, string)
- **`is`/`is not`** : identity operators (`x is None`)
- **`and`/`or` retournent bool** : utiliser `??` pour le pattern `value or default`
- **`fold` et `reduce`** : primitives d'agrégation
- **Import sélectif** : `import('math', 'sqrt', 'pi:p')`
- **Noms dotted** : `import('mylib.utils')` cherche `mylib/utils.cat`
- **Packages `lib.toml`** : répertoire avec manifeste, entry point, filtrage exports
- **Policies nommées** : `--policy <name>` avec `[modules.policies.<name>]` dans `catnip.toml`
- **`ND` et `RUNTIME`** : namespaces builtin (`ND.thread`, `RUNTIME.smallint_max`, etc.)
- **`META.file`** et **`META.main`** : chemin du fichier et détection d'exécution directe
- **Dot-continuation** : chaînage multilignes avec `.` en début de ligne
- **Struct fields** : `;` après les champs est désormais optionnel
- **Précédence `**` vs `-`** : `-x**2` donne `-(x**2)`

### Stdlib

- **Module `io`** : `print`, `write`, `writeln`, `eprint`, `input`, `open` (auto-importé en CLI/REPL)
- **Module `sys`** : `argv`, `environ`, `executable`, `version`, `platform`, `cpu_count`, `exit()`

### CLI

- **`catnip`** : binaire unique pour exécution et outils (remplace `catnip-run`)
- **`catnip lsp`** : serveur LSP (diagnostics, formatting, rename scope-aware)
- **Shell completion** : `catnip completion bash|zsh|fish`
- **`-q/--quiet`** : supprime l'affichage du résultat
- **`CATNIP_CONFIG`** : variable d'environnement pour config alternative
- **Suffixes `-m`** : `-m math:m` (alias), `-m io:!` (injection globals)
- **Validation stricte** : `-o` et pragmas rejettent les valeurs invalides

### REPL

- **Ctrl+C** : interrompt les exécutions longues
- **Ctrl+R** : recherche inversée dans l'historique
- **Complétion d'attributs** : après `.`, propose les attributs réels via `dir()`
- **Affichage struct** : `Point(x=74, y=5.3)` au lieu de `None`
- **Auto-indent** selon le niveau d'imbrication
- **`/context`** : inspecter les variables utilisateur
- **Multiline paste** : les lignes commençant par `.` sont jointes à la précédente

### Outils

- **MCP server** (`catnip-mcp`) : parse, eval, check, format, debug via Model Context Protocol
- **Formatter** : espacement, alignement en colonne, préservation multilignes, indentation des chaînes postfix
- **Linter** W200 scope-aware : "variable non utilisée" ne s'applique plus au scope global
- **Debugger** : sous-mode `repl` dans le scope du point d'arrêt

### Performance

- ND recursion ~200x plus rapide
- JIT warm-start : traces compilées chargées depuis le cache disque

### Bugfixes

- Segfault dans les boucles `for` avec mutation conditionnelle
- Crash réassignation struct entre types différents
- `float('nan')` retournait `0`
- F-strings : `f"{x}"` utilise maintenant `str(x)` correctement
- Closures capturent les variables englobantes dans `fold`, `map`, etc.
- Variables locales préservées après exécution JIT
- Positions UTF-8 correctes dans les messages d'erreur
- Import sélectif atomique (pas de globals partiellement modifié en cas d'erreur)
- Breakpoints dynamiques fonctionnels pendant une session debug active
- Overflow arithmétique détecté au lieu de wrapping silencieux
