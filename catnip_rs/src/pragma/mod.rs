// FILE: catnip_rs/src/pragma/mod.rs
//! Pragma system for Catnip - Rust implementation.
//!
//! Pragmas allow fine-grained control over:
//! - Optimization levels
//! - Compiler warnings
//! - Runtime behavior

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyType};

/// Types of pragmas supported.
#[pyclass(eq, eq_int, module = "catnip._rs", from_py_object)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PragmaType {
    Optimize = 1,
    Warning = 2,
    Inline = 3,
    Pure = 4,
    Cache = 5,
    Debug = 6,
    Tco = 7,
    Jit = 8,
    NdMode = 10,
    NdWorkers = 11,
    NdMemoize = 12,
    NdBatchSize = 13,
    Unknown = 14,
}

#[pymethods]
impl PragmaType {
    #[pyo3(name = "OPTIMIZE")]
    #[classattr]
    fn optimize() -> Self {
        Self::Optimize
    }

    #[pyo3(name = "WARNING")]
    #[classattr]
    fn warning() -> Self {
        Self::Warning
    }

    #[pyo3(name = "INLINE")]
    #[classattr]
    fn inline() -> Self {
        Self::Inline
    }

    #[pyo3(name = "PURE")]
    #[classattr]
    fn pure() -> Self {
        Self::Pure
    }

    #[pyo3(name = "CACHE")]
    #[classattr]
    fn cache() -> Self {
        Self::Cache
    }

    #[pyo3(name = "DEBUG")]
    #[classattr]
    fn debug() -> Self {
        Self::Debug
    }

    #[pyo3(name = "TCO")]
    #[classattr]
    fn tco() -> Self {
        Self::Tco
    }

    #[pyo3(name = "JIT")]
    #[classattr]
    fn jit() -> Self {
        Self::Jit
    }

    #[pyo3(name = "ND_MODE")]
    #[classattr]
    fn nd_mode() -> Self {
        Self::NdMode
    }

    #[pyo3(name = "ND_WORKERS")]
    #[classattr]
    fn nd_workers() -> Self {
        Self::NdWorkers
    }

    #[pyo3(name = "ND_MEMOIZE")]
    #[classattr]
    fn nd_memoize() -> Self {
        Self::NdMemoize
    }

    #[pyo3(name = "ND_BATCH_SIZE")]
    #[classattr]
    fn nd_batch_size() -> Self {
        Self::NdBatchSize
    }

    #[pyo3(name = "UNKNOWN")]
    #[classattr]
    fn unknown() -> Self {
        Self::Unknown
    }

    fn __repr__(&self) -> String {
        format!("PragmaType.{:?}", self)
    }

    /// Return all enum variants as a list.
    #[classmethod]
    #[pyo3(name = "all_variants")]
    fn all_variants(_cls: &Bound<'_, PyType>) -> Vec<Self> {
        vec![
            Self::Optimize,
            Self::Warning,
            Self::Inline,
            Self::Pure,
            Self::Cache,
            Self::Debug,
            Self::Tco,
            Self::Jit,
            Self::NdMode,
            Self::NdWorkers,
            Self::NdMemoize,
            Self::NdBatchSize,
            Self::Unknown,
        ]
    }

    /// Get the integer value of this pragma type.
    #[getter]
    fn value(&self) -> i32 {
        *self as i32
    }
}

/// Represents a pragma directive.
#[pyclass(module = "catnip._rs")]
#[derive(Debug)]
pub struct Pragma {
    #[pyo3(get, set, name = "type")]
    pub pragma_type: PragmaType,

    #[pyo3(get, set)]
    pub directive: String,

    #[pyo3(get, set)]
    pub value: Py<PyAny>,

    #[pyo3(get, set)]
    pub options: Py<PyAny>, // Dict[str, Any]

    #[pyo3(get, set)]
    pub line: Option<i32>,
}

#[pymethods]
impl Pragma {
    #[new]
    #[pyo3(signature = (r#type, directive, value, options, line=None))]
    fn new(
        r#type: PragmaType,
        directive: String,
        value: Py<PyAny>,
        options: Py<PyAny>,
        line: Option<i32>,
    ) -> Self {
        Self {
            pragma_type: r#type,
            directive,
            value,
            options,
            line,
        }
    }

    fn __repr__(&self, py: Python) -> PyResult<String> {
        let value_repr = self.value.bind(py).repr()?.to_string();
        Ok(format!("<Pragma {}={}>", self.directive, value_repr))
    }
}

/// Maintains pragma state during compilation/execution.
#[pyclass(module = "catnip._rs")]
pub struct PragmaContext {
    pragmas: Vec<Py<Pragma>>,

