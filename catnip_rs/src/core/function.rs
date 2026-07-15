// FILE: catnip_rs/src/core/function.rs
//! Rust implementation of Function and Lambda nodes.
//!
//! This module provides high-performance implementations of function
//! and lambda execution, including the trampoline loop for tail-call
//! optimization (TCO).

use crate::constants::*;
use pyo3::PyTraverseError;
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyDict, PyFloat, PyInt, PyList, PySet, PyString, PyTuple};

/// Global registry for pickle reconstruction in worker processes.
static GLOBAL_REGISTRY: std::sync::OnceLock<Py<PyAny>> = std::sync::OnceLock::new();

/// Set the global registry for pickle reconstruction.
#[pyfunction]
pub fn set_global_registry(py: Python<'_>, registry: Py<PyAny>) {
    let _ = GLOBAL_REGISTRY.set(registry.clone_ref(py));
}

/// Get the global registry for pickle reconstruction.
#[pyfunction]
pub fn get_global_registry(py: Python<'_>) -> Option<Py<PyAny>> {
    GLOBAL_REGISTRY.get().map(|r| r.clone_ref(py))
}

thread_local! {
    /// Set by internal dispatch sites (op_call, struct init, trampoline
    /// hand-off) just before invoking a Catnip callable through the Python
    /// protocol; consumed at the top of `__call__`. Absent = the call comes
    /// from arbitrary Python re-entering the runtime (a context HOF): struct
    /// args are then passed as private shallow copies -- the execution-
    /// boundary rule (BROADCAST_SPEC decision 4), mirroring the VM whose
    /// child VM snapshots the parent registry on every Python re-entry.
    static INTERNAL_CALL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Mark the next Catnip-callable `.call()` as internal dispatch (shared
/// instance semantics). Must be immediately followed by the call: the flag is
/// consumed by the callee's `__call__`.
pub(crate) fn mark_internal_call() {
    INTERNAL_CALL.with(|c| c.set(true));
}

fn take_internal_call() -> bool {
    INTERNAL_CALL.with(|c| c.replace(false))
}

/// Apply the execution-boundary rule to a Python-originated call: struct
/// proxy arguments (positional and keyword) are replaced by detached shallow
/// copies. Returns the originals untouched when nothing needs copying.
fn boundary_args<'py>(
    py: Python<'py>,
    args: &Bound<'py, PyTuple>,
    kwargs: Option<&Bound<'py, PyDict>>,
) -> PyResult<(Bound<'py, PyTuple>, Option<Bound<'py, PyDict>>)> {
    use crate::vm::structs::CatnipStructProxy;
    let mut changed = false;
    let mut new_args: Vec<Py<PyAny>> = Vec::with_capacity(args.len());
    for item in args.iter() {
        if let Ok(proxy) = item.cast::<CatnipStructProxy>() {
            new_args.push(proxy.borrow().detached_copy(py)?);
            changed = true;
        } else {
            new_args.push(item.unbind());
        }
    }
    let new_kwargs = match kwargs {
        Some(kw) => {
            let out = PyDict::new(py);
            for (k, v) in kw.iter() {
                if let Ok(proxy) = v.cast::<CatnipStructProxy>() {
                    out.set_item(k, proxy.borrow().detached_copy(py)?)?;
                    changed = true;
                } else {
                    out.set_item(k, v)?;
                }
            }
            Some(out)
        }
        None => None,
    };
    if changed {
        Ok((PyTuple::new(py, &new_args)?, new_kwargs))
    } else {
        Ok((args.clone(), kwargs.cloned()))
    }
}

/// Shared `__call__` body for Function and Lambda: execution-boundary copy,
/// JIT fast path, then trampoline execution against the captured closure.
#[allow(clippy::too_many_arguments)]
fn call_catnip_callable(
    py: Python<'_>,
    jit_compiled_func: Option<&Py<PyAny>>,
    registry: &Py<PyAny>,
    params: &Py<PyAny>,
    body: &Py<PyAny>,
    closure_scope: &Py<PyAny>,
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<Py<PyAny>> {
    // Execution-boundary rule: a call not marked by internal dispatch
    // comes from arbitrary Python -- struct args become private copies.
    let internal = take_internal_call();
    let (args_owned, kwargs_owned) = if internal {
        (args.clone(), kwargs.cloned())
    } else {
        boundary_args(py, args, kwargs)?
    };
    let args = &args_owned;
    let kwargs = kwargs_owned.as_ref();
    // JIT fast path
    if let Some(jit_func) = jit_compiled_func {
        let ctx = registry.getattr(py, "ctx")?;
        let jit_enabled: bool = ctx.getattr(py, "jit_enabled")?.extract(py)?;
        if jit_enabled && (kwargs.is_none() || kwargs.is_some_and(|k| k.is_empty())) {
            if let Some(result) = try_jit_call(py, jit_func, args, params)? {
                return Ok(result);
            }
        }
    }

    // Interpreted execution with trampoline
    let ctx = registry.getattr(py, "ctx")?;
    ctx.call_method1(py, "push_scope_with_capture", (closure_scope,))?;

    // Tail calls may swap to another function's closure mid-loop: the
    // final pop must sync against the closure that actually finished.
    let mut cur_closure = closure_scope.clone_ref(py);
    let result = execute_trampoline(py, body, registry, params, args, kwargs, &mut cur_closure);

    ctx.call_method1(py, "pop_scope_with_sync", (&cur_closure,))?;
    result
}

/// Rust implementation of Function.
#[pyclass(module = "catnip._rs")]
pub struct Function {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub params: Py<PyAny>,
    #[pyo3(get)]
    pub body: Py<PyAny>,
    #[pyo3(get)]
    pub registry: Py<PyAny>,
    pub closure_scope: Py<PyAny>,
    #[pyo3(get)]
    pub jit_func_id: String,
    pub jit_compiled_func: Option<Py<PyAny>>,
    /// Frame token of the defining scope (0 = not a block-local definition).
    /// Set by op_set_locals to drive letrec group patching.
    pub def_frame_token: u64,
}

#[pymethods]
impl Function {
    #[new]
    fn new(py: Python<'_>, name: String, params: Py<PyAny>, body: Py<PyAny>, registry: Py<PyAny>) -> PyResult<Self> {
        let params_list = if params.bind(py).is_instance_of::<PyTuple>() {
            let tuple = params.cast_bound::<PyTuple>(py)?;
            let list = PyList::new(py, tuple.iter())?;
            list.unbind().into()
        } else {
            params
        };

        let ctx = registry.getattr(py, "ctx")?;
        let closure_scope = ctx.call_method0(py, "capture_scope")?;
        let jit_func_id = format!("func_{}_{:x}", name, params_list.as_ptr() as usize);

        Ok(Self {
            name,
            params: params_list,
            body,
            registry,
            closure_scope,
            jit_func_id,
            jit_compiled_func: None,
            def_frame_token: 0,
        })
    }

    #[getter]
    fn get_closure_scope(&self, py: Python<'_>) -> Py<PyAny> {
        self.closure_scope.clone_ref(py)
    }

    // The closure may hold a self-reference (letrec): participate in GC
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.params)?;
        visit.call(&self.body)?;
        visit.call(&self.registry)?;
        visit.call(&self.closure_scope)?;
        if let Some(ref f) = self.jit_compiled_func {
            visit.call(f)?;
        }
        Ok(())
    }

    fn __clear__(&mut self) {
        Python::attach(|py| {
            // Break the lambda <-> closure dict cycle
            self.closure_scope = py.None();
        });
        self.jit_compiled_func = None;
    }

    fn __repr__(&self) -> PyResult<String> {
        Python::attach(|py| {
            let params = self.params.bind(py);
            let param_names: Vec<String> = if let Ok(list) = params.cast::<PyList>() {
                list.iter().filter_map(|p| get_first_elem_as_string(&p)).collect()
            } else {
                vec![]
            };
            Ok(format!("<Function {}({})>", self.name, param_names.join(", ")))
        })
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn __call__(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        call_catnip_callable(
            py,
            self.jit_compiled_func.as_ref(),
            &self.registry,
            &self.params,
            &self.body,
            &self.closure_scope,
            args,
            kwargs,
        )
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let reconstruct_fn = py.import(PY_MOD_RS)?.getattr("_reconstruct_function")?;
        let name_obj: Py<PyAny> = self.name.clone().into_pyobject(py)?.unbind().into();
        let jit_id_obj: Py<PyAny> = self.jit_func_id.clone().into_pyobject(py)?.unbind().into();

        let args = PyTuple::new(
            py,
            [name_obj, self.params.clone_ref(py), self.body.clone_ref(py), jit_id_obj],
        )?;

        let result = PyTuple::new(py, [reconstruct_fn.into_any().unbind(), args.into_any().unbind()])?;
        Ok(result.into_any().unbind())
    }
}

/// Rust implementation of Lambda.
#[pyclass(module = "catnip._rs")]
pub struct Lambda {
    #[pyo3(get)]
    pub body: Py<PyAny>,
    #[pyo3(get)]
    pub registry: Py<PyAny>,
    #[pyo3(get)]
    pub params: Py<PyAny>,
    pub closure_scope: Py<PyAny>,
    pub closure_values: Option<Py<PyAny>>,
    #[pyo3(get)]
    pub jit_func_id: String,
    pub jit_compiled_func: Option<Py<PyAny>>,
    /// Frame token of the defining scope (0 = not a block-local definition).
    /// Set by op_set_locals to drive letrec group patching.
    pub def_frame_token: u64,
}

#[pymethods]
impl Lambda {
    #[new]
    #[pyo3(signature = (body, registry, params=None))]
    pub fn new(py: Python<'_>, body: Py<PyAny>, registry: Py<PyAny>, params: Option<Py<PyAny>>) -> PyResult<Self> {
        let params_list = if let Some(p) = params {
            if p.bind(py).is_instance_of::<PyTuple>() {
                let tuple = p.cast_bound::<PyTuple>(py)?;
                let list = PyList::new(py, tuple.iter())?;
                list.unbind().into()
            } else {
                p
            }
        } else {
            PyList::empty(py).unbind().into()
        };

        let ctx = registry.getattr(py, "ctx")?;
        let closure_scope = ctx.call_method0(py, "capture_scope")?;
        let jit_func_id = format!("lambda_{:x}", body.as_ptr() as usize);
        let closure_values = capture_closure_values(py, &closure_scope)?;

        Ok(Self {
            body,
            registry,
            params: params_list,
            closure_scope,
            closure_values,
            jit_func_id,
            jit_compiled_func: None,
            def_frame_token: 0,
        })
    }

    #[getter]
    fn get_closure_scope(&self, py: Python<'_>) -> Py<PyAny> {
        self.closure_scope.clone_ref(py)
    }

    // The closure may hold a self-reference (letrec): participate in GC
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.params)?;
        visit.call(&self.body)?;
        visit.call(&self.registry)?;
        visit.call(&self.closure_scope)?;
        if let Some(ref v) = self.closure_values {
            visit.call(v)?;
        }
        if let Some(ref f) = self.jit_compiled_func {
            visit.call(f)?;
        }
        Ok(())
    }

    fn __clear__(&mut self) {
        Python::attach(|py| {
            // Break the lambda <-> closure dict cycle
            self.closure_scope = py.None();
        });
        self.closure_values = None;
        self.jit_compiled_func = None;
    }

    fn __repr__(&self) -> PyResult<String> {
        Python::attach(|py| {
            let params = self.params.bind(py);
            if let Ok(list) = params.cast::<PyList>() {
                if list.is_empty() {
                    return Ok("<Lambda>".to_string());
                }
                let param_names: Vec<String> = list.iter().filter_map(|p| get_first_elem_as_string(&p)).collect();
                Ok(format!("<Lambda ({})>", param_names.join(", ")))
            } else {
                Ok("<Lambda>".to_string())
            }
        })
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn __call__(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        call_catnip_callable(
            py,
            self.jit_compiled_func.as_ref(),
            &self.registry,
            &self.params,
            &self.body,
            &self.closure_scope,
            args,
            kwargs,
        )
    }

    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let this = slf.borrow();
        let reconstruct_fn = py.import(PY_MOD_RS)?.getattr("_reconstruct_lambda")?;
        let closure_vals = this
            .closure_values
            .as_ref()
            .map(|v| v.clone_ref(py))
            .unwrap_or_else(|| py.None());
        let jit_id_obj: Py<PyAny> = this.jit_func_id.clone().into_pyobject(py)?.unbind().into();

        let args = PyTuple::new(
            py,
            [
                this.body.clone_ref(py),
                this.params.clone_ref(py),
                jit_id_obj,
                closure_vals,
            ],
        )?;

        // Let-rec self-binding: names in the closure scope that point back at
        // this lambda. Omitted from closure_values (would cycle through fresh
        // reconstructions); re-injected by __setstate__ so a pickled
        // named-recursive lambda keeps resolving its own recursive call.
        // Mirrors the VM's VMFunction self-name restore.
        let self_names = PyList::empty(py);
        if let Ok(dict) = this.closure_scope.cast_bound::<PyDict>(py) {
            for (k, v) in dict.iter() {
                if v.is(slf.as_any()) {
                    self_names.append(k)?;
                }
            }
        }

        let result = PyTuple::new(
            py,
            [
                reconstruct_fn.into_any().unbind(),
                args.into_any().unbind(),
                self_names.into_any().unbind(),
            ],
        )?;
        Ok(result.into_any().unbind())
    }

    /// Restore the let-rec self-binding after unpickling. Serialization omits
    /// it (the closure would cycle); a pickled named-recursive lambda re-binds
    /// itself into its own closure scope here. No-op for non-recursive lambdas
    /// (empty state).
    fn __setstate__(slf: &Bound<'_, Self>, state: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let Ok(names) = state.cast::<PyList>() else {
            return Ok(());
        };
        let this = slf.borrow();
        if let Ok(dict) = this.closure_scope.cast_bound::<PyDict>(py) {
            for name in names.iter() {
                dict.set_item(name, slf.as_any())?;
            }
        }
        Ok(())
    }
}

