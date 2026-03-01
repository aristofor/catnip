# Architecture

Vue d'ensemble de l'architecture Catnip pour contributeurs.

## Stratégie : Rust + Python

Catnip utilise une architecture hybride, pour garder l'ergonomie côté Python et la performance côté Rust :

**Python** : API de haut niveau, orchestration, intégration

- Classe principale `Catnip`, context, REPL et CLI

**Rust** : Composants bas niveau (via PyO3)

- Parser et transformations (tree-sitter)
- Semantic analyzer et optimisations
- Scope management (O(1) lookup)
- VM bytecode et JIT (Cranelift)
- Registry et dispatch d'opérations

**Principe** : Rust fait le travail lourd, Python garde l'interface simple.

### Pourquoi PyO3

[PyO3](https://pyo3.rs/) sert de pont propre entre Python et Rust :

- Interopérabilité zero-cost (pas de sérialisation)
- Memory safety garantie par Rust
- Intégration directe avec l'API Python
- Utilisé par projet production (Ruff, Polars, tiktoken)

**Références** :

- [PyO3 User Guide](https://pyo3.rs/)
- [Extending Python with Rust](https://www.youtube.com/watch?v=jmP_i3C_O4Y) (PyCon 2023)

## Pipeline de Compilation

Catnip transforme le code source en résultat exécutable via un pipeline en 4 étapes :

```mermaid
flowchart LR
    SRC["Source"] --> P["Parsing"]
    P -- "Parse tree (CST)" --> T["Transformation"]
    T -- "IR" --> S["Semantic"]
    S -- "Op" --> E["Execution"]
    E --> R["Result"]
```

### 1. Parsing : Tree-sitter

Le parser utilise [Tree-sitter](https://tree-sitter.github.io/tree-sitter/), un générateur de parseur incrémental :

> Transparence : Tree-sitter n'est pas formellement prouvé dans ce repo.

**Pourquoi Tree-sitter** :

- Parser généré en C (performance native)
- Parsing incrémental (réévalue seulement les modifications)
- Error recovery (robuste face aux erreurs de syntaxe)
- Écosystème riche (syntax highlighting, code folding)

**Avantage vs parser manuel** : la précédence des opérateurs est codée dans la grammaire (`prec.left()`,
`prec.right()`), pas besoin de la recoder ailleurs.

**Références** :

- [Tree-sitter documentation](https://tree-sitter.github.io/tree-sitter/)
- [Tree-sitter in Practice](https://siraben.dev/2022/03/01/tree-sitter.html)

### 2. Transformation : CST → IR

Le transformer convertit l'arbre de syntaxe en IR (Intermediate Representation) :

**IR** : structure basée sur des OpCode (entiers) pour identifier les opérations

- Sortie brute du parser, pas encore optimisée
- Utilise l'enum `IROpCode`
- Type `IRPure` (Rust) sans dépendance PyO3 pour pipeline standalone

72+ transformateurs en Rust pur, wrapper PyO3 pour le bridge Python. Couvrent tout le langage :

- Literals (int, float, string, list, dict)
- Operators (binary, unary, comparison, bitwise)
- Control flow (if, while, for, match, block)
- Functions (lambda, fn_def, call)
- Pattern matching (literal, var, wildcard, or, tuple)
- Broadcasting et accès (chained, getattr, index, slice)

### 3. Semantic Analysis : IR → Op

L'analyse sémantique transforme l'IR en Op exécutable :

**Responsabilités** :

- Résolution des identifiants
- Détection des tail calls (TCO)
- Application des pragmas
- Optimisations (6 passes)

**Optimisations** (optionnel, contrôlé par niveau 0-3) :

- **Passes IR** (niveau expression) : simplifications locales (constant folding, CSE, dead code, etc.)
- **Passes CFG/SSA** (niveau contrôle de flux, level >= 3) : optimisations globales (voir ci-dessous)

Voir [OPTIMIZATIONS](OPTIMIZATIONS.md) pour détails sur les niveaux.

**Op** : structure exécutable finale avec OpCode optimisé

#### CFG/SSA : Optimisations Inter-blocs

À partir du niveau 3, le semantic analyzer construit un **Control Flow Graph** (CFG) puis passe en **SSA** pour pouvoir
optimiser à l'échelle de plusieurs blocs.

> Warning: ce passage augmente la résistance mentale de +5.

**Pipeline CFG/SSA** :

```mermaid
flowchart TD
    IR["IR optimisé<br/>(passes locales)"]
    IR --> CFG["1. Construction CFG<br/>BasicBlocks + edges"]
    CFG --> DOM["2. Analyse dominance<br/>Dominator tree, boucles"]
    DOM --> SSA["3. Construction SSA<br/>SSA values, phi-nodes"]
    SSA --> PASSES["4. Passes SSA<br/>CSE, LICM, GVN, DSE"]
    PASSES --> DESTR["5. Destruction SSA<br/>SetLocals explicites"]
    DESTR --> OPT["6. Optimisations CFG<br/>Dead blocks, merging"]
    OPT --> RECON["7. Reconstruction IR<br/>Op nodes optimisés"]
```

**Passes SSA** (4 passes inter-blocs) :

1. **CSE inter-blocs** - Élimine expressions redondantes entre blocs dominants
1. **LICM** - Hoist les calculs invariants hors des boucles
1. **GVN** - Global Value Numbering, détecte équivalences entre expressions
1. **DSE globale** - Élimine les SetLocals dont le résultat n'est jamais lu

**Construction SSA** : utilise l'algorithme de
[Braun et al. (2013)](https://pp.ipd.kit.edu/uploads/publikationen/braun13cc.pdf), en un seul passage RPO (reverse
postorder), sans calcul explicite des dominance frontiers.

**SetLocals** est le nœud IR central pour l'SSA : chaque affectation crée une nouvelle version de variable. Les
phi-nodes aux jonctions sont convertis en SetLocals explicites lors de la destruction SSA.

> L'SSA garantit que chaque variable n'est assignée qu'une seule fois. Ce qui est pratique pour l'optimiseur, mais
> existentiellement perturbant pour les variables qui se pensaient réassignables.

### 4. Execution : Deux modes

Catnip supporte deux modes d'exécution :

**VM Bytecode** (défaut) :

- Compile Op → bytecode → VM stack-based
- NaN-boxing pour représentation compacte
- JIT Cranelift pour hot loops/functions
- Voir [VM](VM.md) pour détails

**AST Interpretation** (fallback) :

- Interprète Op directement via Registry
- Dispatch O(1) en Rust
- Utilisé pour debug et tests

## Concepts Clés

### OpCode : Identifiants d'Opérations

Les opérations sont identifiées par l'enum `OpCode` (Rust), utilisée pour le dispatch rapide et la cohérence entre
parsing, semantic et exécution.

**Avantages vs strings** :

- Comparaisons O(1) (entiers vs strings)
- Lookups rapides dans dictionnaires
- Consommation mémoire réduite

**Convention** : Opcodes correspondant à mots-clés Python préfixés `OP_` (`OP_IF`, `OP_WHILE`)

### Scope : Variables O(1)

La gestion des scopes utilise un **HashMap plat** en Rust, plutôt qu'une chaîne de scopes parents :

**Approche classique** (O(n) lookup) :

```
Scope 3 → Scope 2 → Scope 1 → Global
```

Recherche d'une variable = remonter la chaîne jusqu'à trouver

**Approche Catnip** (O(1) lookup) :

- Un seul HashMap contenant toutes les variables
- Tracking par frame pour savoir quoi nettoyer au pop
- Shadow stack pour gérer le masquage de variables

**Trade-off** : lookup O(1), cleanup O(n) où n = variables dans le frame

**Références** :

- [Hash table](https://en.wikipedia.org/wiki/Hash_table) (Wikipedia)
- Concept inspiré de V8's hidden classes

> Les scopes classiques sont une tour d'annuaires empilés. Pour trouver un numéro, on monte étage par étage. Catnip
> utilise un annuaire unique avec des post-its de couleur pour savoir quel numéro appartient à quel étage. Chercher est
> instantané, ranger nécessite de lire les post-its.

### Registry : Table des Opérations

Le Registry dispatche les opcodes vers leurs implémentations Rust via pattern matching direct (O(1), branch prediction).
12 modules spécialisés (arithmetic, logical, control_flow, functions, patterns, etc.).

### Tail Call Optimization (TCO)

Catnip utilise un **trampoline pattern** pour éviter que la pile d'appels grossisse :

**Principe** :

1. La fonction tail-recursive retourne `TailCall(func, args)` au lieu d'appeler
1. La boucle trampoline détecte `TailCall`, rebind les paramètres, continue
1. Un seul frame Python pour toute la récursion

**Avantage** : récursion possible sans gonfler la stack (O(1) stack space)

**Détection** : automatique par l'analyseur sémantique (appels en dernière position)

**Références** :

- [Tail call](https://en.wikipedia.org/wiki/Tail_call) (Wikipedia)
- [Proper Tail Calls in Scheme](https://www.scheme.com/tspl4/further.html#./further:h3)

### Lazy Evaluation

Les opérations de contrôle de flux (`if`, `while`, `for`, `match`, etc.) reçoivent leurs arguments **non évalués** :

**Raison** : les blocs doivent être évalués conditionnellement ou plusieurs fois

```python
# if (condition) { then_block } else { else_block }
# → then_block et else_block ne sont PAS évalués immédiatement
# → Seul le bloc choisi sera exécuté
```

**Implémentation** : HashSet `CONTROL_FLOW_OPS` marque les opcodes lazy

### Error Handling : Source Locations

Les erreurs runtime capturent la position source complète (fichier, ligne, colonne) avec une pile d'appels claire.

**Pipeline de propagation** :

```mermaid
flowchart TD
    TS["Tree-sitter<br/>start_byte, end_byte"]
    TS --> IRN["IR nodes<br/>positions natives"]
    IRN --> SA["Semantic Analyzer<br/>propagate_position"]
    SA --> OP["Op + Ref nodes<br/>start_byte, end_byte préservés"]
    OP --> VMC["VM Compiler<br/>line_table par instruction"]
    VMC --> ERR["VM Error → ErrorContext → CatnipError"]
```

**Line table** : le `CodeObject` contient un vecteur parallèle aux instructions qui mappe chaque instruction vers son
`start_byte`. La VM maintient `last_src_byte` (mis a jour a chaque instruction) et une pile d'appels avec nom de
fonction et position source.

**Capture lazy** : quand une erreur se produit, la VM utilise `last_src_byte` (toujours a jour, meme si le frame est
depile pendant la propagation), snapshote le call stack, puis le bridge Python convertit `start_byte` en ligne/colonne
et enrichit l'exception avec un extrait.

**Suggestions "Did you mean?"** : trois niveaux de suggestions automatiques basees sur la distance de
Damerau-Levenshtein (`catnip_tools/src/suggest.rs`) :

- **Variables** : `NameError` collecte locals + globals et suggere les noms proches
- **Attributs struct** : `AttributeError` sur fields/methods suggere l'attribut le plus similaire
- **Attributs Python** : `AttributeError` sur objets Python utilise `dir()` + Damerau-Levenshtein pour suggerer
  (`"hello".uper()` -> `upper`)
- **Keywords syntaxe** : tokens inconnus sont compares aux keywords Catnip + aliases cross-langage (`class` -> `struct`,
  `switch` -> `match`)

Les erreurs semantiques (unknown opcode, unknown pragma) incluent la position source via `start_byte` enrichi par
SourceMap dans le pipeline.

**Resultat** : messages d'erreur avec traceback complet et suggestions :

```
File '<input>', line 1, column 1: Name 'factoral' is not defined
  Did you mean 'factorial'?
    1 | factorial = 1; factoral
    | ^
```

**Details** : voir [VM](VM.md) pour l'architecture.

## Debugger

Le debugger connecte la VM Rust à un frontend (console Rust ou MCP Python) via des channels `std::sync::mpsc`.

### Architecture

```mermaid
sequenceDiagram
    participant VM as VM (thread background)
    participant CB as DebugCallback (Rust)
    participant FE as Frontend

    VM->>CB: hit breakpoint<br/>invoke_debug_callback
    CB->>FE: PauseEvent via event_tx<br/>(line, col, locals, snippet, call_stack)
    Note over CB: py.detach() - libère le GIL
    Note over FE: affichage
    FE->>CB: DebugAction via command_tx<br/>(step/continue)
    Note over CB: recv → GIL réacquis
    CB->>VM: DebugAction<br/>(CONTINUE / STEP_* / ...)
```

### Points d'entrée dans la VM

Le breakpoint opcode est injecté par l'analyseur sémantique quand il rencontre un appel `breakpoint()`. La VM intercepte
aussi les instructions dont le `start_byte` correspond à un breakpoint utilisateur (ajouté via
`add_debug_breakpoint(offset)`).

Au point de pause, la VM snapshote l'état : variables locales (slotmap complet, y compris nil), call stack, et position
source. Le `DebugCallback` Rust construit un `PauseEvent`, l'envoie via `event_tx`, puis libère le GIL pendant
`command_rx.recv_timeout(60s)` (auto-continue après 5 min).

**Composants** : logique pure dans `catnip_tools`, channels et GIL dans `catnip_rs/debug`, wrapper Python dans
`catnip/debug`, 6 tools MCP.

### Step modes

| Action      | Comportement                                        |
| ----------- | --------------------------------------------------- |
| `CONTINUE`  | Reprend jusqu'au prochain breakpoint                |
| `STEP_INTO` | Pause à la prochaine instruction                    |
| `STEP_OVER` | Pause à la prochaine instruction de même profondeur |
| `STEP_OUT`  | Pause au retour du frame courant                    |

> Le debugger observe la VM sans la modifier. Ce qui est pratique, parce qu'un debugger qui modifie l'exécution du
> programme qu'il débogue serait un programme qui s'observe en train de ne pas être lui-même.

## Vérification Formelle

Les propriétés structurelles du langage sont prouvées en [Coq](https://coq.inria.fr/). Chaque fichier modélise un
composant Rust et prouve ses invariants.

| Axe           | Couverture                                                                                                                               |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| Syntaxe       | Grammaire CF, précédence (13 niveaux), monotonie fuel                                                                                    |
| Sémantique    | Broadcasting (foncteur, confluence), ND-récursion                                                                                        |
| Runtime       | IR opcodes, scopes, patterns, fonctions/TCO, NaN-boxing, VM stack safety, frames/IP/jumps, C3 MRO, structs/traits, desugaring opérateurs |
| Optimisations | 10/10 passes IR (constant folding, CSE, DCE, propagation, etc.)                                                                          |
| Analyses      | Liveness/DSE, dominance CFG, SSA complet (49 lemmes), cache                                                                              |

Preuves paramétriques, compilent avec `make proof`. Détails : [COQ_PROOFS](COQ_PROOFS.md).

> Un programme prouvé correct n'a pas de bugs. Il a des hypothèses.

## Où Trouver le Code

| Dossier           | Contenu                                   |
| ----------------- | ----------------------------------------- |
| `catnip/`         | API Python, intégration                   |
| `catnip_rs/`      | Runtime Rust (parser, semantic, VM, JIT)  |
| `catnip_grammar/` | Grammaire Tree-sitter                     |
| `catnip_tools/`   | Outils Rust (formatter, linter, debugger) |
| `proof/`          | Preuves Coq                               |

## Workflow de Développement

```bash
# Après modification Rust
uv pip install -e .

# Tests rapides Rust (~5s)
make rust-test-fast

# Tests complets Python (~25s)
make test

# Après modification grammar.js
make grammar-deps
```
