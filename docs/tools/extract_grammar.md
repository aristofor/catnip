# Extract Grammar

Extraction programmatique de la grammaire Catnip pour générer des outils tiers.

## Vue d'ensemble

Le module `catnip.tools.extract_grammar` analyse la grammaire Tree-sitter (`grammar.json` et `node-types.json`) et
extrait les éléments structurels :

- **Keywords** : mots-clés de contrôle, constantes, types, pragmas
- **Operators** : arithmétiques, comparaison, bitwise, logiques, spéciaux
- **Terminals** : nœuds terminaux Tree-sitter
- **Rules** : règles de production de la grammaire

## Utilisation CLI

### Affichage console

```bash
# Affichage par défaut (résumé)
python -m catnip.tools.extract_grammar

# Afficher une catégorie spécifique
python -m catnip.tools.extract_grammar --print keywords
python -m catnip.tools.extract_grammar --print operators
python -m catnip.tools.extract_grammar --print terminals
python -m catnip.tools.extract_grammar --print rules
python -m catnip.tools.extract_grammar --print all
```

### Export JSON

```bash
# Export JSON complet
python -m catnip.tools.extract_grammar --json grammar.json
```

Structure JSON :

```json
{
  "keywords": {
    "control_flow": ["if", "while", "for", …],
    "constants": ["True", "False", "None"],
    "types": ["list", "dict", "tuple", "set"],
    "pragmas": ["pragma"],
    "all": […]
  },
  "operators": {
    "arithmetic": ["+", "-", "*", "/", …],
    "comparison": ["==", "!=", "<", ">", …],
    "bitwise": ["&", "|", "^", …],
    "logical": ["and", "or", "not"],
    "special": ["=>", ".[", "="],
    "all": […]
  },
  "terminals": [
    {"name": "integer", "named": true},
    …
  ],
  "rules": [
    {"name": "source_file", "type": "SEQ"},
    …
  ],
  "metadata": {
    "source": "…/catnip_grammar/src/grammar.json",
    "parser": "tree-sitter"
  }
}
```

### Génération lexer Pygments

```bash
# Mettre à jour le lexer Pygments officiel
python -m catnip.tools.extract_grammar --update-lexer

# Générer vers un fichier personnalisé
python -m catnip.tools.extract_grammar --lexer custom_lexer.py
```

Le lexer généré :

- Lit automatiquement keywords et operators depuis la grammaire Tree-sitter
- Gère les f-strings, nombres (bin/oct/hex/float), commentaires
- Support du broadcasting `.[op]`
- Compatible avec Pygments pour syntax highlighting

## Utilisation programmatique

```python
from pathlib import Path
from catnip.tools.extract_grammar import GrammarExtractor

# Initialisation (utilise catnip_grammar/src/grammar.json par défaut)
extractor = GrammarExtractor()

# Extraction par catégorie
keywords = extractor.extract_keywords()
operators = extractor.extract_operators()
terminals = extractor.extract_terminals()
rules = extractor.extract_rules()

# Extraction complète
all_data = extractor.extract_all()

# Export JSON
extractor.to_json(Path("output.json"))

# Génération lexer Pygments
extractor.generate_pygments_lexer(Path("lexer.py"))
```

## Cas d'usage

### 1. Mise à jour du lexer Pygments

Quand la grammaire évolue, regénère automatiquement le lexer :

```bash
python -m catnip.tools.extract_grammar --update-lexer
```

Évite de maintenir manuellement la liste des keywords/operators dans
[catnip/tools/pygments.py](../../catnip/tools/pygments.py).

### 2. Export pour éditeurs tiers

Génère un JSON exploitable par VSCode, Vim, Emacs, etc. :

```bash
python -m catnip.tools.extract_grammar --json catnip_grammar.json
```

Les éditeurs peuvent lire ce JSON pour configurer leur syntax highlighting.

### 3. Documentation automatique

Extrait la liste des keywords et operators pour générer la documentation :

```python
from catnip.tools.extract_grammar import GrammarExtractor

extractor = GrammarExtractor()
keywords = extractor.extract_keywords()

print("## Mots-clés du langage")
print(f"- Control flow: {', '.join(keywords['control_flow'])}")
print(f"- Constants: {', '.join(keywords['constants'])}")
```

### 4. Validation et tests

Compare la grammaire parsée avec une référence pour détecter les changements :

```python
from catnip.tools.extract_grammar import GrammarExtractor

extractor = GrammarExtractor()
all_data = extractor.extract_all()

# Vérifie que les keywords attendus sont présents
expected_keywords = ['if', 'while', 'for', 'match']
actual_keywords = all_data['keywords']['control_flow']
assert all(kw in actual_keywords for kw in expected_keywords)
```

## Principe de fonctionnement

1. **Chargement de la grammaire** : Lit `grammar.json` généré par Tree-sitter
1. **Extraction des nœuds** : Lit `node-types.json` pour les métadonnées des nœuds
1. **Classification** : Map les éléments vers catégories (keywords, operators, etc.)
1. **Extraction patterns** : Parse les règles JSON pour extraire les tokens
1. **Export structuré** : Génère JSON ou code Python

## Fichiers sources Tree-sitter

- `catnip_grammar/grammar.js` : Grammaire source (JavaScript DSL)
- `catnip_grammar/src/grammar.json` : Grammaire compilée (utilisée par l'extracteur)
- `catnip_grammar/src/node-types.json` : Métadonnées des types de nœuds
