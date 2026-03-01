# Spécification du Langage

Documentation complète de la syntaxe et des propriétés du langage Catnip.

## Référence Principale

- [SYNTAX](SYNTAX.md) - Syntaxe de base, séparateurs, conventions et annexes
- [TYPES](TYPES.md) - Types de données : nombres, décimales exactes, complexes, chaînes, f-strings, booléens,
  collections
- [EXPRESSIONS](EXPRESSIONS.md) - Expressions multilignes, opérateurs, attributs, indexation et slicing
- [CONTROL_FLOW](CONTROL_FLOW.md) - Structures de contrôle (`if`, `while`, `for`, `break`, `continue`)
- [FUNCTIONS](FUNCTIONS.md) - Fonctions, lambdas, décorateurs, tail calls et fonctions intégrées
- [STRUCTURES](STRUCTURES.md) - Structures, méthodes, traits, héritage, abstract
- [PATTERN_MATCHING](PATTERN_MATCHING.md) - Référence complète du pattern matching

## Concepts du Langage

- [BROADCAST](BROADCAST.md) - Notation vectorielle sur collections Opérations vectorielles avec `A.[op M]`,
  transformations sur listes/dicts, patterns de broadcasting

- [SCOPES_AND_VARIABLES](SCOPES_AND_VARIABLES.md) - Assignation, portée des variables et closures Résolution de scope,
  shadowing, captures de variables, closures, durée de vie

## Directives et Pragmas

- [PRAGMAS](PRAGMAS.md) - Directives de compilation TCO, JIT (`@jit`, `pragma("jit", "all")`), ND-recursion (mode
  parallèle, memoization), import de modules

## Propriétés Formelles

- [TURING_COMPLETENESS](TURING_COMPLETENESS.md) - Preuve de Turing-complétude. Démonstration formelle, critères
  théoriques, exemples d'algorithmes

## Glossaire

- [GLOSSARY](GLOSSARY.md) - Termes techniques Définitions des concepts clés, terminologie du langage
