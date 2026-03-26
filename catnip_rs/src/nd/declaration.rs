// FILE: catnip_rs/src/nd/declaration.rs
//! NDDeclaration - wrapper for ND declaration form.
//!
//! Created by `countdown = ~~(n, recur) => { body }`, wraps the lambda
//! so calling `countdown(seed)` executes ND-recursion with the seed.

use crate::constants::*;
use crate::vm::frame::VMFunction;
use pyo3::prelude::*;

/// Wrapper for ND declaration form: `f = ~~(n, recur) => { body }`.
///
/// Stores the ND lambda and the execution context. When called with `f(seed)`,
/// creates an NDScheduler and dispatches the recursion.
#[pyclass(name = "NDDeclaration", module = "catnip._rs")]
pub struct NDDeclaration {
    nd_lambda: Py<PyAny>,
    ctx: Py<PyAny>,
}

impl NDDeclaration {
    pub fn new(nd_lambda: Py<PyAny>, ctx: Py<PyAny>) -> Self {
        Self { nd_lambda, ctx }
    }
}

#[pymethods]
impl NDDeclaration {
    fn __call__(&self, py: Python<'_>, seed: Py<PyAny>) -> PyResult<Py<PyAny>> {
        // Fast path: VMFunction lambda → use NDVmDecl (frame stack, ~200x faster)
        let lambda_bound = self.nd_lambda.bind(py);
        if lambda_bound.cast::<VMFunction>().is_ok() {
            let decl = pyo3::Py::new(py, super::NDVmDecl::with_memoize(self.nd_lambda.clone_ref(py), true))?;
            return decl.bind(py).call1((seed,)).map(|r| r.unbind());
        }

        let ctx = self.ctx.bind(py);

        // Get or create scheduler from context (same logic as Registry::execute_nd_recursion)
        let nd_scheduler = if !ctx.hasattr("nd_scheduler")? || ctx.getattr("nd_scheduler")?.is_none() {
            let n_workers = ctx
                .getattr("nd_workers")
                .unwrap_or_else(|_| 0_i32.into_pyobject(py).unwrap().into_any());
            let sched_mode = ctx
                .getattr("nd_mode")
                .unwrap_or_else(|_| "sequential".into_pyobject(py).unwrap().into_any());
            let memoize = ctx
                .getattr("nd_memoize")
                .unwrap_or_else(|_| false.into_pyobject(py).unwrap().to_owned().into_any());

            let nd_module = py.import(PY_MOD_ND)?;
            let nd_scheduler_class = nd_module.getattr("NDScheduler")?;
            let scheduler = nd_scheduler_class.call1((n_workers, sched_mode, memoize))?;
            ctx.setattr("nd_scheduler", scheduler.clone())?;
            scheduler
        } else {
            ctx.getattr("nd_scheduler")?
        };

        let sched_mode: String = nd_scheduler.getattr("mode")?.extract()?;
        let method = match sched_mode.as_str() {
            "thread" => "execute_thread",
            "process" => "execute_process",
            _ => "execute_sync",
        };
        let result = nd_scheduler.call_method1(method, (seed, &self.nd_lambda))?;
        Ok(result.unbind())
    }

    fn __repr__(&self) -> String {
        "NDDeclaration(~~)".to_string()
    }
}