// === Helper functions ===

/// Get first element of tuple/list as string.
fn get_first_elem_as_string(item: &Bound<'_, PyAny>) -> Option<String> {
    if let Ok(t) = item.cast::<PyTuple>() {
        t.get_item(0).ok().map(|i| i.to_string())
    } else if let Ok(l) = item.cast::<PyList>() {
        l.get_item(0).ok().map(|i| i.to_string())
    } else {
        Some(item.to_string())
    }
}

/// Try JIT compiled call, returns Some(result) if successful.
fn try_jit_call(
    py: Python<'_>,
    jit_func: &Py<PyAny>,
    args: &Bound<'_, PyTuple>,
    params: &Py<PyAny>,
) -> PyResult<Option<Py<PyAny>>> {
    let num_args = args.len();
    let params_len: usize = params.bind(py).len()?;

    match (params_len, num_args) {
        (1, 1) => Ok(Some(jit_func.call1(py, (args.get_item(0)?,))?)),
        (2, 1) => Ok(Some(jit_func.call1(py, (args.get_item(0)?, 0i64))?)),
        (2, 2) => Ok(Some(jit_func.call1(py, (args.get_item(0)?, args.get_item(1)?))?)),
        (3, 3) => Ok(Some(
            jit_func.call1(py, (args.get_item(0)?, args.get_item(1)?, args.get_item(2)?))?,
        )),
        _ => Ok(None),
    }
}

