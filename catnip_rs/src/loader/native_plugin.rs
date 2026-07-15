// FILE: catnip_rs/src/loader/native_plugin.rs
//! Bridge exposing native catnip_vm plugins (`.so` loaded via libloading) to
//! the PyO3 VM.
//!
//! Native stdlib plugins (e.g. `http`) are compiled against `catnip_vm` and are
//! PureVM-only: the PyO3 loader normally loads stdlib as Python modules and
//! cannot host them. This module wraps a plugin's module functions and objects
//! as Python callables/objects and marshals values across the
//! `catnip_vm::Value` <-> PyObject boundary (`crate::vm::py_interop`).
//!
//! Object attribute/method fidelity is preserved by the VM opcodes, not by
//! Python duck-typing: `OpCode::GetAttr` routes to the plugin's getattr
//! callback and `OpCode::CallMethod` to its method callback (mirroring
//! `catnip_vm::host`). See `crate::vm::host` and `crate::vm::core`.

use std::path::{Path, PathBuf};

use pyo3::exceptions::{PyAttributeError, PyFileNotFoundError, PyRuntimeError};
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use catnip_vm::Value as VmValue;
use catnip_vm::plugin::{self, PluginCallFn, PluginObjectCallbacks, SharedPluginRegistry};

use crate::loader::namespace::ModuleNamespace;
use crate::vm::py_interop::{convert_py_to_vm_value, vm_value_to_py, vm_value_to_py_borrowed};

const PLUGIN_PREFIX: &str = "__plugin::";

/// Map a `catnip_vm::VMError` to a Python exception.
pub(crate) fn vmerr_to_py(msg: String) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(msg)
}

/// A bound module-level plugin function (e.g. `http.get`). Holds the raw call
/// pointer; the `.so` stays loaded for as long as the owning loader keeps the
/// plugin registry alive.
#[pyclass(name = "NativePluginFn", module = "catnip._rs")]
pub struct NativePluginFn {
    call_fn: PluginCallFn,
    short_name: String,
    /// Module object callbacks, used to wrap object handles returned by the call.
    obj_callbacks: PluginObjectCallbacks,
}

impl NativePluginFn {
    pub fn new(call_fn: PluginCallFn, short_name: String, obj_callbacks: PluginObjectCallbacks) -> Self {
        Self {
            call_fn,
            short_name,
            obj_callbacks,
        }
    }
}

#[pymethods]
impl NativePluginFn {
    #[pyo3(signature = (*args))]
    fn __call__(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let mut vm_args: Vec<VmValue> = Vec::with_capacity(args.len());
        for a in args.iter() {
            vm_args.push(convert_py_to_vm_value(&a)?);
        }

        let call_fn = self.call_fn;
        let short = self.short_name.clone();
        let cbs = self.obj_callbacks.clone();
        // Release the GIL: some plugin functions block (network, etc.).
        let result: Result<u64, String> = py.detach(|| {
            plugin::call_plugin_fn(call_fn, &short, &vm_args, &cbs)
                .map(|v| v.bits())
                .map_err(|e| e.to_string())
        });

        for a in &vm_args {
            a.decref();
        }

        let bits = result.map_err(vmerr_to_py)?;
        vm_value_to_py(py, VmValue::from_raw(bits))
    }

    fn __repr__(&self) -> String {
        format!("<native plugin fn '{}'>", self.short_name)
    }
}

/// A handle to a plugin object (e.g. `Response`, `Request`, `Server`).
///
/// Owns exactly one reference to the underlying `catnip_vm` PluginObject value;
/// `Drop` releases it, which calls the plugin's drop callback. Method and
/// attribute dispatch is performed in the host (`crate::vm::host::obj_getattr`
/// and `OpCode::CallMethod`), not here.
/// # Thread-safety
///
/// `catnip_vm::Value` is a `u64` newtype (`Send`), and the only reference-count
/// mutations the bridge performs (`clone_refcount` when passed as an argument,
/// `decref` on `Drop`) happen while the GIL is held, so sharing across Python
/// threads is sound. Plugin objects such as `http.Server` are deliberately used
/// from worker threads (`Server.recv` blocks with the GIL released).
#[pyclass(name = "NativePluginObject", module = "catnip._rs")]
pub struct NativePluginObject {
    vm_value: VmValue,
}

