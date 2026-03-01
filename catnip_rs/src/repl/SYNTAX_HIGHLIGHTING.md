# Syntax Highlighting in Catnip REPL

Le highlighter utilise tree-sitter pour analyser le code en temps réel et applique les couleurs suivantes :

## Color Scheme

| Element            | Color     | Style  | Example                                 |
| ------------------ | --------- | ------ | --------------------------------------- |
| **Keywords**       | Cyan      | Bold   | `if`, `while`, `for`, `match`, `return` |
| **Constants**      | Purple    | Bold   | `True`, `False`, `None`                 |
| **Built-in Types** | Blue      | Normal | `dict`, `list`, `set`, `tuple`          |
| **Numbers**        | Yellow    | Normal | `42`, `3.14`, `0xFF`                    |
| **Strings**        | Green     | Normal | `"hello"`, `f"x={x}"`                   |
| **Comments**       | Dark Gray | Normal | `# comment`                             |
| **Operators**      | Red       | Normal | `+`, `-`, `==`, `=>`                    |
| **Builtins**       | Blue      | Normal | `print`, `len`, `type`, `range`         |
| **Punctuation**    | White     | Normal | `(`, `)`, `[`, `]`, `{`, `}`            |
| **Identifiers**    | Default   | Normal | Variables et fonctions                  |

## Examples

```catnip
# Keywords et control flow
if x > 10:
    while True:
        break

# Constants
result = True if x else False

# Numbers
count = 42
pi = 3.14159
mask = 0xFF

# Strings
name = "Catnip"
message = f"Hello, {name}!"

# Operators
result = (a + b) * c
check = x == y and z != w

# Builtins
print(len(items))
numbers = range(10)

# Functions
fib = (n) => {
    if n <= 1:
        n
    else:
        fib(n-1) + fib(n-2)
}
```

## Implementation

Le highlighting fonctionne par :

1. **Parsing** - tree-sitter parse le code en arbre syntaxique
1. **Traversal** - parcours récursif de l'arbre
1. **Styling** - application des styles selon le type de nœud
1. **Assembly** - reconstruction du texte avec styles pour reedline

Les nœuds ERROR sont aussi traversés pour que le code invalide soit quand même highlighté au mieux.
