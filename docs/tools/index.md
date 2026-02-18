# Outils

Outils de développement et utilitaires pour Catnip.

## Liste des outils

### Intégration MCP

Catnip fournit un serveur [MCP](https://modelcontextprotocol.io/) (Model Context Protocol) pour l'intégration avec Claude et autres agents.

- **10 tools** : parsing, évaluation, validation, formatage, debugging interactif
- **Ressources** : exemples par thème, codex d'intégration Python

### [Linter](lint.md)

Analyse statique du code Catnip : syntaxe, style et sémantique.

**Commandes principales** :

- `catnip lint script.cat` - Analyse complète
- `catnip lint -l syntax script.cat` - Syntaxe seulement
- `catnip lint -l style script.cat` - Style seulement
- `catnip lint -l semantic script.cat` - Sémantique seulement

**Diagnostics détectés** :

- Erreurs de syntaxe (E1xx)
- Problèmes de style/formatage (W2xx)
- Variables non définies (E3xx)
- Variables non utilisées (W3xx)

### [Formatteur de code](format.md)

Formatage automatique du code Catnip avec style opinionated (inspiré de Black).

**Commandes principales** :

- `catnip format script.cat` - Formater un fichier
- `catnip format --` - Formater depuis stdin

**Caractéristiques** :

- Préserve les commentaires et la structure
- Indentation 4 espaces
- Espaces autour des opérateurs binaires
- Approche token-based (pas de reconstruction AST)

### [Debugger](debug.md)

Debugger interactif avec breakpoints, stepping et inspection de variables.

**Commandes principales** :

- `catnip debug -b 5 script.cat` - Debugger avec breakpoint
- `catnip debug -c "code" -b 1` - Debugger du code inline

**Caracteristiques** :

- Breakpoints par ligne (`-b`) ou dans le code (`breakpoint()`)
- Stepping : step into, step over, step out
- Inspection : variables locales, pile d'appels, evaluation d'expressions
- Integration MCP (6 tools pour agents)

### [Extract Grammar](extract_grammar.md)

Extraction programmatique de la grammaire Catnip pour génération d'outils tiers.

**Commandes principales** :

- `python -m catnip.tools.extract_grammar` - Affichage résumé
- `python -m catnip.tools.extract_grammar --json grammar.json` - Export JSON
- `python -m catnip.tools.extract_grammar --update-lexer` - Mise à jour lexer Pygments

**Caractéristiques** :

- Extraction keywords, operators, terminals, rules
- Export JSON structuré
- Génération automatique de lexer Pygments
- API programmatique

### [Lexer Pygments](pygments.md)

Coloration syntaxique du code Catnip pour Sphinx, MkDocs, pygmentize et tous les outils compatibles Pygments.

**Utilisation** :

- `pygmentize -l catnip script.cat` - Coloration terminal
- Code blocks Sphinx/MkDocs avec `.. code-block:: catnip`
- API programmatique avec `CatnipLexer`

**Caractéristiques** :

- Auto-généré depuis la grammaire Tree-sitter (ne pas éditer manuellement)
- Support complet des tokens Catnip (keywords, operators, strings, numbers, broadcasting)
- États lexer pour gérer broadcasting et nested structures
- Enregistré automatiquement via entry_points

## Workflows suggérés

### Linter le projet avant commit

```bash
# Vérification rapide (syntaxe)
catnip lint -l syntax **/*.cat

# Analyse complète
catnip lint **/*.cat

# Ignorer les warnings, échouer seulement sur erreurs
catnip lint **/*.cat 2>/dev/null || exit 1
```

### Formater tout le code du projet

```bash
find . -name "*.cat" -type f -exec sh -c 'catnip format "$1" > "$1.tmp" && mv "$1.tmp" "$1"' _ {} \;
```

### Générer lexer après modification grammaire

```bash
# 1. Modifier catnip_grammar/grammar.js
# 2. Regénérer le parser Tree-sitter
cd catnip_grammar && npx tree-sitter generate
# 3. Régénérer le lexer Pygments
python -m catnip.tools.extract_grammar --update-lexer
```

### Export grammaire pour éditeur tiers

```bash
# Export JSON
python -m catnip.tools.extract_grammar --json catnip_grammar.json

# Utiliser dans VSCode, Vim, etc.
```

### Coloration syntaxique dans la documentation

```bash
# Sphinx : utiliser code-block catnip
# Dans docs/conf.py
# pygments_style = 'monokai'

# MkDocs : activer pygments dans mkdocs.yml
# markdown_extensions:
#   - pymdownx.highlight:
#       use_pygments: true

# Export HTML standalone
pygmentize -l catnip -f html -O full,style=monokai script.cat > output.html
```
