// FILE: catnip_rs/src/tools/shims.rs
// PyO3 shims that delegate to catnip_tools (linked statically).

use pyo3::prelude::*;
use pyo3::types::PyList;

use crate::constants::{FORMAT_ALIGN_DEFAULT, FORMAT_INDENT_SIZE_DEFAULT, FORMAT_LINE_LENGTH_DEFAULT};

// --- FormatConfig ---

#[pyclass(name = "FormatConfig", from_py_object)]
#[derive(Debug, Clone)]
pub struct FormatConfig {
    #[pyo3(get, set)]
    pub indent_size: usize,
    #[pyo3(get, set)]
    pub line_length: usize,
    #[pyo3(get, set)]
    pub align: bool,
}

#[pymethods]
impl FormatConfig {
    #[new]
    #[pyo3(signature = (indent_size=FORMAT_INDENT_SIZE_DEFAULT, line_length=FORMAT_LINE_LENGTH_DEFAULT, align=FORMAT_ALIGN_DEFAULT))]
    pub fn new(indent_size: usize, line_length: usize, align: bool) -> Self {
        Self {
            indent_size,
            line_length,
            align,
        }
    }

    #[staticmethod]
    pub fn from_toml_section(text: &str) -> PyResult<Self> {
        let mut config = Self::default();
        let mut in_format_section = false;

        for line in text.lines() {
            let line = line.trim();

            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if line == "[format]" {
                in_format_section = true;
                continue;
            }

            if line.starts_with('[') && line.ends_with(']') {
                in_format_section = false;
                continue;
            }

            if in_format_section {
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim();
                    let value = value.trim();

                    match key {
                        "indent_size" => {
                            config.indent_size = value.parse().map_err(|_| {
                                pyo3::exceptions::PyValueError::new_err(format!("Invalid indent_size value: {}", value))
                            })?;
                        }
                        "line_length" => {
                            config.line_length = value.parse().map_err(|_| {
                                pyo3::exceptions::PyValueError::new_err(format!("Invalid line_length value: {}", value))
                            })?;
                        }
                        "align" => {
                            config.align = match value {
                                "true" => true,
                                "false" => false,
                                _ => {
                                    return Err(pyo3::exceptions::PyValueError::new_err(format!(
                                        "Invalid align value: {value}. Expected 'true' or 'false'"
                                    )));
                                }
                            };
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(config)
    }

    fn __repr__(&self) -> String {
        format!(
            "FormatConfig(indent_size={}, line_length={}, align={})",
            self.indent_size,
            self.line_length,
            if self.align { "True" } else { "False" },
        )
    }
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            indent_size: FORMAT_INDENT_SIZE_DEFAULT,
            line_length: FORMAT_LINE_LENGTH_DEFAULT,
            align: FORMAT_ALIGN_DEFAULT,
        }
    }
}

impl FormatConfig {
    fn to_tools(&self) -> catnip_tools::config::FormatConfig {
        catnip_tools::config::FormatConfig {
            indent_size: self.indent_size,
            line_length: self.line_length,
            align: self.align,
        }
    }
}

// --- Formatter ---

#[pyclass]
pub struct Formatter {
    config: FormatConfig,
}

#[pymethods]
impl Formatter {
    #[new]
    #[pyo3(signature = (config=None))]
    pub fn new(config: Option<FormatConfig>) -> Self {
        Self {
            config: config.unwrap_or_default(),
        }
    }

    pub fn format(&self, _py: Python, source: &str) -> PyResult<String> {
        catnip_tools::formatter::format_code(source, &self.config.to_tools())
            .map_err(pyo3::exceptions::PyRuntimeError::new_err)
    }
}

// --- format_code function ---

#[pyfunction]
#[pyo3(signature = (source, config=None))]
pub fn format_code(_py: Python, source: &str, config: Option<FormatConfig>) -> PyResult<String> {
    let cfg = config.unwrap_or_default();
    catnip_tools::formatter::format_code(source, &cfg.to_tools()).map_err(pyo3::exceptions::PyRuntimeError::new_err)
}

// --- Severity ---

#[pyclass(eq, eq_int, frozen, hash, skip_from_py_object)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Error = 0,
    Warning = 1,
    Info = 2,
    Hint = 3,
}

#[pymethods]
impl Severity {
    #[getter]
    fn name(&self) -> &'static str {
        match self {
            Severity::Error => "ERROR",
            Severity::Warning => "WARNING",
            Severity::Info => "INFO",
            Severity::Hint => "HINT",
        }
    }

