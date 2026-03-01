# Formatteur de code

Outil de formatage automatique du code Catnip avec un style opinionated.

## Vue d'ensemble

Le formatteur `catnip format` applique un style cohérent sur tout le code Catnip. Contrairement à un pretty-printer qui
reconstruit le code depuis l'AST, il utilise une approche **token-based** qui préserve :

- **Les commentaires** (inline et standalone)
- **Les newlines intentionnelles** (pas de reformatage destructif)
- **La structure du code** (seuls les espaces et l'indentation sont ajustés)

Le style appliqué s'inspire de Black (Python) : un seul style, pas de configuration, zéro débat.

## Utilisation CLI

### Formater un fichier

```bash
# Formater et afficher sur stdout
catnip format script.cat

# Formater en place
catnip format -i script.cat

# Formater un dossier (récursif, tous les .cat)
catnip format -i src/

# Vérifier le formatage (CI)
catnip format --check src/

# Afficher le diff
catnip format --diff script.cat
```

### Options

```bash
catnip format -l 80 script.cat        # Line length (défaut: 120)
catnip format --indent-size 2 src/     # Taille d'indentation (défaut: 4)
```

### Formater depuis stdin

```bash
echo 'x=1+2*3' | catnip format --stdin
cat script.cat | catnip format --stdin
```

## Règles de formatage

### Espaces

**Opérateurs binaires** : espace avant et après

```python
# Avant
x=1+2*3
y==42

# Après
x = 1 + 2 * 3
y == 42
```

**Opérateurs unaires** : pas d'espace entre l'opérateur et l'opérande

```python
# Avant
x = - 1
y = a + - b
z = + x
w = ~ a

# Après
x = -1
y = a + -b
z = +x
w = ~a
```

**`not` keyword** : espace avant (sauf debut de ligne) et espace apres

```python
# Avant
a and notb
notx

# Après
a and not b
not x
```

**Assignation et lambda**

```python
# Avant
x=42
f=(n)=>{n*2}

# Après
x = 42
f = (n) => { n * 2 }
```

**Keyword arguments** : pas d'espace autour de `=` dans les appels

```python
# Avant
dict(name = "catnip", version = "0.1")
f(timeout = 5.0)

# Après
dict(name="catnip", version="0.1")
f(timeout=5.0)
```

L'assignation normale (`x = 1`) conserve ses espaces. La distinction est automatique : `=` entre parenthèses = keyword
argument.

**Méthodes et attributs après DOT** : pas d'espace

```python
# Les keywords utilisés comme noms de méthode ne sont pas espacés
re.match(pattern, text)    # pas: re. match (pattern, text)
obj.return_value           # pas: obj. return_value
```

**Virgules** : espace après, pas avant

```python
# Avant
list(1,2,3)
func(a,b,c)

# Après
list(1, 2, 3)
func(a, b, c)
```

**Point-virgules** : espace apres, pas avant

```python
# Avant
x = 1;y = 2;z = 3

# Après
x = 1; y = 2; z = 3
```

**Parenthèses** : pas d'espace intérieur

```python
# Avant
func( x , y )
( a + b )

# Après
func(x, y)
(a + b)
```

**Broadcasting** : pas d'espace après `.[`

```python
# Avant
numbers.[ * 2 ]
data.[ if > 10 ]

# Après
numbers.[* 2]
data.[if > 10]
```

### Indentation

**Blocs** : 4 espaces par niveau

```python
# Avant
if x{
y=1
}

# Après
if x {
    y = 1
}
```

**Structures imbriquées**

```python
# Avant
match x{
1=>{print("one")}
_=>{
if y{print("other")}
}
}

# Après
match x {
    1 => {print("one")}
    _ => {
        if y {print("other")}
    }
}
```

### Commentaires

**Commentaires inline** : minimum 2 espaces avant

```python
# Avant
x = 42# important

# Après
x = 42  # important
```

**Commentaires standalone** : indentation normale

```python
# Avant
if x {
# case one
y = 1
}

# Après
if x {
    # case one
    y = 1
}
```

### F-strings et b-strings

Les f-strings et b-strings sont preservees intactes (pas de reformatage du contenu) :

```python
x = f"hello {name}"       # préservé
y = b'raw bytes'           # préservé
```

### Newlines

**Lignes vides preservees** : les sauts de ligne intentionnels du codeur sont respectes.

**Maximum 2 newlines consecutives** : les sequences de 3+ newlines sont reduites a 2.

**Pas de newlines en debut de fichier** : les lignes vides initiales sont supprimees.

**Une seule newline en fin de fichier.**

### Jointure de lignes

Les lignes de continuation qui tiennent dans `line_length` sont jointes automatiquement :

```python
# Avant (coupure manuelle inutile si line_length=120)
f(aaaa,
    bbbb,
    cccc)

# Après
f(aaaa, bbbb, cccc)
```

La jointure détecte les continuations par :

- Fin de ligne : `,`, `(`, `[`, `+`, `-`, `*`, `/`, `and`, `or`, `=>`
- Début de ligne suivante : `+`, `-`, `)`, `]`, `and`, `or`

Les lignes vides (séparateurs de sections) ne sont jamais jointes.

#### Magic trailing comma

Une virgule terminale avant `)` ou `]` force le maintien de la disposition multilignes, même si tout tiendrait sur une
seule ligne. Sans virgule terminale, les lignes sont jointes normalement.

```python
# Virgule terminale -> multilignes préservé
data = dict(
    a=1,
    b=2,
)

# Pas de virgule terminale -> jointure en inline
data = dict(a=1, b=2)
```

Ce comportement s'inspire de Black (Python) et Prettier (JS) : la virgule terminale est un signal explicite du
développeur pour conserver la disposition verticale.

#### Fermetures empilées

Les délimiteurs fermants empilés à des niveaux d'indentation différents sont préservés sur des lignes séparées :

```python
# Préservé (pas de fusion en "))") :
users = list(
    dict(
        name="Alice",
    ),
)
```

### Coupure de lignes longues

Les lignes qui dépassent `line_length` sont coupées automatiquement aux meilleurs points :

```python
# Avant (line_length=40)
f(aaaa, bbbb, cccc, dddd, eeee)

# Après
f(aaaa, bbbb, cccc, dddd,
    eeee)
```

Points de coupure par ordre de priorité :

1. Après `,` dans un contexte parenthèse/crochet
1. Avant opérateur binaire (`+`, `-`, `and`, `or`...)
1. Avant `=>`
1. Après `(` / `[` d'ouverture

La ligne de continuation est indentée d'un niveau supplémentaire.

## Exemples

### Code mal formaté

```python
# Factorial
factorial   =(   n   )   =>   {
if    n<=1{return 1}
else{return n*factorial(n-1)}  # recursive
}

x=factorial(   5   )
print(x)
```

### Code formaté

```python
# Factorial
factorial = (n) => {
    if n <= 1 { return 1}
    else { return n * factorial(n - 1)}  # recursive
}

x = factorial(5)
print(x)
```

### Pattern matching

```python
# Avant
check=(x)=>{
match x{
1|2|3=>{print("small")}
n if n>10=>{print("big")}  # guard
_=>{print("other")}
}
}

# Après
check = (x) => {
    match x {
        1 | 2 | 3 => {print("small")}
        n if n > 10 => {print("big")}  # guard
        _ => {print("other")}
    }
}
```

### Broadcasting et listes

```python
# Avant
numbers=list(1,2,3,4,5)
doubled=numbers.[*2]
filtered=numbers.[if>3]

# Après
numbers = list(1, 2, 3, 4, 5)
doubled = numbers.[* 2]
filtered = numbers.[if > 3]
```

## Architecture interne

Le formatteur utilise une approche **token-based** en 5 phases, toutes en Rust :

### 1. Tokenisation

Le tokenizer Tree-sitter extrait les tokens depuis l'arbre syntaxique. Chaque token contient : type, value, line,
column, end_line, end_column. Les commentaires sont préservés comme nœuds de l'arbre. Les f-strings et b-strings sont
traités comme tokens opaques (pas de décomposition interne).

### 2. Injection de newlines

Les tokens sont sur la même ligne dans l'arbre. La phase 2 injecte des tokens `_NEWLINE` entre les tokens situés sur des
lignes différentes, en respectant le nombre de lignes vides de la source (un gap de N lignes → N tokens `_NEWLINE`).

### 3. Application des règles de formatage

Le formatteur parcourt les tokens et applique les règles localement :

- `needs_space_before()` : opérateurs binaires, keywords, détection du contexte unaire
- `needs_space_after()` : virgules, point-virgules, `=>`, keywords
- Gestion de l'indentation via `LBRACE`/`RBRACE`
- Préservation des commentaires avec espacement minimal

La détection unaire utilise le dernier token significatif (non-newline) pour distinguer `-x` (unaire) de `a - x`
(binaire), même à travers des sauts de ligne.

### 4. Jointure et coupure de lignes

Deux passes sur le texte formaté :

- **Jointure** (`join_short_lines`) : fusionne les lignes de continuation qui tiennent dans `line_length`
- **Coupure** (`wrap_long_lines`) : découpe les lignes qui dépassent `line_length` aux meilleurs points de coupure

### 5. Normalisation finale

- Suppression des espaces en fin de ligne (trailing whitespace)
- Suppression des newlines en début de fichier
- Maximum 2 newlines consécutives
- Exactement une newline en fin de fichier

## Différences avec un pretty-printer AST

| Critère          | Pretty-printer AST             | Formatteur token-based   |
| ---------------- | ------------------------------ | ------------------------ |
| **Commentaires** | ✗ Perdus (ignorés par parser)  | ✓ Préservés              |
| **Newlines**     | ✗ Reconstruites arbitrairement | ✓ Respectées             |
| **Structure**    | ✗ Reformatée complètement      | ✓ Ajustée localement     |
| **Fidélité**     | ✗ Code reconstruit             | ✓ Code original préservé |
| **Performance**  | Plus lent (parse + transform)  | Plus rapide (lex only)   |

Le formatteur token-based est un **formatter respectueux** comme Black, pas un **uglifier** qui détruit le code.

## Utilisation programmatique

```python
from catnip.tools import format_code
from catnip._rs import FormatConfig

source = """
x=1+2*3  # no spaces
y  =   42   # too many
"""

formatted = format_code(source)
print(formatted)
# Output:
# x = 1 + 2 * 3  # no spaces
# y = 42  # too many
```

### Formatteur avec options

```python
config = FormatConfig(indent_size=2, line_length=80)
formatted = format_code(source, config)
```

## Intégration éditeurs

### VSCode

Créer `.vscode/settings.json` :

```json
{
  "[catnip]": {
    "editor.defaultFormatter": "catnip-formatter",
    "editor.formatOnSave": true
  }
}
```

### Vim/Neovim

```vim
" Format on save
autocmd BufWritePre *.cat !catnip format % > %.tmp && mv %.tmp %

" Format selection
vnoremap <leader>f :!catnip format --<CR>
```

### Pre-commit hook

`.git/hooks/pre-commit` :

```bash
#!/bin/bash
for file in $(git diff --cached --name-only --diff-filter=ACM | grep '\.cat$'); do
    catnip format "$file" > "$file.tmp" && mv "$file.tmp" "$file"
    git add "$file"
done
```

## Limitations connues

1. **Broadcasting edge cases** : `.[` peut avoir un espace superflu dans certains cas complexes
1. **Blocs single-line** : pas d'optimisation pour `{ return 1 }` vs `{\n    return 1\n}`
1. **Jointure non-idempotente** : formatter deux fois un code avec des lignes jointes puis recoupees peut donner un
   resultat different (les coupures de ligne ne reproduisent pas forcement l'indentation originale)

> Le formatteur ajuste les espaces et l'indentation, pas la structure logique du code.
