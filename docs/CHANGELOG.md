# Changelog

## 0.0.7 (unreleased)

Changements depuis v0.0.6 (2026-03-02).

### BREAKING CHANGES

- **`META.path` renommé `META.file`** : aligné avec `__file__` Python
- **`print` et `input` retirés des builtins** : fournis par le module `io` (auto-importé en CLI/REPL)
- **`exit()` retiré des builtins** : utiliser `import("sys")` puis `sys.exit()`
- **`Decimal` retiré des builtins** : utiliser `import("decimal", "Decimal")`. Les littéraux `42d` restent disponibles
- **`pragma("nd_mode", "thread")` remplacé par `ND.*`** : utiliser `pragma("nd_mode", ND.thread)` etc.
- **Alias de pragmas retirés** : seule la forme canonique est acceptée
- **`and`/`or` retournent bool** : `"ok" and 42` donne `True` (pas `42`). Utiliser `??` pour `value or default`
- **`type()` renommé `typeof()`** : intrinsic natif retournant des strings. `type` retiré des builtins
- **Refonte `import()`** : résolution par nom uniquement, chemins de fichier retirés

### Langage

- **`freeze()`/`thaw()`** : sérialisation binaire. `freeze(value) -> bytes`, `thaw(bytes) -> value`
- **`typeof()` natif** : retourne le nom du type (`"int"`, `"float"`, `"bool"`, `"nil"`, `"string"`, nom du struct)
- **`globals()` et `locals()`** : intrinsics d'introspection du scope
- **Nil-coalescing `??`** : `a ?? b` retourne `a` si non-`None`, sinon évalue `b`
- **`in`/`not in`** : membership operators (list, tuple, dict, set, string)
- **`is`/`is not`** : identity operators (`x is None`)
- **Import sélectif** : `import("math", "sqrt", "pi:p")`
- **Noms dotted** : `import("mylib.utils")` cherche `mylib/utils.cat`
- **Packages `lib.toml`** : répertoire avec manifeste, entry point, filtrage exports
- **Auto-import par mode** : `[modules.repl]`, `[modules.cli]`, `[modules.dsl]` dans la config
- **Policies nommées** : `[modules.policies.<name>]` dans `catnip.toml`, `--policy <name>`
- **Wild import** : `META` et les noms préfixés `_` sont exclus
- **Struct fields** : `;` après les champs est désormais optionnel
- **Précédence `**` vs `-`** : `-x**2` donne `-(x**2)`
- **Dot-continuation** : chaînage multilignes avec `.` en début de ligne
- **`fold` et `reduce`** : primitives d'agrégation (`fold(xs, init, f)`, `reduce(xs, f)`)
- **`ND` et `INT`** : namespaces builtin (`ND.thread`, `INT.max`, etc.)
- **`META.main`** : `True` si exécuté directement, `False` si importé

### Stdlib

- **Module `io`** : `print`, `write`, `writeln`, `eprint`, `input`, `open`
- **Module `sys`** : `argv`, `environ`, `executable`, `version`, `platform`, `cpu_count`, `exit()`

### CLI

- **`catnip-run` renommé `catnip`** : binaire unique pour exécution et outils
- **`catnip lsp`** : serveur LSP (diagnostics, formatting, rename scope-aware)
- **`--version --full`** : affiche commit SHA et date de build
- **Shell completion** : `catnip completion bash|zsh|fish` (fichiers + sous-commandes)
- **Auto-load `io`** : `print()` disponible sans import en CLI et REPL
- **`CATNIP_CONFIG`** : variable d'environnement pour config alternative
- **Suffixes `-m`** : `-m math:m` (alias), `-m io:!` (injection globals)
- **Validation stricte** : `-o` et pragmas rejettent les valeurs invalides
- **`-q/--quiet`** : supprime l'affichage du résultat

### REPL

- **Ctrl+C** : interrompt les exécutions longues
- **Ctrl+R** : recherche inversée dans l'historique
- **Complétion d'attributs** : après `.`, propose les attributs réels via `dir()`
- **Affichage struct** : `Point(x=74, y=5.3)` au lieu de `None`
- **Auto-indent** : Enter indente selon le niveau d'imbrication
- **`/context`** : inspecter les variables utilisateur
- **`/help` dynamique** : synchronisé avec les commandes disponibles
- **Multiline paste** : les lignes commençant par `.` sont jointes à la précédente

### Outils

- **MCP server** (`catnip-mcp`) : serveur MCP pur Rust. 10 tools (parse, eval, check, format, debug) et resources
  (examples, docs, codex)
