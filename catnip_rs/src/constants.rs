// FILE: catnip_rs/src/constants.rs
//! Default constants for Catnip
//!
//! Visual constants (prompts, colors, highlighting) are generated
//! from visual.toml by build.rs. Non-visual constants are defined here.

// Generated from visual.toml: prompts, colors, highlighting, ui_colors, box_drawing
include!(concat!(env!("OUT_DIR"), "/theme_generated.rs"));

// ============================================================================
// REPL - Messages
// ============================================================================

/// Welcome message
pub const REPL_WELCOME_TEMPLATE: &str = "Catnip REPL v{version}
Type /help for help, /exit to quit
";

/// Normal exit messages
pub const REPL_EXIT_OK: &[&str] = &[
    "state resolved.",
    "collapse complete.",
    "reality committed.",
    "no more universes.",
    "halt.",
    "done.",
    "execution finished.",
    "session closed.",
    "context released.",
    "evaluation complete.",
    "final state reached.",
    "steady state achieved.",
    "nothing left to compute.",
    "all branches resolved.",
    "timeline collapsed.",
    "determinism restored.",
    "output committed.",
    "vm halted.",
    "runtime quiet.",
    "dimensions collapsed.",
    "superposition cleared.",
    "result fixed.",
    "fixed point reached.",
    "evaluation converged.",
    "normal termination.",
    "clean exit.",
    "control returned.",
    "stack unwound.",
    "heap stable.",
    "worldline closed.",
    "causality preserved.",
    "entropy minimized.",
    "final form assumed.",
    "state accepted.",
    "resolution complete.",
    "all paths exhausted.",
    "computation satisfied.",
    "no pending effects.",
    "effects committed.",
    "interpretation complete.",
    "program exhausted.",
];

/// Abort exit messages
pub const REPL_EXIT_ABORT: &[&str] = &[
    "context destroyed.",
    "vm is dead.",
    "execution aborted.",
    "aborted.",
    "forced termination.",
    "runtime interrupted.",
    "signal received.",
    "panic.",
    "inconsistent state.",
    "evaluation aborted.",
    "non-local exit.",
    "control lost.",
    "stack corrupted.",
    "worldline severed.",
    "causality broken.",
    "branch rejected.",
    "timeline aborted.",
    "state discarded.",
    "partial evaluation.",
    "side effects lost.",
    "rollback.",
    "rollback required.",
    "rollback failed.",
    "dirty exit.",
    "emergency stop.",
    "halted by force.",
    "user interruption.",
    "constraint violated.",
    "invariant broken.",
    "undefined behavior.",
    "resolution failed.",
    "non convergent.",
    "runtime instability.",
    "execution rejected.",
    "state invalid.",
    "context dropped.",
    "dimensions rejected.",
    "evaluation terminated.",
    "abort complete.",
];

/// Rare messages (1% of exits)
pub const REPL_EXIT_WEIRD: &[&str] = &[
    "this was an exit.",
    "evaluation evaluated itself.",
    "the repl has decided.",
    "no further interpretation available.",
    "this message is final.",
    "nothing follows.",
    "this statement terminates.",
    "the rest is silence.",
    "control returned to nowhere.",
    "the computation noticed you watching.",
];

/// Help text
pub const REPL_HELP_TEXT: &str = r#"Catnip REPL Commands:

  /help           Show this help
  /exit           Exit REPL
  /clear          Clear screen
  /history        Show command history
  /load <file>    Load and execute a file
  /stats          Show execution statistics
  /jit            Toggle JIT compiler
  /verbose        Toggle verbose mode (show timings)
  /debug          Toggle debug mode (show IR and bytecode)
  /time <expr>    Benchmark an expression (adaptive iterations)
  /version        Show Catnip version

Keyboard shortcuts:
  Ctrl+D          Exit REPL
  Ctrl+C          Cancel current input
  ↑/↓             Navigate history
"#;

// ============================================================================
// REPL - History
// ============================================================================

/// Default history file
pub const REPL_HISTORY_FILE: &str = ".catnip_history";

/// Max history size (number of lines)
pub const REPL_MAX_HISTORY: usize = 1000;

// ============================================================================
// JIT - Configuration
// ============================================================================

/// JIT enabled by default
pub const JIT_ENABLED_DEFAULT: bool = true;

/// Hot detection threshold (iterations before compilation)
pub const JIT_THRESHOLD_DEFAULT: u32 = 100;

/// Max recursion depth before interpreter fallback
pub const JIT_MAX_RECURSION_DEPTH: usize = 10000;

// ============================================================================
// VM - Configuration
// ============================================================================

/// Initial VM stack size
pub const VM_STACK_INITIAL_SIZE: usize = 256;

/// Frame pool size
pub const VM_FRAME_POOL_SIZE: usize = 64;

// ============================================================================
// Cache - Configuration
// ============================================================================

/// Max memory cache size (number of entries)
pub const CACHE_MEMORY_MAX_SIZE: usize = 1000;

/// Default disk cache TTL (seconds)
pub const CACHE_DISK_TTL_DEFAULT: u64 = 86400; // 24 hours

/// Max disk cache size (MB)
pub const CACHE_DISK_MAX_SIZE_MB_DEFAULT: u64 = 100;

// ============================================================================
// Optimization - Niveaux
// ============================================================================

/// Default optimization level (0-3)
pub const OPTIMIZATION_LEVEL_DEFAULT: u8 = 2;

/// TCO enabled by default
pub const TCO_ENABLED_DEFAULT: bool = true;

// ============================================================================
// JIT - Pure Builtins
// ============================================================================

/// Builtins with native Cranelift codegen (int args)
pub const JIT_NATIVE_BUILTINS: &[&str] = &["abs", "bool", "int", "max", "min", "round"];

/// Pure builtins callable via extern C callback (int/float args)
pub const JIT_CALLBACK_BUILTINS: &[&str] = &["float"];

/// All pure builtins (union, for LoadGlobal)
pub const JIT_PURE_BUILTINS: &[&str] = &[
    "abs",
    "all",
    "any",
    "bool",
    "dict",
    "enumerate",
    "filter",
    "float",
    "int",
    "len",
    "list",
    "map",
    "max",
    "min",
    "range",
    "round",
    "set",
    "sorted",
    "str",
    "sum",
    "tuple",
    "zip",
];
