// FILE: catnip_rs/src/nd/scheduler.rs
//! NDScheduler - execution manager for ND recursion.
//!
//! Supports 3 execution modes:
//! - sequential: Single-threaded, no concurrency
//! - thread: ThreadPoolExecutor, shared memory, GIL-limited parallelism
//! - process: ProcessPoolExecutor, true parallelism, separate memory spaces

use super::recur::NDRecur;
use crate::constants::*;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::cell::Cell;
use std::collections::HashMap;

/// Scheduler for ND recursion.
///
/// Manages execution in sequential, threaded, or process-based modes.
#[pyclass(name = "NDScheduler", module = "catnip._rs", unsendable)]
pub struct NDScheduler {
    /// Worker count for the pools
    #[pyo3(get)]
    pub n_workers: usize,
    /// Process executor (None = not initialized)
    executor: Option<Py<PyAny>>,
    /// Thread executor (None = not initialized)
    thread_executor: Option<Py<PyAny>>,
    /// Execution mode: 'sequential', 'thread', or 'process'
    #[pyo3(get)]
    pub mode: String,
    /// Memoization cache
    memoize_cache: HashMap<i64, Py<PyAny>>,
    /// Memoization enabled flag
    #[pyo3(get)]
    pub memoize_enabled: bool,
    /// Processes available flag
    processes_available: bool,
    /// Recursion depth counter (prevents stack overflow)
    recursion_depth: Cell<usize>,
}

#[pymethods]
impl NDScheduler {
    #[new]
    #[pyo3(signature = (n_workers=0, mode="sequential", memoize_enabled=false))]
    fn new(py: Python<'_>, n_workers: usize, mode: &str, memoize_enabled: bool) -> PyResult<Self> {
        let actual_workers = if n_workers > 0 {
            n_workers
        } else {
            // Get CPU count
            let os = py.import("os")?;
            os.call_method0("cpu_count")?.extract::<Option<usize>>()?.unwrap_or(4)
        };

        Ok(Self {
            n_workers: actual_workers,
            executor: None,
            thread_executor: None,
            mode: mode.to_string(),
            memoize_cache: HashMap::new(),
            memoize_enabled,
            processes_available: true,
            recursion_depth: Cell::new(0),
        })
    }

    /// Synchronous (sequential) execution.
    #[pyo3(signature = (seed, nd_lambda))]
    fn execute_sync(self_: Bound<'_, Self>, seed: Py<PyAny>, nd_lambda: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let py = self_.py();

        // Check memoization cache
        {
            let self_ref = self_.borrow();
            if self_ref.memoize_enabled {
                let cache_key = self_ref.make_cache_key(py, &seed)?;
                if let Some(cached) = self_ref.memoize_cache.get(&cache_key) {
                    return Ok(cached.clone_ref(py));
                }
            }
        }

        // Create recur handle for nested calls (pass self_ as Py<PyAny>)
        let self_obj: Py<PyAny> = self_.clone().into_any().unbind();
        let recur = Py::new(
            py,
            NDRecur::create(self_obj, nd_lambda.clone_ref(py), None, "sequential"),
        )?;

        // Call the lambda with seed and recur
        let result = nd_lambda.bind(py).call1((seed.bind(py), recur))?;

        // Store in cache
        {
            let mut self_ref = self_.borrow_mut();
            if self_ref.memoize_enabled {
                let cache_key = self_ref.make_cache_key(py, &seed)?;
                self_ref.memoize_cache.insert(cache_key, result.clone().unbind());
            }
        }

        Ok(result.unbind())
    }

    /// Thread-based execution (with ThreadPoolExecutor).
    #[pyo3(signature = (seed, nd_lambda))]
    fn execute_thread(self_: Bound<'_, Self>, seed: Py<PyAny>, nd_lambda: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let py = self_.py();

        // Check memoization cache
        {
            let self_ref = self_.borrow();
            if self_ref.memoize_enabled {
                let cache_key = self_ref.make_cache_key(py, &seed)?;
                if let Some(cached) = self_ref.memoize_cache.get(&cache_key) {
                    return Ok(cached.clone_ref(py));
                }
            }
        }

        // Create the thread executor on demand
        {
            let mut self_ref = self_.borrow_mut();
            if self_ref.thread_executor.is_none() {
                let cf = py.import("concurrent.futures")?;
                let executor = cf.getattr("ThreadPoolExecutor")?.call1((self_ref.n_workers,))?;
                self_ref.thread_executor = Some(executor.unbind());
            }
        }

        // Create recur handle
        let self_obj: Py<PyAny> = self_.clone().into_any().unbind();
        let recur = Py::new(py, NDRecur::create(self_obj, nd_lambda.clone_ref(py), None, "thread"))?;

        // Call the lambda with seed and recur
        let result = nd_lambda.bind(py).call1((seed.bind(py), recur))?;

        // Store in cache
        {
            let mut self_ref = self_.borrow_mut();
            if self_ref.memoize_enabled {
                let cache_key = self_ref.make_cache_key(py, &seed)?;
                self_ref.memoize_cache.insert(cache_key, result.clone().unbind());
            }
        }

        Ok(result.unbind())
    }

