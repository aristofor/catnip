# Catnip REPL - Phase 2

REPL pure Rust avec pipeline complet et syntax highlighting temps réel.

## Architecture

```
Input → Tree-sitter → IRPure → Semantic → Bytecode → VM → Result
                                    ↓
                         Syntax Highlighting
```

## Composants

### Executor (`executor.rs`)

Pipeline d'exécution complet :

- Parsing via Tree-sitter
- Transformation vers IRPure
- Analyse sémantique + optimisations
- Compilation bytecode
- Exécution VM
- Contexte Python persistant

### Highlighter (`highlighter.rs`) ✨

Colorisation syntaxique temps réel :

- Parse avec Tree-sitter
- Traverse l'AST pour identifier les tokens
- Applique les styles via nu-ansi-term
- Supporte les nœuds ERROR (code incomplet)

Voir [SYNTAX_HIGHLIGHTING.md](./SYNTAX_HIGHLIGHTING.md) pour le color scheme complet.

### Completer (`completer.rs`)

Autocomplétion contextuelle :

- Commandes REPL (`/help`, `/exit`, etc.)
- Keywords (`if`, `while`, `for`, etc.)
- Builtins (`print`, `len`, `type`, etc.)
- Variables du contexte (mise à jour dynamique)
- Attributs (TODO Phase 1b)

### Validator (`validator.rs`)

Détection multiline intelligente :

- Délimiteurs non fermés (`(`, `[`, `{`)
- Opérateurs de continuation (`+`, `=`, `,`)
- Keywords de continuation (`if`, `while`, etc.)

### Config (`config.rs`)

Configuration tunable :

- Prompts (main: `▸`, continuation: `▹`)
- Couleurs (prompt, output, error, info, success)
- Messages (welcome, exit OK/abort/weird)
- Comportement (JIT, verbose, history)

## Build

```bash
# Compiler la REPL standalone
cargo build --bin catnip-repl --release --features repl-standalone

# Exécuter
target/release/catnip-repl
```

## Tests

```bash
# Tests unitaires du highlighter
cargo test --lib highlighter::tests --no-default-features --features embedded

# Tests unitaires du completer
cargo test --lib completer::tests --no-default-features --features embedded

# Tests unitaires du validator
cargo test --lib validator::tests --no-default-features --features embedded

# Tous les tests REPL
cargo test --lib repl:: --no-default-features --features embedded
```

## Commandes REPL

| Commande   | Alias         | Description                        |
| ---------- | ------------- | ---------------------------------- |
| `/help`    | `/h`          | Affiche l'aide                     |
| `/exit`    | `/quit`, `/q` | Quitte la REPL                     |
| `/clear`   | `/cls`        | Efface l'écran                     |
| `/stats`   | -             | Stats d'exécution (variables, JIT) |
| `/jit`     | -             | Toggle JIT on/off                  |
| `/verbose` | -             | Toggle verbose mode                |
| `/version` | `/v`          | Version de Catnip                  |

## Configuration par défaut

```rust
ReplConfig {
    prompt_main: "▸ ",
    prompt_continuation: "▹ ",
    enable_jit: true,
    jit_threshold: 100,
    history_file: ".catnip_history",
    max_history: 1000,
    // Colors
    color_prompt: BRIGHT_CYAN,
    color_output: RESET,
    color_error: BRIGHT_RED,
    color_info: BRIGHT_BLUE,
    color_success: BRIGHT_GREEN,
    // Exit messages in constants.rs (EXIT_OK, EXIT_ABORT, EXIT_WEIRD)
}
```

## Customization

Modifier `create_config()` dans `bin/repl.rs` :

```rust
fn create_config() -> ReplConfig {
    ReplConfig::default()
        .verbose()           // Montrer timings
        .no_jit()           // Désactiver JIT
        .with_jit_threshold(50)  // Custom threshold
}
```

## Limitations actuelles

- Nécessite un TTY (pas de mode stdin pipe)
- History non persistante (TODO Phase 1b+)
- Attribute completion non implémenté (TODO Phase 1b)

## Roadmap

- [ ] Phase 1b: FileBackedHistory persistante
- [ ] Phase 1c: Attribute completion
- [ ] Phase 2: Stdin fallback pour tests
- [ ] Phase 3: Custom key bindings
