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
}

#[pymethods]
impl LintConfig {
    #[new]
    #[pyo3(signature = (check_syntax=true, check_style=true, check_semantic=true, check_ir=false, check_names=false))]
    pub fn new(check_syntax: bool, check_style: bool, check_semantic: bool, check_ir: bool, check_names: bool) -> Self {
        Self {
            check_syntax,
            check_style,
            check_semantic,
            check_ir,
            check_names,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "LintConfig(syntax={}, style={}, semantic={}, ir={}, names={})",
            self.check_syntax, self.check_style, self.check_semantic, self.check_ir, self.check_names
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
    };

    let diagnostics =
        catnip_tools::linter::lint_code(source, &tools_cfg).map_err(pyo3::exceptions::PyRuntimeError::new_err)?;

    let list = PyList::empty(py);
    for d in diagnostics {
        let diag: Diagnostic = d.into();
        list.append(Py::new(py, diag)?)?;
    }

    Ok(list.into())
}
