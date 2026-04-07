// FILE: catnip_rs/src/constants.rs
//! Default constants for Catnip.
//!
//! Visual constants (prompts, colors, highlighting) are generated
//! from visual.toml by build.rs. Non-visual constants live in catnip_core.

// Generated from visual.toml: prompts, colors, highlighting, ui_colors
include!(concat!(env!("OUT_DIR"), "/theme_generated.rs"));

// All non-visual constants from catnip_core
pub use catnip_core::constants::*;

// ============================================================================
// Python Module Paths (for py.import())
// ============================================================================

pub const PY_MOD_RS: &str = "catnip._rs";
pub const PY_MOD_NODES: &str = "catnip.nodes";
pub const PY_MOD_EXC: &str = "catnip.exc";
pub const PY_MOD_CONTEXT: &str = "catnip.context";
pub const PY_MOD_LOADER: &str = "catnip.loader";
pub const PY_MOD_SEMANTIC: &str = "catnip.semantic";
pub const PY_MOD_SEMANTIC_OPCODE: &str = "catnip.semantic.opcode";
pub const PY_MOD_TRANSFORMER: &str = "catnip.transformer";
pub const PY_MOD_PARSER: &str = "catnip.parser";
pub const PY_MOD_PRAGMA: &str = "catnip.pragma";
pub const PY_MOD_ND: &str = "catnip.nd";
pub const PY_MOD_VERSION: &str = "catnip._version";
pub const PY_MOD_SUGGEST: &str = "catnip.suggest";
pub const PY_MOD_CLI: &str = "catnip.cli";
pub const PY_MOD_JIT: &str = "catnip.jit";
pub const PY_MOD_VM_EXECUTOR: &str = "catnip.vm.executor";
pub const PY_MOD_UTILS: &str = "catnip.utils";
pub const PY_MOD_EXTENSIONS: &str = "catnip.extensions";
pub const PY_MOD_MEMOIZATION: &str = "catnip.cachesys.memoization";
pub const PY_MOD_CONFIG: &str = "catnip.config";

// ============================================================================
// Config Keys
// ============================================================================

pub const CFG_NO_COLOR: &str = "no_color";
pub const CFG_JIT: &str = "jit";
pub const CFG_TCO: &str = "tco";
pub const CFG_OPTIMIZE: &str = "optimize";
pub const CFG_EXECUTOR: &str = "executor";
pub const CFG_THEME: &str = "theme";
pub const CFG_ENABLE_CACHE: &str = "enable_cache";
pub const CFG_CACHE_MAX_SIZE_MB: &str = "cache_max_size_mb";
pub const CFG_CACHE_TTL_SECONDS: &str = "cache_ttl_seconds";
pub const CFG_LOG_WEIRD_ERRORS: &str = "log_weird_errors";
pub const CFG_MAX_WEIRD_LOGS: &str = "max_weird_logs";
pub const CFG_MEMORY_LIMIT: &str = "memory_limit";
pub const CFG_FMT_INDENT_SIZE: &str = "indent_size";
pub const CFG_FMT_LINE_LENGTH: &str = "line_length";
pub const CFG_FMT_ALIGN: &str = "align";
