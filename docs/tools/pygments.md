# Lexer Pygments

Lexer Pygments pour la coloration syntaxique du code Catnip.

## Vue d'ensemble

Le module `catnip.tools.pygments` fournit un lexer Pygments pour colorer du code Catnip dans tous les outils compatibles
: Sphinx, MkDocs, éditeurs, outils de diff, sites web, etc.

**Fichier** : `catnip/tools/pygments.py` **Classe** : `CatnipLexer` **Génération** : Auto-généré depuis la grammaire
Tree-sitter - **NE PAS ÉDITER MANUELLEMENT**

Le lexer est synchronisé automatiquement avec la grammaire pour garantir que les mots-clés, opérateurs et syntaxe
restent cohérents avec le parser Tree-sitter.

## Installation

### Package Pygments

Le lexer est inclus dans le package Catnip et enregistré automatiquement via `entry_points` dans `setup.py` :

```python
entry_points={
    'pygments.lexers': [
        'catnip = catnip.tools.pygments:CatnipLexer',
    ],
}
```

Après installation de Catnip, le lexer est disponible globalement :

```bash
pip install catnip
pygmentize -l catnip script.cat
```

### Vérification

```bash
# Lister les lexers installés
pygmentize -L lexers | grep -i catnip

# Tester la coloration
echo 'x = (n) => { n * 2 }' | pygmentize -l catnip -f terminal
```

## Utilisation

### CLI avec pygmentize

```bash
# Coloration terminal
pygmentize -l catnip script.cat

# Export HTML
pygmentize -l catnip -f html -o script.html script.cat

# Export HTML avec CSS inline
pygmentize -l catnip -f html -O full,style=monokai script.cat > script.html

# Export LaTeX
pygmentize -l catnip -f latex script.cat > script.tex
```

### Sphinx

Configuration `conf.py` :

```python
# Pygments est utilisé automatiquement pour la coloration
pygments_style = 'monokai'

# Dans les fichiers .rst
.. code-block:: catnip

   factorial = (n) => {
       if n <= 1 { return 1 }
       else { return n * factorial(n - 1) }
   }
```

### MkDocs

Configuration `mkdocs.yml` :

```yaml
markdown_extensions:
  - pymdownx.highlight:
      use_pygments: true
  - pymdownx.superfences
```

Fichiers Markdown :

````markdown
```catnip
numbers = list(1, 2, 3, 4, 5)
doubled = numbers.[* 2]
print(doubled)
```
````

### GitLab/GitHub

GitLab et GitHub n'utilisent pas Pygments nativement, mais on peut rendre le code lisible :

````markdown
```python
# GitLab/GitHub n'ont pas de support Catnip natif
# On utilise 'python' comme fallback pour une coloration partielle
factorial = (n) => {
    if n <= 1 { return 1 }
    else { return n * factorial(n - 1) }
}
```
````

