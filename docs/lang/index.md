# Spécification du langage

Documentation de référence du langage Catnip, du plus accessible au plus avancé.

## Commencer ici

- [SYNTAX](SYNTAX.md) - Syntaxe de base, séparateurs, conventions et annexes
- [TYPES](TYPES.md) - Types de données : nombres, décimales exactes, complexes, chaînes, f-strings, booléens,
  collections
- [EXPRESSIONS](EXPRESSIONS.md) - Expressions multilignes, opérateurs, attributs, indexation et slicing
- [CONTROL_FLOW](CONTROL_FLOW.md) - Structures de contrôle (`if`, `while`, `for`, `break`, `continue`)
- [FUNCTIONS](FUNCTIONS.md) - Fonctions, lambdas, décorateurs, appels terminaux et fonctions intégrées
- [STRUCTURES](STRUCTURES.md) - Structures, méthodes, traits, héritage et abstractions
- [PATTERN_MATCHING](PATTERN_MATCHING.md) - Référence complète du filtrage par motifs (pattern matching)

## Aller plus loin

- [BROADCAST](BROADCAST.md) - Notation vectorielle sur collections : opérations avec `A.[op M]`, transformations sur
  listes et dictionnaires, motifs de broadcasting

- [SCOPES_AND_VARIABLES](SCOPES_AND_VARIABLES.md) - Affectation, portée des variables et closures : résolution de
  portée, masquage (shadowing), captures et durée de vie

- [PRAGMAS](PRAGMAS.md) - Directives de compilation (TCO, JIT avec `@jit` et `pragma("jit", "all")`), récursion ND (mode
  parallèle, mémoïsation) et import de modules

## Théorie et preuves (optionnel)

- [TURING_COMPLETENESS](TURING_COMPLETENESS.md) - Le socle formel du langage : il peut exprimer tout type d'algorithme.
  Les preuves formelles servent à vérifier ces garanties (audit, recherche, sécurité), sans être nécessaires pour coder
  au quotidien
- [COQ_PROOFS](../dev/COQ_PROOFS.md) - Référentiel des preuves Coq : ce qui est prouvé, comment vérifier (`make proof`)
  et où trouver les modules de preuve

## Glossaire

- [GLOSSARY](GLOSSARY.md) - Définitions des concepts clés et de la terminologie du langage
