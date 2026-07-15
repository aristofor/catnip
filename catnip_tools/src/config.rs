// FILE: catnip_tools/src/config.rs
/// Configuration for the formatter
#[derive(Debug, Clone)]
pub struct FormatConfig {
    pub indent_size: usize,
    pub line_length: usize,
    pub align: bool,
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            indent_size: 4,
            line_length: 120,
            align: true,
        }
    }
}

/// Default lint metric thresholds. Single Rust source of truth, consumed by
/// the PyO3 shim (catnip_rs/src/tools/shims.rs); the Python CLI reads them
/// back through `catnip._rs.LintConfig()`.
pub const LINT_MAX_NESTING_DEPTH: usize = 5;
pub const LINT_MAX_CYCLOMATIC_COMPLEXITY: usize = 10;
pub const LINT_MAX_FUNCTION_LENGTH: usize = 30;
pub const LINT_MAX_PARAMETERS: usize = 6;

/// Configuration for the linter
#[derive(Debug, Clone)]
pub struct LintConfig {
    pub check_syntax: bool,
    pub check_style: bool,
    pub check_semantic: bool,
    pub check_ir: bool,
    /// Check for undefined names (E200). Off by default because names
    /// may come from `-m` imports, REPL context, or runtime config
    /// that the static linter cannot see.
    pub check_names: bool,
    /// Metric thresholds (0 = disabled)
    pub max_nesting_depth: usize,
    pub max_cyclomatic_complexity: usize,
    pub max_function_length: usize,
    pub max_parameters: usize,
    /// Diagnostic codes to suppress globally (post-analysis filter,
    /// applied alongside `# noqa`). Empty = nothing suppressed.
    pub disabled_codes: std::collections::HashSet<String>,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            check_syntax: true,
            check_style: true,
            check_semantic: true,
            check_ir: false,
            check_names: false,
            max_nesting_depth: LINT_MAX_NESTING_DEPTH,
            max_cyclomatic_complexity: LINT_MAX_CYCLOMATIC_COMPLEXITY,
            max_function_length: LINT_MAX_FUNCTION_LENGTH,
            max_parameters: LINT_MAX_PARAMETERS,
            disabled_codes: std::collections::HashSet::new(),
        }
    }
}