Pour une vraie coloration sur ces plateformes, il faut contribuer au projet
[github/linguist](https://github.com/github/linguist).

### Programmatique

```python
from pygments import highlight
from pygments.formatters import TerminalFormatter, HtmlFormatter
from catnip.tools.pygments import CatnipLexer

code = """
factorial = (n) => {
    if n <= 1 { return 1 }
    else { return n * factorial(n - 1) }
}

print(factorial(5))
"""

# Terminal colorisé
lexer = CatnipLexer()
formatter = TerminalFormatter()
result = highlight(code, lexer, formatter)
print(result)

# HTML
formatter = HtmlFormatter(style='monokai', full=True)
html = highlight(code, lexer, formatter)
with open('output.html', 'w') as f:
    f.write(html)
```

## Tokens supportés

Le lexer reconnaît les catégories de tokens suivantes :

### Commentaires

```python
# Ceci est un commentaire
x = 42  # commentaire inline
```

- Type Pygments : `Comment.Single`
- Pattern : `#.*?$`

### Mots-clés

**Contrôle de flux** : `if`, `elif`, `else`, `for`, `while`, `match`, `return`, `break`, `continue`

```python
if x { print("yes") }
for i in range(10) { print(i) }
match x { 1 => { "one" } }
```

- Type : `Keyword`

**Opérateurs logiques** : `and`, `or`, `not`, `in`

```python
if x and y { print("both") }
if not z { print("nope") }
```

- Type : `Keyword`

**Pragmas** : `pragma`

```python
pragma("tco", True)
```

- Type : `Keyword`

### Constantes

`True`, `False`, `None`

```python
x = True
y = None
```

- Type : `Keyword.Constant`

### Types builtin

`list`, `dict`, `set`, `tuple`

```python
numbers = list(1, 2, 3)
mapping = dict()
```

- Type : `Keyword.Type`

### Chaînes de caractères

**Strings simples et triples**

```python
text = "hello"
long_text = """
multi
line
"""
```

- Type : `String`

**F-strings**

```python
name = "Alice"
greeting = f"Hello, {name}!"
```

- Type : `String`
- Pattern : `(?:f|F)("""…""" | '''…''' | "…" | '…')`

### Nombres

**Entiers décimaux** : `42`, `1000`

```python
x = 42
```

- Type : `Number.Integer`

**Binaires** : `0b1010`

```python
flags = 0b11010110
```

- Type : `Number.Bin`

**Octaux** : `0o755`

```python
permissions = 0o755
```

- Type : `Number.Oct`

**Hexadécimaux** : `0xFF`, `0xDEADBEEF`

```python
color = 0xFF5733
```

- Type : `Number.Hex`

**Flottants** : `3.14`, `1e-5`, `2.5e10`

```python
pi = 3.14159
epsilon = 1e-10
```

- Type : `Number.Float`

### Opérateurs

**Arithmétiques** : `+`, `-`, `*`, `/`, `//`, `**`, `%`

```python
x = 2 ** 10
y = 17 % 5
```

- Type : `Operator`

**Comparaison** : `==`, `!=`, `<`, `>`, `<=`, `>=`

```python
if x >= 10 { print("big") }
```

- Type : `Operator`

**Bitwise** : `&`, `|`, `^`, `~`, `<<`, `>>`

```python
flags = 0b1010 | 0b0101
```

- Type : `Operator`

**Spéciaux** : `=`, `=>`

```python
x = 42
f = (n) => { n * 2 }
```

- Type : `Operator`

### Ponctuations

`{`, `}`, `(`, `)`, `[`, `]`, `,`, `:`, `;`, `.`

```python
func(a, b, c)
list[0]
{ x = 1; y = 2 }
```

- Type : `Punctuation`

### Identifiants

Noms de variables, fonctions, paramètres

```python
factorial = (n) => { n * factorial(n - 1) }
```

- Type : `Name`

## États du lexer

Le lexer utilise deux états pour gérer correctement le broadcasting :

### État `root`

État par défaut pour le code normal.

```python
x = 42
y = list(1, 2, 3)
```

Tous les tokens standards sont reconnus.

### État `broadcast`

État spécial activé après `.[` pour gérer les expressions de broadcasting.

```python
numbers.[* 2]
data.[if > 10]
```

Le lexer entre dans cet état quand il rencontre `.[` et en sort sur `]`. Cela permet de parser correctement les
expressions complexes dans le broadcast, y compris :

- Nested brackets : `data.[[x, y]]`
- Nested braces : `data.[{ x = compute(); x * 2 }]`
- Keywords comme `if` : `data.[if condition]`

> Le lexer entre en mode broadcast quand il voit `.[`, ce qui déclenche un changement d'état qui affecte… le lexer
> lui-même. C'est un lexer qui se reconfigure en temps réel en fonction de ce qu'il lit. Une forme d'auto-modification
> contrôlée, parfaitement prévisible et totalement délibérée.

## Régénération du lexer

Le lexer est **auto-généré** depuis la grammaire Tree-sitter. Il ne faut **jamais l'éditer manuellement**.

### Quand régénérer ?

Régénérer après toute modification de `catnip_grammar/grammar.js` qui affecte :

- Les mots-clés (`if`, `while`, `match`, etc.)
- Les opérateurs (`+`, `=>`, `.[`, etc.)
- Les terminaux de base (nombres, strings, identifiants, etc.)

### Commande de régénération

```bash
python -m catnip.tools.extract_grammar --update-lexer
```

Cela :

1. Lit `catnip_grammar/src/grammar.json` (généré par Tree-sitter)
1. Extrait keywords, operators, terminals
1. Génère `catnip/tools/pygments.py` avec les règles de tokenisation
1. Écrase le fichier existant

### Workflow complet

```bash
# 1. Modifier la grammaire Tree-sitter
vim catnip_grammar/grammar.js

# 2. Regénérer le parser Tree-sitter
cd catnip_grammar && npx tree-sitter generate && cd ../..

# 3. Régénérer le lexer Pygments
python -m catnip.tools.extract_grammar --update-lexer

# 4. Tester
echo 'x = (n) => { n * 2 }' | pygmentize -l catnip -f terminal

# 5. Commit (si pertinent)
git add catnip/tools/pygments.py catnip_grammar/
git commit -m "update grammar and regenerate lexer"
```

> Le lexer se génère depuis la grammaire, qui elle-même décrit comment parser le code qui génère… le lexer. C'est une
> boucle d'auto-génération dont la sortie valide l'entrée qui a permis de la produire. Le serpent qui se mord la queue,
> version outillage compilateur.

## Architecture interne

Le lexer utilise `RegexLexer` de Pygments avec des règles de correspondance par expressions régulières.

### Structure de base

```python
class CatnipLexer(RegexLexer):
    name = 'Catnip'
    aliases = ['catnip', 'cat']
    filenames = ['*.cat', '*.catnip']
    mimetypes = ['text/x-catnip']

    tokens = {
        'root': [
            # Liste de patterns (regex, token_type)
            (r'#.*?$', Comment.Single),
            (r'\s+', Whitespace),
            # …
        ],
        'broadcast': [
            # Patterns spécifiques au broadcast
            (r'\]', Punctuation, '#pop'),  # Sort de l'état
            # …
        ],
    }
```

### Ordre des patterns

**Critique** : Les patterns sont évalués dans l'ordre de définition. Les plus spécifiques doivent venir **avant** les
plus génériques.

```python
# BON : f-strings avant strings normales
(r'[fF]"(?:[^"\\]|\\.)*"', String),
(r'"(?:[^"\\]|\\.)*"', String),

# BON : opérateurs multi-char avant single-char
(r'(>>|>=|==|<=|<<|//|\*\*|!=|…)', Operator),
(r'[+\-*/%]', Operator),  # Après !

# MAUVAIS : identifiants avant keywords
(r'[a-zA-Z_]\w*', Name),  # Matche "if", "while", etc.
(words(('if', 'while', …), suffix=r'\b'), Keyword)  # Jamais atteint !
```

Le générateur gère cet ordre automatiquement.

### Exemples de patterns

**Keywords avec word boundary**

```python
(
    words(
        ('if', 'while', 'for', …),
        suffix=r'\b',  # Boundary pour éviter "iffy" ou "whilex"
    ),
    Keyword,
)
```

**F-strings multi-formats**

```python
(
    r'(?:f|F)("""(?:[^"\\]|\\.)*?"""'  # Triple quotes
    r"|'''(?:[^'\\]|\\.)*?'''"      # Triple single quotes
    r'|"(?:[^"\\]|\\.)*"'           # Double quotes
    r"|'(?:[^'\\]|\\.)*')",         # Single quotes
    String,
)
```

**Nombres avec bases multiples**

```python
(r'0[bB][01]+', Number.Bin),           # Binaire : 0b1010
(r'0[oO][0-7]+', Number.Oct),          # Octal : 0o755
(r'0[xX][0-9a-fA-F]+', Number.Hex),    # Hexa : 0xFF
(r'\d+\.\d+([eE][+-]?\d+)?', Number.Float),  # Float : 3.14, 1e-5
(r'\d+[eE][+-]?\d+', Number.Float),    # Scientific : 2e10
(r'\d+', Number.Integer),              # Décimal : 42
```

### Gestion du broadcasting

Le broadcast `.[expression]` nécessite un état séparé pour gérer correctement les nested structures.

**Transition `root` → `broadcast`**

```python
# Dans 'root'
(r'\.\[', Punctuation, 'broadcast'),  # Push état 'broadcast'
```

**Sortie `broadcast` → `root`**

```python
# Dans 'broadcast'
(r'\]', Punctuation, '#pop'),  # Pop état, retour à 'root'
```

Cela permet de parser correctement :

```python
data.[{ if x > 10 { [x, y] } else { [0, 0] } }]
#      ^                                       ^ état 'broadcast'
#    push                                    pop
```

> Le lexer bascule d'état quand il rencontre un délimiteur qui change les règles de parsing pour la suite du code. C'est
> comme si le lexer disait : « Bon, maintenant je lis du broadcast, donc je change mes lunettes ». Sauf que les lunettes
> sont une table de regex et le changement est un push/pop de stack. Métaphore parfaitement technique.

## Limitations connues

1. **Pas de support nativement dans GitHub/GitLab** : Ces plateformes utilisent leurs propres systèmes de coloration
   (Linguist, Rouge). Il faut contribuer à ces projets séparément.

1. **Highlighting partiel dans certains éditeurs** : Certains éditeurs (Sublime Text, Atom) nécessitent des plugins
   spécifiques qui ne se basent pas uniquement sur Pygments.

1. **Pas de semantic highlighting** : Le lexer fait du tokenisation purement syntaxique. Il ne distingue pas :

   - Variables vs fonctions (tous → `Name`)
   - Fonctions builtin vs user-defined
   - Scope ou résolution de noms

1. **Edge cases de broadcasting** : Les nested broadcasts avec quotes complexes peuvent parfois matcher incorrectement :

   ```python
   # OK
   data.[if > 10]

   # Edge case : nested f-string dans broadcast
   data.[f"{x if x > 10 else 0}"]  # Peut confondre le ]
   ```

   Ces cas sont rares en pratique et n'affectent pas la majorité du code.

> Un lexer qui reconnaît tous les tokens sauf quelques edge cases rarissimes, c'est comme un système de tri postal qui
> fonctionne parfaitement sauf pour les colis qui contiennent d'autres colis qui contiennent des adresses. Techniquement
> imparfait, pratiquement suffisant, philosophiquement acceptable.

## Intégration dans les outils

### VSCode / TextMate

VSCode n'utilise pas Pygments mais TextMate grammars. Pour VSCode, il faut créer une extension séparée avec un fichier
`.tmLanguage.json`.

Voir : [VSCode Language Extensions](https://code.visualstudio.com/api/language-extensions/syntax-highlight-guide)

### Vim / Neovim

Vim utilise ses propres syntaxes (`.vim` files). Le lexer Pygments ne s'applique pas directement, mais peut servir de
référence pour créer un `syntax/catnip.vim`.

Exemple de conversion manuelle :

```vim
" syntax/catnip.vim
syn keyword catnipKeyword if elif else while for match return break continue
syn keyword catnipConstant True False None
syn keyword catnipOperator and or not in
syn match catnipComment "#.*$"
syn region catnipString start='"' end='"' skip='\\"'
```

### Emacs

Emacs utilise des modes majeurs avec des règles de fontification. Le lexer Pygments peut inspirer un `catnip-mode.el`.

### JupyterLab / Notebook

JupyterLab utilise CodeMirror pour la coloration, pas Pygments. Il faut créer un mode CodeMirror séparé.

Voir : [CodeMirror Language Modes](https://codemirror.net/docs/ref/#language)

## Exemples complets

### Script complet avec coloration

```python
# Fibonacci avec memoization
fib_cache = dict()

fib = (n) => {
    if n in fib_cache {
        return fib_cache[n]
    }

    if n <= 1 {
        result = n
    } else {
        result = fib(n - 1) + fib(n - 2)
    }

    fib_cache[n] = result
    return result
}

# Calcul
numbers = list(0, 1, 2, 3, 4, 5, 10, 15, 20)
results = numbers.[fib]

# Affichage
for i in range(len(numbers)) {
    n = numbers[i]
    f = results[i]
    print(f"fib({n}) = {f}")
}
```

Tokens reconnus :

- **Comments** : `# Fibonacci avec memoization`, `# Calcul`, `# Affichage`
- **Identifiers** : `fib_cache`, `fib`, `n`, `result`, `numbers`, `results`, `i`, `f`
- **Keywords** : `if`, `else`, `for`, `in`, `return`
- **Types** : `dict`, `list`
- **Operators** : `=`, `=>`, `+`, `-`
- **Punctuations** : `{`, `}`, `(`, `)`, `[`, `]`, `,`
- **Strings** : `f"fib({n}) = {f}"`
- **Numbers** : `0`, `1`, `2`, `3`, `4`, `5`, `10`, `15`, `20`

### Export HTML styled

```python
from pygments import highlight
from pygments.formatters import HtmlFormatter
from catnip.tools.pygments import CatnipLexer

code = open('script.cat').read()
lexer = CatnipLexer()
formatter = HtmlFormatter(
    style='monokai',
    full=True,
    linenos='table',
    title='Catnip Script'
)

html = highlight(code, lexer, formatter)
with open('output.html', 'w') as f:
    f.write(html)
```

Résultat : Un fichier HTML autonome avec CSS inline, numéros de ligne en table, thème Monokai.

### Documentation Sphinx

`docs/conf.py` :

```python
extensions = ['sphinx.ext.autodoc']
pygments_style = 'monokai'
```

`docs/examples.rst` :

```rst
Exemples Catnip
===============

Factorial
---------

.. code-block:: catnip

   factorial = (n) => {
       if n <= 1 { return 1 }
       else { return n * factorial(n - 1) }
   }

   print(factorial(5))  # 120
```

Compilation :

```bash
sphinx-build -b html docs/ docs/_build/
```

Le code Catnip est automatiquement colorisé via le lexer Pygments.

> Documentation qui génère du HTML à partir de RST qui embed du code Catnip qui est parsé par un lexer généré depuis une
> grammaire qui parse le code Catnip. Le cycle complet de la documentation autoréférentielle : le code s'explique
> lui-même via des outils qu'il génère pour se documenter. Meta-circulaire.
