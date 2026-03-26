// FILE: catnip_rs/src/vm/compiler.rs
//! PyO3 wrapper for the bytecode compiler.
//!
//! Delegates to `UnifiedCompiler` for all compilation.

use super::frame::PyCodeObject;
use super::unified_compiler::FunctionCompileMeta;
use super::value::Value;
use pyo3::prelude::*;
use pyo3::types::PyList;

/// PyO3 wrapper for the Compiler (delegates to UnifiedCompiler).
#[pyclass(name = "Compiler", module = "catnip._rs")]
pub struct PyCompiler {
    inner: super::unified_compiler::UnifiedCompiler,
}

#[pymethods]
impl PyCompiler {
    #[new]
    fn new() -> Self {
        Self {
            inner: super::unified_compiler::UnifiedCompiler::new(),
        }
    }

    /// Compile IR to bytecode and return PyCodeObject.
    #[pyo3(signature = (node, name=None))]
    fn compile(&mut self, py: Python<'_>, node: &Bound<'_, PyAny>, name: Option<&str>) -> PyResult<PyCodeObject> {
        let mut code = self.inner.compile_py(py, node)?;
        if let Some(n) = name {
            code.name = n.to_string();
        }
        Ok(PyCodeObject::new(code))
    }

    /// Compile a function body with parameters.
    #[pyo3(signature = (params, body, name, defaults=None))]
    fn compile_function(
        &mut self,
        py: Python<'_>,
        params: Vec<String>,
        body: &Bound<'_, PyAny>,
        name: &str,
        defaults: Option<&Bound<'_, PyList>>,
    ) -> PyResult<PyCodeObject> {
        let defaults_vec = match defaults {
            Some(list) => {
                let mut v = Vec::new();
                for item in list.iter() {
                    v.push(Value::from_pyobject(py, &item)?);
                }
                v
            }
            None => Vec::new(),
        };
        let code = self.inner.compile_function_py(
            py,
            body,
            FunctionCompileMeta {
                params,
                name,
                defaults: defaults_vec,
                vararg_idx: -1,
                parent_nesting_depth: 0,
            },
        )?;
        Ok(PyCodeObject::new(code))
    }
}
