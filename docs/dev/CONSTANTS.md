# Constants - Configuration par défaut

Le fichier `catnip_rs/src/constants.rs` centralise **toutes les constantes par défaut** du runtime Catnip en Rust.

## Pourquoi ce fichier ?

Plutôt que d'avoir des valeurs magiques dispersées dans différents modules, tout est ici :

- **Facile à trouver** - un seul endroit pour tous les defaults
- **Facile à modifier** - changer une couleur, un prompt, un seuil
- **Facile à maintenir** - pas de duplication, source unique de vérité

## Sections

### REPL - Prompts

```rust
pub const REPL_PROMPT_MAIN: &str = "▸ ";
pub const REPL_PROMPT_CONTINUATION: &str = "▹ ";
```

**Important** : ces prompts sont **identiques à la version Python** pour cohérence UX.

### REPL - Couleurs

```rust
pub mod colors {
    pub const PROMPT: &str = "\x1b[96m"; // Cyan bright
    pub const ERROR: &str = "\x1b[91m";  // Red bright
    // ...
}
```

Codes ANSI pour les messages REPL (prompts, erreurs, info, succès).

### REPL - Syntax Highlighting

```rust
pub mod highlighting {
    pub const KEYWORD_COLOR: Color = Color::Cyan;
    pub const KEYWORD_BOLD: bool = true;
    // ...
}
```

Couleurs pour le syntax highlighting temps réel (keywords, strings, numbers, etc.).

### REPL - Messages

```rust
pub const REPL_WELCOME_TEMPLATE: &str = "...";
pub const REPL_GOODBYE: &str = "Bye!";
pub const REPL_HELP_TEXT: &str = r#"..."#;
```

Messages affichés au démarrage, à la sortie, et dans `/help`.

### JIT - Configuration

```rust
pub const JIT_ENABLED_DEFAULT: bool = true;
pub const JIT_THRESHOLD_DEFAULT: u32 = 100;
pub const JIT_MAX_RECURSION_DEPTH: usize = 10000;
```

Paramètres du JIT Cranelift (hot loop detection, recursion depth).

### VM - Configuration

```rust
pub const VM_STACK_INITIAL_SIZE: usize = 256;
pub const VM_FRAME_POOL_SIZE: usize = 64;
```

Tailles des structures internes de la VM.

### Cache - Configuration

```rust
pub const CACHE_MEMORY_MAX_SIZE: usize = 1000;
pub const CACHE_DISK_TTL_DEFAULT: u64 = 86400; // 24h
pub const CACHE_DISK_MAX_SIZE_MB_DEFAULT: u64 = 100;
```

Paramètres du système de cache (mémoire et disque).

### Optimization - Niveaux

```rust
pub const OPTIMIZATION_LEVEL_DEFAULT: u8 = 2;
pub const TCO_ENABLED_DEFAULT: bool = true;
```

Niveau d'optimisation et TCO par défaut.

## Utilisation

Les modules consomment ces constantes via `crate::constants::*` :

```rust
// Dans repl/config.rs
impl Default for ReplConfig {
    fn default() -> Self {
        Self {
            prompt_main: crate::constants::REPL_PROMPT_MAIN.to_string(),
            enable_jit: crate::constants::JIT_ENABLED_DEFAULT,
            // ...
        }
    }
}

// Dans repl/highlighter.rs
use crate::constants::highlighting as hl;

fn get_style(&self, node: &Node) -> Style {
    match node.kind() {
        "if" => Style::new().fg(hl::KEYWORD_COLOR),
        // ...
    }
}
```

## Modification

**Pour changer un default** :

1. Modifier la constante dans `catnip_rs/src/constants.rs`
1. Recompiler : `cargo build` ou `make install-lang`
1. Les changements s'appliquent partout automatiquement

**Exemples** :

- Changer le prompt : modifier `REPL_PROMPT_MAIN`
- Changer la couleur des keywords : modifier `highlighting::KEYWORD_COLOR`
- Augmenter le seuil JIT : modifier `JIT_THRESHOLD_DEFAULT`

Aucun code dans les autres modules n'a besoin de changer.
