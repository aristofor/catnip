// FILE: catnip_core/src/constants.rs
//! Default constants for Catnip (non-visual).
//!
//! Visual constants (prompts, colors, highlighting) are generated
//! from visual.toml by build.rs in catnip_rs.

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
pub const REPL_EXIT_RARE: &[&str] = &[
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

// REPL_HELP_TEXT removed: help text is now generated from catnip_repl::commands::COMMANDS

// ============================================================================
// REPL - History
// ============================================================================

/// Default history file (inside XDG_STATE_HOME/catnip/)
pub const REPL_HISTORY_FILE: &str = "repl_history";

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

/// Max ND recursion depth before RecursionError.
/// Each ND recursive call creates a new VM stack frame (~16KB).
/// On 8MB thread stacks, overflow occurs around ~494 frames.
/// 200 provides safe margin across platforms (200 × 16KB = 3.2MB).
pub const ND_MAX_RECURSION_DEPTH: usize = 200;

// ============================================================================
// VM - Configuration
// ============================================================================

/// Initial VM stack size
pub const VM_STACK_INITIAL_SIZE: usize = 256;

/// Initial stack capacity per frame
pub const VM_FRAME_STACK_CAPACITY: usize = 32;

/// Frame stack capacity (max call depth before realloc)
pub const VM_FRAME_STACK_INIT: usize = 64;

/// Frame pool size
pub const VM_FRAME_POOL_SIZE: usize = 64;

/// Default memory limit in MB (0 = disabled)
pub const MEMORY_LIMIT_DEFAULT_MB: u64 = 2048;

/// Check RSS every N instructions (power of 2 for bitwise AND mask)
pub const MEMORY_CHECK_INTERVAL: u64 = 0xFFFF; // 65535

// ============================================================================
// Weird Log - Configuration
// ============================================================================

/// Max crash logs to keep
pub const WEIRD_LOG_MAX_DEFAULT: usize = 50;

// ============================================================================
// Format - Defaults
// ============================================================================

/// Default indentation size (spaces)
pub const FORMAT_INDENT_SIZE_DEFAULT: usize = 4;

/// Default max line length
pub const FORMAT_LINE_LENGTH_DEFAULT: usize = 120;

/// Default align mode
pub const FORMAT_ALIGN_DEFAULT: bool = true;

// ============================================================================
// JIT - Inlining
// ============================================================================

/// Max ops to inline per function
pub const JIT_MAX_INLINE_OPS: usize = 20;

/// Max inlining depth
pub const JIT_MAX_INLINE_DEPTH: usize = 2;

// ============================================================================
// Benchmark - Defaults
// ============================================================================

/// Default benchmark iterations
pub const BENCH_DEFAULT_ITERATIONS: usize = 10;

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
// GENERATED FROM catnip/context.py KNOWN_PURE_FUNCTIONS - do not edit manually.
// Run: python catnip_tools/gen_builtins.py
// @generated-pure-builtins-start
pub const JIT_PURE_BUILTINS: &[&str] = &[
    "abs",
    "all",
    "any",
    "bool",
    "complex",
    "dict",
    "divmod",
    "enumerate",
    "filter",
    "float",
    "fold",
    "int",
    "len",
    "list",
    "map",
    "max",
    "min",
    "pow",
    "range",
    "reduce",
    "round",
    "set",
    "sorted",
    "str",
    "sum",
    "tuple",
    "zip",
];
// @generated-pure-builtins-end

// ============================================================================
// Config Keys
// ============================================================================

/// All valid config keys.
pub const CONFIG_VALID_KEYS: &[&str] = &[
    "no_color",
    "jit",
    "tco",
    "optimize",
    "executor",
    "cache_max_size_mb",
    "cache_ttl_seconds",
    "theme",
    "memory_limit",
    "enable_cache",
    "log_weird_errors",
    "max_weird_logs",
];

/// All valid format config keys.
pub const CONFIG_VALID_FORMAT_KEYS: &[&str] = &["indent_size", "line_length", "align"];

// ============================================================================
// Error Messages
// ============================================================================

/// Format a NameError message.
pub fn format_name_error(name: &str) -> String {
    format!("name '{name}' is not defined")
}

/// Extract the identifier from a NameError message.
/// Returns `Some(name)` if the message matches `"name '...' is not defined"`.
pub fn extract_name_from_error(msg: &str) -> Option<&str> {
    let rest = msg.strip_prefix("name '")?;
    let end = rest.find("' is not defined")?;
    Some(&rest[..end])
}

// ============================================================================
// Boolean Parsing
// ============================================================================

/// Parse a string as a boolean value.
/// Returns None for unrecognized values.
pub fn parse_bool_value(s: &str) -> Option<bool> {
    match s {
        "on" | "true" | "1" | "yes" => Some(true),
        "off" | "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

// ============================================================================
// Pragma Directives (canonical names, single form)
// ============================================================================

/// All known pragma directive names.
pub const PRAGMA_DIRECTIVES: &[&str] = &[
    "optimize",
    "warning",
    "inline",
    "pure",
    "cache",
    "debug",
    "tco",
    "jit",
    "nd_mode",
    "nd_workers",
    "nd_memoize",
    "nd_batch_size",
];

/// Boolean pragmas (accept True/False).
pub const PRAGMA_BOOL: &[&str] = &["tco", "cache", "debug", "warning", "nd_memoize"];

/// Non-negative integer pragmas.
pub const PRAGMA_UINT: &[&str] = &["nd_workers", "nd_batch_size"];

/// Pragmas validated elsewhere (inline, pure).
pub const PRAGMA_DEFERRED: &[&str] = &["inline", "pure"];

// Individual directive names
pub const PRAGMA_OPTIMIZE: &str = "optimize";
pub const PRAGMA_JIT: &str = "jit";
pub const PRAGMA_ND_MODE: &str = "nd_mode";

/// Valid ND mode values.
pub const ND_MODE_VALUES: &[&str] = &["sequential", "thread", "process"];

/// Max optimization level.
pub const OPTIMIZE_MAX: i64 = 3;