    #[pyo3(get, set)]
    optimize_level: i32,

    #[pyo3(get)]
    warnings: Py<pyo3::types::PyDict>,

    #[pyo3(get)]
    inline_hints: Py<pyo3::types::PyDict>,

    #[pyo3(get)]
    pure_functions: Py<pyo3::types::PySet>,

    #[pyo3(get, set)]
    cache_enabled: bool,

    #[pyo3(get, set)]
    debug_mode: bool,

    #[pyo3(get, set)]
    tco_enabled: bool,

    #[pyo3(get, set)]
    jit_enabled: bool,

    #[pyo3(get, set)]
    jit_all: bool,

    #[pyo3(get, set)]
    nd_mode: String,

    #[pyo3(get, set)]
    nd_workers: i32,

    #[pyo3(get, set)]
    nd_memoize: bool,

    #[pyo3(get, set)]
    nd_batch_size: i32,

    state_stack: Vec<PragmaState>,
}

struct PragmaState {
    optimize_level: i32,
    warnings: Py<pyo3::types::PyDict>,
    inline_hints: Py<pyo3::types::PyDict>,
    pure_functions: Py<pyo3::types::PySet>,
    cache_enabled: bool,
    debug_mode: bool,
    nd_mode: String,
    nd_workers: i32,
    nd_memoize: bool,
    nd_batch_size: i32,
}

#[pymethods]
impl PragmaContext {
    #[new]
    fn new(py: Python) -> Self {
        Self {
            pragmas: Vec::new(),
            optimize_level: 0,
            warnings: pyo3::types::PyDict::new(py).into(),
            inline_hints: pyo3::types::PyDict::new(py).into(),
            pure_functions: pyo3::types::PySet::empty(py).unwrap().into(),
            cache_enabled: true,
            debug_mode: false,
            tco_enabled: true,
            jit_enabled: false, // Controlled by Python ConfigManager
            jit_all: false,
            nd_mode: "sequential".to_string(),
            nd_workers: 0,
            nd_memoize: false,
            nd_batch_size: 0,
            state_stack: Vec::new(),
        }
    }

    /// Add a pragma and apply its effects.
    fn add(&mut self, py: Python, pragma: Py<Pragma>) -> PyResult<()> {
        self.apply_pragma(py, pragma.clone_ref(py))?;
        self.pragmas.push(pragma);
        Ok(())
    }

    /// Push current state onto stack.
    fn push_state(&mut self, py: Python) {
        let state = PragmaState {
            optimize_level: self.optimize_level,
            warnings: self.warnings.clone_ref(py),
            inline_hints: self.inline_hints.clone_ref(py),
            pure_functions: self.pure_functions.clone_ref(py),
            cache_enabled: self.cache_enabled,
            debug_mode: self.debug_mode,
            nd_mode: self.nd_mode.clone(),
            nd_workers: self.nd_workers,
            nd_memoize: self.nd_memoize,
            nd_batch_size: self.nd_batch_size,
        };
        self.state_stack.push(state);
    }

    /// Restore state from stack.
    fn pop_state(&mut self, py: Python<'_>) -> PyResult<()> {
        let state = self.state_stack.pop().ok_or_else(|| {
            let exc_module = py.import("catnip.exc").unwrap();
            let internal_error = exc_module.getattr("CatnipInternalError").unwrap();
            PyErr::from_value(internal_error.call1(("No state to pop",)).unwrap())
        })?;

        self.optimize_level = state.optimize_level;
        self.warnings = state.warnings;
        self.inline_hints = state.inline_hints;
        self.pure_functions = state.pure_functions;
        self.cache_enabled = state.cache_enabled;
        self.debug_mode = state.debug_mode;
        self.nd_mode = state.nd_mode;
        self.nd_workers = state.nd_workers;
        self.nd_memoize = state.nd_memoize;
        self.nd_batch_size = state.nd_batch_size;
        Ok(())
    }

    /// Check if a warning is enabled.
    fn is_warning_enabled(&self, py: Python, name: &str) -> bool {
        let warnings_dict = self.warnings.bind(py);

        // Try specific warning first
        if let Ok(Some(value)) = warnings_dict.get_item(name) {
            return value.extract().unwrap_or(true);
        }

        // Fallback to "all"
        if let Ok(Some(value)) = warnings_dict.get_item("all") {
            return value.extract().unwrap_or(true);
        }

        // Default to true
        true
    }

