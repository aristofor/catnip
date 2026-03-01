// FILE: catnip_tools/src/config.rs
/// Configuration for the formatter
#[derive(Debug, Clone)]
pub struct FormatConfig {
    pub indent_size: usize,
    pub line_length: usize,
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            indent_size: 4,
            line_length: 120,
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
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            check_syntax: true,
            check_style: true,
            check_semantic: true,
            check_ir: false,
        }
    }
}
