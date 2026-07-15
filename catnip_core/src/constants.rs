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

/// JIT default for the standalone/embedded world (catnip binary via `!no_jit`,
/// REPL config): performance-first, opt out with `--no-jit`.
pub const JIT_DEFAULT_STANDALONE: bool = true;

/// JIT default for the Python pipeline (CLI `catnip`, `Catnip()` API): off --
/// auto mode, file pragmas (`pragma("jit", ...)`) opt in per script.
pub const JIT_DEFAULT_PIPELINE: bool = false;

/// Hot detection threshold (iterations before compilation)
pub const JIT_THRESHOLD_DEFAULT: u32 = 100;

/// Max ops recorded per trace before aborting (loop too large to JIT)
pub const JIT_MAX_TRACE_OPS: usize = 10000;

/// Max recursion depth before interpreter fallback
pub const JIT_MAX_RECURSION_DEPTH: usize = 10000;

/// Max ND recursion depth before RecursionError (PyO3 pipeline).
/// Each ND recursive call burns native C stack shared with CPython (historical
/// estimate ~16KB/frame, overflow around ~494 frames on 8MB stacks). 300 keeps
/// a ~40% margin under that estimate (300 × 16KB = 4.8MB); raise only after
/// re-measuring the per-frame cost. The pure VM allows 10_000: its depth lives
/// on the heap, not the C stack -- see ND_MAX_DEPTH in catnip_vm/src/vm/broadcast.rs.
pub const ND_MAX_RECURSION_DEPTH: usize = 300;

// ============================================================================
// VM - Configuration
// ============================================================================

/// Initial stack capacity per frame
pub const VM_FRAME_STACK_CAPACITY: usize = 32;

/// Frame stack capacity (max call depth before realloc)
pub const VM_FRAME_STACK_INIT: usize = 64;

/// Frame pool size
pub const VM_FRAME_POOL_SIZE: usize = 64;

/// Default memory limit in MB (0 = disabled)
pub const MEMORY_LIMIT_DEFAULT_MB: u64 = 2048;

/// Periodic-check mask (interrupt flag + RSS), applied as `count & MASK == 0`
/// so checks fire every 65536 instructions.
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

/// Default disk cache TTL (seconds)
pub const CACHE_DISK_TTL_DEFAULT: u64 = 86400; // 24 hours

/// Max disk cache size (MB)
pub const CACHE_DISK_MAX_SIZE_MB_DEFAULT: u64 = 100;

// ============================================================================
// Optimization - Niveaux
// ============================================================================

/// Default optimization level (0-3)
pub const OPTIMIZATION_LEVEL_DEFAULT: u8 = 3;

/// TCO enabled by default
pub const TCO_ENABLED_DEFAULT: bool = true;

// ============================================================================
// Executor / Theme
// ============================================================================

/// Default executor ("vm" or "ast"). Mirrored in catnip/config.py via
/// DEFAULT_CONFIG (built from ConfigManager defaults).
pub const EXECUTOR_DEFAULT: &str = "vm";

/// Theme env var name. Mirrored in catnip/_theme.py (no PyO3 at that layer).
pub const ENV_THEME: &str = "CATNIP_THEME";

/// Valid theme values ("auto" resolves via terminal detection).
pub const THEME_VALUES: &[&str] = &["auto", "dark", "light"];

/// Default theme.
pub const THEME_DEFAULT: &str = "auto";

// ============================================================================
// Module Resolution - Env Vars
// ============================================================================

/// Module search path env var. Mirrored in catnip/loader.py (no PyO3 at that layer).
pub const ENV_CATNIP_PATH: &str = "CATNIP_PATH";

/// Native stdlib plugin search path env var (colon-separated).
pub const ENV_STDLIB_PATH: &str = "CATNIP_STDLIB_PATH";

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