    /// Get inline hint for a function.
    fn should_inline(&self, py: Python, func_name: &str) -> String {
        let hints_dict = self.inline_hints.bind(py);

        // Try specific function first
        if let Ok(Some(value)) = hints_dict.get_item(func_name) {
            return value.extract().unwrap_or_else(|_| "auto".to_string());
        }

        // Fallback to __default__
        if let Ok(Some(value)) = hints_dict.get_item("__default__") {
            return value.extract().unwrap_or_else(|_| "auto".to_string());
        }

        // Default to auto
        "auto".to_string()
    }

    /// Check if function is marked as pure.
    fn is_pure(&self, py: Python, func_name: &str) -> bool {
        let pure_set = self.pure_functions.bind(py);
        pure_set.contains(func_name).unwrap_or(false)
    }

    /// Set warnings dict (replaces entire dict).
    #[setter]
    fn set_warnings(&mut self, py: Python, value: &Bound<PyAny>) -> PyResult<()> {
        // Create new dict and copy items
        let new_dict = pyo3::types::PyDict::new(py);
        if let Ok(dict) = value.cast::<pyo3::types::PyDict>() {
            for (k, v) in dict.iter() {
                new_dict.set_item(k, v)?;
            }
        }
        self.warnings = new_dict.into();
        Ok(())
    }

    /// Set inline hints dict (replaces entire dict).
    #[setter]
    fn set_inline_hints(&mut self, py: Python, value: &Bound<PyAny>) -> PyResult<()> {
        // Create new dict and copy items
        let new_dict = pyo3::types::PyDict::new(py);
        if let Ok(dict) = value.cast::<pyo3::types::PyDict>() {
            for (k, v) in dict.iter() {
                new_dict.set_item(k, v)?;
            }
        }
        self.inline_hints = new_dict.into();
        Ok(())
    }

    /// Set pure functions set (replaces entire set).
    #[setter]
    fn set_pure_functions(&mut self, py: Python, value: &Bound<PyAny>) -> PyResult<()> {
        // Create new set and copy items
        let new_set = pyo3::types::PySet::empty(py)?;
        if let Ok(set) = value.cast::<pyo3::types::PySet>() {
            for item in set.iter() {
                new_set.add(item)?;
            }
        } else if let Ok(iter) = value.try_iter() {
            // Support any iterable (including Python sets converted from {"a", "b"})
            for item in iter {
                new_set.add(item?)?;
            }
        }
        self.pure_functions = new_set.into();
        Ok(())
    }

    /// Get pragmas as Python list.
    #[getter]
    fn pragmas(&self, py: Python) -> PyResult<Py<PyAny>> {
        let list = pyo3::types::PyList::empty(py);
        for pragma in &self.pragmas {
            list.append(pragma.clone_ref(py))?;
        }
        Ok(list.into())
    }
}

impl PragmaContext {
    /// Apply pragma effects to context via dispatch.
    fn apply_pragma(&mut self, py: Python, pragma: Py<Pragma>) -> PyResult<()> {
        let pragma_ref = pragma.borrow(py);

        match pragma_ref.pragma_type {
            PragmaType::Optimize => self.apply_optimize(py, &pragma_ref)?,
            PragmaType::Warning => self.apply_warning(py, &pragma_ref)?,
            PragmaType::Inline => self.apply_inline(py, &pragma_ref)?,
            PragmaType::Pure => self.apply_pure(py, &pragma_ref)?,
            PragmaType::Cache => self.apply_cache(py, &pragma_ref)?,
            PragmaType::Debug => self.apply_debug(py, &pragma_ref)?,
            PragmaType::Tco => self.apply_tco(py, &pragma_ref)?,
            PragmaType::Jit => self.apply_jit(py, &pragma_ref)?,
            PragmaType::NdMode => self.apply_nd_mode(py, &pragma_ref)?,
            PragmaType::NdWorkers => self.apply_nd_workers(py, &pragma_ref)?,
            PragmaType::NdMemoize => self.apply_nd_memoize(py, &pragma_ref)?,
            PragmaType::NdBatchSize => self.apply_nd_batch_size(py, &pragma_ref)?,
            PragmaType::Unknown => {}
        }

        Ok(())
    }

