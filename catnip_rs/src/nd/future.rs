// FILE: catnip_rs/src/nd/future.rs
//! NDFuture - compute node in the ND-recursion DAG.
//!
//! Represents an ND computation with state, dependencies, and result.

use pyo3::prelude::*;
use pyo3::types::PyList;

/// NDFuture states.
#[pyclass(name = "NDState", module = "catnip._rs", eq, eq_int, from_py_object)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NDState {
    PENDING = 0,
    RUNNING = 1,
    COMPLETED = 2,
    FAILED = 3,
}

/// Future for ND computations.
///
/// Tracks state, result/exception, and dependency graph.
/// Uses Python lists for dependencies/dependents to avoid Sync issues.
#[pyclass(name = "NDFuture", module = "catnip._rs", unsendable)]
pub struct NDFuture {
    /// Current state
    #[pyo3(get)]
    pub state: NDState,
    /// Computation result (if COMPLETED)
    #[pyo3(get, set)]
    pub value: Option<Py<PyAny>>,
    /// Raised exception (if FAILED)
    #[pyo3(get)]
    pub exception: Option<Py<PyAny>>,
    /// Futures this computation depends on (Python list)
    dependencies_list: Py<PyList>,
    /// Futures that depend on this computation (Python list)
    dependents_list: Py<PyList>,
    /// Task to run (seed + lambda)
    #[pyo3(get, set)]
    pub task: Option<Py<PyAny>>,
    /// concurrent.futures.Future (parallel mode)
    #[pyo3(get, set)]
    pub py_future: Option<Py<PyAny>>,
}

#[pymethods]
impl NDFuture {
    #[new]
    #[pyo3(signature = (task=None, dependencies=None))]
    fn new(
        py: Python<'_>,
        task: Option<Py<PyAny>>,
        dependencies: Option<Bound<'_, PyList>>,
    ) -> PyResult<Self> {
        let deps_list = match dependencies {
            Some(d) => d.clone().unbind(),
            None => PyList::empty(py).unbind(),
        };
        Ok(Self {
            state: NDState::PENDING,
            value: None,
            exception: None,
            dependencies_list: deps_list,
            dependents_list: PyList::empty(py).unbind(),
            task,
            py_future: None,
        })
    }

    /// Check whether the future can run now.
    fn is_ready(&self, py: Python<'_>) -> PyResult<bool> {
        if self.state != NDState::PENDING {
            return Ok(false);
        }

        let deps = self.dependencies_list.bind(py);
        for dep in deps.iter() {
            let dep_state: NDState = dep.getattr("state")?.extract()?;
            if dep_state != NDState::COMPLETED {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Check whether the future is finished (COMPLETED or FAILED).
    fn is_done(&self) -> bool {
        self.state == NDState::COMPLETED || self.state == NDState::FAILED
    }

    /// Fetch the result (blocks if RUNNING in parallel mode).
    #[pyo3(signature = (timeout=-1.0))]
    fn result(&self, py: Python<'_>, timeout: f64) -> PyResult<Py<PyAny>> {
        if self.state == NDState::FAILED {
            if let Some(ref exc) = self.exception {
                return Err(PyErr::from_value(exc.bind(py).clone()));
            }
            return Err(pyo3::exceptions::PyRuntimeError::new_err("NDFuture failed"));
        }

        if self.state == NDState::COMPLETED {
            return Ok(self
                .value
                .as_ref()
                .map(|v| v.clone_ref(py))
                .unwrap_or_else(|| py.None()));
        }

        // Parallel mode: wait on the Python future
        if let Some(ref py_future) = self.py_future {
            let result = if timeout >= 0.0 {
                py_future.bind(py).call_method1("result", (timeout,))?
            } else {
                py_future.bind(py).call_method0("result")?
            };
            return Ok(result.unbind());
        }

        Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
            "NDFuture not completed (state={:?})",
            self.state
        )))
    }

    /// Set the result and mark as COMPLETED.
    fn set_result(&mut self, value: Py<PyAny>) {
        self.state = NDState::COMPLETED;
        self.value = Some(value);
        self.exception = None;
    }

    /// Set the exception and mark as FAILED.
    fn set_exception(&mut self, exc: Py<PyAny>) {
        self.state = NDState::FAILED;
        self.value = None;
        self.exception = Some(exc);
    }

    /// Mark the future as RUNNING.
    fn set_running(&mut self) {
        self.state = NDState::RUNNING;
    }

    /// Add a dependency.
    fn add_dependency(self_: Bound<'_, Self>, dep: Py<PyAny>) -> PyResult<()> {
        let py = self_.py();
        let self_ref = self_.borrow();
        let deps = self_ref.dependencies_list.bind(py);
        // Check if not already present
        if !deps.contains(&dep)? {
            deps.append(&dep)?;
            // Add self to dep's dependents
            let self_obj: Py<PyAny> = self_.clone().into_any().unbind();
            let dep_dependents: Bound<'_, PyList> = dep.getattr(py, "dependents")?.extract(py)?;
            dep_dependents.append(self_obj)?;
        }
        Ok(())
    }

    /// Get dependencies list.
    #[getter]
    fn dependencies(&self, py: Python<'_>) -> Py<PyAny> {
        self.dependencies_list.clone_ref(py).into_any()
    }

    /// Get dependents list.
    #[getter]
    fn dependents(&self, py: Python<'_>) -> Py<PyAny> {
        self.dependents_list.clone_ref(py).into_any()
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        match self.state {
            NDState::COMPLETED => {
                let val_str = self
                    .value
                    .as_ref()
                    .map(|v| {
                        v.bind(py)
                            .repr()
                            .map(|r| r.to_string())
                            .unwrap_or_else(|_| "?".to_string())
                    })
                    .unwrap_or_else(|| "None".to_string());
                format!("NDFuture(COMPLETED, value={})", val_str)
            }
            NDState::FAILED => {
                let exc_str = self
                    .exception
                    .as_ref()
                    .map(|e| {
                        e.bind(py)
                            .repr()
                            .map(|r| r.to_string())
                            .unwrap_or_else(|_| "?".to_string())
                    })
                    .unwrap_or_else(|| "None".to_string());
                format!("NDFuture(FAILED, exc={})", exc_str)
            }
            _ => format!("NDFuture({:?})", self.state),
        }
    }
}