    fn __repr__(&self) -> String {
        format!("Severity.{}", self.name())
    }
}

// --- LintConfig ---

#[pyclass(from_py_object)]
#[derive(Debug, Clone)]
pub struct LintConfig {
    #[pyo3(get, set)]
    pub check_syntax: bool,
    #[pyo3(get, set)]
    pub check_style: bool,
    #[pyo3(get, set)]
    pub check_semantic: bool,
    #[pyo3(get, set)]
    pub check_ir: bool,
    #[pyo3(get, set)]
    pub check_names: bool,
    #[pyo3(get, set)]
    pub max_nesting_depth: usize,
    #[pyo3(get, set)]
    pub max_cyclomatic_complexity: usize,
    #[pyo3(get, set)]
    pub max_function_length: usize,
    #[pyo3(get, set)]
    pub max_parameters: usize,
}

#[pymethods]
impl LintConfig {
    #[new]
    #[pyo3(signature = (
        check_syntax=true,
        check_style=true,
        check_semantic=true,
        check_ir=false,
        check_names=false,
        max_nesting_depth=5,
        max_cyclomatic_complexity=10,
        max_function_length=30,
        max_parameters=6,
    ))]
    pub fn new(
        check_syntax: bool,
        check_style: bool,
        check_semantic: bool,
        check_ir: bool,
        check_names: bool,
        max_nesting_depth: usize,
        max_cyclomatic_complexity: usize,
        max_function_length: usize,
        max_parameters: usize,
    ) -> Self {
        Self {
            check_syntax,
            check_style,
            check_semantic,
            check_ir,
            check_names,
            max_nesting_depth,
            max_cyclomatic_complexity,
            max_function_length,
            max_parameters,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "LintConfig(syntax={}, style={}, semantic={}, ir={}, names={}, max_nesting={}, max_complexity={}, max_length={}, max_params={})",
            self.check_syntax,
            self.check_style,
            self.check_semantic,
            self.check_ir,
            self.check_names,
            self.max_nesting_depth,
            self.max_cyclomatic_complexity,
            self.max_function_length,
            self.max_parameters,
        )
    }
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

// --- Diagnostic ---

#[pyclass(get_all, from_py_object)]
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub severity: Severity,
    pub line: usize,
    pub column: usize,
    pub end_line: Option<usize>,
    pub end_column: Option<usize>,
    pub source_line: Option<String>,
    pub suggestion: Option<String>,
}

#[pymethods]
impl Diagnostic {
    fn __str__(&self) -> String {
        let loc = if let (Some(el), Some(ec)) = (self.end_line, self.end_column) {
            format!("{}:{}-{}:{}", self.line, self.column, el, ec)
        } else {
            format!("{}:{}", self.line, self.column)
        };
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Hint => "hint",
        };
        format!("{}: {} [{}]: {}", loc, sev, self.code, self.message)
    }

    fn __repr__(&self) -> String {
        format!("Diagnostic({}, {}:{})", self.code, self.line, self.column)
    }

    #[getter]
    fn has_suggestion(&self) -> bool {
        self.suggestion.is_some()
    }
}