    /// Apply optimization level pragma.
    fn apply_optimize(&mut self, py: Python, pragma: &Pragma) -> PyResult<()> {
        let value = pragma.value.bind(py);

        // Try to parse as int
        if let Ok(level) = value.extract::<i32>() {
            if (0..=3).contains(&level) {
                self.optimize_level = level;
                return Ok(());
            } else {
                let exc_module = py.import("catnip.exc")?;
                let pragma_error = exc_module.getattr("CatnipPragmaError")?;
                return Err(PyErr::from_value(pragma_error.call1((format!(
                    "Optimization level must be 0-3, got {}",
                    level
                ),))?));
            }
        }

        // Try to parse as string
        if let Ok(level_str) = value.extract::<String>() {
            let level_str_lower = level_str.to_lowercase();
            let level = match level_str_lower.as_str() {
                "none" | "off" => 0,
                "basic" | "low" => 1,
                "medium" | "default" => 2,
                "high" | "full" | "aggressive" => 3,
                _ => {
                    let exc_module = py.import("catnip.exc")?;
                    let pragma_error = exc_module.getattr("CatnipPragmaError")?;
                    return Err(PyErr::from_value(
                        pragma_error
                            .call1((format!("Unknown optimization level: {}", level_str),))?,
                    ));
                }
            };
            self.optimize_level = level;
            return Ok(());
        }

        let exc_module = py.import("catnip.exc")?;
        let pragma_error = exc_module.getattr("CatnipPragmaError")?;
        Err(PyErr::from_value(
            pragma_error.call1(("Optimization level must be int or str",))?,
        ))
    }

    /// Apply warning control pragma.
    fn apply_warning(&mut self, py: Python, pragma: &Pragma) -> PyResult<()> {
        let value = pragma.value.bind(py);
        let action = value.extract::<String>()?.to_lowercase();

        let options = pragma.options.bind(py).cast::<PyDict>()?;
        let warning_name = options
            .get_item("name")?
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_else(|| "all".to_string());

        let warnings_dict = self.warnings.bind(py);
        match action.as_str() {
            "on" | "yes" => warnings_dict.set_item(warning_name, true)?,
            "off" | "no" => warnings_dict.set_item(warning_name, false)?,
            _ => {
                let exc_module = py.import("catnip.exc")?;
                let pragma_error = exc_module.getattr("CatnipPragmaError")?;
                return Err(PyErr::from_value(
                    pragma_error.call1((format!("Unknown warning action: {}", action),))?,
                ));
            }
        };

        Ok(())
    }

    /// Apply inline hint pragma.
    fn apply_inline(&mut self, py: Python, pragma: &Pragma) -> PyResult<()> {
        let value = pragma.value.bind(py);
        let hint = value.extract::<String>()?.to_lowercase();

        let options = pragma.options.bind(py).cast::<PyDict>()?;
        let func_name = options
            .get_item("function")?
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_else(|| "__next__".to_string());

        let hints_dict = self.inline_hints.bind(py);
        match hint.as_str() {
            "always" | "never" | "auto" => {
                hints_dict.set_item(func_name, hint)?;
                Ok(())
            }
            _ => {
                let exc_module = py.import("catnip.exc")?;
                let pragma_error = exc_module.getattr("CatnipPragmaError")?;
                Err(PyErr::from_value(
                    pragma_error.call1((format!("Unknown inline hint: {}", hint),))?,
                ))
            }
        }
    }

    /// Mark function as pure (no side effects).
    fn apply_pure(&mut self, py: Python, pragma: &Pragma) -> PyResult<()> {
        let value = pragma.value.bind(py);
        let func_name = value.extract::<String>()?;

        let options = pragma.options.bind(py).cast::<PyDict>()?;
        let enable = options
            .get_item("enable")?
            .and_then(|v| v.extract::<bool>().ok())
            .unwrap_or(true);

        let pure_set = self.pure_functions.bind(py);
        if enable {
            pure_set.add(func_name)?;
        } else {
            pure_set.discard(func_name)?;
        }

        Ok(())
    }

    /// Control caching.
    fn apply_cache(&mut self, py: Python, pragma: &Pragma) -> PyResult<()> {
        let value = pragma.value.bind(py);

        if let Ok(bool_val) = value.extract::<bool>() {
            self.cache_enabled = bool_val;
            return Ok(());
        }

        if let Ok(action) = value.extract::<String>() {
            let action_lower = action.to_lowercase();
            match action_lower.as_str() {
                "on" | "yes" | "true" => self.cache_enabled = true,
                "off" | "no" | "false" => self.cache_enabled = false,
                _ => {}
            }
        }

        Ok(())
    }

    /// Control debug mode.
    fn apply_debug(&mut self, py: Python, pragma: &Pragma) -> PyResult<()> {
        let value = pragma.value.bind(py);

        if let Ok(bool_val) = value.extract::<bool>() {
            self.debug_mode = bool_val;
            return Ok(());
        }

        if let Ok(action) = value.extract::<String>() {
            let action_lower = action.to_lowercase();
            match action_lower.as_str() {
                "on" | "yes" | "true" => self.debug_mode = true,
                "off" | "no" | "false" => self.debug_mode = false,
                _ => {}
            }
        }

        Ok(())
    }

