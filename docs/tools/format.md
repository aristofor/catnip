# Formatteur de code

Outil de formatage automatique du code Catnip avec un style opinionated.

## Vue d'ensemble

Le formatteur `catnip format` applique un style cohérent sur tout le code Catnip. Il utilise un pretty-printer
**Wadler-Leijen** qui reconstruit le code depuis l'arbre syntaxique (tree-sitter CST) en préservant :

- **Les commentaires** (inline et standalone)
- **Les newlines intentionnelles** (pas de reformatage destructif)
- **La structure du code** (seuls les espaces et l'indentation sont ajustés)

Le style appliqué s'inspire de Black (Python) : un seul style, pas de configuration, zéro débat.

**Code invalide** : le formatteur opère sur l'arbre syntaxique. Si le code ne parse pas correctement (noeuds ERROR dans
tree-sitter), le texte source est préservé tel quel sans tentative de normalisation partielle.

## Utilisation CLI

### Formater un fichier

```bash
# Formater et afficher sur stdout
catnip format script.cat

# Formater en place
catnip format -i script.cat

# Formater un dossier (récursif, tous les .cat)
catnip format -i src/

# Si aucun .cat trouvé : affiche "No .cat files found"

# Vérifier le formatage (CI)
catnip format --check src/

# Afficher le diff
catnip format --diff script.cat
```

### Options

