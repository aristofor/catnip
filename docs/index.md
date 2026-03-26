# Catnip <img alt="Catnip" class="inline size-14" src="assets/catnip-logo.svg"/>

version <!-- catnip:version -->0.0.7<!-- /catnip:version -->

Né comme langage de script embarquable pour Python.

Catnip vise un équilibre entre simplicité d'usage, expressivité et performances. Minimal en surface, précis dans sa
conception.

<!-- doc-snapshot: index/repl-ieee754 -->

```console
Catnip REPL v0.0.7
Type /help for help, /exit to quit
▸ 0.1 + 0.2
0.30000000000000004
# tout est conforme
```

> Officieusement : un rite désespéré pour calmer les entités multidimensionnelles qui vivent dans les coins du code.
>
> Si tu sens ta conscience boucler sans condition d'arrêt, c'est une feature.
>
> Respire. Reprends ton café froid. Ce n'est pas encore classé "incident".

### Turfu

Encore un brouillon, moins sale.

- Type énuméré `enum`
- Support `with` pour les context managers Python
- Réécriture du formatter avec un AST, dans le genre Wadler-Leijen
- Détection de patterns louches par le linter
- Un minimum de packaging

### Repos

- Framagit (principal) : [https://framagit.org/aristofor/catnip](https://framagit.org/aristofor/catnip)
- GitHub (miroir) : [https://github.com/aristofor/catnip](https://github.com/aristofor/catnip)

### Bonus

Le langage est prouvé avec Coq. Voir : [COQ_PROOFS](dev/COQ_PROOFS.md)

> Transparence maximale : Tree-sitter n'est pas formellement prouvé ici, et Cranelift non plus.
>
> On a donc des preuves solides, et un petit pacte avec le dieu des parseurs.
>
> Par conséquent l'usage de Catnip est déconseillé dans les contextes safety-critical, alors si on ne scripte pas un
> Airbus, un missile, ou une centrale nucléaire, c'est OK.

______________________________________________________________________

## Origine

- [INTRODUCTION](INTRODUCTION.md) - Ambition, sources d'inspiration et philosophie de Catnip
- [CHANGELOG](CHANGELOG.md) - Changements par rapport à la prerelease précédente
- [META](META.md) - Conventions transverses de lecture et d'écriture de la doc

## Primitives

Démarrage express. **[tuto/](tuto/)**

- [QUICKSTART_0MIN](tuto/QUICKSTART_0MIN.md) - Aperçu très (très) court
- [QUICKSTART_2MIN](tuto/QUICKSTART_2MIN.md) - Bases essentielles en 2 minutes
- [QUICKSTART_5MIN](tuto/QUICKSTART_5MIN.md) - Fonctionnalités complètes en 5 minutes

## Gestes

Recettes d'utilisation. **[how-to/](how-to/)**

- [PATTERN_MATCHING](how-to/PATTERN_MATCHING.md) - Mettre en place du pattern matching pas à pas
- [ND_CONCURRENCY](how-to/ND_CONCURRENCY.md) - Choisir entre `sequential`, `thread` et `process` pour la ND-recursion
- [CLICK_INTEGRATION](how-to/CLICK_INTEGRATION.md) - Construire une CLI avec Click depuis Catnip
- [INTERACTIVE_MENUS](how-to/INTERACTIVE_MENUS.md) - Menus interactifs avec `prompt_toolkit`

## Interface

Guide utilisateur **[user/](user/)**

**Embedding (Catnip comme DSL)** :

- [EMBEDDING_GUIDE](user/EMBEDDING_GUIDE.md) - Guide complet d'embedding Catnip
- [HOST_INTEGRATION](user/HOST_INTEGRATION.md) - Intégrer Catnip dans une app Python
- [EXTENDING_CONTEXT](user/EXTENDING_CONTEXT.md) - API pour étendre le contexte

**Standalone et REPL** :

Apprendre et utiliser Catnip.

- [CLI](user/CLI.md) - Options ligne de commande (scripts et REPL)
- [REPL](user/REPL.md) - Mode interactif pour exploration
- [RUN](user/RUN.md) - Mode standalone avec binaire `catnip`
- [CONFIG](user/CONFIG.md) - Configuration hiérarchique et overrides par mode
- [SHEBANG_GUIDE](user/SHEBANG_GUIDE.md) - Scripts Catnip exécutables comme commandes système
- [MODULE_LOADING](user/MODULE_LOADING.md) - Charger des modules Python

## Structure

Spécification **[lang/](lang/)**

Référence complète de la syntaxe et des concepts du langage.

- [SYNTAX](lang/SYNTAX.md) - Syntaxe de base, séparateurs, conventions et annexes
- [TYPES](lang/TYPES.md) - Types de données : nombres, chaînes, f-strings, booléens, collections
- [EXPRESSIONS](lang/EXPRESSIONS.md) - Expressions multilignes, opérateurs, attributs, indexation et slicing
- [CONTROL_FLOW](lang/CONTROL_FLOW.md) - Structures de contrôle (`if`, `while`, `for`, `break`, `continue`)
- [FUNCTIONS](lang/FUNCTIONS.md) - Fonctions, lambdas, décorateurs, appels terminaux et fonctions intégrées
- [STRUCTURES](lang/STRUCTURES.md) - Structures, méthodes, traits, héritage et abstractions
- [PATTERN_MATCHING](lang/PATTERN_MATCHING.md) - Référence complète du filtrage par motifs (pattern matching)
- [BROADCAST](lang/BROADCAST.md) - Notation vectorielle sur collections
  - [BROADCAST_SPEC](lang/BROADCAST_SPEC.md) - Spécification normative (syntaxe et sémantique)
  - [BROADCAST_RATIONALE](lang/BROADCAST_RATIONALE.md) - Motivation et comparaisons
  - [BROADCAST_GUIDE](lang/BROADCAST_GUIDE.md) - Cas d'usage pratiques
  - [BROADCAST_RUNTIME](lang/BROADCAST_RUNTIME.md) - Détails d'implémentation runtime
- [COMPREHENSIONS](lang/COMPREHENSIONS.md) - Traduction des comprehensions Python vers Catnip
- [FOLD_GUIDE](lang/FOLD_GUIDE.md) - Fold et reduce : agrégation de structures
- [SCOPES_AND_VARIABLES](lang/SCOPES_AND_VARIABLES.md) - Portée des variables et closures
- [PRAGMAS](lang/PRAGMAS.md) - Pragmas (TCO, JIT, ND-recursion, modules)
- [TURING_COMPLETENESS](lang/TURING_COMPLETENESS.md) - Socle formel et complétude de Turing
- [COQ_PROOFS](dev/COQ_PROOFS.md) - Référentiel des preuves Coq
- [GLOSSARY](lang/GLOSSARY.md) - Termes techniques

## Matière

Exemples **[examples/](examples/)**

- [basics/](examples/basics/) - Bases et syntaxe
- [functions/](examples/functions/) - Fonctions et closures
- [pattern-matching/](examples/pattern-matching/) - Pattern matching
- [control-flow/](examples/control-flow/) - Break et continue
- [broadcast/](examples/broadcast/) - Broadcasting
- [cfg/](examples/cfg/) - Control Flow Graph (analyse et optimisations)
- [module-loading/](examples/module-loading/) - Chargement de modules (bases)
- [run/](examples/run/) - Exemples standalone (CLI)
- [codex/](codex/) - Exemples thématiques (écosystème Python)
- [advanced/](examples/advanced/) - Techniques avancées
- [embedding/](examples/embedding/) - Embedding Catnip dans Python
- [tools/](examples/tools/) - Utilisation des outils
- [performance/](examples/performance/) - Benchmarks et profiling

## Écosystème

Exemples thématiques utilisant les libs Python. **[codex/](codex/)**

- [files-formats/](codex/files-formats/) - Fichiers, formats et parsing
- [data-analytics/](codex/data-analytics/) - Data science et analytics
- [web/](codex/web/) - HTTP et APIs
- [images-media/](codex/images-media/) - Images et multimédia
- [geospatial/](codex/geospatial/) - Données raster, distance et Sentinel-2
- [geometry/](codex/geometry/) - Géométrie algorithmique 2D
- [symbolic-graphs/](codex/symbolic-graphs/) - Calcul symbolique et graphes (SymPy, NetworkX)
- [visualization/](codex/visualization/) - Dashboards et visualisation
- [utils/](codex/utils/) - Utilitaires Python réutilisables

## Cœur

Doc développeur **[dev/](dev/)**

Architecture interne et contribution au projet.

- [CONTRIBUTING](dev/CONTRIBUTING.md) - Prérequis et guide de contribution
- [ARCHITECTURE](dev/ARCHITECTURE.md) - Pipeline, parsing, analyse sémantique
- [VM](dev/VM.md) - Machine virtuelle Rust et NaN-boxing
- [JIT](dev/JIT.md) - Compilation JIT via Cranelift
- [OPTIMIZATIONS](dev/OPTIMIZATIONS.md) - Passes d'optimisation, TCO, JIT
- [ND_VM_ARCHITECTURE](dev/ND_VM_ARCHITECTURE.md) - Opcodes ND dans la VM et le bytecode
- [CACHE](dev/CACHE.md) - Système de cache multi-niveaux
- [CONSTANTS](dev/CONSTANTS.md) - Constantes centralisées du runtime Rust
- [BENCHMARKING](dev/BENCHMARKING.md) - Méthodologie de benchmarking
- [TEST_STRATEGY](dev/TEST_STRATEGY.md) - Stratégie de test (VM et AST)
- [EXTENDING](dev/EXTENDING.md) - Ajouter opcodes et opérations

## Outillage

Outils **[tools/](tools/)**

Outils de développement et utilitaires pour Catnip.

- [mcp](tools/mcp.md) - Serveur MCP (parsing, inspection, debug, évaluation pour agents)
- [lint](tools/lint.md) - Vérificateur de syntaxe
- [debug](tools/debug.md) - Debugger
- [format](tools/format.md) - Formatteur de code
- [pygments](tools/pygments.md) - Syntax highlighter
- [extract_grammar](tools/extract_grammar.md) - Extraction de la grammaire (export JSON, lexer Pygments)