    /// Process-based execution (with ProcessPoolExecutor).
    #[pyo3(signature = (seed, nd_lambda))]
    fn execute_process(self_: Bound<'_, Self>, seed: Py<PyAny>, nd_lambda: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let py = self_.py();

        // Check if processes are available
        {
            let self_ref = self_.borrow();
            if !self_ref.processes_available {
                return Self::execute_sync(self_, seed, nd_lambda);
            }
        }

        // Create the executor on demand
        {
            let mut self_ref = self_.borrow_mut();
            if self_ref.executor.is_none() && self_ref.processes_available {
                match create_process_executor(py, self_ref.n_workers) {
                    Ok(executor) => self_ref.executor = Some(executor),
                    Err(_) => {
                        self_ref.processes_available = false;
                        drop(self_ref);
                        return Self::execute_sync(self_, seed, nd_lambda);
                    }
                }
            }
        }

        // Get the worker function from the nd module
        let nd_module = py.import(PY_MOD_ND)?;
        let worker_fn = nd_module.getattr("_worker_execute_simple")?;

        // Submit to executor
        let py_future = {
            let self_ref = self_.borrow();
            let executor = self_ref.executor.as_ref().unwrap();
            executor
                .bind(py)
                .call_method1("submit", (worker_fn, &seed, &nd_lambda))?
        };

        // Wait for result
        py_future.call_method0("result")?.extract().map_err(Into::into)
    }

    /// Submit a recursive call.
    #[pyo3(signature = (value, nd_lambda, _parent_future=None))]
    fn submit_recursive(
        self_: Bound<'_, Self>,
        value: Py<PyAny>,
        nd_lambda: Py<PyAny>,
        _parent_future: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        // Depth guard: prevent stack overflow from infinite ND recursion
        let depth = self_.borrow().recursion_depth.get();
        if depth >= ND_MAX_RECURSION_DEPTH {
            return Err(pyo3::exceptions::PyRecursionError::new_err(
                "maximum ND recursion depth exceeded",
            ));
        }
        self_.borrow().recursion_depth.set(depth + 1);

        let mode = self_.borrow().mode.clone();
        let result = match mode.as_str() {
            "thread" => Self::execute_thread(self_.clone(), value, nd_lambda),
            "process" => Self::execute_sync(self_.clone(), value, nd_lambda), // Recursive calls run inline
            _ => Self::execute_sync(self_.clone(), value, nd_lambda),
        };

        // Always decrement, even on error
        self_
            .borrow()
            .recursion_depth
            .set(self_.borrow().recursion_depth.get() - 1);

        result
    }

    /// Change the execution mode.
    fn set_mode(&mut self, py: Python<'_>, mode: &str) -> PyResult<()> {
        if !["sequential", "thread", "process"].contains(&mode) {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Invalid mode: {}. Use 'sequential', 'thread', or 'process'",
                mode
            )));
        }

        self.mode = mode.to_string();

        // Create executors on demand
        if mode == "thread" && self.thread_executor.is_none() {
            let cf = py.import("concurrent.futures")?;
            let executor = cf.getattr("ThreadPoolExecutor")?.call1((self.n_workers,))?;
            self.thread_executor = Some(executor.unbind());
        } else if mode == "process" && self.executor.is_none() && self.processes_available {
            match create_process_executor(py, self.n_workers) {
                Ok(executor) => self.executor = Some(executor),
                Err(_) => self.processes_available = false,
            }
        }

        Ok(())
    }

    /// Clear the memoization cache.
    fn clear_cache(&mut self) {
        self.memoize_cache.clear();
    }

    fn __repr__(&self) -> String {
        if self.memoize_enabled {
            format!(
                "NDScheduler(workers={}, mode='{}', cache={})",
                self.n_workers,
                self.mode,
                self.memoize_cache.len()
            )
        } else {
            format!("NDScheduler(workers={}, mode='{}')", self.n_workers, self.mode)
        }
    }
}

impl NDScheduler {
    /// Create a hashable cache key from seed.
    fn make_cache_key(&self, py: Python<'_>, seed: &Py<PyAny>) -> PyResult<i64> {
        let builtins = py.import("builtins")?;

        // Try to hash directly
        if let Ok(h) = builtins.call_method1("hash", (seed.bind(py),)) {
            if let Ok(val) = h.extract::<i64>() {
                return Ok(val);
            }
        }

        // Fallback: use str representation
        let s = seed.bind(py).str()?;
        builtins.call_method1("hash", (&s,))?.extract()
    }
}

/// Create ProcessPoolExecutor with worker initialization.
fn create_process_executor(py: Python<'_>, n_workers: usize) -> PyResult<Py<PyAny>> {
    let mp = py.import("multiprocessing")?;
    let mp_context = mp.call_method1("get_context", ("spawn",))?;

    let cf = py.import("concurrent.futures")?;

    // Get worker init function
    let nd_module = py.import(PY_MOD_ND)?;
    let worker_init = nd_module.getattr("_worker_init")?;

    // Create kwargs dict
    let kwargs = PyDict::new(py);
    kwargs.set_item("max_workers", n_workers)?;
    kwargs.set_item("mp_context", mp_context)?;
    kwargs.set_item("initializer", worker_init)?;

    let executor = cf.getattr("ProcessPoolExecutor")?.call((), Some(&kwargs))?;
    Ok(executor.unbind())
}

impl Drop for NDScheduler {
    fn drop(&mut self) {
        // Shutdown executors
        Python::attach(|py| {
            if let Some(ref executor) = self.executor {
                let _ = executor.bind(py).call_method1("shutdown", (true,));
            }
            if let Some(ref executor) = self.thread_executor {
                let _ = executor.bind(py).call_method1("shutdown", (true,));
            }
        });
    }
}