```bash
catnip format -l 80 script.cat            # Line length (défaut: 120)
catnip format --indent-size 2 src/        # Taille d'indentation (défaut: 4)
catnip format --align src/               # Forcer --align (activé par défaut)
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

**`not` keyword** : espace avant (sauf début de ligne) et espace après

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

**Keywords en position valeur** : pas d'espace avant `,` ou `;`

```python
# (op, a, b) → préservé tel quel (pas "op , a, b")
# struct S { op; x; } → pas "op ; x ;"
```

**Point-virgules** : espace après, pas avant

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

**Broadcasting** : pas d'espace après `.[`, espace entre opérateur et opérande

```python
# Avant
numbers.[ * 2 ]
data.[ if > 10 ]
data.[~>abs]

# Après
numbers.[* 2]
data.[if > 10]
data.[~> abs]
```

**Fullslice** : pas d'espace autour des `:`

```python
# Avant
data.[ 1 : 3 ]
data.[-2 : ]

# Après
data.[1:3]
data.[-2:]
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

**Instanciation de structs** : pas d'espace avant `{`

```python
# Les blocs de contrôle gardent l'espace
if x { ... }
while y { ... }
struct Point { x; y; }

# Les instanciations de structs n'ont pas d'espace
Point{1, 2}
f(Point{x, y}, other)
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

**Commentaires inline** : exactement 2 espaces avant `#`

```python
# Avant
x = 42# important
y = 1,# trailing

# Après
x = 42  # important
y = 1,  # trailing
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
y = b'raw bytes'          # préservé
```

### Newlines

**Lignes vides preservees** : les sauts de ligne intentionnels du codeur sont respectes.

**Maximum 2 newlines consecutives** : les sequences de 3+ newlines sont reduites a 2.

**Pas de newlines en début de fichier** : les lignes vides initiales sont supprimées.

**Une seule newline en fin de fichier.**

**Décorateurs isolés sur leur propre ligne** : chaque `@decorator` est placé seul sur sa ligne, comme en Python.

```
# Avant
@jit f = (n) => { n * 2 }
@jit @pure g = (x) => { x + 1 }

# Après
@jit
f = (n) => { n * 2 }
@jit
@pure
g = (x) => { x + 1 }
```

Cette règle s'applique aux décorateurs d'assignments (`@jit`, `@pure`, `@cached`) et aux décorateurs de méthodes dans
les structs/traits (`@abstract`, `@static`).

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
- Début de ligne suivante : `.`, `+`, `-`, `)`, `]`, `and`, `or`

Les lignes vides (séparateurs de sections) ne sont jamais jointes.

#### Magic trailing comma

Une virgule terminale avant `)` ou `]` force le maintien de la disposition multilignes, même si tout tiendrait sur une
seule ligne.

```python
# Virgule terminale -> multilignes préservé
data = dict(
    a=1,
    b=2,
)

# Fonctionne aussi avec des commentaires trailing
points = list(
    Point(0, 0),
    Point(1, 1),
    Point(2, 2),  # doublon
)
```

Ce comportement s'inspire de Black (Python) et Prettier (JS) : la virgule terminale est un signal explicite du
développeur pour conserver la disposition verticale.

#### Disposition source-aware (sans trailing comma)

Sans virgule terminale, le formatteur **respecte le choix du développeur** :

- **Premier argument sur la même ligne que `(`** → le formatteur garde le layout inline. Les coupures se font à
  l'intérieur des groupes imbriqués si nécessaire.
- **Premier argument sur la ligne suivante** → le formatteur force le multiline (chaque argument sur sa propre ligne).

```python
# Inline -> reste inline (breaks internes si nécessaire)
report = SalesReport(list(
    Item("a", 10),
    Item("b", 20)
))

# Multiline -> reste multiline
data = dict(
    a=1,
    b=2
)

# Inline court -> reste inline
f(a, b)
```

Ce mécanisme s'applique aux appels de fonction, `list()`, `tuple()`, `set()` et `dict()`.

#### Concaténation de strings multilignes

Les concaténations de strings explicitement réparties sur plusieurs lignes sont préservées. Quand les deux opérandes du
`+` sont des littéraux string, le formatteur considère que la coupure est intentionnelle (lisibilité, SQL, HTML...) :

```python
# Préservé tel quel
query = "SELECT * FROM users " +
    "WHERE active = 1 " +
    "ORDER BY name"

# Les concaténations non-string sont toujours jointes si elles tiennent
x = a +
    b
# → x = a + b
```

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

## Alignement en colonne

Le formatteur aligne les symboles `=>` (match arms) et `#` (commentaires trailing) sur les groupes de lignes
consécutives. L'alignement des assignations `=` est désactivé : il casse dès qu'on renomme une variable et crée du bruit
dans les diffs git.

### Symboles alignés

**Match arms (`=>`)** — alignement des flèches dans les bras de match

```python
match x {
    1     => { "one" }
    22    => { "twenty-two" }
    other => { "other" }
}
```

**Commentaires trailing (`#`)** — alignement des `#` inline (les commentaires standalone sont ignorés)

```python
x = 1       # first
longer = 2  # second
```

### Groupement

Un **groupe** est une suite de lignes consécutives non-vides, au même niveau d'indentation, contenant toutes le symbole
cible. Une ligne vide, un changement d'indentation, ou une ligne à l'intérieur d'un string literal multilignes casse le
groupe. Minimum 2 lignes pour déclencher l'alignement.

### Configuration

Activé par défaut. Désactivable via TOML :

```toml
[format]
align = false
```

## Architecture interne

Le formatteur est un **pretty-printer algébrique** Wadler-Leijen, implémenté en Rust dans `catnip_tools/src/pretty/`. Le
pipeline a 4 étapes :

```
Tree-sitter CST
      │
      ▼
  convert.rs  (dispatch récursif, ~50 types de nœuds)
      │
      ▼
  Doc (arena-allocated document algebra)
      │
      ▼
  layout.rs  (Leijen greedy best-fit, O(n))
      │
      ▼
  (String, Vec<usize>)  ─── texte + line_map
      │
      ▼
  align.rs  (post-processing optionnel)
      │
      ▼
  String finale
```

### 1. Conversion CST → Doc

Le convertisseur parcourt récursivement l'arbre Tree-sitter et produit un document algébrique. Chaque type de nœud a un
traitement dédié :

- **Expressions** (`convert_expr.rs`) : binaires (opérateur en fin de ligne en mode break), unaires, appels, chaînes
  d'accès, collections, kwargs
- **Statements** (`convert_stmt.rs`) : if/elif/else, while, for, match/patterns, pragma
- **Déclarations** (`convert_decl.rs`) : lambda, struct, trait avec champs/méthodes/extends/implements
- **Commentaires** : rattachés au nœud précédent (trailing) ou émis en standalone selon la position source

L'algebra documentaire (`doc.rs`) fournit les primitives : `Text`, `Line`, `SoftLine`, `HardLine`, `Nest`, `Group`,
`Concat`, `Verbatim`, `SourceLine`. Les combinateurs dérivés (`combinators.rs`) : `bracket`, `surround`, `intersperse`,
`comma_line`.

### 2. Layout greedy

L'algorithme de Leijen (stack-based, itératif) décide pour chaque `Group` : flat ou break.

- Mesure la largeur flat via `flat_width()` avec short-circuit
- Si flat tient dans la largeur restante : `Line` → espace, `SoftLine` → rien
- Sinon break : `Line`/`SoftLine` → `\n` + indentation courante
- `HardLine` force toujours un saut de ligne
- `Verbatim` est émis tel quel (strings multilignes)

Le `line_map` est construit naturellement pendant le layout via les annotations `SourceLine`.

### 3. Préservation d'intention source

Le formatteur respecte les choix explicites du programmeur :

- **Trailing comma magic** : une virgule avant `)` force le mode multiline
- **Disposition source-aware** : le premier argument inline → inline préservé ; premier argument à la ligne → multiline
  préservé (appels, collections, dicts)
- **If/elif/else** : `} else {` sur la même ligne ou `}\nelse {` sur des lignes séparées -- le choix source est préservé
- **Method chains** : `obj.method(...)` sur une ligne reste sur une ligne ; les breaks source entre membres sont
  préservés avec indentation
- **Struct body** : les champs/méthodes respectent le layout source (inline `x; y;`, multiline, blank lines entre champs
  et méthodes)
- **Blocs multilignes** : un bloc `{ }` qui occupe plusieurs lignes dans la source reste multiline
- **String concat multiline** : `"a" +\n "b"` où les deux opérandes sont des strings reste sur plusieurs lignes
- **Blank lines** : un gap ≥ 2 lignes entre statements produit une ligne vide
- **Semicolons struct** : les `;` sur les champs sont préservés si présents dans la source, pas ajoutés sinon

### 4. Alignement en colonne (post-processing)

Compare le texte formaté au source original via le `line_map`. Aligne `=>` (match arms) et `#` (commentaires trailing)
sur les groupes de lignes consécutives au même indent. L'alignement des `=` est désactivé.

### 5. Normalisation finale

- Suppression des newlines en début de fichier
- Maximum 2 newlines consécutives
- Suppression du trailing whitespace sur les lignes vides
- Exactement une newline en fin de fichier

### Références

- Wadler, "A prettier printer" (1998)
- Lindig, "Strictly Pretty" (2000)
- Leijen, "PPrint, a prettier printer" (2001)

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

# Avec alignement en colonne
config = FormatConfig(align=True)
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

1. **Broadcasting edge cases** : `.[` peut avoir une espace superflue dans certains cas complexes
1. **Jointure non-idempotente** : formatter deux fois un code avec des lignes jointes puis recoupees peut donner un
   resultat different (les coupures de ligne ne reproduisent pas forcement l'indentation originale)

> Le formatteur ajuste les espaces et l'indentation, pas la structure logique du code.