impl NativePluginObject {
    /// Take ownership of a `catnip_vm` PluginObject value.
    pub fn from_vm(vm_value: VmValue) -> Self {
        Self { vm_value }
    }

    /// The underlying value (still owned by this object; do not decref).
    pub fn vm_value(&self) -> VmValue {
        self.vm_value
    }

    /// Extract the object handle and its callbacks.
    pub fn handle_and_callbacks(&self) -> Option<(u64, PluginObjectCallbacks)> {
        // SAFETY: self.vm_value is the PluginObject value this struct owns exactly one
        // live reference to (released only in Drop), so the backing Arc is alive here.
        unsafe { self.vm_value.as_plugin_object_ref() }
    }
}

impl Drop for NativePluginObject {
    fn drop(&mut self) {
        self.vm_value.decref();
    }
}

#[pymethods]
impl NativePluginObject {
    fn __repr__(&self) -> String {
        "<native plugin object>".to_string()
    }

    /// Python-side attribute protocol, used by the AST executor (the VM routes
    /// `OpCode::GetAttr`/`CallMethod` through the host instead). Tries the
    /// plugin's getattr callback; on miss, binds the name as a method only when
    /// the plugin's membership probe confirms it exists (the AST lowers
    /// `obj.m(...)` to getattr-then-call, so attribute-vs-method cannot be
    /// decided syntactically here). A name that is neither attribute nor method
    /// raises AttributeError, matching the VM and keeping `hasattr` honest.
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        // Underscore names are Python protocol probes (__reduce__, __iter__,
        // ...), never plugin members: refuse them so hasattr stays honest
        if name.starts_with('_') {
            return Err(PyAttributeError::new_err(format!(
                "'NativePluginObject' object has no attribute '{name}'"
            )));
        }
        let (handle, cbs) = self
            .handle_and_callbacks()
            .ok_or_else(|| PyRuntimeError::new_err("invalid plugin object"))?;
        if let Some(getattr_fn) = cbs.getattr {
            let name_owned = name.to_string();
            let cbs_clone = cbs.clone();
            let result: Result<u64, String> = py.detach(move || {
                plugin::call_plugin_getattr(handle, getattr_fn, &name_owned, &cbs_clone)
                    .map(|v| v.bits())
                    .map_err(|e| e.to_string())
            });
            if let Ok(bits) = result {
                return vm_value_to_py(py, VmValue::from_raw(bits));
            }
        }
        if cbs.method.is_some() {
            // Bind a method only if the object actually exposes this member.
            // A pre-v3 plugin (no probe) keeps the optimistic fallback.
            let is_member = match cbs.has_member {
                Some(hm) => plugin::call_plugin_has_member(handle, hm, name),
                None => true,
            };
            if is_member {
                self.vm_value.clone_refcount();
                let bound = NativePluginBoundMethod {
                    vm_value: self.vm_value,
                    method_name: name.to_string(),
                };
                return Ok(Py::new(py, bound)?.into_any());
            }
        }
        Err(PyAttributeError::new_err(format!(
            "'NativePluginObject' object has no attribute '{name}'"
        )))
    }
}

/// A method bound to a plugin object by `NativePluginObject.__getattr__`.
/// Owns one reference to the object (released on Drop); dispatches through the
/// plugin's method callback at call time, mirroring `OpCode::CallMethod`.
#[pyclass(name = "NativePluginBoundMethod", module = "catnip._rs")]
pub struct NativePluginBoundMethod {
    vm_value: VmValue,
    method_name: String,
}

impl Drop for NativePluginBoundMethod {
    fn drop(&mut self) {
        self.vm_value.decref();
    }
}

