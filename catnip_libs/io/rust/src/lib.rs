// FILE: catnip_libs/io/rust/src/lib.rs
//! Catnip `io` module: print, write, input with Python-compatible signatures.

use pyo3::prelude::*;
use pyo3::types::PyTuple;

/// Get a writable file object, defaulting to sys.stdout.
fn get_output<'py>(py: Python<'py>, file: Option<&Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    match file {
        Some(f) => Ok(f.clone()),
        None => {
            let sys = py.import("sys")?;
            Ok(sys.getattr("stdout")?.into_any())
        }
    }
}

fn get_stderr(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    let sys = py.import("sys")?;
    Ok(sys.getattr("stderr")?.into_any())
}

/// Convert values to strings joined by sep, write to file with end appended.
fn do_print(
    py: Python<'_>,
    values: &Bound<'_, PyTuple>,
    sep: &str,
    end: &str,
    file: Option<&Bound<'_, PyAny>>,
    flush: bool,
) -> PyResult<()> {
    let out = get_output(py, file)?;

    let parts: Vec<String> = values
        .iter()
        .map(|v| {
            v.str()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|_| format!("{:?}", v))
        })
        .collect();

    let msg = format!("{}{}", parts.join(sep), end);
    out.call_method1("write", (msg,))?;

    if flush {
        out.call_method0("flush")?;
    }

    Ok(())
}

#[pyfunction]
#[pyo3(signature = (*values, sep=" ", end="\n", file=None, flush=false))]
fn print(
    py: Python<'_>,
    values: &Bound<'_, PyTuple>,
    sep: &str,
    end: &str,
    file: Option<&Bound<'_, PyAny>>,
    flush: bool,
) -> PyResult<()> {
    do_print(py, values, sep, end, file, flush)
}

#[pyfunction]
#[pyo3(signature = (*values, file=None, flush=true))]
fn write(py: Python<'_>, values: &Bound<'_, PyTuple>, file: Option<&Bound<'_, PyAny>>, flush: bool) -> PyResult<()> {
    do_print(py, values, "", "", file, flush)
}

#[pyfunction]
#[pyo3(signature = (*values, file=None, flush=true))]
fn writeln(py: Python<'_>, values: &Bound<'_, PyTuple>, file: Option<&Bound<'_, PyAny>>, flush: bool) -> PyResult<()> {
    do_print(py, values, "", "\n", file, flush)
}

#[pyfunction]
#[pyo3(signature = (*values, sep=" ", end="\n", flush=true))]
fn eprint(py: Python<'_>, values: &Bound<'_, PyTuple>, sep: &str, end: &str, flush: bool) -> PyResult<()> {
    let stderr = get_stderr(py)?;
    do_print(py, values, sep, end, Some(&stderr), flush)
}

#[pyfunction]
#[pyo3(signature = (file, mode="r", buffering=-1, encoding=None, errors=None, newline=None, closefd=true, opener=None))]
fn open<'py>(
    py: Python<'py>,
    file: &Bound<'py, PyAny>,
    mode: &str,
    buffering: i32,
    encoding: Option<&str>,
    errors: Option<&str>,
    newline: Option<&str>,
    closefd: bool,
    opener: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let builtins = py.import("builtins")?;
    let open_fn = builtins.getattr("open")?;
    let kwargs = pyo3::types::PyDict::new(py);
    kwargs.set_item("mode", mode)?;
    kwargs.set_item("buffering", buffering)?;
    kwargs.set_item("closefd", closefd)?;
    if let Some(v) = encoding {
        kwargs.set_item("encoding", v)?;
    }
    if let Some(v) = errors {
        kwargs.set_item("errors", v)?;
    }
    if let Some(v) = newline {
        kwargs.set_item("newline", v)?;
    }
    if let Some(v) = opener {
        kwargs.set_item("opener", v)?;
    }
    open_fn.call((file,), Some(&kwargs))
}

#[pyfunction]
#[pyo3(signature = (prompt=""))]
fn input(py: Python<'_>, prompt: &str) -> PyResult<String> {
    let sys = py.import("sys")?;

    if !prompt.is_empty() {
        let stdout = sys.getattr("stdout")?;
        stdout.call_method1("write", (prompt,))?;
        stdout.call_method0("flush")?;
    }

    let stdin = sys.getattr("stdin")?;
    let line: String = stdin.call_method0("readline")?.extract()?;

    if line.is_empty() {
        return Err(pyo3::exceptions::PyEOFError::new_err("end of input"));
    }

    Ok(line.trim_end_matches('\n').to_string())
}

/// Build and return the `io` module as a Python module object.
pub fn build_module(py: Python<'_>) -> PyResult<Py<PyModule>> {
    let m = PyModule::new(py, "io")?;
    register_items(&m)?;
    Ok(m.unbind())
}

fn register_items(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("PROTOCOL", "rust")?;
    m.add("VERSION", "0.1.0")?;
    m.add_function(wrap_pyfunction!(print, m)?)?;
    m.add_function(wrap_pyfunction!(write, m)?)?;
    m.add_function(wrap_pyfunction!(writeln, m)?)?;
    m.add_function(wrap_pyfunction!(eprint, m)?)?;
    m.add_function(wrap_pyfunction!(input, m)?)?;
    m.add_function(wrap_pyfunction!(open, m)?)?;
    Ok(())
}

/// Standalone PyO3 module init for dynamic loading.
#[pymodule]
fn catnip_io(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_items(m)
}