/// Execute body with trampoline loop for TCO.
///
/// `cur_closure` is in/out: on entry, the closure scope already pushed by
/// the caller; on exit, the closure scope of the function that actually
/// finished. A tail call to a different function swaps scopes mid-loop,
/// so the caller must pop-with-sync against the final value, not against
/// the entry function's closure.
fn execute_trampoline(
    py: Python<'_>,
    body: &Py<PyAny>,
    registry: &Py<PyAny>,
    params: &Py<PyAny>,
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
    cur_closure: &mut Py<PyAny>,
) -> PyResult<Py<PyAny>> {
    // Import classes
    let nodes = py.import(PY_MOD_NODES)?;
    let tail_call_class = nodes.getattr("TailCall")?;
    let return_value_class = nodes.getattr("ReturnValue")?;

    // Callable type checks (100% Rust now)
    let rs_module = py.import(PY_MOD_RS)?;
    let rust_lambda = rs_module.getattr("Lambda")?;
    let rust_function = rs_module.getattr("Function")?;

    // Trampoline state
    let mut cur_body = body.clone_ref(py);
    let mut cur_registry = registry.clone_ref(py);
    let mut cur_params = params.clone_ref(py);
    let mut cur_args: Py<PyTuple> = args.clone().unbind();
    let mut cur_kwargs: Option<Py<PyDict>> = kwargs.map(|k| k.clone().unbind());
    let mut is_first = true;

    loop {
        // Bind parameters
        let params_bound = cur_params.bind(py);
        if params_bound.len()? > 0 {
            bind_params(py, &cur_params, &cur_registry, &cur_args, &cur_kwargs)?;
            // TH2-B step 0b: enforce + coerce annotated params, mirroring the VM
            // prologue CheckType so AST mode has the same typed-param boundary.
            coerce_typed_params(py, &cur_params, &cur_registry)?;
        }

        // Consume pending super proxy (set by op_call for struct method calls).
        // Must happen after push_scope_with_capture + bind_params so that
        // "super" ends up in the method's own scope, not the caller's.
        if is_first {
            let ctx = cur_registry.getattr(py, "ctx")?;
            if let Ok(pending) = ctx.getattr(py, "_pending_super") {
                if !pending.is_none(py) {
                    let locals = ctx.getattr(py, "locals")?;
                    locals.call_method1(py, "_set", ("super", &pending))?;
                    ctx.setattr(py, "_pending_super", py.None())?;
                }
            }
        }

        // Execute body
        let result: Py<PyAny> = match cur_registry.call_method1(py, "exec_stmt", (&cur_body,)) {
            Ok(r) => r,
            Err(e) => {
                if e.is_instance(py, &return_value_class) {
                    return e.value(py).getattr("value")?.extract().map_err(Into::into);
                }
                return Err(e);
            }
        };

        // Check for TailCall
        if result.bind(py).is_instance(&tail_call_class)? {
            let target = result.getattr(py, "func")?;
            let new_args: Py<PyTuple> = result.getattr(py, "args")?.extract(py)?;
            let new_kwargs: Option<Py<PyDict>> = result
                .getattr(py, "kwargs")
                .ok()
                .and_then(|k| if k.is_none(py) { None } else { k.extract(py).ok() });

            // Check if Catnip callable (100% Rust now)
            let is_catnip =
                target.bind(py).is_instance(&rust_lambda)? || target.bind(py).is_instance(&rust_function)?;

            if !is_catnip {
                let kw_ref = new_kwargs.as_ref().map(|k| k.bind(py));
                return target.call(py, new_args.bind(py), kw_ref);
            }

            // Scope swap for mutual recursion. Pop WITH sync: the current
            // function may have written to its captured variables, and a
            // plain pop would drop those updates (a normal call returning
            // here would have synced them into its closure dict and the
            // parent scope). Same closure object implies same function,
            // hence same context: the checks below only run on real swaps.
            let new_closure = target.getattr(py, "closure_scope")?;
            if !new_closure.is(&*cur_closure) {
                // A target from another Catnip context cannot be hosted by
                // this trampoline: its scope stack lives on its own context.
                // Call it directly -- its __call__ pushes and pops there.
                let target_ctx = target.getattr(py, "registry")?.getattr(py, "ctx")?;
                let ctx = cur_registry.getattr(py, "ctx")?;
                if !target_ctx.is(&ctx) {
                    let kw_ref = new_kwargs.as_ref().map(|k| k.bind(py));
                    // Catnip-to-Catnip hand-off: keep shared-instance semantics.
                    mark_internal_call();
                    return target.call(py, new_args.bind(py), kw_ref);
                }
                ctx.call_method1(py, "pop_scope_with_sync", (&*cur_closure,))?;
                ctx.call_method1(py, "push_scope_with_capture", (&new_closure,))?;
                *cur_closure = new_closure;
            }

            // Update state
            cur_body = target.getattr(py, "body")?;
            cur_registry = target.getattr(py, "registry")?;
            cur_params = target.getattr(py, "params")?;
            cur_args = new_args;
            cur_kwargs = new_kwargs;
            is_first = false;
            continue;
        }

        return Ok(result);
    }
}