    /// Control tail-call optimization.
    fn apply_tco(&mut self, py: Python, pragma: &Pragma) -> PyResult<()> {
        let value = pragma.value.bind(py);

        if let Ok(bool_val) = value.extract::<bool>() {
            self.tco_enabled = bool_val;
            return Ok(());
        }

        if let Ok(action) = value.extract::<String>() {
            let action_lower = action.to_lowercase();
            match action_lower.as_str() {
                "on" | "yes" | "true" => self.tco_enabled = true,
                "off" | "no" | "false" => self.tco_enabled = false,
                _ => {}
            }
        }

        Ok(())
    }

    /// Control JIT compilation.
    fn apply_jit(&mut self, py: Python, pragma: &Pragma) -> PyResult<()> {
        let value = pragma.value.bind(py);

        if let Ok(bool_val) = value.extract::<bool>() {
            self.jit_enabled = bool_val;
            self.jit_all = false;
            return Ok(());
        }

        if let Ok(action) = value.extract::<String>() {
            let action_lower = action.to_lowercase();
            match action_lower.as_str() {
                "all" => {
                    self.jit_enabled = true;
                    self.jit_all = true;
                }
                "on" | "yes" | "true" => {
                    self.jit_enabled = true;
                    self.jit_all = false;
                }
                "off" | "no" | "false" => {
                    self.jit_enabled = false;
                    self.jit_all = false;
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Control ND-recursion execution mode.
    fn apply_nd_mode(&mut self, py: Python, pragma: &Pragma) -> PyResult<()> {
        let value = pragma.value.bind(py);
        let mode = value.extract::<String>()?.to_lowercase();

        match mode.as_str() {
            "sequential" => self.nd_mode = "sequential".to_string(),
            "thread" => self.nd_mode = "thread".to_string(),
            "process" => self.nd_mode = "process".to_string(),
            _ => {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Unknown ND mode: {}. Use 'sequential', 'thread', or 'process'",
                    mode
                )))
            }
        }

        Ok(())
    }

    /// Control ND-recursion worker count.
    fn apply_nd_workers(&mut self, py: Python, pragma: &Pragma) -> PyResult<()> {
        let value = pragma.value.bind(py);

        // Try int extraction first, then fallback to string parsing
        let workers = if let Ok(i) = value.extract::<i32>() {
            i
        } else if let Ok(s) = value.extract::<String>() {
            // Parse string to int
            s.trim().parse::<i32>().map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "nd_workers must be integer or parseable string, got '{}': {}",
                    s, e
                ))
            })?
        } else {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "nd_workers must be int or string",
            ));
        };

        if workers < 0 {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "ND workers must be non-negative, got {}",
                workers
            )));
        }

        self.nd_workers = workers;
        Ok(())
    }

    /// Control ND-recursion memoization.
    fn apply_nd_memoize(&mut self, py: Python, pragma: &Pragma) -> PyResult<()> {
        let value = pragma.value.bind(py);

        if let Ok(bool_val) = value.extract::<bool>() {
            self.nd_memoize = bool_val;
            return Ok(());
        }

        if let Ok(action) = value.extract::<String>() {
            let action_lower = action.to_lowercase();
            match action_lower.as_str() {
                "on" | "yes" | "true" => self.nd_memoize = true,
                "off" | "no" | "false" => self.nd_memoize = false,
                _ => {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "ND memoize must be on/off, got {}",
                        action
                    )))
                }
            }
        }

        Ok(())
    }

    /// Control ND-recursion batch size for parallel execution.
    fn apply_nd_batch_size(&mut self, py: Python, pragma: &Pragma) -> PyResult<()> {
        let value = pragma.value.bind(py);

        // Try int extraction first, then fallback to string parsing
        let batch_size = if let Ok(i) = value.extract::<i32>() {
            i
        } else if let Ok(s) = value.extract::<String>() {
            // Parse string to int
            s.trim().parse::<i32>().map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "nd_batch_size must be integer or parseable string, got '{}': {}",
                    s, e
                ))
            })?
        } else {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "nd_batch_size must be int or string",
            ));
        };

        if batch_size < 0 {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "ND batch size must be non-negative, got {}",
                batch_size
            )));
        }

        self.nd_batch_size = batch_size;
        Ok(())
    }
}

pub fn register_module(parent_module: &Bound<'_, PyModule>) -> PyResult<()> {
    parent_module.add_class::<PragmaType>()?;
    parent_module.add_class::<Pragma>()?;
    parent_module.add_class::<PragmaContext>()?;
    Ok(())
}
