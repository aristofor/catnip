// FILE: catnip_rs/src/core/registry/functions.rs
//! Function operations: lambda factory and call with TCO
//!
//! Creates RustLambda instances with full TCO trampoline support.
//! Handles function calls with tail-call optimization detection.

use super::Registry;
use crate::core::function::RustLambda;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

impl Registry {
    /// Execute a function call with Tail-Call Optimization detection.
    ///
    /// Arguments come in unevaluated (since call is a control flow op).
    /// We must evaluate them before calling or creating a TailCall.
    ///
    /// If the call is in tail position (_node.tail = True) and TCO is enabled,
    /// returns a TailCall signal instead of executing. This allows the
    /// trampoline loop in Function/Lambda to optimize tail recursion.
    pub(crate) fn op_call(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
        node: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        if args.is_empty() {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "call requires at least 1 argument: func_expr",
            ));
        }

        // First arg is function expression (unevaluated)
        let func_expr = args.get_item(0)?;

        // Evaluate the function expression
        let func = self.exec_stmt_impl(py, func_expr.unbind())?;

        // Evaluate all positional arguments (args[1:])
        let mut eval_args: Vec<Py<PyAny>> = Vec::with_capacity(args.len() - 1);
        for i in 1..args.len() {
            let arg = args.get_item(i)?;
            let evaluated = self.exec_stmt_impl(py, arg.unbind())?;
            eval_args.push(evaluated);
        }

        // Extract and evaluate kwargs from the Op (passed via node)
        let eval_kwargs = if let Some(ref node_obj) = node {
            // The node is the Op itself, which has a kwargs attribute
            node_obj.bind(py).getattr("kwargs").ok().map(|kwargs_dict| {
                let result_dict = PyDict::new(py);
                if let Ok(items) = kwargs_dict.call_method0("items") {
                    for item in items.try_iter().unwrap() {
                        if let Ok(item) = item {
                            if let (Ok(key), Ok(value)) = (item.get_item(0), item.get_item(1)) {
                                // Evaluate the kwarg value
                                if let Ok(eval_val) = self.exec_stmt_impl(py, value.unbind()) {
                                    let _ = result_dict.set_item(key, eval_val);
                                }
                            }
                        }
                    }
                }
                result_dict
            })
        } else {
            None
        };

        // Check if function needs context passed
        let pass_context = func
            .bind(py)
            .getattr("pass_context")
            .map(|attr| attr.is_truthy().unwrap_or(false))
            .unwrap_or(false);

        if pass_context {
            // Prepend context to args
            let ctx = self.ctx.clone_ref(py);
            eval_args.insert(0, ctx);
        }

        // Check if this is a tail call
        let is_tail_call = if let Some(ref node_obj) = node {
            node_obj
                .bind(py)
                .getattr("tail")
                .map(|attr| attr.is_truthy().unwrap_or(false))
                .unwrap_or(false)
        } else {
            false
        };

        // Check if TCO is enabled
        let tco_enabled = self
            .ctx
            .bind(py)
            .getattr("tco_enabled")
            .map(|attr| attr.is_truthy().unwrap_or(true))
            .unwrap_or(true);

        if is_tail_call && tco_enabled {
            // Return a TailCall signal instead of executing
            let nodes = py.import("catnip.nodes")?;
            let tail_call_class = nodes.getattr("TailCall")?;

            let args_tuple = PyTuple::new(py, &eval_args)?;
            let kwargs_dict = eval_kwargs.unwrap_or_else(|| PyDict::new(py));

            let tail_call =
                tail_call_class.call1((func, args_tuple.into_any(), kwargs_dict.into_any()))?;
            return Ok(tail_call.unbind());
        }

        // Inject super proxy for bound method calls on struct instances with parent methods.
        // We store the proxy on ctx._pending_super so that execute_trampoline can inject
        // it into the method's scope AFTER push_scope_with_capture (which creates a new
        // scope chain from the closure, making direct scope injection useless here).
        let mut had_super = false;
        let func_bound = func.bind(py);
        if let Ok(bm) = func_bound.cast::<crate::core::BoundCatnipMethod>() {
            let bm_ref = bm.borrow();
            if let Some(proxy_py) = self.build_super_proxy(py, &bm_ref)? {
                let ctx = self.ctx.bind(py);
                ctx.setattr("_pending_super", proxy_py)?;
                had_super = true;
            }
        }
        // Execute the function call
        let args_tuple = PyTuple::new(py, &eval_args)?;
        let kwargs_opt = eval_kwargs.as_ref().map(|d| d as &Bound<'_, PyDict>);
        let mut result = func
            .bind(py)
            .call(args_tuple, kwargs_opt)
            .map(|r| r.unbind())?;

        // Clean up pending super if it wasn't consumed (e.g. non-RustLambda call)
        if had_super {
            let ctx = self.ctx.bind(py);
            let _ = ctx.setattr("_pending_super", py.None());
        }

        // Post-constructor: call __catnip_init__ after struct instantiation
        result = self.maybe_call_post_init(py, result)?;

        Ok(result)
    }
    /// Build a SuperProxy for a bound method call, MRO-based.
    fn build_super_proxy(
        &self,
        py: Python<'_>,
        bm: &crate::core::BoundCatnipMethod,
    ) -> PyResult<Option<Py<PyAny>>> {
        // Get the real type's MRO from the instance
        let real_type_info =
            if let Ok(proxy_ref) = bm.instance.bind(py).cast::<crate::vm::CatnipStructProxy>() {
                let st_ref = proxy_ref
                    .borrow()
                    .struct_type
                    .as_ref()
                    .map(|s| s.clone_ref(py));
                st_ref.and_then(|st_py| {
                    let st = st_py.bind(py).borrow();
                    if st.parent_names.is_empty() {
                        None
                    } else {
                        Some((st.mro.clone(), st.parent_names.clone()))
                    }
                })
            } else {
                None
            };

        let Some((mro, _parent_names)) = real_type_info else {
            return Ok(None);
        };

        // Find start position in MRO
        let start_pos = if let Some(ref source) = bm.super_source_type {
            mro.iter()
                .position(|n| n == source)
                .map(|p| p + 1)
                .unwrap_or(1)
        } else {
            1 // Skip self (pos 0)
        };

        if start_pos >= mro.len() {
            return Ok(None);
        }

        // Collect methods from MRO[start_pos:], first-wins, with provenance
        let ctx = self.ctx.bind(py);
        let globals = ctx.getattr("globals")?;
        let mut methods: std::collections::HashMap<String, Py<PyAny>> =
            std::collections::HashMap::new();
        let mut method_sources: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        for mro_type_name in &mro[start_pos..] {
            let cls = globals.call_method1("get", (mro_type_name.as_str(), py.None()))?;
            if cls.is_none() {
                continue;
            }
            if let Ok(st_bound) = cls.cast::<crate::vm::CatnipStructType>() {
                let st = st_bound.borrow();
                for (k, v) in &st.methods {
                    if !methods.contains_key(k) {
                        methods.insert(k.clone(), v.clone_ref(py));
                        method_sources.insert(k.clone(), mro_type_name.clone());
                    }
                }
            }
        }

        if methods.is_empty() {
            return Ok(None);
        }

        let proxy = crate::vm::SuperProxy {
            methods,
            instance: bm.instance.clone_ref(py),
            method_sources,
            native_instance_idx: None,
        };
        Ok(Some(Py::new(py, proxy)?.into_any()))
    }

    /// Call init post-constructor on a freshly created struct instance if defined.
    /// Return value of init is discarded, the instance is returned.
    fn maybe_call_post_init(&self, py: Python<'_>, result: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let result_bound = result.bind(py);

        let Ok(proxy) = result_bound.cast::<crate::vm::CatnipStructProxy>() else {
            return Ok(result);
        };
        let st_ref = proxy.borrow().struct_type.as_ref().map(|s| s.clone_ref(py));
        let Some(st_py) = st_ref else {
            return Ok(result);
        };
        let st = st_py.bind(py).borrow();
        let Some(ref init_fn) = st.init_fn else {
            return Ok(result);
        };

        let init_fn = init_fn.clone_ref(py);
        let has_parents = !st.parent_names.is_empty();
        let mro = st.mro.clone();
        drop(st); // release borrow before calling

        if has_parents && mro.len() > 1 {
            // Build MRO-based super proxy: skip self (pos 0), start at 1
            let ctx = self.ctx.bind(py);
            let globals = ctx.getattr("globals")?;
            let mut methods: std::collections::HashMap<String, Py<PyAny>> =
                std::collections::HashMap::new();
            let mut method_sources: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            for mro_type_name in &mro[1..] {
                let cls = globals.call_method1("get", (mro_type_name.as_str(), py.None()))?;
                if cls.is_none() {
                    continue;
                }
                if let Ok(st_bound) = cls.cast::<crate::vm::CatnipStructType>() {
                    let st = st_bound.borrow();
                    for (k, v) in &st.methods {
                        if !methods.contains_key(k) {
                            methods.insert(k.clone(), v.clone_ref(py));
                            method_sources.insert(k.clone(), mro_type_name.clone());
                        }
                    }
                }
            }
            if !methods.is_empty() {
                let proxy = crate::vm::SuperProxy {
                    methods,
                    instance: result.clone_ref(py),
                    method_sources,
                    native_instance_idx: None,
                };
                ctx.setattr("_pending_super", Py::new(py, proxy)?)?;
            }
        }

        let args = PyTuple::new(py, &[result.clone_ref(py)])?;
        let _ = init_fn.bind(py).call1(args)?;

        if has_parents {
            let ctx = self.ctx.bind(py);
            let _ = ctx.setattr("_pending_super", py.None());
        }

        Ok(result)
    }

    /// Create a lambda: _lambda(params, body)
    ///
    /// Creates a RustLambda with full trampoline TCO support.
    /// The lambda captures the current closure scope at creation time.
    ///
    /// Takes a Bound reference to self to pass it to RustLambda constructor.
    pub(crate) fn op_lambda(
        bound_self: &Bound<'_, Self>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        let py = bound_self.py();

        if args.len() < 2 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "lambda requires 2 arguments: params, body",
            ));
        }

        let params = args.get_item(0)?.unbind();
        let body = args.get_item(1)?.unbind();

        // Create RustLambda directly (100% Rust implementation)
        let registry_obj = bound_self.clone().into_any().unbind();
        let lambda = RustLambda::new(py, body, registry_obj, Some(params))?;

        // Convert to PyObject
        Ok(Py::new(py, lambda)?.into_any())
    }
}
