# Constantes

Le fichier `catnip_rs/src/constants.rs` centralise **toutes les constantes par défaut** du runtime Catnip en Rust.

## Pourquoi ce fichier ?

Plutôt que d'avoir des valeurs magiques dispersées dans différents modules, tout est ici :

- **Facile à trouver** - un seul endroit pour tous les defaults
- **Facile à modifier** - changer un seuil, un prompt, un message
- **Facile à maintenir** - pas de duplication, source unique de vérité

## Sources

Les constantes proviennent de deux sources :

- **`constants.rs`** - Valeurs non-visuelles (messages, seuils, tailles)
- **`visual.toml`** - Couleurs et prompts (OKLCH), injectées via `build.rs` dans `theme_generated.rs`

Les constantes visuelles sont incluses dans `constants.rs` via `include!()`, donc tous les modules accèdent à tout via
`crate::constants::*`.

## Sections

### REPL - Prompts (visual.toml)

```toml
[prompts]
main = "▸ "
continuation = "▹ "
```

Définis dans `visual.toml`, convertis en constantes Rust par `build.rs`.

### REPL - Couleurs (visual.toml)

```toml
[ui]
prompt  = "oklch(0.6125 0.110175 181.3166)"
error   = "oklch(0.6504 0.1973 28.59)"
info    = "oklch(0.6338 0.179 258.35)"
success = "oklch(0.6206 0.2023 142)"
dim     = "oklch(0.7047 0 0)"
```

Toutes les couleurs sont en OKLCH dans `visual.toml`. Le build génère les codes ANSI correspondants. Deux palettes
(`accent.dark` / `accent.light`, `base.dark` / `base.light`) pour le syntax highlighting.

### REPL - Messages

```rust
pub const REPL_WELCOME_TEMPLATE: &str = "Catnip REPL v{version}\nType /help for help, /exit to quit\n";

pub const REPL_EXIT_OK: &[&str] = &["state resolved.", "collapse complete.", ...];
pub const REPL_EXIT_ABORT: &[&str] = &["context destroyed.", "vm is dead.", ...];
```

Deux pools de messages de sortie selon le contexte (exit normal, abort). Un message est choisi au hasard à chaque
fermeture.

### REPL - Historique

```rust
pub const REPL_HISTORY_FILE: &str = "repl_history";
pub const REPL_MAX_HISTORY: usize = 1000;
```

### JIT - Configuration

```rust
pub const JIT_ENABLED_DEFAULT: bool = true;
pub const JIT_THRESHOLD_DEFAULT: u32 = 100;
pub const JIT_MAX_RECURSION_DEPTH: usize = 10000;
```

Paramètres du JIT Cranelift (hot detection après 100 itérations/appels, fallback interpréteur au-delà de 10000 niveaux
de récursion).

### JIT - Pure Builtins

```rust
pub const JIT_NATIVE_BUILTINS: &[&str] = &["abs", "bool", "int", "max", "min", "round"];
pub const JIT_CALLBACK_BUILTINS: &[&str] = &["float"];
pub const JIT_PURE_BUILTINS: &[&str] = &[
    "abs", "all", "any", "bool", "dict", "enumerate", "filter", "float",
    "int", "len", "list", "map", "max", "min", "range", "round",
    "set", "sorted", "str", "sum", "tuple", "zip",
];
```

Trois catégories de builtins purs pour le JIT :

- **Native** : codegen Cranelift direct (args entiers)
- **Callback** : appel extern C (int/float)
- **Pure** : union des deux + builtins sans side effects (pour `LoadGlobal`)

### VM - Configuration

```rust
pub const VM_STACK_INITIAL_SIZE: usize = 256;
pub const VM_FRAME_POOL_SIZE: usize = 64;
```

Tailles initiales des structures internes de la VM.

### Cache - Configuration

```rust
pub const CACHE_MEMORY_MAX_SIZE: usize = 1000;
pub const CACHE_DISK_TTL_DEFAULT: u64 = 86400; // 24h
pub const CACHE_DISK_MAX_SIZE_MB_DEFAULT: u64 = 100;
```

Paramètres du système de cache (mémoire FIFO et disque LRU).

### Optimization - Niveaux

```rust
pub const OPTIMIZATION_LEVEL_DEFAULT: u8 = 2;
pub const TCO_ENABLED_DEFAULT: bool = true;
```

Niveau d'optimisation (0-3) et TCO par défaut.

## Utilisation

Les modules consomment ces constantes via `crate::constants::*` :

```rust
// Dans vm/mod.rs
use crate::constants::{VM_STACK_INITIAL_SIZE, VM_FRAME_POOL_SIZE};

// Dans jit/detector.rs
use crate::constants::JIT_THRESHOLD_DEFAULT;
```

## Modification

**Pour changer un default runtime** :

1. Modifier la constante dans `catnip_rs/src/constants.rs`
1. Recompiler : `make compile`

**Pour changer une couleur ou un prompt** :

1. Modifier `catnip_rs/visual.toml`
1. Recompiler : `make compile` (le `build.rs` régénère `theme_generated.rs`)

Aucun code dans les autres modules n'a besoin de changer.
