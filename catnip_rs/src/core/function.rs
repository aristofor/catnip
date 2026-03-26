// FILE: catnip_rs/src/core/function.rs
//! Rust implementation of Function and Lambda nodes.
//!
//! This module provides high-performance implementations of function
//! and lambda execution, including the trampoline loop for tail-call
//! optimization (TCO).

use crate::constants::*;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyList, PyTuple};

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
        })
    }

    #[getter]
    fn get_closure_scope(&self, py: Python<'_>) -> Py<PyAny> {
        self.closure_scope.clone_ref(py)
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
        // JIT fast path
        if let Some(ref jit_func) = self.jit_compiled_func {
            let ctx = self.registry.getattr(py, "ctx")?;
            let jit_enabled: bool = ctx.getattr(py, "jit_enabled")?.extract(py)?;
            if jit_enabled && (kwargs.is_none() || kwargs.is_some_and(|k| k.is_empty())) {
                if let Some(result) = try_jit_call(py, jit_func, args, &self.params)? {
                    return Ok(result);
                }
            }
        }

        // Interpreted execution with trampoline
        let ctx = self.registry.getattr(py, "ctx")?;
        ctx.call_method1(py, "push_scope_with_capture", (&self.closure_scope,))?;

        let result = execute_trampoline(
            py,
            &self.body,
            &self.registry,
            &self.params,
            &self.closure_scope,
            args,
            kwargs,
        );

        ctx.call_method1(py, "pop_scope_with_sync", (&self.closure_scope,))?;
        result
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
        })
    }

    #[getter]
    fn get_closure_scope(&self, py: Python<'_>) -> Py<PyAny> {
        self.closure_scope.clone_ref(py)
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
        // JIT fast path
        if let Some(ref jit_func) = self.jit_compiled_func {
            let ctx = self.registry.getattr(py, "ctx")?;
            let jit_enabled: bool = ctx.getattr(py, "jit_enabled")?.extract(py)?;
            if jit_enabled && (kwargs.is_none() || kwargs.is_some_and(|k| k.is_empty())) {
                if let Some(result) = try_jit_call(py, jit_func, args, &self.params)? {
                    return Ok(result);
                }
            }
        }

        // Interpreted execution with trampoline
        let ctx = self.registry.getattr(py, "ctx")?;
        ctx.call_method1(py, "push_scope_with_capture", (&self.closure_scope,))?;

        let result = execute_trampoline(
            py,
            &self.body,
            &self.registry,
            &self.params,
            &self.closure_scope,
            args,
            kwargs,
        );

        ctx.call_method1(py, "pop_scope_with_sync", (&self.closure_scope,))?;
        result
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let reconstruct_fn = py.import(PY_MOD_RS)?.getattr("_reconstruct_lambda")?;
        let closure_vals = self
            .closure_values
            .as_ref()
            .map(|v| v.clone_ref(py))
            .unwrap_or_else(|| py.None());
        let jit_id_obj: Py<PyAny> = self.jit_func_id.clone().into_pyobject(py)?.unbind().into();

        let args = PyTuple::new(
            py,
            [
                self.body.clone_ref(py),
                self.params.clone_ref(py),
                jit_id_obj,
                closure_vals,
            ],
        )?;

        let result = PyTuple::new(py, [reconstruct_fn.into_any().unbind(), args.into_any().unbind()])?;
        Ok(result.into_any().unbind())
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
fn execute_trampoline(
    py: Python<'_>,
    body: &Py<PyAny>,
    registry: &Py<PyAny>,
    params: &Py<PyAny>,
    closure_scope: &Py<PyAny>,
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
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
    let mut cur_closure = closure_scope.clone_ref(py);
    let mut cur_args: Py<PyTuple> = args.clone().unbind();
    let mut cur_kwargs: Option<Py<PyDict>> = kwargs.map(|k| k.clone().unbind());
    let mut is_first = true;

    loop {
        // Bind parameters
        let params_bound = cur_params.bind(py);
        if params_bound.len()? > 0 {
            bind_params(py, &cur_params, &cur_registry, &cur_args, &cur_kwargs, is_first)?;
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

            // Scope swap for mutual recursion
            let new_closure = target.getattr(py, "closure_scope")?;
            if !new_closure.is(&cur_closure) {
                let ctx = registry.getattr(py, "ctx")?;
                ctx.call_method0(py, "pop_scope")?;
                ctx.call_method1(py, "push_scope_with_capture", (&new_closure,))?;
                cur_closure = new_closure;
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
    is_first: bool,
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

    // Defaults (only on first iteration)
    if is_first {
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
        Python::initialize();
        Python::attach(|py| {
            assert!(get_global_registry(py).is_none());
        });
    }
}