impl From<catnip_tools::linter::Diagnostic> for Diagnostic {
    fn from(d: catnip_tools::linter::Diagnostic) -> Self {
        let severity = match d.severity {
            catnip_tools::linter::Severity::Error => Severity::Error,
            catnip_tools::linter::Severity::Warning => Severity::Warning,
            catnip_tools::linter::Severity::Info => Severity::Info,
            catnip_tools::linter::Severity::Hint => Severity::Hint,
        };
        Self {
            code: d.code,
            message: d.message,
            severity,
            line: d.line,
            column: d.column,
            end_line: d.end_line,
            end_column: d.end_column,
            source_line: d.source_line,
            suggestion: d.suggestion,
        }
    }
}

// --- lint_code function ---

/// Parse source to IR for semantic analysis. Returns None on parse/transform error.
fn parse_and_transform(source: &str) -> Option<catnip_core::ir::IR> {
    let language = crate::get_tree_sitter_language();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    crate::parser::transform_pure(root, source).ok()
}

/// Convert byte offset to (line, column), both 1-based.
fn byte_to_line_col(source: &str, byte: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in source.char_indices() {
        if i >= byte {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

#[pyfunction]
#[pyo3(signature = (source, config=None))]
pub fn lint_code(py: Python, source: &str, config: Option<LintConfig>) -> PyResult<Py<PyList>> {
    let cfg = config.unwrap_or_default();
    let tools_cfg = catnip_tools::config::LintConfig {
        check_syntax: cfg.check_syntax,
        check_style: cfg.check_style,
        check_semantic: cfg.check_semantic,
        check_ir: cfg.check_ir,
        check_names: cfg.check_names,
        max_nesting_depth: cfg.max_nesting_depth,
        max_cyclomatic_complexity: cfg.max_cyclomatic_complexity,
        max_function_length: cfg.max_function_length,
        max_parameters: cfg.max_parameters,
    };

    let mut diagnostics =
        catnip_tools::linter::lint_code(source, &tools_cfg).map_err(pyo3::exceptions::PyRuntimeError::new_err)?;

    // Semantic I103: replace CST-level I103 with type-aware check from semantic analyzer
    if cfg.check_semantic {
        if let Some(ir) = parse_and_transform(source) {
            let mut analyzer = catnip_core::pipeline::SemanticAnalyzer::new();
            if let Ok(result) = analyzer.analyze_full(&ir) {
                // Always remove CST-level I103: semantic check is authoritative
                diagnostics.retain(|d| d.code != "I103");
                let source_lines: Vec<&str> = source.lines().collect();
                // Collect noqa directives for filtering semantic diagnostics
                let noqa = catnip_tools::linter::collect_noqa(source);
                for sd in result.diagnostics {
                    let (line, column) = byte_to_line_col(source, sd.start_byte);
                    // Apply noqa suppression
                    if let Some(codes) = noqa.get(&line) {
                        if codes.is_empty() || codes.contains(&sd.code) {
                            continue; // noqa: all or noqa: <matching code>
                        }
                    }
                    let (end_line, end_column) = byte_to_line_col(source, sd.end_byte);
                    diagnostics.push(catnip_tools::linter::Diagnostic {
                        code: sd.code,
                        message: sd.message,
                        severity: match sd.severity {
                            catnip_core::pipeline::SemanticSeverity::Warning => catnip_tools::linter::Severity::Warning,
                            catnip_core::pipeline::SemanticSeverity::Hint => catnip_tools::linter::Severity::Hint,
                        },
                        line,
                        column,
                        end_line: Some(end_line),
                        end_column: Some(end_column),
                        source_line: source_lines.get(line.saturating_sub(1)).map(|s| s.to_string()),
                        suggestion: None,
                    });
                }
                // Re-sort after appending semantic diagnostics
                diagnostics.sort_by(|a, b| a.line.cmp(&b.line).then(a.column.cmp(&b.column)));
            }
        }
    }

    let list = PyList::empty(py);
    for d in diagnostics {
        let diag: Diagnostic = d.into();
        list.append(Py::new(py, diag)?)?;
    }

    Ok(list.into())
}
