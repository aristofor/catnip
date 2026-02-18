// FILE: catnip_rs/src/pipeline.rs
//! Pipeline orchestration - parse → semantic → execute
//!
//! Centralizes the Catnip pipeline logic in Rust to reduce Python overhead.
//! Parsing levels (0-3) are handled here; Python only handles display.

use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Process input at different parsing levels
///
/// Parsing levels:
/// - 0: Parse tree only (tree-sitter AST)
/// - 1: IR only (after transformer, before semantic)
/// - 2: Executable IR (after semantic analysis)
/// - 3: Execute and return result (default)
#[pyfunction]
#[pyo3(signature = (catnip, text, level=3))]
pub fn process_input(
    py: Python<'_>,
    catnip: &Bound<'_, PyAny>,
    text: &str,
    level: u8,
) -> PyResult<Py<PyAny>> {
    match level {
        0 => process_level_0(py, catnip, text),
        1 => process_level_1(py, catnip, text),
        2 => process_level_2(py, catnip, text),
        3 => process_level_3(py, catnip, text),
        _ => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "Invalid parsing level: {}. Must be 0-3",
            level
        ))),
    }
}

/// Level 0: Parse tree only (no transformation)
fn process_level_0(py: Python<'_>, catnip: &Bound<'_, PyAny>, text: &str) -> PyResult<Py<PyAny>> {
    // Create Parser without transformer
    let parser_class = py.import("catnip.parser")?.getattr("Parser")?;
    let parser = parser_class.call1((py.None(),))?;
    catnip.setattr("parser", parser)?;

    // Parse and return tree
    let result = catnip.call_method1("parse", (text,))?;
    Ok(result.unbind())
}

/// Level 1: IR only (after transformer, before semantic)
fn process_level_1(py: Python<'_>, catnip: &Bound<'_, PyAny>, text: &str) -> PyResult<Py<PyAny>> {
    // Parse with semantic=False
    let kwargs = PyDict::new(py);
    kwargs.set_item("semantic", false)?;
    let result = catnip.call_method("parse", (text,), Some(&kwargs))?;
    Ok(result.unbind())
}

/// Level 2: Executable IR (after semantic analysis)
fn process_level_2(py: Python<'_>, catnip: &Bound<'_, PyAny>, text: &str) -> PyResult<Py<PyAny>> {
    // Parse with semantic=True
    let kwargs = PyDict::new(py);
    kwargs.set_item("semantic", true)?;
    let result = catnip.call_method("parse", (text,), Some(&kwargs))?;
    Ok(result.unbind())
}

/// Level 3: Execute and return result
fn process_level_3(_py: Python<'_>, catnip: &Bound<'_, PyAny>, text: &str) -> PyResult<Py<PyAny>> {
    // Parse
    catnip.call_method1("parse", (text,))?;

    // Execute and return result
    let result = catnip.call_method0("execute")?;
    Ok(result.unbind())
}

/// Pipeline module initialization
pub fn init_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(process_input, m)?)?;
    Ok(())
}
