// FILE: catnip_rs/src/jit/detector.rs
//! PyO3 wrapper for HotLoopDetector.
//!
//! Core logic lives in `catnip_core::jit::detector`.

use crate::constants::JIT_THRESHOLD_DEFAULT;
use catnip_core::jit::detector::HotLoopDetector;
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Python-exposed hot loop detector (thin wrapper around catnip_core).
#[pyclass(name = "HotLoopDetector")]
#[pyo3(module = "catnip._rs")]
pub struct PyHotLoopDetector {
    pub inner: HotLoopDetector,
}

#[pymethods]
impl PyHotLoopDetector {
    #[new]
    #[pyo3(signature = (threshold=JIT_THRESHOLD_DEFAULT))]
    fn py_new(threshold: u32) -> Self {
        Self {
            inner: HotLoopDetector::new(threshold),
        }
    }

    /// Record a function call. Returns true if function just became hot.
    fn record_call(&mut self, func_id: &str) -> bool {
        self.inner.record_call_internal(func_id)
    }

    /// Mark a function as compiled (stop tracking it).
    fn mark_compiled(&mut self, func_id: &str) {
        self.inner.mark_compiled_internal(func_id);
    }

    /// Check if a function is currently hot.
    fn is_hot(&self, func_id: &str) -> bool {
        self.inner.is_hot_internal(func_id)
    }

    /// Check if a function has been compiled.
    fn is_compiled(&self, func_id: &str) -> bool {
        self.inner.is_compiled_internal(func_id)
    }

    /// Get profiling statistics.
    fn get_stats(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let stats = self.inner.stats();
        let dict = PyDict::new(py);
        dict.set_item("total_functions_tracked", stats.total_loops_tracked)?;
        dict.set_item("hot_functions", stats.hot_loops)?;
        dict.set_item("compiled_functions", stats.compiled_loops)?;
        Ok(dict.into())
    }

    /// Reset all profiling data.
    #[pyo3(name = "reset")]
    fn py_reset(&mut self) {
        self.inner.reset();
    }
}