/// Bind parameters to scope.
fn bind_params(
    py: Python<'_>,
    params: &Py<PyAny>,
    registry: &Py<PyAny>,
    args: &Py<PyTuple>,
    kwargs: &Option<Py<PyDict>>,
) -> PyResult<()> {
    let ctx = registry.getattr(py, "ctx")?;
    let locals = ctx.getattr(py, "locals")?;
    let params_list = params.cast_bound::<PyList>(py)?;
    let args_bound = args.bind(py);

    let num_args = args_bound.len();
    let _num_params = params_list.len();

    // Check variadic
    let (var_name, num_regular) = check_variadic(py, params_list)?;

    // Bind positional
    for i in 0..num_args.min(num_regular) {
        let param = params_list.get_item(i)?;
        let name = extract_param_name(&param)?;
        let arg = args_bound.get_item(i)?;
        locals.call_method1(py, "_set_param", (&name, &arg))?;
    }

    // Variadic
    if let Some(ref vname) = var_name {
        let varargs: Vec<Py<PyAny>> = (num_regular..num_args)
            .map(|i| args_bound.get_item(i).map(|a| a.unbind()))
            .collect::<PyResult<_>>()?;
        let varlist = PyList::new(py, varargs)?;
        locals.call_method1(py, "_set_param", (vname, varlist))?;
    }

    // Kwargs
    if let Some(ref kw) = kwargs {
        for (key, value) in kw.bind(py).iter() {
            let name: String = key.extract()?;
            locals.call_method1(py, "_set_param", (&name, &value))?;
        }
    }

    // Defaults: bound on every trampoline iteration. A tail call can target
    // a different function than the one bound on entry (mutual recursion),
    // whose omitted parameters were never bound. No-op when all args are
    // passed (the common case: num_args == num_regular).
    for i in num_args..num_regular {
        let param = params_list.get_item(i)?;
        let name = extract_param_name(&param)?;

        if kwargs
            .as_ref()
            .map(|k| k.bind(py).contains(&name).unwrap_or(false))
            .unwrap_or(false)
        {
            continue;
        }

        let plen = get_param_len(&param);
        if plen > 1 {
            let default = get_param_default(&param)?;
            let val = if default.is_none() {
                py.None()
            } else {
                registry.call_method1(py, "exec_stmt", (&default,))?
            };
            locals.call_method1(py, "_set_param", (&name, val))?;
        } else {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
                "Missing required argument: '{}'",
                name
            )));
        }
    }

    Ok(())
}

/// Check for variadic parameter.
fn check_variadic(_py: Python<'_>, params: &Bound<'_, PyList>) -> PyResult<(Option<String>, usize)> {
    let n = params.len();
    if n == 0 {
        return Ok((None, 0));
    }

    let last = params.get_item(n - 1)?;
    let (first, len) = if let Ok(t) = last.cast::<PyTuple>() {
        (t.get_item(0).ok(), t.len())
    } else if let Ok(l) = last.cast::<PyList>() {
        (l.get_item(0).ok(), l.len())
    } else {
        return Ok((None, n));
    };

    if len == 2 {
        if let Some(f) = first {
            if f.extract::<String>().map(|s| s == "*").unwrap_or(false) {
                let vname = if let Ok(t) = last.cast::<PyTuple>() {
                    t.get_item(1)?.extract::<String>()?
                } else if let Ok(l) = last.cast::<PyList>() {
                    l.get_item(1)?.extract::<String>()?
                } else {
                    return Ok((None, n));
                };
                return Ok((Some(vname), n - 1));
            }
        }
    }
    Ok((None, n))
}

/// TH2-B step 0b: raw declared type annotation of a param (index 2 of its
/// tuple), or `None` when unannotated. The text is classified by
/// [`ParamCheck::from_annotation`] into a primitive, a nominal, or nothing.
fn extract_param_annotation(param: &Bound<'_, PyAny>) -> Option<String> {
    if get_param_len(param) < 3 {
        return None;
    }
    let ty = if let Ok(t) = param.cast::<PyTuple>() {
        t.get_item(2).ok()?
    } else if let Ok(l) = param.cast::<PyList>() {
        l.get_item(2).ok()?
    } else {
        return None;
    };
    ty.extract().ok()
}

/// TH2-B step 0b boundary check + numeric-tower coercion on a Python value,
/// mirroring the VM `boundary_coerce`. AST values are real Python objects, so
/// `bool` (a subtype of `int`) must be matched before `int`.
pub(crate) fn boundary_coerce_py(py: Python<'_>, value: Py<PyAny>, code: u8) -> PyResult<Py<PyAny>> {
    use catnip_core::vm::opcode::type_code;
    let v = value.bind(py);
    let is_bool = v.is_instance_of::<PyBool>();
    let mismatch = || {
        let tn = v
            .get_type()
            .name()
            .ok()
            .and_then(|n| n.to_str().ok().map(|s| s.to_string()))
            .unwrap_or_else(|| "value".to_string());
        PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
            "typed parameter expects '{}' but got '{}'",
            type_code::name(code),
            tn
        ))
    };
    match code {
        type_code::INT => {
            if is_bool {
                Ok(v.call_method0("__int__")?.unbind())
            } else if v.is_instance_of::<PyInt>() {
                Ok(value)
            } else {
                Err(mismatch())
            }
        }
        type_code::FLOAT => {
            if v.is_instance_of::<PyFloat>() {
                Ok(value)
            } else if is_bool || v.is_instance_of::<PyInt>() {
                // __float__ raises OverflowError for an int too large for f64;
                // surface it as a boundary TypeError so it matches the VM and
                // every other boundary failure (one exception class across
                // executors), instead of leaking Python's unmapped OverflowError.
                match v.call_method0("__float__") {
                    Ok(f) => Ok(f.unbind()),
                    Err(e) if e.is_instance_of::<pyo3::exceptions::PyOverflowError>(py) => {
                        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                            "int too large to convert to float".to_string(),
                        ))
                    }
                    Err(e) => Err(e),
                }
            } else {
                Err(mismatch())
            }
        }
        type_code::STR => {
            if v.is_instance_of::<PyString>() {
                Ok(value)
            } else {
                Err(mismatch())
            }
        }
        type_code::BOOL => {
            if is_bool {
                Ok(value)
            } else {
                Err(mismatch())
            }
        }
        type_code::NONE => {
            if v.is_none() {
                Ok(value)
            } else {
                Err(mismatch())
            }
        }
        // Composites are enforced at the constructor level (params ignored), no
        // coercion: AST values are real Python objects, matched by container type.
        type_code::LIST => {
            if v.is_instance_of::<PyList>() {
                Ok(value)
            } else {
                Err(mismatch())
            }
        }
        type_code::DICT => {
            if v.is_instance_of::<PyDict>() {
                Ok(value)
            } else {
                Err(mismatch())
            }
        }
        _ => Ok(value),
    }
}

