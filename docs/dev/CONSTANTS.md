# Constantes

Le fichier `catnip_core/src/constants.rs` centralise **toutes les constantes par défaut** du runtime Catnip en Rust.

## Pourquoi ce fichier ?

Plutôt que d'avoir des valeurs magiques dispersées dans différents modules, tout est ici :

- **Facile à trouver** - un seul endroit pour tous les defaults
- **Facile à modifier** - changer un seuil, un prompt, un message
- **Facile à maintenir** - pas de duplication, source unique de vérité

## Sources

Les constantes proviennent de deux sources :

- **`constants.rs`** - Valeurs non-visuelles (messages, seuils, tailles)
- **`visual.toml`** - Couleurs et prompts (OKLCH), injectées via `build.rs` dans `theme_generated.rs`

Les constantes non-visuelles vivent dans `catnip_core` (pure Rust, sans PyO3). Les constantes visuelles sont generees
par `build.rs` dans `catnip_rs`. Le fichier `catnip_rs/src/constants.rs` re-exporte tout via
`pub use catnip_core::constants::*` + `include!()`, donc tous les modules accèdent à tout via `crate::constants::*`.

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

### ND Recursion

```rust
pub const ND_MAX_RECURSION_DEPTH: usize = 200;
```

Profondeur maximale de récursion ND (`~~`). Chaque appel récursif via `recur()` crée une nouvelle instance VM sur la
stack Rust (~16KB par frame). Sur une stack de 8MB, le stack overflow survient autour de ~494 frames. La limite à 200
donne une marge confortable cross-platform (200 × 16KB = 3.2MB). Le guard est appliqué dans
`VMHost.execute_nd_recursion`.

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
pub const VM_FRAME_STACK_CAPACITY: usize = 32;
pub const VM_FRAME_STACK_INIT: usize = 64;
pub const VM_FRAME_POOL_SIZE: usize = 64;
```

Tailles initiales des structures internes de la VM. `VM_FRAME_STACK_CAPACITY` est la capacité du stack d'opérandes par
frame, `VM_FRAME_STACK_INIT` la capacité initiale du stack de frames (profondeur d'appels avant réallocation).

### Weird Log

```rust
pub const WEIRD_LOG_MAX_DEFAULT: usize = 50;
```

Nombre maximum de crash logs conservés dans `~/.local/state/catnip/`.

### Format - Defaults

```rust
pub const FORMAT_INDENT_SIZE_DEFAULT: usize = 4;
pub const FORMAT_LINE_LENGTH_DEFAULT: usize = 120;
pub const FORMAT_ALIGN_DEFAULT: bool = true;
```

Valeurs par défaut du formatter (`catnip format`). Overridables via `[format]` dans `catnip.toml`.

### JIT - Inlining

```rust
pub const JIT_MAX_INLINE_OPS: usize = 20;
pub const JIT_MAX_INLINE_DEPTH: usize = 2;
```

Limites de l'inliner JIT. `MAX_INLINE_OPS` contrôle la taille maximale d'une fonction inlinée, `MAX_INLINE_DEPTH` la
profondeur de l'inlining récursif.

### Benchmark

```rust
pub const BENCH_DEFAULT_ITERATIONS: usize = 10;
```

Nombre d'itérations par défaut pour `catnip bench`.

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

### Python Module Paths

```rust
pub const PY_MOD_RS: &str = "catnip._rs";
pub const PY_MOD_NODES: &str = "catnip.nodes";
pub const PY_MOD_CONTEXT: &str = "catnip.context";
pub const PY_MOD_LOADER: &str = "catnip.loader";
pub const PY_MOD_SEMANTIC: &str = "catnip.semantic";
// ... 12 constantes au total
```

Chemins `py.import()` utilisés par les modules Rust (via PyO3) pour accéder aux modules Python. Définis dans
`catnip_rs/src/constants.rs` (pas dans `catnip_core`, car ils dépendent de PyO3).

### Config Keys

```rust
pub const CFG_NO_COLOR: &str = "no_color";
pub const CFG_JIT: &str = "jit";
pub const CFG_TCO: &str = "tco";
pub const CFG_OPTIMIZE: &str = "optimize";
pub const CFG_EXECUTOR: &str = "executor";
// ... 14 constantes au total
```

Clés de configuration utilisées par `ConfigManager`. Le type `&'static str` permet d'utiliser
`HashMap<&'static str, ConfigValue>` au lieu de `HashMap<String, ConfigValue>`, éliminant les allocations heap pour les
clés.

### Config Validation

```rust
pub const CONFIG_VALID_KEYS: &[&str] = &[
    "no_color", "jit", "tco", "optimize", "executor",
    "cache_max_size_mb", "cache_ttl_seconds", "theme",
    "memory_limit", "enable_cache", "log_weird_errors", "max_weird_logs",
];
pub const CONFIG_VALID_FORMAT_KEYS: &[&str] = &["indent_size", "line_length", "align"];
```

Listes exhaustives des clés valides, exposées à Python via `valid_config_keys()` / `valid_format_keys()`. Les frozensets
Python (`VALID_KEYS`, `VALID_FORMAT_KEYS`) en dérivent directement -- source unique en Rust.

### Error Messages

```rust
pub fn format_name_error(name: &str) -> String;
pub fn extract_name_from_error(msg: &str) -> Option<&str>;
```

Format centralisé pour les NameError (`"name 'x' is not defined"`). Tous les sites Rust (scope, frame, registry, VM)
utilisent `format_name_error`. L'extraction est exposée à Python via `extract_name_from_error()` (PyO3), remplaçant le
regex côté `compat.py`.

### Boolean Parsing

```rust
pub fn parse_bool_value(s: &str) -> Option<bool> {
    match s {
        "on" | "true" | "1" | "yes" => Some(true),
        "off" | "false" | "0" | "no" => Some(false),
        _ => None,
    }
}
```

Parsing centralisé pour les valeurs booléennes textuelles (config, pragmas, options CLI).

## Architecture

Deux fichiers de constantes :

- **`catnip_core/src/constants.rs`** : constantes non-visuelles pures Rust (messages, seuils, tailles, builtins,
  `parse_bool_value`)
- **`catnip_rs/src/constants.rs`** : ré-exporte `catnip_core::constants::*`, ajoute les constantes PyO3 (`PY_MOD_*`,
  `CFG_*`), et inclut les constantes visuelles via `build.rs`

Tous les modules accèdent à tout via `use crate::constants::*`.

## Utilisation

```rust
// Import de module Python
use crate::constants::PY_MOD_SEMANTIC;
let module = py.import(PY_MOD_SEMANTIC)?;

// Clé de config (zero-alloc)
use crate::constants::CFG_TCO;
self.values.insert(CFG_TCO, config_value);

// Parsing booléen
use crate::constants::parse_bool_value;
let enabled = parse_bool_value("on"); // Some(true)
```

## Modification

**Pour changer un default runtime** :

1. Modifier la constante dans `catnip_core/src/constants.rs`
1. Recompiler : `make compile`

**Pour changer une couleur ou un prompt** :

1. Modifier `catnip_rs/visual.toml`
1. Recompiler : `make compile` (le `build.rs` régénère `theme_generated.rs`)

Aucun code dans les autres modules n'a besoin de changer.
