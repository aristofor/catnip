// FILE: catnip_libs/sys/rust/src/lib.rs
//! Catnip `sys` module: runtime introspection (argv, environ, version, platform, exit).
//!
//! Pure Rust -- no Python `sys`/`os` imports. All data comes from `std::env`.

use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Build and return the `sys` module.
///
/// If `argv` is provided, use it directly. Otherwise fall back to
/// `std::env::args()` (useful for the standalone binary).
pub fn build_module(py: Python<'_>, argv: Option<Vec<String>>, executable: Option<String>) -> PyResult<Py<PyModule>> {
    let m = PyModule::new(py, "sys")?;
    register_items(&m, argv, executable)?;
    Ok(m.unbind())
}

fn register_items(m: &Bound<'_, PyModule>, argv: Option<Vec<String>>, executable: Option<String>) -> PyResult<()> {
    let py = m.py();
    m.add("PROTOCOL", "rust")?;

    let argv = argv.unwrap_or_else(|| std::env::args().collect());
    m.add("argv", argv)?;

    let environ = PyDict::new(py);
    for (k, v) in std::env::vars() {
        environ.set_item(&k, &v)?;
    }
    m.add("environ", environ)?;

    let executable = executable.unwrap_or_else(|| {
        std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    m.add("executable", executable)?;

    m.add("version", env!("CARGO_PKG_VERSION"))?;
    m.add("platform", std::env::consts::OS)?;

    let cpu_count = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    m.add("cpu_count", cpu_count)?;

    // exit(code=0): Python lambda that raises SystemExit
    let exit_fn = py.eval(
        pyo3::ffi::c_str!("lambda code=0: (_ for _ in ()).throw(SystemExit(code))"),
        None,
        None,
    )?;
    m.add("exit", exit_fn)?;

    Ok(())
}

/// Standalone PyO3 module init for dynamic loading (OS defaults).
#[pymodule]
fn catnip_sys(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_items(m, None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::types::{PyDict, PyList};

    #[test]
    fn test_build_module_defaults() {
        Python::initialize();
        Python::attach(|py| {
            let module = build_module(py, None, None).unwrap();
            let m = module.bind(py);

            let argv = m.getattr("argv").unwrap();
            let list = argv.cast::<PyList>().unwrap();
            assert!(list.len() >= 1);

            let environ = m.getattr("environ").unwrap();
            let dict = environ.cast::<PyDict>().unwrap();
            assert!(dict.len() > 0);

            let executable: String = m.getattr("executable").unwrap().extract().unwrap();
            assert!(!executable.is_empty());

            let platform: String = m.getattr("platform").unwrap().extract().unwrap();
            assert!(!platform.is_empty());
            let version: String = m.getattr("version").unwrap().extract().unwrap();
            assert!(!version.is_empty());
        });
    }

    #[test]
    fn test_build_module_with_argv() {
        Python::initialize();
        Python::attach(|py| {
            let argv = vec!["script.cat".to_string(), "arg1".to_string()];
            let module = build_module(py, Some(argv), None).unwrap();
            let m = module.bind(py);

            let got: Vec<String> = m.getattr("argv").unwrap().extract().unwrap();
            assert_eq!(got, vec!["script.cat", "arg1"]);
        });
    }

    #[test]
    fn test_build_module_with_executable() {
        Python::initialize();
        Python::attach(|py| {
            let exe = "/usr/local/bin/catnip".to_string();
            let module = build_module(py, None, Some(exe.clone())).unwrap();
            let m = module.bind(py);

            let got: String = m.getattr("executable").unwrap().extract().unwrap();
            assert_eq!(got, exe);
        });
    }
}