/// True if `value` is a member of the nominal type `name`, with subtyping.
///
/// AST values are real Python objects, so membership reads the runtime object:
/// a [`CatnipStructProxy`] matches when its type name, a union-variant prefix
/// (`Union.Variant`), or -- via its `struct_type` back-ref -- its MRO, parent
/// names, or implemented traits include `name`; a [`CatnipEnumVariant`] matches
/// when its enum name equals `name` (covering plain enums and nullary union
/// variants). Anything else is not a member.
fn value_is_member_of(py: Python<'_>, value: &Bound<'_, PyAny>, name: &str) -> bool {
    use crate::vm::enums::CatnipEnumVariant;
    use crate::vm::structs::CatnipStructProxy;

    if let Ok(proxy) = value.cast::<CatnipStructProxy>() {
        let p = proxy.borrow();
        if p.type_name == name {
            return true;
        }
        // Tagged-union variant with a named payload: `Union.Variant`.
        if let Some((union, _)) = p.type_name.split_once('.') {
            if union == name {
                return true;
            }
        }
        if let Some(ref st) = p.struct_type {
            let st = st.borrow(py);
            if st.mro.iter().any(|n| n == name)
                || st.parent_names.iter().any(|n| n == name)
                || st.implements.iter().any(|n| n == name)
            {
                return true;
            }
        }
        return false;
    }

    if let Ok(variant) = value.cast::<CatnipEnumVariant>() {
        return variant.get().enum_name == name;
    }

    false
}

