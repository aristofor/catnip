// FILE: catnip_rs/src/tools/debugger_shims.rs
// PyO3 shims for debugger command parsing, formatting, and SourceMap.

use pyo3::prelude::*;

// --- DebugCommand ---

/// Discriminant tag exposed to Python as an integer enum.
#[pyclass(name = "DebugCommand", frozen, eq, eq_int, skip_from_py_object)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyDebugCommandKind {
    Continue = 0,
    StepInto = 1,
    StepOver = 2,
    StepOut = 3,
    Break = 4,
    RemoveBreak = 5,
    Print = 6,
    Vars = 7,
    List = 8,
    Backtrace = 9,
    Repl = 10,
    Quit = 11,
    Help = 12,
    Repeat = 13,
    Unknown = 14,
}

/// Result of parsing a debug command.
///
/// Attributes:
///     kind: DebugCommand enum variant
///     arg_int: line number for Break/RemoveBreak (0 otherwise)
///     arg_str: expression for Print, raw input for Unknown ("" otherwise)
#[pyclass(name = "ParsedDebugCommand", frozen, get_all, skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct PyParsedDebugCommand {
    pub kind: PyDebugCommandKind,
    pub arg_int: u32,
    pub arg_str: String,
}

#[pymethods]
impl PyParsedDebugCommand {
    fn __repr__(&self) -> String {
        match self.kind {
            PyDebugCommandKind::Break => format!("ParsedDebugCommand(Break, {})", self.arg_int),
            PyDebugCommandKind::RemoveBreak => {
                format!("ParsedDebugCommand(RemoveBreak, {})", self.arg_int)
            }
            PyDebugCommandKind::Print => {
                format!("ParsedDebugCommand(Print, {:?})", self.arg_str)
            }
            PyDebugCommandKind::Unknown => {
                format!("ParsedDebugCommand(Unknown, {:?})", self.arg_str)
            }
            _ => format!("ParsedDebugCommand({:?})", self.kind),
        }
    }
}

impl From<catnip_tools::debugger::DebugCommand> for PyParsedDebugCommand {
    fn from(cmd: catnip_tools::debugger::DebugCommand) -> Self {
        use catnip_tools::debugger::DebugCommand;
        match cmd {
            DebugCommand::Continue => Self {
                kind: PyDebugCommandKind::Continue,
                arg_int: 0,
                arg_str: String::new(),
            },
            DebugCommand::StepInto => Self {
                kind: PyDebugCommandKind::StepInto,
                arg_int: 0,
                arg_str: String::new(),
            },
            DebugCommand::StepOver => Self {
                kind: PyDebugCommandKind::StepOver,
                arg_int: 0,
                arg_str: String::new(),
            },
            DebugCommand::StepOut => Self {
                kind: PyDebugCommandKind::StepOut,
                arg_int: 0,
                arg_str: String::new(),
            },
            DebugCommand::Break(line) => Self {
                kind: PyDebugCommandKind::Break,
                arg_int: line,
                arg_str: String::new(),
            },
            DebugCommand::RemoveBreak(line) => Self {
                kind: PyDebugCommandKind::RemoveBreak,
                arg_int: line,
                arg_str: String::new(),
            },
            DebugCommand::Print(expr) => Self {
                kind: PyDebugCommandKind::Print,
                arg_int: 0,
                arg_str: expr,
            },
            DebugCommand::Vars => Self {
                kind: PyDebugCommandKind::Vars,
                arg_int: 0,
                arg_str: String::new(),
            },
            DebugCommand::List => Self {
                kind: PyDebugCommandKind::List,
                arg_int: 0,
                arg_str: String::new(),
            },
            DebugCommand::Backtrace => Self {
                kind: PyDebugCommandKind::Backtrace,
                arg_int: 0,
                arg_str: String::new(),
            },
            DebugCommand::Repl => Self {
                kind: PyDebugCommandKind::Repl,
                arg_int: 0,
                arg_str: String::new(),
            },
            DebugCommand::Quit => Self {
                kind: PyDebugCommandKind::Quit,
                arg_int: 0,
                arg_str: String::new(),
            },
            DebugCommand::Help => Self {
                kind: PyDebugCommandKind::Help,
                arg_int: 0,
                arg_str: String::new(),
            },
            DebugCommand::Repeat => Self {
                kind: PyDebugCommandKind::Repeat,
                arg_int: 0,
                arg_str: String::new(),
            },
            DebugCommand::Unknown(s) => Self {
                kind: PyDebugCommandKind::Unknown,
                arg_int: 0,
                arg_str: s,
            },
        }
    }
}

// --- parse function ---

#[pyfunction]
pub fn parse_debug_command(input: &str) -> PyParsedDebugCommand {
    catnip_tools::debugger::parse_command(input).into()
}

// --- Formatting functions ---

#[pyfunction]
pub fn format_debug_help() -> String {
    catnip_tools::debugger::format_help()
}

#[pyfunction]
pub fn format_debug_header() -> String {
    catnip_tools::debugger::format_header()
}

#[pyfunction]
pub fn format_debug_pause(line: u32, col: u32, snippet: &str) -> String {
    catnip_tools::debugger::format_pause(line, col, snippet)
}

#[pyfunction]
pub fn format_debug_vars(vars: Vec<(String, String)>) -> String {
    catnip_tools::debugger::format_vars(&vars)
}

#[pyfunction]
pub fn format_debug_backtrace(frames: Vec<(String, u32)>) -> String {
    catnip_tools::debugger::format_backtrace(&frames)
}

#[pyfunction]
pub fn format_debug_unknown_command(cmd: &str) -> String {
    catnip_tools::debugger::format_unknown_command(cmd)
}

// --- SourceMap ---

#[pyclass(name = "SourceMap")]
pub struct PySourceMap {
    inner: catnip_tools::sourcemap::SourceMap,
}

#[pymethods]
impl PySourceMap {
    #[new]
    #[pyo3(signature = (source, filename="<input>"))]
    fn new(source: Vec<u8>, filename: &str) -> Self {
        Self {
            inner: catnip_tools::sourcemap::SourceMap::new(source, filename.to_string()),
        }
    }

    /// Convert byte offset to (line, column) - 1-indexed.
    fn byte_to_line_col(&mut self, byte_offset: usize) -> (usize, usize) {
        self.inner.byte_to_line_col(byte_offset)
    }

    /// Get a single line by 1-indexed line number.
    fn get_line(&mut self, line_num: usize) -> String {
        self.inner.get_line(line_num)
    }

    /// Extract code snippet with pointer.
    #[pyo3(signature = (start_byte, end_byte, context_lines=0))]
    fn get_snippet(&mut self, start_byte: usize, end_byte: usize, context_lines: usize) -> String {
        self.inner.get_snippet(start_byte, end_byte, context_lines)
    }

    /// Convert a 1-indexed line number to byte offset.
    fn line_to_offset(&mut self, line: usize) -> Option<usize> {
        self.inner.line_to_offset(line)
    }

    /// Total number of lines.
    fn line_count(&mut self) -> usize {
        self.inner.line_count()
    }

    #[getter]
    fn source(&self) -> &[u8] {
        self.inner.source()
    }

    #[getter]
    fn filename(&self) -> &str {
        self.inner.filename()
    }

    fn __repr__(&self) -> String {
        format!(
            "SourceMap({:?}, {} bytes)",
            self.inner.filename(),
            self.inner.source().len()
        )
    }
}
