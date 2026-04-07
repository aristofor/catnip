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
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            check_syntax: true,
            check_style: true,
            check_semantic: true,
            check_ir: false,
            check_names: false,
            max_nesting_depth: 5,
            max_cyclomatic_complexity: 10,
            max_function_length: 30,
            max_parameters: 6,
        }
    }
}