/// True if `name` resolves to a known nominal type (struct/enum/union/trait) in
/// the AST execution context. Type definitions are stored in the context (locals
/// first, then globals, mirroring identifier resolution; traits live in the
/// `__traits__` dict). An unknown name -- a typo or a composite like `list` --
/// makes the nominal check inert.
fn nominal_type_is_known(py: Python<'_>, ctx: &Py<PyAny>, name: &str) -> bool {
    use crate::vm::enums::CatnipEnumType;
    use crate::vm::structs::CatnipStructType;
    use crate::vm::unions::CatnipUnionType;

    let is_type = |obj: &Bound<'_, PyAny>| {
        obj.is_instance_of::<CatnipStructType>()
            || obj.is_instance_of::<CatnipEnumType>()
            || obj.is_instance_of::<CatnipUnionType>()
    };

    let ctx = ctx.bind(py);
    if let Ok(locals) = ctx.getattr("locals") {
        if let Ok(exists) = locals.call_method1("contains", (name,)) {
            if exists.is_truthy().unwrap_or(false) {
                if let Ok(value) = locals.call_method1("get", (name,)) {
                    if is_type(&value) {
                        return true;
                    }
                }
            }
        }
    }
    if let Ok(globals) = ctx.getattr("globals") {
        if let Ok(value) = globals.call_method1("get", (name,)) {
            if is_type(&value) {
                return true;
            }
        }
        // Traits are not top-level type objects: they live in the `__traits__`
        // dict. A trait annotation accepts a struct that implements it, so a
        // known trait name makes a non-implementer a type error, not a no-op.
        if let Ok(traits) = globals.call_method1("get", ("__traits__", py.None())) {
            if !traits.is_none() {
                if let Ok(t) = traits.call_method1("get", (name, py.None())) {
                    if !t.is_none() {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Best-effort Catnip type name of an AST value for a boundary error message:
/// the nominal type name for a struct proxy / enum variant, else the Python
/// class name. Mirrors the VM `nominal_value_type_name` so the two modes report
/// the same "got '<type>'".
pub(crate) fn nominal_value_type_name_ast(_py: Python<'_>, value: &Bound<'_, PyAny>) -> String {
    use crate::vm::enums::CatnipEnumVariant;
    use crate::vm::structs::CatnipStructProxy;
    if let Ok(proxy) = value.cast::<CatnipStructProxy>() {
        return proxy.borrow().type_name.clone();
    }
    if let Ok(variant) = value.cast::<CatnipEnumVariant>() {
        let v = variant.get();
        return catnip_core::symbols::qualified_name(&v.enum_name, &v.variant_name);
    }
    value
        .get_type()
        .name()
        .ok()
        .and_then(|n| n.to_str().ok().map(|s| s.to_string()))
        .unwrap_or_else(|| "value".to_string())
}

/// TH2-B step 0b nominal boundary check on a Python value -- the AST-mode mirror
/// of the VM `CheckNominal`. A nominal annotation never coerces. Returns `Ok`
/// when `value` is a member of `name` (with subtyping) or when `name` is not a
/// known nominal type at runtime (the annotation is inert). Returns a
/// `TypeError` when `name` is a known nominal type but `value` is not a member.
fn nominal_check_py(py: Python<'_>, value: &Bound<'_, PyAny>, name: &str, ctx: &Py<PyAny>) -> PyResult<()> {
    if value_is_member_of(py, value, name) {
        return Ok(());
    }
    if nominal_type_is_known(py, ctx, name) {
        let tn = nominal_value_type_name_ast(py, value);
        return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
            "typed parameter expects '{}' but got '{}'",
            name, tn
        )));
    }
    Ok(())
}

/// Classify a Python value into its [`PrimitiveClass`] for the shared union
/// membership test ([`catnip_core::vm::opcode::primitive_membership`]). The
/// numeric tower lives in core; this only maps the value's Python type. `PyBool`
/// is a subclass of `PyInt` in CPython, so `int_like` already holds for a bool;
/// `bool_like` distinguishes it for the `bool` member. No coercion.
fn value_primitive_class_py(value: &Bound<'_, PyAny>) -> catnip_core::vm::opcode::PrimitiveClass {
    catnip_core::vm::opcode::PrimitiveClass {
        int_like: value.is_instance_of::<PyInt>(),
        float_like: value.is_instance_of::<PyFloat>(),
        str_like: value.is_instance_of::<PyString>(),
        bool_like: value.is_instance_of::<PyBool>(),
        nil_like: value.is_none(),
        list_like: value.is_instance_of::<PyList>(),
        set_like: value.is_instance_of::<PySet>(),
        dict_like: value.is_instance_of::<PyDict>(),
        tuple_like: value.is_instance_of::<PyTuple>(),
    }
}

/// TH2-B type-union boundary check on a Python value -- the AST-mode mirror of the
/// VM `CheckUnion`. Accepts when `value` satisfies any member (primitive by the
/// numeric tower, nominal by subtyping); no coercion. Like `CheckNominal`, a
/// nominal member whose name is unknown at runtime keeps the union inert rather
/// than rejecting a possibly-valid value. Raises `TypeError` naming the union.
fn union_check_py(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    members: &[catnip_core::vm::opcode::ParamCheck],
    ctx: &Py<PyAny>,
) -> PyResult<()> {
    use catnip_core::vm::opcode::{ParamCheck, format_union_members, primitive_membership};
    let class = value_primitive_class_py(value);
    let mut unknown_nominal = false;
    for m in members {
        match m {
            ParamCheck::Primitive(code) => {
                if primitive_membership(*code, &class) {
                    return Ok(());
                }
            }
            ParamCheck::Nominal(name) => {
                if value_is_member_of(py, value, name) {
                    return Ok(());
                }
                if !nominal_type_is_known(py, ctx, name) {
                    unknown_nominal = true;
                }
            }
            // A composite member is checked in full (container + parameters),
            // mirroring `composite_check_py`.
            ParamCheck::Composite { .. } => {
                if composite_check_py(py, value, m, ctx).is_ok() {
                    return Ok(());
                }
            }
            // A generic-nominal member (`Option[int]`): a member with a matching
            // payload accepts; a member with a mismatched payload is not this
            // alternative; an unknown union name keeps the check inert.
            ParamCheck::Generic { name, .. } => {
                if value_is_member_of(py, value, name) {
                    if generic_check_py(py, value, m, ctx).is_ok() {
                        return Ok(());
                    }
                } else if !nominal_type_is_known(py, ctx, name) {
                    unknown_nominal = true;
                }
            }
            // A function-type member (`None | (int) -> int`): full
            // callability + arity acceptance, mirroring the prologue check.
            ParamCheck::Callable { arity } => {
                if callable_check_py(py, value, *arity).is_ok() {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
    if unknown_nominal {
        return Ok(());
    }
    Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
        "typed parameter expects '{}' but got '{}'",
        format_union_members(members),
        nominal_value_type_name_ast(py, value)
    )))
}

/// TH2-B generic-nominal boundary check on a Python value -- the AST-mode mirror
/// of the VM `CheckGeneric` (`Option[int]`). Union membership (same rule as
/// `nominal_check_py`, an unknown union name is inert) plus the parametric payload
/// substitution: a payload variant is a `CatnipStructProxy` whose `struct_type`
/// carries the per-field [`FieldTemplate`]s; each `Param(k)` field is checked
/// against `args[k]`, each `Fixed` against its own check, via
/// [`value_satisfies_ast`]. A nullary variant carries no payload and passes.
/// No coercion, read-only.
fn generic_check_py(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    spec: &catnip_core::vm::opcode::ParamCheck,
    ctx: &Py<PyAny>,
) -> PyResult<()> {
    use crate::vm::structs::CatnipStructProxy;
    use catnip_core::vm::opcode::{FieldTemplate, ParamCheck, format_param_check};
    let ParamCheck::Generic { name, args } = spec else {
        return Ok(());
    };
    if !value_is_member_of(py, value, name) {
        if nominal_type_is_known(py, ctx, name) {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
                "typed parameter expects '{}' but got '{}'",
                format_param_check(spec),
                nominal_value_type_name_ast(py, value)
            )));
        }
        return Ok(()); // unknown union -> inert
    }
    let Ok(proxy) = value.cast::<CatnipStructProxy>() else {
        return Ok(()); // nullary variant -> no payload
    };
    let p = proxy.borrow();
    let Some(ref st) = p.struct_type else {
        return Ok(()); // no back-reference -> membership-only
    };
    let templates: Vec<FieldTemplate> = st.borrow(py).field_templates.clone();
    for (i, tmpl) in templates.iter().enumerate() {
        let Some(fval) = p.field_values.get(i) else { break };
        let required: Option<&ParamCheck> = match tmpl {
            FieldTemplate::Param(k) => args.get(*k),
            FieldTemplate::Fixed(c) => Some(c),
        };
        if let Some(check) = required {
            if !matches!(check, ParamCheck::None) && !value_satisfies_ast(py, fval.bind(py), check, ctx) {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
                    "typed parameter expects '{}' but a payload field has the wrong type",
                    format_param_check(spec)
                )));
            }
        }
    }
    Ok(())
}

