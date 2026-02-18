# Catnip <img alt="Catnip" class="inline size-12" src="assets/catnip-logo.svg"/>

version <!-- catnip:version -->0.0.5<!-- /catnip:version -->

Langage qui fait un 360 no-scope pendant que les chatons compilent les killcams.

> Officieusement : un rite désespéré pour calmer les entités multidimensionnelles qui vivent dans les coins du code.
>
> Si tu sens ta conscience boucler sans condition d'arrêt, c'est une feature.
>
> Respire. Reprends ton café froid. Ce n'est pas encore classé "incident".

### Turfu

Ce projet est en chantier : docs et specs peuvent évoluer.

Ce qui va arriver dans le langage :

- modules (en cours de validation)
- struct (en cours d'implé, manque new/init et MRO)
- traits

______________________________________________________________________

## Origine

- [INTRODUCTION](INTRODUCTION.md) - Ambition, sources d'inspiration et philosophie de Catnip

## Primitives

Démarrage express. **[tuto/](tuto/)**

- [QUICKSTART_0MIN](tuto/QUICKSTART_0MIN.md) - Aperçu très (très) court
- [QUICKSTART_2MIN](tuto/QUICKSTART_2MIN.md) - Bases essentielles en 2 minutes
- [QUICKSTART_5MIN](tuto/QUICKSTART_5MIN.md) - Fonctionnalités complètes en 5 minutes

## Interface

Guide Utilisateur **[user/](user/)**

**Embedding (Catnip comme DSL)** :

- [EMBEDDING_GUIDE](user/EMBEDDING_GUIDE.md) - Guide complet d'embedding Catnip
- [HOST_INTEGRATION](user/HOST_INTEGRATION.md) - Intégrer Catnip dans une app Python
- [EXTENDING_CONTEXT](user/EXTENDING_CONTEXT.md) - API pour étendre le contexte

**Standalone et REPL** :

Apprendre et utiliser Catnip.

- [CLI](user/CLI.md) - Options ligne de commande (scripts et REPL)
- [REPL](user/REPL.md) - Mode interactif pour exploration
- [MODULE_LOADING](user/MODULE_LOADING.md) - Charger des modules Python

## Structure

Spécification **[lang/](lang/)**

Référence complète de la syntaxe et des propriétés du langage.

- [SYNTAX](SYNTAX.md) - Syntaxe de base, types, assignation, structures, conventions et annexes
- [EXPRESSIONS](EXPRESSIONS.md) - Expressions multilignes, opérateurs, attributs, indexation et slicing
- [CONTROL_FLOW](CONTROL_FLOW.md) - Structures de contrôle (`if`, `while`, `for`, `break`, `continue`)
- [FUNCTIONS](FUNCTIONS.md) - Fonctions, lambdas, décorateurs, tail calls et fonctions intégrées
- [PATTERN_MATCHING](PATTERN_MATCHING.md) - Référence complète du pattern matching
- [BROADCAST](lang/BROADCAST.md) - Notation vectorielle sur collections
- [SCOPES_AND_VARIABLES](lang/SCOPES_AND_VARIABLES.md) - Portée des variables et closures
- [PRAGMAS](lang/PRAGMAS.md) - Pragmas (TCO, JIT, ND-recursion, modules)
- [TURING_COMPLETENESS](lang/TURING_COMPLETENESS.md) - Preuve de Turing-complétude
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
- [codex/](codex/) - Exemples thématiques (écosystème Python)
- [advanced/](examples/advanced/) - Techniques avancées
- [performance/](examples/performance/) - Performance
- [embedding/](examples/embedding/) - Embedding Catnip dans Python
- [tools/](examples/tools/) - Utilisation des outils

## Écosystème

Exemples thématiques utilisant les libs Python. **[index](codex/)**

- [files-formats/](codex/files-formats/) - Fichiers, formats et parsing
- [data-analytics/](codex/data-analytics/) - Data science et analytics
- [web/](codex/web/) - HTTP et APIs
- [images-media/](codex/images-media/) - Images et multimédia
- [geometry/](codex/geometry/) - Géométrie algorithmique 2D

## Cœur

Doc Développeur **[dev/](dev/)**

Architecture interne et contribution au projet.

- [ARCHITECTURE](dev/ARCHITECTURE.md) - Pipeline, parsing, analyse sémantique
- [VM](dev/VM.md) - Machine virtuelle Rust et NaN-boxing
- [OPTIMIZATIONS](dev/OPTIMIZATIONS.md) - Passes d'optimisation, TCO, JIT
- [EXTENDING](dev/EXTENDING.md) - Ajouter opcodes et opérations

## Outillage

Outils **[tools/](tools/)**

Outils de développement et utilitaires pour Catnip.

- [lint](tools/lint.md) - Vérificateur de syntaxe
- [format](tools/format.md) - Formatteur de code
- [pygments](tools/pygments.md) - Syntax highlighter
- [extract_grammar](tools/extract_grammar.md) - Extraction de la grammaire (export JSON, lexer Pygments)
