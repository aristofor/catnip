# Changelog

## 0.0.9 (unreleased)

### Langage

- **Unions taggées (ADT)** : `union Name[T] { Variant(field: T); Nullary; }` déclare un type somme fermé. Les variantes
  avec payload se construisent comme `Option.Some(42)` et se matchent avec `Option.Some{value} => { ... }` ; les
  nullaires s'utilisent comme des variantes d'enum (`Option.None`). Égalité structurelle, variantes hashables (nullaires
  et payload), toutes truthy. Paramètres génériques et annotations de type parsés (vérification statique à venir). Câblé
  sur les deux exécuteurs (VM par défaut + AST) via le nouvel opcode `MakeUnion`. Voir [UNIONS](lang/UNIONS.md).
- **Structs hashables** : les instances de struct sont utilisables comme clés `dict` / membres de `set`. Hash structural
  par défaut, override via `op_hash(self)`. Définir `op ==` sans `op_hash` rend l'instance unhashable (`TypeError`) pour
  préserver le contrat `a == b ⇒ hash(a) == hash(b)`. Une instance est figée dès qu'elle est hashée : toute mutation
  ultérieure d'un champ lève `TypeError`, garantissant la stabilité du hash pendant la vie de la clé. Voir
  [STRUCTURES § Hashabilité](lang/STRUCTURES.md#hashabilit%C3%A9).

### Tooling

- **Linter `I103` exhaustivité** : généralisation aux `union` taggées. Le linter émet une diagnostic si un `match` sur
  une union ne couvre pas toutes ses variantes (et pas de wildcard `_`), avec la liste des variantes manquantes.
  Mutualise l'infrastructure existante pour `enum` et booleans.
- **Linter `W312` dead store** (`--deep`) : détecte une variable assignée puis écrasée avant lecture, via liveness
  backward sur le CFG. L'analyse construit un CFG par scope (top-level + chaque corps de lambda, paramètres seedés comme
  defs implicites du block entry), donc fonctionne aussi dans les fonctions. Les bindings implicites (for-var, pattern
  de `match`, `except`) sont exclus. Les variables jamais lues nulle part restent du ressort de W200.
- **Linter `W313` guard clause hint** (`--deep`) : signale `elif`/`else` rendus redondants par une branche précédente
  qui termine toujours (`return`/`raise`/`break`/`continue`). Suggestion : aplatir en guard clause. Severity `hint`. Les
  `if`/`else` symétriquement terminants (`if X { return } else { return }`) ne sont pas signalés.

### Module `http`

- **Client HTTP** : `http.get(url)`, `http.post(url, body)`, `http.put(url, body)`, `http.delete(url)` retournent une
  `Response` avec `.status`, `.headers`, `.body`. Backend ureq (rustls + gzip). Les 4xx/5xx remontent comme `Response`,
  les erreurs réseau lèvent une exception. Body lu jusqu'à 32 MB par défaut.
- **`http.request(method, url, opts)`** : dict d'options `headers`, `body`, `timeout` (secondes), `max_body` (bytes).
  `Response.json()` parse le body en dict/list/str/int/float/bool/nil. Grands entiers (> SMALLINT_MAX, u64 > i64::MAX)
  promus en BigInt sans perte.
- **Helpers auth** : `http.basic_auth(user, pass)` → `"Basic <base64>"`. `http.bearer(token)` → `"Bearer <token>"`. À
  utiliser dans `opts.headers.Authorization`.
- **Serveur mode async** : `Server.start()` lance un thread accept qui pousse les requêtes dans un channel.
  `Server.recv_async()` pop sans bloquer (retourne `Request` ou `nil`). `Server.close()` joint le thread proprement, et
  le drop du Server le fait automatiquement aussi.
- **Streaming chunked + SSE** : `Request.start_chunked(status, content_type)` et `Request.start_sse()` consomment la
  requête et retournent un `Chunked` writer (`send_chunk`, `send_event(data, event_type?)`, `end()`). `Drop` envoie le
  terminator si oublié. Refuse HEAD, HTTP/1.0 et statuses no-body (1xx/204/304) — utiliser `respond()` pour ces cas.
- **Multipart côté serveur** : `Request.multipart()` retourne une liste de
  `{ name, filename?, content_type?, data: bytes }`. Boundary ancré sur les delimiter lines (pas de truncation sur bytes
  intérieurs), headers et paramètres case-insensitive (RFC 7578).
- **Cookies côté serveur** : `Request.cookies` parse le header `Cookie:` en `dict[str, str]`. Plusieurs headers
  `Cookie:` sont fusionnés.

## 0.0.8 (2026-04-11)

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
