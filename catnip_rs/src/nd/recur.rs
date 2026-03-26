// FILE: catnip_rs/src/nd/recur.rs
//! NDRecur - callable for ND recursion.
//!
//! Passed as the second argument to ND lambdas to enable recursive calls.

use pyo3::prelude::*;

/// Callable for ND recursion.
///
/// Holds a reference to the scheduler and lambda, enabling recursive calls
/// from within ND lambda bodies.
#[pyclass(name = "NDRecur", module = "catnip._rs")]
pub struct NDRecur {
    /// Scheduler managing execution
    scheduler: Py<PyAny>,
    /// ND function to call recursively
    lambda_func: Py<PyAny>,
    /// Parent future (unused in current implementation)
    parent_future: Option<Py<PyAny>>,
    /// Execution mode: 'sequential', 'threads', or 'processes'
    #[pyo3(get)]
    pub mode: String,
}

#[pymethods]
impl NDRecur {
    #[new]
    #[pyo3(signature = (scheduler, lambda_func, parent_future=None, mode="sequential"))]
    fn new(scheduler: Py<PyAny>, lambda_func: Py<PyAny>, parent_future: Option<Py<PyAny>>, mode: &str) -> Self {
        Self {
            scheduler,
            lambda_func,
            parent_future,
            mode: mode.to_string(),
        }
    }

    /// Recursive call.
    ///
    /// Dispatches to the scheduler based on current mode.
    fn __call__(&self, py: Python<'_>, value: Py<PyAny>) -> PyResult<Py<PyAny>> {
        self.scheduler
            .bind(py)
            .call_method1("submit_recursive", (value, &self.lambda_func, &self.parent_future))?
            .extract()
            .map_err(Into::into)
    }

    fn __repr__(&self) -> String {
        format!("NDRecur(mode='{}')", self.mode)
    }
}

impl NDRecur {
    /// Create from Rust code.
    pub fn create(scheduler: Py<PyAny>, lambda_func: Py<PyAny>, parent_future: Option<Py<PyAny>>, mode: &str) -> Self {
        Self {
            scheduler,
            lambda_func,
            parent_future,
            mode: mode.to_string(),
        }
    }
}
