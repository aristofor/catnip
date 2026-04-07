# Mode standalone

Catnip s'installe via `pip install catnip-lang`. Le binaire `catnip` est disponible immédiatement.

## Usage

```bash
catnip script.cat              # Exécuter un script
catnip -c "x = 10; x * 2"     # Évaluer une expression
echo "2 + 3" | catnip --stdin  # Lire depuis stdin
catnip --version               # Version et build info
catnip -v script.cat           # Mode verbose (timings pipeline)
```

### Options

```bash
catnip -o jit script.cat           # Activer le JIT
catnip -o tco:off script.cat       # Désactiver le TCO
catnip -o level:2 script.cat       # Niveau d'optimisation (0-3)
catnip -m numpy script.cat         # Charger un module Python
catnip -m math:m script.cat        # Module avec alias
catnip -x ast script.cat           # Interpréteur AST (au lieu de VM)
catnip -p 1 script.cat             # Niveau de parsing (0-3, voir ci-dessous)
```

### Niveaux de parsing

`-p` / `--parsing` permet d'inspecter les étapes du pipeline sans exécuter :

| Niveau | Sortie                                              |
| ------ | --------------------------------------------------- |
| 0      | Parse tree (AST Tree-sitter brut)                   |
| 1      | IR (après transformation, avant analyse sémantique) |
| 2      | IR exécutable (après analyse sémantique)            |
| 3      | Exécution et résultat (défaut)                      |

### Mode benchmark

```bash
catnip bench script.cat        # 10 itérations par défaut
catnip bench 50 script.cat     # 50 itérations
```

### Info runtime

```bash
catnip info
```

## Scripts exécutables (shebang)

Ajouter `#!/usr/bin/env catnip` en première ligne et `chmod +x` :

```bash
#!/usr/bin/env catnip
# process.cat

import('sys')
input_file = sys.argv[1]
print('Processing ' + input_file)
```

```bash
chmod +x process.cat
./process.cat data.json
# => Processing data.json
```

`sys.argv[0]` contient le chemin du script, les arguments suivent à partir de `sys.argv[1]`.

Si le nom du script entre en collision avec une sous-commande (`format`, `lint`, etc.), utiliser `--` :

```bash
catnip -- format arg1 arg2
```

## REPL

La REPL se lance avec `catnip` sans arguments (ou `catnip repl`).

### Commandes

Toutes les commandes commencent par `/` :

| Commande         | Description                               |
| ---------------- | ----------------------------------------- |
| `/help`          | Aide complète                             |
| `/exit`, `/quit` | Quitter                                   |
| `/clear`         | Effacer l'écran                           |
| `/version`       | Version et build                          |
| `/history`       | Historique (20 dernières entrées)         |
| `/load <file>`   | Charger et exécuter un fichier .cat       |
| `/stats`         | Statistiques d'exécution (variables, JIT) |
| `/jit`           | Toggle JIT                                |
| `/verbose`       | Toggle mode verbose (timings)             |
| `/debug`         | Toggle mode debug (IR + bytecode)         |
| `/time <expr>`   | Benchmarker une expression                |

### Mode debug

Affiche l'IR optimisé et le bytecode sans exécuter :

<!-- check: no-check -->

```catnip
▸ /debug
Debug mode: enabled (shows IR and bytecode)

▸ x = 10 + 20
Program(
    [
        Op {
            opcode: SetLocals,
            args: [
                Tuple(
                    [
                        Ref(
                            "x",
                            0,
                            1,
                        ),
                    ],
                ),
                Op {
                    opcode: Add,
                    args: [
                        Int(
                            10,
                        ),
                        Int(
                            20,
                        ),
                    ],
                    kwargs: {},
                    tail: false,
                    start_byte: 4,
                    end_byte: 11,
                },
                Bool(
                    false,
                ),
            ],
            kwargs: {},
            tail: false,
            start_byte: 0,
            end_byte: 11,
        },
    ],
)

=== Bytecode ===
Instructions:
    0: Instruction { op: LoadConst, arg: 0 }
    1: Instruction { op: LoadConst, arg: 1 }
    2: Instruction { op: Add, arg: 0 }
    3: Instruction { op: DupTop, arg: 0 }
    4: Instruction { op: DupTop, arg: 0 }
    5: Instruction { op: StoreLocal, arg: 0 }
    6: Instruction { op: StoreScope, arg: 0 }
    7: Instruction { op: Halt, arg: 0 }

Constants: 2 values
Names: ["x"]
```

### Auto-complétion

Tab complète :

- Commandes REPL (`/he` -> `/help`)
- Keywords (`wh` -> `while`)
- Builtins (`pri` -> `print`)
- Variables définies
- Méthodes après `.` (string/list/dict)

<!-- check: no-check -->

```catnip
▸ x = "hello"
▸ x.<Tab>
capitalize  casefold  center  count  encode  endswith  find  format
index  isalnum  isalpha  join  lower  replace  split  strip  upper
```

### Raccourcis clavier

| Raccourci | Action              |
| --------- | ------------------- |
| Ctrl+D    | Quitter             |
| Ctrl+C    | Annuler saisie      |
| Up/Down   | Naviguer historique |
| Tab       | Auto-complétion     |

Historique persistant dans `$XDG_STATE_HOME/catnip/repl_history` (1000 entrées max, défaut
`~/.local/state/catnip/repl_history`).

### Syntax highlighting

Coloration live en temps réel :

- Keywords : cyan bold (if, while, for, match)
- Constants : magenta bold (True, False, None)
- Types : teal (dict, list, tuple, set)
- Numbers : vert pâle
- Strings : orange
- Comments : gris
- Operators : gris clair
- Builtins : jaune (print, len, range)

## Couverture du langage

Le mode standalone couvre 100% des features du langage Catnip : variables, fonctions, closures, structs, traits, pattern
matching, broadcasting, pragmas, imports, memoization, runtime introspection, extensions et module policies.
