# Meta

Conventions transverses de documentation Catnip, destinées aux lecteurs humains comme aux agents IA.

______________________________________________________________________

## Conventions dans les snippets

- `# → ...` : assertion attendue sur le résultat d'une expression dans un exemple.
- `# ← ...` : entrée utilisateur (input) dans un simulateur de terminal.

Exemples :

```catnip
2 + 2
# → 4
```

```catnip
# ← 1 + 1
2
# → 2
```

Notes :

- Ce ne sont pas des instructions exécutées, mais des marqueurs de validation visuelle.
- Garder la forme courte et factuelle.
- En cas de résultat long, préférer un extrait représentatif plutôt qu'un dump complet.
- `# RUN: ...` est un tag documentaire et de validation, pas un shebang ; il peut être exploité hors shell (ex:
  validateur/doc runner).
- `# FILE: ...`, `/* FILE: ...`, `# TAGS: ...` métadonnées de catalogage internes, sans sémantique langage.

______________________________________________________________________

## Intention

- Réduire les ambiguïtés de lecture.
- Expliciter les choix documentaires volontaires.
- Donner un référentiel unique pour toute la doc.

______________________________________________________________________

## Portée

- S'applique à l'ensemble de `docs/`, sauf mention contraire dans une page locale.