/// TH2-B composite boundary check on a Python value -- the AST-mode mirror of the
/// VM `CheckComposite`. Checks the container tag (`list`/`set`/`dict`/`tuple`) and,
/// when the spec carries parameters, that each element (and, for a dict, each
/// key/value; for a tuple, position `i` against `params[i]`, arity included)
/// satisfies the corresponding parameter check, recursively. No coercion. Raises
/// `TypeError` naming the composite.
fn composite_check_py(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    spec: &catnip_core::vm::opcode::ParamCheck,
    ctx: &Py<PyAny>,
) -> PyResult<()> {
    use catnip_core::vm::opcode::{ParamCheck, format_param_check, primitive_membership, type_code};
    let ParamCheck::Composite { head, params } = spec else {
        return Ok(());
    };
    if !primitive_membership(*head, &value_primitive_class_py(value)) {
        return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
            "typed parameter expects '{}' but got '{}'",
            format_param_check(spec),
            nominal_value_type_name_ast(py, value)
        )));
    }
    let enforced = |p: &ParamCheck| !matches!(p, ParamCheck::None);
    let elem_err = |what: &str| {
        PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
            "typed parameter expects '{}' but {} has the wrong type",
            format_param_check(spec),
            what
        ))
    };
    match *head {
        type_code::LIST => {
            if let Some(elem) = params.first().filter(|p| enforced(p)) {
                if let Ok(list) = value.cast::<PyList>() {
                    for it in list.iter() {
                        if !value_satisfies_ast(py, &it, elem, ctx) {
                            return Err(elem_err("an element"));
                        }
                    }
                }
            }
        }
        type_code::SET => {
            if let Some(elem) = params.first().filter(|p| enforced(p)) {
                if let Ok(set) = value.cast::<PySet>() {
                    for it in set.iter() {
                        if !value_satisfies_ast(py, &it, elem, ctx) {
                            return Err(elem_err("an element"));
                        }
                    }
                }
            }
        }
        type_code::TUPLE => {
            // Positional: `params.len()` is the enforced arity, position `i` is
            // checked against `params[i]`. A bare `tuple` checks only the container.
            if !params.is_empty() {
                if let Ok(tuple) = value.cast::<PyTuple>() {
                    if tuple.len() != params.len() {
                        return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
                            "typed parameter expects '{}' but got a tuple of length {}",
                            format_param_check(spec),
                            tuple.len()
                        )));
                    }
                    for (it, p) in tuple.iter().zip(params.iter()) {
                        if !value_satisfies_ast(py, &it, p, ctx) {
                            return Err(elem_err("an element"));
                        }
                    }
                }
            }
        }
        type_code::DICT if params.len() == 2 => {
            let (kc, vc) = (&params[0], &params[1]);
            if let Ok(dict) = value.cast::<PyDict>() {
                for (k, v) in dict.iter() {
                    if enforced(kc) && !value_satisfies_ast(py, &k, kc, ctx) {
                        return Err(elem_err("a key"));
                    }
                    if enforced(vc) && !value_satisfies_ast(py, &v, vc, ctx) {
                        return Err(elem_err("a value"));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Whether a single Python value satisfies a `ParamCheck`, for the AST composite
/// element pass. Primitives by the numeric tower, nominals by subtyping (an
/// unknown name is inert -> satisfied), unions by any member, composites
/// recursively.
pub(crate) fn value_satisfies_ast(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    check: &catnip_core::vm::opcode::ParamCheck,
    ctx: &Py<PyAny>,
) -> bool {
    use catnip_core::vm::opcode::{ParamCheck, primitive_membership};
    match check {
        ParamCheck::None => true,
        ParamCheck::Primitive(code) => primitive_membership(*code, &value_primitive_class_py(value)),
        ParamCheck::Nominal(name) => value_is_member_of(py, value, name) || !nominal_type_is_known(py, ctx, name),
        ParamCheck::Union(members) => members.iter().any(|m| value_satisfies_ast(py, value, m, ctx)),
        ParamCheck::Composite { .. } => composite_check_py(py, value, check, ctx).is_ok(),
        ParamCheck::Generic { .. } => generic_check_py(py, value, check, ctx).is_ok(),
        ParamCheck::Callable { arity } => callable_check_py(py, value, *arity).is_ok(),
    }
}

/// Read-only validation of a constructor field value against its annotation, the
/// AST mirror of the VM `field_value_ok`. Like [`value_satisfies_ast`] but
/// rejects up front an int that overflows `f64` for a `float` slot -- the one
/// input [`boundary_coerce_py`] fails on -- so `CatnipStructType::__call__` can
/// validate first and coerce second.
pub(crate) fn field_value_ok_ast(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    check: &catnip_core::vm::opcode::ParamCheck,
    ctx: &Py<PyAny>,
) -> bool {
    use catnip_core::vm::opcode::{ParamCheck, type_code};
    if let ParamCheck::Primitive(type_code::FLOAT) = check {
        // A Python int (not bool) into a float slot: accept only if it converts
        // to a finite f64 (an overflowing int makes float() raise -- rejected).
        if value.is_instance_of::<PyInt>() && !value.is_instance_of::<PyBool>() {
            return value.extract::<f64>().map(|f| f.is_finite()).unwrap_or(false);
        }
    }
    value_satisfies_ast(py, value, check, ctx)
}

/// TH2-B step 0b: after params are bound, enforce each annotated param in the
/// current scope -- the AST-mode mirror of the VM prologue `CheckType` (primitive
/// coercion) and `CheckNominal` (nominal subtyping, no coercion).
/// Enforce an annotation on a value outside a param prologue (FT2-A: the
/// declared return of a callback, checked on the caller side). Same
/// classification and helpers as `coerce_typed_params`; primitives coerce
/// (bigint -> float), every other check validates and passes the value
/// through. An unenforceable annotation is inert.
pub(crate) fn apply_annotation_check(
    py: Python<'_>,
    value: Py<PyAny>,
    annotation: &str,
    ctx: &Py<PyAny>,
) -> PyResult<Py<PyAny>> {
    use catnip_core::vm::opcode::ParamCheck;
    match ParamCheck::from_annotation(annotation) {
        ParamCheck::Primitive(code) => boundary_coerce_py(py, value, code),
        ParamCheck::Nominal(type_name) => {
            nominal_check_py(py, value.bind(py), &type_name, ctx)?;
            Ok(value)
        }
        ParamCheck::Union(members) => {
            union_check_py(py, value.bind(py), &members, ctx)?;
            Ok(value)
        }
        composite @ ParamCheck::Composite { .. } => {
            composite_check_py(py, value.bind(py), &composite, ctx)?;
            Ok(value)
        }
        generic @ ParamCheck::Generic { .. } => {
            generic_check_py(py, value.bind(py), &generic, ctx)?;
            Ok(value)
        }
        ParamCheck::Callable { arity } => {
            callable_check_py(py, value.bind(py), arity)?;
            Ok(value)
        }
        ParamCheck::None => Ok(value),
    }
}

/// Function-type boundary (FT3), AST mode. Callable + declared-arity
/// acceptance: a Catnip `Function`/`Lambda` exposes its params list, so the
/// arity is introspectable (`required <= arity <= positional`, or `arity >=
/// required` when variadic); any other callable passes on callability alone
/// (a Python callable's arity is not introspectable without `inspect`).
/// Parameter/return types are NOT checked (observable only at calls).
fn callable_check_py(py: Python<'_>, value: &Bound<'_, PyAny>, arity: u32) -> PyResult<()> {
    use catnip_core::vm::opcode::callable_arity_accepts;
    let arity = arity as usize;
    if !value.is_callable() {
        return Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "typed parameter expects a callable taking {arity} argument(s) but got a non-callable value"
        )));
    }
    // A struct constructor: field range (parity with the VM boundary).
    if let Ok(st) = value.cast::<crate::vm::structs::CatnipStructType>() {
        let st = st.borrow();
        let fixed = st.field_names.len();
        let defaults = st.field_defaults.iter().filter(|d| d.is_some()).count().min(fixed);
        let (required, accepts) = callable_arity_accepts(fixed, false, defaults, arity);
        if !accepts {
            return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "typed parameter expects a callable taking {arity} argument(s) but the constructor requires {required}"
            )));
        }
        return Ok(());
    }
    // A Catnip function/lambda: read the arity off its params list.
    if let Ok(params_attr) = value.getattr("params") {
        if let Ok(params_list) = params_attr.cast::<PyList>() {
            let (vararg, positional) = check_variadic(py, params_list)?;
            // A real default is a non-None default slot -- the binder's own
            // rule (an absent default is None-encoded and binds None).
            let mut defaults = 0usize;
            for i in 0..positional.min(params_list.len()) {
                let param = params_list.get_item(i)?;
                if get_param_len(&param) > 1 && !get_param_default(&param)?.is_none() {
                    defaults += 1;
                }
            }
            let (required, accepts) = callable_arity_accepts(positional, vararg.is_some(), defaults, arity);
            if !accepts {
                return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                    "typed parameter expects a callable taking {arity} argument(s) but the function requires {required}"
                )));
            }
        }
    }
    Ok(())
}