#[pymethods]
impl NativePluginBoundMethod {
    #[pyo3(signature = (*args))]
    fn __call__(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        // SAFETY: self.vm_value is the PluginObject value this bound method owns one
        // live reference to (released in Drop), so the backing Arc is alive for this borrow.
        let (handle, cbs) = unsafe { self.vm_value.as_plugin_object_ref() }
            .ok_or_else(|| PyRuntimeError::new_err("invalid plugin object"))?;
        let method_fn = cbs
            .method
            .ok_or_else(|| PyRuntimeError::new_err(format!("plugin object has no method '{}'", self.method_name)))?;

        let mut vm_args: Vec<VmValue> = Vec::with_capacity(args.len());
        for a in args.iter() {
            vm_args.push(convert_py_to_vm_value(&a)?);
        }

        let name = self.method_name.clone();
        // Release the GIL: some plugin methods block (network, etc.).
        let result: Result<u64, String> = py.detach(|| {
            plugin::call_plugin_method(handle, method_fn, &name, &vm_args, &cbs)
                .map(|v| v.bits())
                .map_err(|e| e.to_string())
        });

        for a in &vm_args {
            a.decref();
        }

        let bits = result.map_err(vmerr_to_py)?;
        vm_value_to_py(py, VmValue::from_raw(bits))
    }

    fn __repr__(&self) -> String {
        format!("<native plugin method '{}'>", self.method_name)
    }
}

// ---------------------------------------------------------------------------
// Discovery + loading
// ---------------------------------------------------------------------------

/// Directories searched for `libcatnip_<name>.so` plugins.
///
/// Order: `$CATNIP_STDLIB_PATH` (colon-separated) then the installed `catnip`
/// package directory (where `_rs` and the bundled plugins live).
fn discover_plugin_paths(py: Python<'_>) -> Vec<PathBuf> {
    let mut paths = catnip_core::paths::stdlib_env_paths();

    if let Ok(catnip) = py.import("catnip") {
        if let Ok(file) = catnip.getattr("__file__") {
            if let Ok(s) = file.extract::<String>() {
                if let Some(dir) = Path::new(&s).parent() {
                    paths.push(dir.to_path_buf());
                }
            }
        }
    }

    paths
}

/// Load a PureVM-only native stdlib module (e.g. `http`) and wrap it as a
/// `ModuleNamespace` of Python-callable functions and converted attributes.
///
/// The `.so` is loaded once into the shared registry, which the owning loader
/// keeps alive (so the raw call pointers held by `NativePluginFn` stay valid).
pub fn load_native_module(py: Python<'_>, registry: &SharedPluginRegistry, name: &str) -> PyResult<Py<PyAny>> {
    let lib_name = format!("libcatnip_{name}{}", plugin::native_suffix());
    let path = discover_plugin_paths(py)
        .iter()
        .map(|dir| dir.join(&lib_name))
        .find(|cand| cand.is_file())
        .ok_or_else(|| {
            PyFileNotFoundError::new_err(format!(
                "native plugin '{lib_name}' not found (looked in $CATNIP_STDLIB_PATH and the catnip package directory)"
            ))
        })?;

    // Load the plugin and read its callbacks while borrowing the registry.
    let (module_ns, obj_cbs) = {
        let mut reg = registry.borrow_mut();
        let ns = reg
            .load(&path, name)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        let cbs = reg.object_callbacks(name).unwrap_or_else(PluginObjectCallbacks::empty);
        (ns, cbs)
    };

    let mut rs_ns = ModuleNamespace::new(name.to_string());
    for (attr_name, val) in module_ns.attrs.iter() {
        let py_val: Py<PyAny> = if val.is_native_str() {
            // SAFETY: is_native_str() was verified on the line above, and val borrows a
            // live entry of module_ns.attrs, so the backing NativeString Arc is alive.
            let s = unsafe { val.as_native_str_ref() }.unwrap();
            if let Some(_short) = s.strip_prefix(PLUGIN_PREFIX) {
                let qualified = s.to_string();
                let short_name = qualified.rsplit("::").next().unwrap_or(&qualified).to_string();
                let call_fn = registry.borrow().call_fn(&qualified).ok_or_else(|| {
                    PyRuntimeError::new_err(format!("plugin '{name}': missing call pointer for '{qualified}'"))
                })?;
                Py::new(py, NativePluginFn::new(call_fn, short_name, obj_cbs.clone()))?.into_any()
            } else {
                vm_value_to_py_borrowed(py, *val)?
            }
        } else {
            vm_value_to_py_borrowed(py, *val)?
        };
        rs_ns.set_attr(attr_name.clone(), py_val);
    }

    Ok(Py::new(py, rs_ns)?.into_any())
}