- Formatter : meilleur espacement, alignement en colonne, blocs multilignes préservés, commentaires inline normalisés
- Formatter : préservation des concaténations string multilignes
- Formatter : indentation des chaînes postfix (`.[]`, `.method()`, `.field`)
- Linter W310 scope-aware : le warning "variable non utilisée" ne s'applique plus au scope global
- Debugger : sous-mode `repl` dans le scope du point d'arrêt

### Performance

- ND recursion : frame stack au lieu de VMs imbriquées (~200x speedup sur récursion profonde)
- ND mode process : `pragma("nd_mode", ND.process)` distribue via `ProcessPoolExecutor`
- ND workers natifs : pool persistant de workers Rust avec IPC bincode (fallback Python transparent)
- JIT warm-start : traces compilées chargées depuis le cache disque dès la première itération

### Sécurité

- Import shadowing : `importlib` tenté avant la recherche filesystem
- Package traversal : `entry` dans `lib.toml` confiné au répertoire du package
- Import failure : module partiellement initialisé retiré de `sys.modules`
- Import policy : `import("..secret")` ne contourne plus `module_policy` via chemin relatif

### Bugfixes

**Crashes** :

- Segfault `for + if + affectation` dans une boucle avec mutation
- Crash réassignation struct quand les deux types diffèrent
- Crash `--parsing 0`
- JIT : heap corruption sur boucles top-level produisant des BigInt
- VM : double-free sur reassignment de variable struct (`b = B(0); b = B(1)` panic)

**Résultats incorrects** :

- `~~(lambda)` retournait nil
- `float('nan')` retournait `0` (collision bits NaN/SmallInt)
- JIT cache stale : code machine obsolète chargé sans vérification de version
- Variables locales perdues après exécution JIT d'une boucle
- `cached(() => { None })` réexécutait la fonction à chaque appel (nil traité comme cache miss)
- F-strings : `f"{x}"` appelle `format(x, '')` au lieu de `str(x)`
- Tail-call incorrect dans le body de boucles `while`/`for`
- Closures ne capturant pas les variables du scope englobant dans `fold`, `map`, etc.
- Arithmétique VM : overflow wrapping au lieu de checked ops

**CLI et config** :

- Erreur wrappée en double (`Error: TypeError: ...` au lieu de `TypeError: ...`)
- `catnip -o jit:off config show` échouait (options Python non reconnues avant délégation)
- `xxhash` manquant des dépendances runtime
- `config get`/`set` rejetaient `indent_size` et `line_length`
- `config get` ignorait les variables d'environnement `CATNIP_*`
- `DiskCache.clear()` supprimait tous les fichiers du répertoire au lieu des seuls fichiers Catnip

**Erreurs et positions** :

- Positions UTF-8 décalées (offsets bytes vs codepoints)
- Exceptions VM sans position source (ligne/colonne)
- `execute_quiet()` sans validation syntaxique (erreurs runtime trompeuses au lieu d'erreurs de syntaxe)
- Pragmas : valeurs invalides passaient silencieusement
- Pragma `push_state`/`pop_state` ne sauvegardaient pas TCO/JIT

**Debugger** :

- `vm_mode` non restauré après session, config CLI perdue, exit code ignoré, chemin source perdu
- Breakpoints dynamiques inopérants pendant une session active

**Module loader** :

- Policy non héritée par les sous-modules, cache ignorant le protocol
- Pollution `sys.modules` entre contextes
- Import sélectif non atomique : `globals` partiellement modifié en cas d'erreur

**MCP** :

- `eval_catnip` : contexte JSON, isolation entre appels, arrays/objects récursifs
- `parse_catnip` level 0 : retourne le s-expression tree-sitter brut
- `debug_breakpoint` fonctionnel, `debug_eval` avec garde sur session terminée
- Payloads normalisés entre serveurs Python et Rust

**Outils** :

- Formatter : alignement en colonne n'intervient plus dans les string literals multilignes
- Highlighting `=>` en contexte broadcast
- Linter : `import("math", "factorial:fact")` définit correctement `fact` dans le scope
- `--parsing 2` applique maintenant les optimisations

**Autres** :

- Pipeline : `tco_enabled` ignoré par `execute()`, `execute_quiet()`, `execute_timed()`
- TCO : le compilateur émet `TailCall` pour les appels récursifs terminaux (O(1) mémoire)
- ND recursion : `RecursionError` au lieu de segfault sur récursion infinie
- `debug()` builtin incompatible avec `logging.Logger` standard (passait `sep=` non supporté)
- VM : conversion CodeObject stricte (TypeError au lieu d'ignorer silencieusement les champs invalides)