fn coerce_typed_params(py: Python<'_>, params: &Py<PyAny>, registry: &Py<PyAny>) -> PyResult<()> {
    use catnip_core::vm::opcode::ParamCheck;

    let params_list = params.cast_bound::<PyList>(py)?;
    let ctx = registry.getattr(py, "ctx")?;
    let locals = ctx.getattr(py, "locals")?;
    for i in 0..params_list.len() {
        let param = params_list.get_item(i)?;
        let Some(annotation) = extract_param_annotation(&param) else {
            continue;
        };
        match ParamCheck::from_annotation(&annotation) {
            ParamCheck::Primitive(code) => {
                let name = extract_param_name(&param)?;
                let value = locals.call_method1(py, "get", (&name,))?;
                let coerced = boundary_coerce_py(py, value, code)?;
                locals.call_method1(py, "_set_param", (&name, coerced))?;
            }
            ParamCheck::Nominal(type_name) => {
                let name = extract_param_name(&param)?;
                let value = locals.call_method1(py, "get", (&name,))?;
                nominal_check_py(py, value.bind(py), &type_name, &ctx)?;
            }
            ParamCheck::Union(members) => {
                let name = extract_param_name(&param)?;
                let value = locals.call_method1(py, "get", (&name,))?;
                union_check_py(py, value.bind(py), &members, &ctx)?;
            }
            composite @ ParamCheck::Composite { .. } => {
                let name = extract_param_name(&param)?;
                let value = locals.call_method1(py, "get", (&name,))?;
                composite_check_py(py, value.bind(py), &composite, &ctx)?;
            }
            generic @ ParamCheck::Generic { .. } => {
                let name = extract_param_name(&param)?;
                let value = locals.call_method1(py, "get", (&name,))?;
                generic_check_py(py, value.bind(py), &generic, &ctx)?;
            }
            ParamCheck::Callable { arity } => {
                let name = extract_param_name(&param)?;
                let value = locals.call_method1(py, "get", (&name,))?;
                callable_check_py(py, value.bind(py), arity)?;
            }
            ParamCheck::None => {}
        }
    }
    Ok(())
}

/// Extract parameter name.
fn extract_param_name(param: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(t) = param.cast::<PyTuple>() {
        t.get_item(0)?.extract()
    } else if let Ok(l) = param.cast::<PyList>() {
        l.get_item(0)?.extract()
    } else {
        param.extract()
    }
}

/// Get parameter tuple/list length.
fn get_param_len(param: &Bound<'_, PyAny>) -> usize {
    if let Ok(t) = param.cast::<PyTuple>() {
        t.len()
    } else if let Ok(l) = param.cast::<PyList>() {
        l.len()
    } else {
        1
    }
}

/// Get default value from parameter.
fn get_param_default<'a>(param: &'a Bound<'a, PyAny>) -> PyResult<Bound<'a, PyAny>> {
    if let Ok(t) = param.cast::<PyTuple>() {
        t.get_item(1)
    } else if let Ok(l) = param.cast::<PyList>() {
        l.get_item(1)
    } else {
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>("Invalid param"))
    }
}

/// Capture closure values for pickle.
fn capture_closure_values(py: Python<'_>, scope: &Py<PyAny>) -> PyResult<Option<Py<PyAny>>> {
    let pickle = py.import("pickle")?;
    let captured = PyDict::new(py);
    let mut current = scope.clone_ref(py);

    loop {
        if let Ok(symbols) = current.getattr(py, "_symbols") {
            if let Ok(dict) = symbols.cast_bound::<PyDict>(py) {
                for (key, value) in dict.iter() {
                    let name: String = key.extract()?;
                    if !captured.contains(&name)? && pickle.call_method1("dumps", (&value,)).is_ok() {
                        captured.set_item(&name, &value)?;
                    }
                }
            }
        }

        if let Ok(parent) = current.getattr(py, "_parent") {
            if parent.is_none(py) {
                break;
            }
            current = parent;
        } else {
            break;
        }
    }

    if captured.is_empty() {
        Ok(None)
    } else {
        Ok(Some(captured.into_any().unbind()))
    }
}

// === Pickle reconstruction ===

#[pyfunction]
pub fn _reconstruct_function(
    py: Python<'_>,
    name: String,
    params: Py<PyAny>,
    body: Py<PyAny>,
    jit_func_id: String,
) -> PyResult<Function> {
    let registry = get_global_registry(py).ok_or_else(|| {
        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
            "No registry for pickle reconstruction. Call set_global_registry() first.",
        )
    })?;

    let mut func = Function::new(py, name, params, body, registry)?;
    func.jit_func_id = jit_func_id;
    Ok(func)
}

#[pyfunction]
#[pyo3(signature = (body, params, jit_func_id, closure_values=None))]
pub fn _reconstruct_lambda(
    py: Python<'_>,
    body: Py<PyAny>,
    params: Py<PyAny>,
    jit_func_id: String,
    closure_values: Option<Py<PyAny>>,
) -> PyResult<Lambda> {
    let registry = get_global_registry(py).ok_or_else(|| {
        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
            "No registry for pickle reconstruction. Call set_global_registry() first.",
        )
    })?;

    let mut lambda = Lambda::new(py, body, registry.clone_ref(py), Some(params))?;
    lambda.jit_func_id = jit_func_id;

    if let Some(values) = closure_values {
        if let Ok(dict) = values.cast_bound::<PyDict>(py) {
            let ctx = registry.getattr(py, "ctx")?;
            let locals = ctx.getattr(py, "locals")?;
            for (name, value) in dict.iter() {
                locals.call_method1(py, "_set", (name, value))?;
            }
        }
    }

    Ok(lambda)
}

/// Register function module.
pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Function>()?;
    m.add_class::<Lambda>()?;
    m.add_function(wrap_pyfunction!(set_global_registry, m)?)?;
    m.add_function(wrap_pyfunction!(get_global_registry, m)?)?;
    m.add_function(wrap_pyfunction!(_reconstruct_function, m)?)?;
    m.add_function(wrap_pyfunction!(_reconstruct_lambda, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_registry_initially_none() {
        crate::test_support::init_python();
        Python::attach(|py| {
            assert!(get_global_registry(py).is_none());
        });
    }
}
