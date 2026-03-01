# Notation `target.[op operand]`

## Vue d'ensemble

Le contenu broadcast est maintenant séparé pour clarifier les usages :

- [BROADCAST_SPEC.md](./BROADCAST_SPEC.md) : syntaxe normative, sémantique, règles ND implicites.
- [BROADCAST_GUIDE.md](./BROADCAST_GUIDE.md) : exemples progressifs, patterns d'usage, cas concrets.
- [BROADCAST_RUNTIME.md](./BROADCAST_RUNTIME.md) : détails d'implémentation, fast paths, contraintes runtime.
- [BROADCAST_RATIONALE.md](./BROADCAST_RATIONALE.md) : motivation, comparaisons, base théorique.

## Broadcast Deep

Le broadcast descend automatiquement dans les structures imbriquées (listes, tuples) jusqu'aux feuilles scalaires. La
forme `.[.[...]]` reste valide mais n'est plus nécessaire.
