// FILE: catnip_rs/src/core/registry/nd.rs
//! ND operations: nd_recursion, nd_map, nd_empty_topos
//!
//! Implements Catnip's non-deterministic operations for concurrent execution.
//! These operations delegate to Python NDScheduler and NDTopos for actual execution.

use super::Registry;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

impl Registry {
    /// Return the empty topos singleton @[]
    ///
    /// The empty topos is the identity element for ND operations.
    pub(crate) fn op_nd_empty_topos(
        &self,
        py: Python<'_>,
        _args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        // Import NDTopos and return singleton
        let nd_module = py.import("catnip.nd")?;
        let nd_topos_class = nd_module.getattr("NDTopos")?;
        let instance = nd_topos_class.call_method0("instance")?;
        Ok(instance.unbind())
    }

    /// Execute ND-recursion: @@ operator
    ///
    /// Forms:
    /// - @@(seed, lambda): Combinator form - execute recursion with seed
    /// - @@ lambda: Declaration form - return wrapped ND-recursive function
    /// - data.[@@ lambda]: Broadcast form - apply to each element
    ///
    /// Args:
    ///     data_or_seed: First argument (data or seed, unevaluated)
    ///     lambda_node: Lambda expression (unevaluated)
    pub(crate) fn op_nd_recursion(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        if args.len() < 2 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "nd_recursion requires 2 arguments: data_or_seed, lambda_node",
            ));
        }

        let data_or_seed_node = args.get_item(0)?;
        let lambda_node = args.get_item(1)?;

        // Evaluate lambda to get the function
        let nd_lambda = if !lambda_node.is_none() {
            Some(self.exec_stmt_impl(py, lambda_node.unbind())?)
        } else {
            None
        };

        // Broadcast form: data.[@@ lambda]
        if !data_or_seed_node.is_none() {
            // Evaluate data/seed
            let data = self.exec_stmt_impl(py, data_or_seed_node.unbind())?;

            if nd_lambda.is_none() {
                // Declaration form: @@ lambda (data_or_seed is actually the lambda)
                return Ok(data);
            }

            // Execute ND-recursion on data
            return self.execute_nd_recursion(py, &data, &nd_lambda.unwrap());
        }

        // Declaration form: @@ lambda (seed is None)
        if let Some(lambda) = nd_lambda {
            return Ok(lambda);
        }

        // Return empty topos
        let nd_module = py.import("catnip.nd")?;
        let nd_topos_class = nd_module.getattr("NDTopos")?;
        let instance = nd_topos_class.call_method0("instance")?;
        Ok(instance.unbind())
    }

    /// Validate ND lambda/function arity.
    /// Checks `.params` (AST Function/Lambda) or `.vm_code.nargs` (VMFunction).
    /// Skips silently for Python builtins that expose neither.
    fn validate_nd_arity(
        py: Python<'_>,
        func: &Py<PyAny>,
        expected: usize,
        op_name: &str,
    ) -> PyResult<()> {
        let func_bound = func.bind(py);

        // AST mode: Function/Lambda expose .params (PyList)
        let arity = if let Ok(params) = func_bound.getattr("params") {
            params.len().ok()
        }
        // VM mode: VMFunction expose .vm_code.nargs
        else if let Ok(vm_code) = func_bound.getattr("vm_code") {
            vm_code
                .getattr("nargs")
                .ok()
                .and_then(|a| a.extract::<usize>().ok())
        } else {
            None
        };

        if let Some(n) = arity {
            if n != expected {
                let label = if expected == 2 {
                    "(value, recur)"
                } else {
                    "(value)"
                };
                return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                    "{} expects a function with {} parameters {}, got {}",
                    op_name, expected, label, n
                )));
            }
        }
        Ok(())
    }

    /// Execute ND-recursion starting from seed
    ///
    /// Uses NDScheduler for execution. Mode is determined by pragmas:
    /// - sequential: Single-threaded, no concurrency
    /// - thread: ThreadPoolExecutor, shared memory
    /// - process: ProcessPoolExecutor, true parallelism
    pub(crate) fn execute_nd_recursion(
        &self,
        py: Python<'_>,
        seed: &Py<PyAny>,
        nd_lambda: &Py<PyAny>,
    ) -> PyResult<Py<PyAny>> {
        Self::validate_nd_arity(py, nd_lambda, 2, "@@")?;

        let ctx = self.ctx.bind(py);

        // Get or create scheduler from context
        // NOTE: pragma values are already synced to Context in Catnip.execute()
        let nd_scheduler =
            if !ctx.hasattr("nd_scheduler")? || ctx.getattr("nd_scheduler")?.is_none() {
                // Create scheduler with mode from context (via pragmas)
                let n_workers = ctx
                    .getattr("nd_workers")
                    .unwrap_or_else(|_| 0_i32.into_pyobject(py).unwrap().into_any());
                let sched_mode = ctx
                    .getattr("nd_mode")
                    .unwrap_or_else(|_| "sequential".into_pyobject(py).unwrap().into_any());
                let memoize = ctx
                    .getattr("nd_memoize")
                    .unwrap_or_else(|_| false.into_pyobject(py).unwrap().to_owned().into_any());

                let nd_module = py.import("catnip.nd")?;
                let nd_scheduler_class = nd_module.getattr("NDScheduler")?;
                let scheduler = nd_scheduler_class.call1((n_workers, sched_mode, memoize))?;

                ctx.setattr("nd_scheduler", scheduler.clone())?;
                scheduler
            } else {
                ctx.getattr("nd_scheduler")?
            };

        // Wrap Catnip Function to make it callable if needed
        let callable_lambda = self.wrap_function_for_nd(py, nd_lambda)?;

        // Dispatch based on mode
        let sched_mode_obj = nd_scheduler.getattr("mode")?;
        let sched_mode: String = sched_mode_obj.extract()?;

        match sched_mode.as_str() {
            "thread" => {
                let result =
                    nd_scheduler.call_method1("execute_thread", (seed, &callable_lambda))?;
                Ok(result.unbind())
            }
            "process" => {
                let result =
                    nd_scheduler.call_method1("execute_process", (seed, &callable_lambda))?;
                Ok(result.unbind())
            }
            _ => {
                // Default: sequential
                let result = nd_scheduler.call_method1("execute_sync", (seed, &callable_lambda))?;
                Ok(result.unbind())
            }
        }
    }

    /// Wrap a Catnip Function to be callable for ND-recursion
    ///
    /// Returns a Python callable that binds parameters and executes the body.
    ///
    /// Note: This is a simplified wrapper. The actual wrapping happens in Python
    /// via _wrap_function_for_nd in registry_minimal.pyx when needed.
    /// For now, we delegate to Python by returning the function as-is.
    fn wrap_function_for_nd(&self, py: Python<'_>, func: &Py<PyAny>) -> PyResult<Py<PyAny>> {
        // Check if already callable
        let func_bound = func.bind(py);
        if func_bound.is_callable() {
            return Ok(func.clone_ref(py));
        }

        // For non-callable Function objects, return as-is and let Python's
        // NDScheduler handle the wrapping via its own internal mechanisms
        Ok(func.clone_ref(py))
    }

    /// Execute ND-map: @> operator
    ///
    /// Forms:
    /// - @>(data, f): Applicative form - apply f to data in ND context
    /// - @> f: Lift form - lift function to ND context
    /// - data.[@> f]: Broadcast form - apply f to each element
    ///
    /// Args:
    ///     data_or_func: First argument (data or function, unevaluated)
    ///     func_node: Function expression (unevaluated)
    pub(crate) fn op_nd_map(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        if args.len() < 2 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "nd_map requires 2 arguments: data_or_func, func_node",
            ));
        }

        let data_or_func_node = args.get_item(0)?;
        let func_node = args.get_item(1)?;

        // Evaluate func to get the function
        let func = if !func_node.is_none() {
            Some(self.exec_stmt_impl(py, func_node.unbind())?)
        } else {
            None
        };

        // Broadcast form: data.[@> f]
        if !data_or_func_node.is_none() {
            let data = self.exec_stmt_impl(py, data_or_func_node.unbind())?;

            if func.is_none() {
                // Lift form: @> f (data_or_func is actually the function)
                return Ok(data);
            }

            // Apply ND-map to data
            return self.execute_nd_map(py, &data, &func.unwrap());
        }

        // Lift form: @> f (data is None)
        if let Some(f) = func {
            return Ok(f);
        }

        // Return empty topos
        let nd_module = py.import("catnip.nd")?;
        let nd_topos_class = nd_module.getattr("NDTopos")?;
        let instance = nd_topos_class.call_method0("instance")?;
        Ok(instance.unbind())
    }

    /// Execute ND-map on data
    ///
    /// Sequential implementation. Concurrency handled by NDScheduler.
    pub(crate) fn execute_nd_map(
        &self,
        py: Python<'_>,
        data: &Py<PyAny>,
        func: &Py<PyAny>,
    ) -> PyResult<Py<PyAny>> {
        Self::validate_nd_arity(py, func, 1, "@>")?;

        let data_bound = data.bind(py);
        let func_bound = func.bind(py);

        // Check if data is iterable (but not string or bytes)
        let data_type = data_bound.get_type();
        let type_name = data_type.name()?;
        let type_str = type_name.to_str()?;

        if type_str == "str" || type_str == "bytes" {
            // For scalars/strings, just apply func
            if func_bound.is_callable() {
                let result = func_bound.call1((data,))?;
                return Ok(result.unbind());
            }
            return Ok(data.clone_ref(py));
        }

        // Try to iterate
        match data_bound.try_iter() {
            Ok(iter) => {
                // Map func over each element
                let mut results = Vec::new();
                for item_result in iter {
                    let item = item_result?;
                    if func_bound.is_callable() {
                        let mapped = func_bound.call1((item.clone(),))?;
                        results.push(mapped.unbind());
                    } else {
                        results.push(item.unbind());
                    }
                }

                // Preserve type
                if type_str == "tuple" {
                    let result_tuple = PyTuple::new(py, &results)?;
                    Ok(result_tuple.unbind().into())
                } else {
                    // Return as list
                    let py_list = pyo3::types::PyList::new(py, &results)?;
                    Ok(py_list.unbind().into())
                }
            }
            Err(_) => {
                // Not iterable, treat as scalar
                if func_bound.is_callable() {
                    let result = func_bound.call1((data,))?;
                    Ok(result.unbind())
                } else {
                    Ok(data.clone_ref(py))
                }
            }
        }
    }
}
