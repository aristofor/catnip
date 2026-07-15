// FILE: catnip_vm/src/host.rs
//! VmHost trait and PureHost -- pure Rust, no PyO3.
//!
//! VmHost abstracts host operations so the VM dispatch loop doesn't
//! depend on Python. PureHost is a 100% Rust implementation with
//! native globals and builtin functions.

use crate::error::{VMError, VMResult};
use crate::ops::{arith, collection, errors, string};
use crate::value::Value;
use indexmap::IndexMap;
use rug::Integer;
use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

/// Globals storage: shared reference to mutable ordered map.
pub type Globals = Rc<RefCell<IndexMap<String, Value>>>;

/// Drain every entry from a globals map, decref'ing each value.
///
/// Uses `mem::take` so the `RefCell` borrow is dropped before any decref;
/// a cascading Drop (nested module namespace, closure clearing its own
/// closure scope globals link) can never re-borrow the same cell.
///
/// After this call the map is empty. The caller is responsible for ensuring
/// no other handles to the globals remain live (otherwise a subsequent
/// decref through those handles would be a use-after-free or underflow).
pub(crate) fn drain_globals(globals: &Globals) {
    let map = std::mem::take(&mut *globals.borrow_mut());
    for (_, val) in map {
        val.decref();
    }
}

/// Binary operators delegated to the host.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    TrueDiv,
    FloorDiv,
    Mod,
    Pow,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Host operations required by the VM dispatch loop.
///
/// No `Python<'_>` in signatures -- pure Rust.
pub trait VmHost {
    // --- Global resolution ---
    fn lookup_global(&self, name: &str) -> VMResult<Option<Value>>;

    /// Whether `name` is bound in the host globals, without refcount effect
    /// (scope checks -- e.g. an enum pattern naming a type must be resolvable).
    fn has_global(&self, name: &str) -> bool;
    fn store_global(&self, name: &str, value: Value);
    fn delete_global(&self, name: &str);

    // --- Binary operator fallback ---
    fn binary_op(&self, op: BinaryOp, a: Value, b: Value) -> VMResult<Value>;

    // --- Iteration ---
    fn get_iter(&self, obj: Value) -> VMResult<Box<dyn ValueIter>>;

    // --- Function calls ---
    fn call_function(&self, func: Value, args: &[Value]) -> VMResult<Value>;

    // --- Attribute/item access ---
    fn obj_getattr(&self, obj: Value, name: &str) -> VMResult<Value>;
    fn obj_setattr(&self, obj: Value, name: &str, val: Value) -> VMResult<()>;
    fn obj_getitem(&self, obj: Value, key: Value) -> VMResult<Value>;
    fn obj_setitem(&self, obj: Value, key: Value, val: Value) -> VMResult<()>;

    // --- Method calls ---
    fn call_method(&self, obj: Value, method: &str, args: &[Value]) -> VMResult<Value>;

    // --- Membership test ---
    fn contains_op(&self, item: Value, container: Value) -> VMResult<bool>;

    // --- Opaque context (for bridge) ---
    fn context_raw(&self) -> Option<*const ()> {
        None
    }

    // --- Globals handle (for closure parent chain) ---
    fn globals_rc(&self) -> Option<Globals> {
        None
    }

    // --- Introspection ---
    /// Return all globals as (name, value) pairs.
    fn collect_all_globals(&self) -> Vec<(String, Value)> {
        Vec::new()
    }
}

/// Iterator over VM values.
pub trait ValueIter {
    fn next_value(&mut self) -> VMResult<Option<Value>>;
}

// ---------------------------------------------------------------------------
// PureHost -- 100% Rust host implementation
// ---------------------------------------------------------------------------

/// Compute (start, stop, step) indices for a slice of length `len`,
/// following Python's `slice.indices()` semantics exactly.
/// Ref: https://docs.python.org/3/reference/datamodel.html#slice.indices
fn slice_indices(start: Option<i64>, stop: Option<i64>, step: Option<i64>, len: i64) -> VMResult<(i64, i64, i64)> {
    let step = step.unwrap_or(1);
    if step == 0 {
        return Err(VMError::ValueError("slice step cannot be zero".into()));
    }

    let clamp = |idx: i64, low: i64, high: i64| idx.clamp(low, high);

    let (def_start, def_stop) = if step > 0 { (0, len) } else { (len - 1, -1) };

    let resolve = |idx: i64| if idx < 0 { idx + len } else { idx };

    let s = match start {
        Some(i) => {
            let r = resolve(i);
            if step > 0 {
                clamp(r, 0, len)
            } else {
                clamp(r, -1, len - 1)
            }
        }
        None => def_start,
    };
    let e = match stop {
        Some(i) => {
            let r = resolve(i);
            if step > 0 {
                clamp(r, 0, len)
            } else {
                clamp(r, -1, len - 1)
            }
        }
        None => def_stop,
    };

    Ok((s, e, step))
}

/// Collect indices produced by a slice into a Vec.
fn collect_slice_indices(start: i64, stop: i64, step: i64) -> Vec<usize> {
    let mut indices = Vec::new();
    let mut i = start;
    if step > 0 {
        while i < stop {
            indices.push(i as usize);
            i += step;
        }
    } else {
        while i > stop {
            indices.push(i as usize);
            i += step;
        }
    }
    indices
}

/// Apply a slice operation. Called from the VM when GetItem has arg=1.
/// start/stop/step are raw Values from the stack (integers or NIL).
pub fn apply_slice(obj: Value, start: Value, stop: Value, step: Value) -> VMResult<Value> {
    // Convert Value to Option<i64>, validating types
    let as_bound = |v: Value, name: &str| -> VMResult<Option<i64>> {
        if v.is_nil() {
            return Ok(None);
        }
        v.as_int()
            .map(Some)
            .ok_or_else(|| VMError::TypeError(format!("slice {name} must be an integer")))
    };
    let s = as_bound(start, "start")?;
    let e = as_bound(stop, "stop")?;
    let st = as_bound(step, "step")?;

    if obj.is_native_list() {
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
        let list = unsafe { obj.as_native_list_ref().unwrap() };
        let inner = list.as_slice_cloned();
        let len = inner.len() as i64;
        let (si, ei, step) = slice_indices(s, e, st, len)?;
        let indices = collect_slice_indices(si, ei, step);
        let result: Vec<Value> = indices
            .into_iter()
            .map(|i| {
                inner[i].clone_refcount();
                inner[i]
            })
            .collect();
        // Drop the cloned refs from as_slice_cloned
        for v in &inner {
            v.decref();
        }
        return Ok(Value::from_list(result));
    }
    if obj.is_native_str() {
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
        let s_ref = unsafe { obj.as_native_str_ref().unwrap() };
        let chars: Vec<char> = s_ref.chars().collect();
        let len = chars.len() as i64;
        let (si, ei, step) = slice_indices(s, e, st, len)?;
        let indices = collect_slice_indices(si, ei, step);
        let result: String = indices.into_iter().map(|i| chars[i]).collect();
        return Ok(Value::from_string(result));
    }
    if obj.is_native_tuple() {
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
        let tuple = unsafe { obj.as_native_tuple_ref().unwrap() };
        let items = tuple.as_slice();
        let len = items.len() as i64;
        let (si, ei, step) = slice_indices(s, e, st, len)?;
        let indices = collect_slice_indices(si, ei, step);
        let result: Vec<Value> = indices
            .into_iter()
            .map(|i| {
                items[i].clone_refcount();
                items[i]
            })
            .collect();
        return Ok(Value::from_tuple(result));
    }
    if obj.is_native_bytes() {
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeBytes> owned by the caller; the borrow does not outlive it.
        let bytes = unsafe { obj.as_native_bytes_ref().unwrap() };
        let data = bytes.as_bytes();
        let len = data.len() as i64;
        let (si, ei, step) = slice_indices(s, e, st, len)?;
        let indices = collect_slice_indices(si, ei, step);
        let result: Vec<u8> = indices.into_iter().map(|i| data[i]).collect();
        return Ok(Value::from_bytes(result));
    }
    Err(VMError::TypeError(format!("'{}' is not sliceable", obj.type_name())))
}

/// Host backed by Rust-owned globals with native builtins.
pub struct PureHost {
    globals: Globals,
    plugin_registry: Option<crate::plugin::SharedPluginRegistry>,
}

impl PureHost {
    /// Create a new PureHost with empty globals.
    pub fn new() -> Self {
        Self {
            globals: Rc::new(RefCell::new(IndexMap::new())),
            plugin_registry: None,
        }
    }

    /// Create a PureHost with standard builtins injected into globals.
    pub fn with_builtins() -> Self {
        let host = Self::new();
        {
            let mut g = host.globals.borrow_mut();
            // Constants
            g.insert("True".to_string(), Value::TRUE);
            g.insert("False".to_string(), Value::FALSE);
            g.insert("None".to_string(), Value::NIL);
            g.insert("true".to_string(), Value::TRUE);
            g.insert("false".to_string(), Value::FALSE);
            g.insert("nil".to_string(), Value::NIL);

            // Builtin functions (stored as NativeStr, dispatched by call_function)
            for name in BUILTIN_NAMES {
                g.insert(name.to_string(), Value::from_str(name));
            }

            // META object
            let meta = crate::value::NativeMeta::new();
            meta.set("main", Value::TRUE);
            g.insert("META".to_string(), Value::from_meta(meta));
        }
        host
    }

    /// Set the plugin registry for native plugin dispatch.
    pub fn set_plugin_registry(&mut self, registry: crate::plugin::SharedPluginRegistry) {
        self.plugin_registry = Some(registry);
    }

    /// Get a reference to globals.
    pub fn globals(&self) -> &Globals {
        &self.globals
    }

    /// Release every global's ref and empty the map. `Value` is `Copy` with no
    /// `Drop`, so dropping the `IndexMap` alone would leak every heap global
    /// (struct instance, list, module...). Called at pipeline `reset()` once the
    /// old VM (hence its closures, which share this `Rc`) is already dropped, so
    /// the host is the sole holder and the decref is balanced.
    pub fn clear_globals(&self) {
        drain_globals(&self.globals);
    }
}

/// Builtin function names available as globals.
pub(crate) const BUILTIN_NAMES: &[&str] = &[
    "abs",
    "len",
    "str",
    "int",
    "float",
    "bool",
    "type",
    "print",
    "import",
    "min",
    "max",
    "list",
    "tuple",
    "dict",
    "set",
    "range",
    "sorted",
    "reversed",
    "sum",
    "any",
    "all",
    "enumerate",
    "zip",
    "map",
    "filter",
    "fold",
    "reduce",
    "round",
    "pow",
    "divmod",
    "chr",
    "ord",
    "hex",
    "bin",
    "oct",
    "repr",
    "hash",
    "callable",
    "isinstance",
    "complex",
];

impl Default for PureHost {
    fn default() -> Self {
        Self::with_builtins()
    }
}

impl VmHost for PureHost {
    #[inline]
    fn lookup_global(&self, name: &str) -> VMResult<Option<Value>> {
        Ok(self.globals.borrow().get(name).copied())
    }

    #[inline]
    fn store_global(&self, name: &str, value: Value) {
        // The map owns one ref per entry. StoreScope transfers the popped ref (and
        // the type/import producers pass a freshly owned value), so the incoming
        // `value` becomes the map's ref -- no clone needed. Releasing the
        // overwritten entry keeps globals from growing by name reuse (the bounded
        // growth the long-lived MCP host relies on).
        if let Some(old) = self.globals.borrow_mut().insert(name.to_string(), value) {
            old.decref();
        }
    }

    #[inline]
    fn delete_global(&self, name: &str) {
        if let Some(old) = self.globals.borrow_mut().swap_remove(name) {
            old.decref();
        }
    }

    #[inline]
    fn has_global(&self, name: &str) -> bool {
        self.globals.borrow().contains_key(name)
    }

    fn binary_op(&self, op: BinaryOp, a: Value, b: Value) -> VMResult<Value> {
        // Dispatch to native binary operations
        match op {
            BinaryOp::Add => native_add(a, b),
            BinaryOp::Sub => native_sub(a, b),
            BinaryOp::Mul => native_mul(a, b),
            BinaryOp::TrueDiv => native_truediv(a, b),
            BinaryOp::FloorDiv => native_floordiv(a, b),
            BinaryOp::Mod => native_mod(a, b),
            BinaryOp::Pow => native_pow(a, b),
            BinaryOp::Lt => native_cmp(a, b, |o| o.is_lt()),
            BinaryOp::Le => native_cmp(a, b, |o| o.is_le()),
            BinaryOp::Gt => native_cmp(a, b, |o| o.is_gt()),
            BinaryOp::Ge => native_cmp(a, b, |o| o.is_ge()),
        }
    }

    fn get_iter(&self, obj: Value) -> VMResult<Box<dyn ValueIter>> {
        if obj.is_native_str() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
            let s = unsafe { obj.as_native_str_ref().unwrap().to_string() };
            Ok(Box::new(StrCharIter {
                chars: s.chars().collect(),
                pos: 0,
            }))
        } else if obj.is_native_list() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
            let list = unsafe { obj.as_native_list_ref().unwrap() };
            let items = list.as_slice_cloned();
            Ok(Box::new(VecIter { items, pos: 0 }))
        } else if obj.is_native_tuple() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
            let tuple = unsafe { obj.as_native_tuple_ref().unwrap() };
            let items: Vec<Value> = tuple.as_slice().to_vec();
            for v in &items {
                v.clone_refcount();
            }
            Ok(Box::new(VecIter { items, pos: 0 }))
        } else if obj.is_native_dict() {
            // Iterate over keys
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeDict> owned by the caller; the borrow does not outlive it.
            let dict = unsafe { obj.as_native_dict_ref().unwrap() };
            let keys = dict.keys();
            Ok(Box::new(VecIter { items: keys, pos: 0 }))
        } else if obj.is_native_set() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeSet> owned by the caller; the borrow does not outlive it.
            let set = unsafe { obj.as_native_set_ref().unwrap() };
            let vals = set.to_values();
            Ok(Box::new(VecIter { items: vals, pos: 0 }))
        } else if obj.is_native_bytes() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeBytes> owned by the caller; the borrow does not outlive it.
            let bytes = unsafe { obj.as_native_bytes_ref().unwrap() };
            let items: Vec<Value> = bytes.as_bytes().iter().map(|&b| Value::from_int(b as i64)).collect();
            Ok(Box::new(VecIter { items, pos: 0 }))
        } else {
            Err(VMError::TypeError(format!("cannot iterate over {:?}", obj)))
        }
    }

    fn call_function(&self, func: Value, args: &[Value]) -> VMResult<Value> {
        // Builtin functions stored as NativeStr names
        if func.is_native_str() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
            let name = unsafe { func.as_native_str_ref().unwrap() };
            // Plugin dispatch: __plugin::module::fn
            if crate::plugin::is_plugin_call(name) {
                if let Some(reg) = &self.plugin_registry {
                    if let Some(result) = reg.borrow().try_call(name, args) {
                        return result;
                    }
                }
                return Err(VMError::RuntimeError(format!("plugin function not found: '{}'", name)));
            }
            return call_builtin(name, args);
        }
        // Callable collections: list(idx), dict(key), tuple(idx), str(idx)
        if (func.is_native_list() || func.is_native_tuple() || func.is_native_dict() || func.is_native_str())
            && args.len() == 1
        {
            return self.obj_getitem(func, args[0]);
        }
        Err(VMError::TypeError(format!("cannot call {}", func.display_string())))
    }

    fn call_method(&self, obj: Value, method: &str, args: &[Value]) -> VMResult<Value> {
        // Complex methods
        if obj.is_complex() {
            // SAFETY: is_complex() was checked above, so the payload is a live complex value owned by the caller; the read does not outlive it.
            let (r, i) = unsafe { obj.as_complex_parts().unwrap() };
            return match method {
                "conjugate" => Ok(Value::from_complex(r, -i)),
                _ => Err(VMError::TypeError(format!("'complex' has no method '{}'", method))),
            };
        }
        // Plugin object method dispatch
        if obj.is_plugin_object() {
            // SAFETY: is_plugin_object() was checked above, so the payload is a live plugin object owned by the caller; the borrow does not outlive it.
            let (handle, cbs) = unsafe { obj.as_plugin_object_ref().unwrap() };
            let method_fn = cbs
                .method
                .ok_or_else(|| VMError::TypeError("plugin object does not support method calls".into()))?;
            if let Some(reg) = &self.plugin_registry {
                return reg
                    .borrow()
                    .call_method_on_object(handle, method_fn, method, args, &cbs);
            }
            return Err(VMError::RuntimeError("plugin registry not available".into()));
        }
        // Collection method dispatch
        if let Some(result) = collection::call_method(obj, method, args)? {
            return Ok(result);
        }
        // String methods with arguments
        if obj.is_native_str() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
            let s = unsafe { obj.as_native_str_ref().unwrap() };
            return str_method_with_args(s, method, args);
        }
        // Legacy file/http methods removed -- dispatched through PluginObject above
        Err(VMError::TypeError(format!(
            "'{}' has no method '{}'",
            obj.type_name(),
            method
        )))
    }

    fn obj_getattr(&self, obj: Value, name: &str) -> VMResult<Value> {
        // Complex attributes
        if obj.is_complex() {
            // SAFETY: is_complex() was checked above, so the payload is a live complex value owned by the caller; the read does not outlive it.
            let (r, i) = unsafe { obj.as_complex_parts().unwrap() };
            return match name {
                "real" => Ok(Value::from_float(r)),
                "imag" => Ok(Value::from_float(i)),
                _ => Err(VMError::AttributeError(format!(
                    "'complex' has no attribute '{}'",
                    name
                ))),
            };
        }
        // Plugin object attribute dispatch
        if obj.is_plugin_object() {
            // SAFETY: is_plugin_object() was checked above, so the payload is a live plugin object owned by the caller; the borrow does not outlive it.
            let (handle, cbs) = unsafe { obj.as_plugin_object_ref().unwrap() };
            let getattr_fn = cbs
                .getattr
                .ok_or_else(|| VMError::AttributeError("plugin object does not support attribute access".into()))?;
            if let Some(reg) = &self.plugin_registry {
                return reg.borrow().call_getattr_on_object(handle, getattr_fn, name, &cbs);
            }
            return Err(VMError::RuntimeError("plugin registry not available".into()));
        }
        // String methods (0-arg only -- called as attributes)
        if obj.is_native_str() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
            let s = unsafe { obj.as_native_str_ref().unwrap() };
            return str_method_dispatch(s, name);
        }
        // META attributes
        if obj.is_meta() {
            // SAFETY: is_meta() was checked above, so the payload is a live Arc<NativeMeta> owned by the caller; the borrow does not outlive it.
            let m = unsafe { obj.as_meta_ref().unwrap() };
            if let Some(v) = m.get(name) {
                v.clone_refcount();
                return Ok(v);
            }
            return Err(VMError::AttributeError(format!("META has no attribute '{}'", name)));
        }
        Err(VMError::TypeError(format!(
            "'{}' has no attribute '{}'",
            obj.type_name(),
            name
        )))
    }

    fn obj_setattr(&self, obj: Value, name: &str, val: Value) -> VMResult<()> {
        if obj.is_meta() {
            // SAFETY: is_meta() was checked above, so the payload is a live Arc<NativeMeta> owned by the caller; the borrow does not outlive it.
            let m = unsafe { obj.as_meta_ref().unwrap() };
            m.set(name, val);
            return Ok(());
        }
        Err(VMError::TypeError(format!(
            "cannot set attribute '{}' on {:?}",
            name, obj
        )))
    }

    fn obj_getitem(&self, obj: Value, key: Value) -> VMResult<Value> {
        if obj.is_native_str() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
            let s = unsafe { obj.as_native_str_ref().unwrap() };
            if let Some(i) = key.as_int() {
                return string::str_getitem(s, i);
            }
            return Err(VMError::TypeError("string indices must be integers".to_string()));
        }
        if obj.is_native_list() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
            let list = unsafe { obj.as_native_list_ref().unwrap() };
            if let Some(i) = key.as_int() {
                return list.get(i);
            }
            return Err(VMError::TypeError("list indices must be integers".into()));
        }
        if obj.is_native_tuple() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
            let tuple = unsafe { obj.as_native_tuple_ref().unwrap() };
            if let Some(i) = key.as_int() {
                return tuple.get(i);
            }
            return Err(VMError::TypeError("tuple indices must be integers".into()));
        }
        if obj.is_native_dict() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeDict> owned by the caller; the borrow does not outlive it.
            let dict = unsafe { obj.as_native_dict_ref().unwrap() };
            let k = key.to_key()?;
            return dict.get_item(&k);
        }
        if obj.is_native_bytes() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeBytes> owned by the caller; the borrow does not outlive it.
            let bytes = unsafe { obj.as_native_bytes_ref().unwrap() };
            if let Some(i) = key.as_int() {
                return bytes.get(i);
            }
            return Err(VMError::TypeError("bytes indices must be integers".into()));
        }
        Err(VMError::TypeError(format!(
            "'{}' is not subscriptable",
            obj.type_name()
        )))
    }

    fn obj_setitem(&self, obj: Value, key: Value, val: Value) -> VMResult<()> {
        if obj.is_native_list() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
            let list = unsafe { obj.as_native_list_ref().unwrap() };
            if let Some(i) = key.as_int() {
                return list.set(i, val);
            }
            return Err(VMError::TypeError("list indices must be integers".into()));
        }
        if obj.is_native_dict() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeDict> owned by the caller; the borrow does not outlive it.
            let dict = unsafe { obj.as_native_dict_ref().unwrap() };
            let k = key.to_key()?;
            dict.set_item(k, val);
            return Ok(());
        }
        Err(VMError::TypeError(format!(
            "'{}' does not support item assignment",
            obj.type_name()
        )))
    }

    fn globals_rc(&self) -> Option<Globals> {
        Some(Rc::clone(&self.globals))
    }

    fn collect_all_globals(&self) -> Vec<(String, Value)> {
        self.globals.borrow().iter().map(|(k, &v)| (k.clone(), v)).collect()
    }

    fn contains_op(&self, item: Value, container: Value) -> VMResult<bool> {
        if container.is_native_str() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
            let haystack = unsafe { container.as_native_str_ref().unwrap() };
            if item.is_native_str() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                let needle = unsafe { item.as_native_str_ref().unwrap() };
                return Ok(string::str_contains(haystack, needle));
            }
            return Err(VMError::TypeError(
                "'in' requires string as left operand for string containment".to_string(),
            ));
        }
        if container.is_native_list() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
            let list = unsafe { container.as_native_list_ref().unwrap() };
            return Ok(list.contains(item));
        }
        if container.is_native_tuple() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
            let tuple = unsafe { container.as_native_tuple_ref().unwrap() };
            return Ok(tuple.contains(item));
        }
        if container.is_native_dict() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeDict> owned by the caller; the borrow does not outlive it.
            let dict = unsafe { container.as_native_dict_ref().unwrap() };
            let key = item.to_key()?;
            return Ok(dict.contains_key(&key));
        }
        if container.is_native_set() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeSet> owned by the caller; the borrow does not outlive it.
            let set = unsafe { container.as_native_set_ref().unwrap() };
            let key = item.to_key()?;
            return Ok(set.contains(&key));
        }
        if container.is_native_bytes() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeBytes> owned by the caller; the borrow does not outlive it.
            let bytes = unsafe { container.as_native_bytes_ref().unwrap() };
            if let Some(i) = item.as_int() {
                if (0..=255).contains(&i) {
                    return Ok(bytes.contains_byte(i as u8));
                }
                return Err(VMError::ValueError("byte must be in range(0, 256)".into()));
            }
            return Err(VMError::TypeError("a bytes-like object is required".into()));
        }
        Err(VMError::TypeError(format!(
            "argument of type '{}' is not iterable",
            container.type_name()
        )))
    }
}

// ---------------------------------------------------------------------------
// String method dispatch (obj.method pattern)
// ---------------------------------------------------------------------------

/// Return a "method reference" for string methods.
/// In pure mode, we return the method result directly for 0-arg methods,
/// or a marker for methods that need arguments.
fn str_method_dispatch(s: &str, method: &str) -> VMResult<Value> {
    match method {
        "upper" => Ok(string::str_upper(s)),
        "lower" => Ok(string::str_lower(s)),
        "strip" => Ok(string::str_strip(s)),
        "lstrip" => Ok(string::str_lstrip(s)),
        "rstrip" => Ok(string::str_rstrip(s)),
        "title" => Ok(string::str_title(s)),
        "capitalize" => Ok(string::str_capitalize(s)),
        "isdigit" => Ok(Value::from_bool(string::str_isdigit(s))),
        "isalpha" => Ok(Value::from_bool(string::str_isalpha(s))),
        "isalnum" => Ok(Value::from_bool(string::str_isalnum(s))),
        _ => Err(VMError::TypeError(format!("'str' has no attribute '{}'", method))),
    }
}

/// String methods that take arguments (called via call_method).
fn str_method_with_args(s: &str, method: &str, args: &[Value]) -> VMResult<Value> {
    match method {
        // 0-arg methods (also accessible via getattr)
        "upper" => Ok(string::str_upper(s)),
        "lower" => Ok(string::str_lower(s)),
        "strip" => Ok(string::str_strip(s)),
        "lstrip" => Ok(string::str_lstrip(s)),
        "rstrip" => Ok(string::str_rstrip(s)),
        "title" => Ok(string::str_title(s)),
        "capitalize" => Ok(string::str_capitalize(s)),
        "isdigit" => Ok(Value::from_bool(string::str_isdigit(s))),
        "isalpha" => Ok(Value::from_bool(string::str_isalpha(s))),
        "isalnum" => Ok(Value::from_bool(string::str_isalnum(s))),
        // Methods with arguments
        "split" => {
            let sep = if args.is_empty() {
                " "
            } else if args[0].is_native_str() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                unsafe { args[0].as_native_str_ref().unwrap() }
            } else {
                return Err(VMError::TypeError("split() separator must be a string".into()));
            };
            let parts = string::str_split(s, sep);
            Ok(Value::from_list(parts))
        }
        "join" => {
            let arg = args
                .first()
                .ok_or_else(|| VMError::TypeError("join() takes 1 argument".into()))?;
            if arg.is_native_list() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                let list = unsafe { arg.as_native_list_ref().unwrap() };
                let items = list.as_slice_cloned();
                let result = string::str_join(s, &items);
                for v in &items {
                    v.decref();
                }
                result
            } else if arg.is_native_tuple() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
                let tuple = unsafe { arg.as_native_tuple_ref().unwrap() };
                string::str_join(s, tuple.as_slice())
            } else {
                Err(VMError::TypeError("join() argument must be iterable".into()))
            }
        }
        "replace" => {
            if args.len() < 2 {
                return Err(VMError::TypeError("replace() takes at least 2 arguments".into()));
            }
            let old = if args[0].is_native_str() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                unsafe { args[0].as_native_str_ref().unwrap() }
            } else {
                return Err(VMError::TypeError("replace() arguments must be strings".into()));
            };
            let new = if args[1].is_native_str() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                unsafe { args[1].as_native_str_ref().unwrap() }
            } else {
                return Err(VMError::TypeError("replace() arguments must be strings".into()));
            };
            Ok(string::str_replace(s, old, new))
        }
        "find" => {
            let sub = if let Some(a) = args.first() {
                if a.is_native_str() {
                    // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                    unsafe { a.as_native_str_ref().unwrap() }
                } else {
                    return Err(VMError::TypeError("find() argument must be a string".into()));
                }
            } else {
                return Err(VMError::TypeError("find() takes 1 argument".into()));
            };
            Ok(Value::from_int(string::str_find(s, sub)))
        }
        "count" => {
            let sub = if let Some(a) = args.first() {
                if a.is_native_str() {
                    // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                    unsafe { a.as_native_str_ref().unwrap() }
                } else {
                    return Err(VMError::TypeError("count() argument must be a string".into()));
                }
            } else {
                return Err(VMError::TypeError("count() takes 1 argument".into()));
            };
            Ok(Value::from_int(string::str_count(s, sub)))
        }
        "startswith" => {
            let prefix = if let Some(a) = args.first() {
                if a.is_native_str() {
                    // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                    unsafe { a.as_native_str_ref().unwrap() }
                } else {
                    return Err(VMError::TypeError("startswith() argument must be a string".into()));
                }
            } else {
                return Err(VMError::TypeError("startswith() takes 1 argument".into()));
            };
            Ok(Value::from_bool(string::str_startswith(s, prefix)))
        }
        "endswith" => {
            let suffix = if let Some(a) = args.first() {
                if a.is_native_str() {
                    // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                    unsafe { a.as_native_str_ref().unwrap() }
                } else {
                    return Err(VMError::TypeError("endswith() argument must be a string".into()));
                }
            } else {
                return Err(VMError::TypeError("endswith() takes 1 argument".into()));
            };
            Ok(Value::from_bool(string::str_endswith(s, suffix)))
        }
        "contains" => {
            let sub = if let Some(a) = args.first() {
                if a.is_native_str() {
                    // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                    unsafe { a.as_native_str_ref().unwrap() }
                } else {
                    return Err(VMError::TypeError("contains() argument must be a string".into()));
                }
            } else {
                return Err(VMError::TypeError("contains() takes 1 argument".into()));
            };
            Ok(Value::from_bool(string::str_contains(s, sub)))
        }
        _ => Err(VMError::TypeError(format!("'str' has no method '{}'", method))),
    }
}

// ---------------------------------------------------------------------------
// Native binary operations (same patterns as catnip_rs/src/vm/core.rs)
// ---------------------------------------------------------------------------

// Helpers re-exported from arith module.
use arith::{bigint_cmp, to_f64};

#[inline]
fn native_add(a: Value, b: Value) -> VMResult<Value> {
    // Numerics (shared with catnip_rs)
    if let Ok(v) = arith::numeric_add(a, b) {
        return Ok(v);
    }
    // String concat
    if a.is_native_str() && b.is_native_str() {
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
        let sa = unsafe { a.as_native_str_ref().unwrap() };
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
        let sb = unsafe { b.as_native_str_ref().unwrap() };
        return Ok(string::str_concat(sa, sb));
    }
    // List concat
    if a.is_native_list() && b.is_native_list() {
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
        let la = unsafe { a.as_native_list_ref().unwrap() };
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
        let lb = unsafe { b.as_native_list_ref().unwrap() };
        let mut items = la.as_slice_cloned();
        let items_b = lb.as_slice_cloned();
        items.extend(items_b);
        return Ok(Value::from_list(items));
    }
    // Tuple concat
    if a.is_native_tuple() && b.is_native_tuple() {
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
        let ta = unsafe { a.as_native_tuple_ref().unwrap() };
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
        let tb = unsafe { b.as_native_tuple_ref().unwrap() };
        let mut items: Vec<Value> = ta.as_slice().to_vec();
        items.extend_from_slice(tb.as_slice());
        for v in &items {
            v.clone_refcount();
        }
        return Ok(Value::from_tuple(items));
    }
    // Bytes concat
    if a.is_native_bytes() && b.is_native_bytes() {
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeBytes> owned by the caller; the borrow does not outlive it.
        let ba = unsafe { a.as_native_bytes_ref().unwrap() };
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeBytes> owned by the caller; the borrow does not outlive it.
        let bb = unsafe { b.as_native_bytes_ref().unwrap() };
        let mut data = ba.as_bytes().to_vec();
        data.extend_from_slice(bb.as_bytes());
        return Ok(Value::from_bytes(data));
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_ADD.into()))
}

#[inline]
fn native_sub(a: Value, b: Value) -> VMResult<Value> {
    arith::numeric_sub(a, b)
}

#[inline]
fn native_mul(a: Value, b: Value) -> VMResult<Value> {
    if let Ok(v) = arith::numeric_mul(a, b) {
        return Ok(v);
    }
    // String repeat: str * int
    if a.is_native_str() {
        if let Some(n) = b.as_int() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
            let s = unsafe { a.as_native_str_ref().unwrap() };
            return string::str_repeat(s, n);
        }
    }
    if b.is_native_str() {
        if let Some(n) = a.as_int() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
            let s = unsafe { b.as_native_str_ref().unwrap() };
            return string::str_repeat(s, n);
        }
    }
    // List repeat: list * int
    if a.is_native_list() {
        if let Some(n) = b.as_int() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
            let list = unsafe { a.as_native_list_ref().unwrap() };
            let items = list.as_slice_cloned();
            let mut result = Vec::with_capacity(items.len() * n.max(0) as usize);
            for _ in 0..n.max(0) {
                for v in &items {
                    v.clone_refcount();
                    result.push(*v);
                }
            }
            for v in &items {
                v.decref(); // undo as_slice_cloned
            }
            return Ok(Value::from_list(result));
        }
    }
    if b.is_native_list() {
        if let Some(n) = a.as_int() {
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
            let list = unsafe { b.as_native_list_ref().unwrap() };
            let items = list.as_slice_cloned();
            let mut result = Vec::with_capacity(items.len() * n.max(0) as usize);
            for _ in 0..n.max(0) {
                for v in &items {
                    v.clone_refcount();
                    result.push(*v);
                }
            }
            for v in &items {
                v.decref();
            }
            return Ok(Value::from_list(result));
        }
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_MUL.into()))
}

#[inline]
fn native_truediv(a: Value, b: Value) -> VMResult<Value> {
    arith::numeric_div(a, b)
}

#[inline]
fn native_floordiv(a: Value, b: Value) -> VMResult<Value> {
    arith::numeric_floordiv(a, b)
}

#[inline]
fn native_mod(a: Value, b: Value) -> VMResult<Value> {
    arith::numeric_mod(a, b)
}

#[inline]
fn native_pow(a: Value, b: Value) -> VMResult<Value> {
    arith::numeric_pow(a, b)
}

#[inline]
fn native_cmp<F>(a: Value, b: Value, pred: F) -> VMResult<Value>
where
    F: Fn(std::cmp::Ordering) -> bool,
{
    // SmallInt cmp
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(pred(ai.cmp(&bi))));
    }
    // BigInt cmp
    if a.is_bigint() || b.is_bigint() {
        if let Some(result) = bigint_cmp(a, b, |x, y| pred(x.cmp(y))) {
            return Ok(Value::from_bool(result));
        }
    }
    // Float cmp
    if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
        return Ok(Value::from_bool(af.partial_cmp(&bf).is_some_and(&pred)));
    }
    // String cmp
    if a.is_native_str() && b.is_native_str() {
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
        let sa = unsafe { a.as_native_str_ref().unwrap() };
        // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
        let sb = unsafe { b.as_native_str_ref().unwrap() };
        return Ok(Value::from_bool(pred(sa.cmp(sb))));
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_COMPARISON.into()))
}

// ---------------------------------------------------------------------------
// Builtin function dispatch
// ---------------------------------------------------------------------------

/// Python-compatible banker's rounding (tie-to-even).
fn round_half_even(x: f64) -> f64 {
    let frac = x.fract().abs();
    if frac == 0.5 {
        // Tie: round to nearest even integer
        let floor = x.floor();
        let ceil = x.ceil();
        if floor % 2.0 == 0.0 { floor } else { ceil }
    } else {
        x.round()
    }
}

fn call_builtin(name: &str, args: &[Value]) -> VMResult<Value> {
    match name {
        "abs" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("abs() takes 1 argument".into()))?;
            if let Some(i) = a.as_int() {
                return Ok(Value::from_int(i.abs()));
            }
            if let Some(f) = a.as_float() {
                return Ok(Value::from_float(f.abs()));
            }
            if a.is_bigint() {
                // SAFETY: is_bigint() was checked above, so the payload is a live Arc<Integer> owned by the caller; the borrow does not outlive it.
                let n = unsafe { a.as_bigint_ref().unwrap() };
                return Ok(Value::from_bigint_or_demote(n.clone().abs()));
            }
            if a.is_complex() {
                // SAFETY: is_complex() was checked above, so the payload is a live complex value owned by the caller; the read does not outlive it.
                let (r, i) = unsafe { a.as_complex_parts().unwrap() };
                return Ok(Value::from_float((r * r + i * i).sqrt()));
            }
            Err(VMError::TypeError("bad operand type for abs()".into()))
        }
        "len" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("len() takes 1 argument".into()))?;
            if a.is_native_str() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                let s = unsafe { a.as_native_str_ref().unwrap() };
                return Ok(Value::from_int(string::str_len(s) as i64));
            }
            if a.is_native_list() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                let list = unsafe { a.as_native_list_ref().unwrap() };
                return Ok(Value::from_int(list.len() as i64));
            }
            if a.is_native_tuple() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
                let tuple = unsafe { a.as_native_tuple_ref().unwrap() };
                return Ok(Value::from_int(tuple.len() as i64));
            }
            if a.is_native_dict() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeDict> owned by the caller; the borrow does not outlive it.
                let dict = unsafe { a.as_native_dict_ref().unwrap() };
                return Ok(Value::from_int(dict.len() as i64));
            }
            if a.is_native_set() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeSet> owned by the caller; the borrow does not outlive it.
                let set = unsafe { a.as_native_set_ref().unwrap() };
                return Ok(Value::from_int(set.len() as i64));
            }
            if a.is_native_bytes() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeBytes> owned by the caller; the borrow does not outlive it.
                let bytes = unsafe { a.as_native_bytes_ref().unwrap() };
                return Ok(Value::from_int(bytes.len() as i64));
            }
            Err(VMError::TypeError(format!(
                "object of type '{}' has no len()",
                a.type_name()
            )))
        }
        "str" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("str() takes 1 argument".into()))?;
            Ok(Value::from_string(a.display_string()))
        }
        "int" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("int() takes 1 argument".into()))?;
            if let Some(i) = a.as_int() {
                return Ok(Value::from_int(i));
            }
            if let Some(f) = a.as_float() {
                return Ok(Value::from_int(f as i64));
            }
            if a.is_native_str() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                let s = unsafe { a.as_native_str_ref().unwrap() };
                match s.trim().parse::<i64>() {
                    Ok(i) => {
                        if let Some(v) = Value::try_from_int(i) {
                            return Ok(v);
                        }
                        return Ok(Value::from_bigint(Integer::from(i)));
                    }
                    Err(_) => return Err(VMError::ValueError(format!("invalid literal for int(): '{}'", s))),
                }
            }
            Err(VMError::TypeError(format!(
                "int() argument must be a string or number, not {:?}",
                a
            )))
        }
        "float" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("float() takes 1 argument".into()))?;
            if let Some(f) = a.as_float() {
                return Ok(Value::from_float(f));
            }
            if let Some(i) = a.as_int() {
                return Ok(Value::from_float(i as f64));
            }
            if a.is_native_str() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                let s = unsafe { a.as_native_str_ref().unwrap() };
                match s.trim().parse::<f64>() {
                    Ok(f) => return Ok(Value::from_float(f)),
                    Err(_) => {
                        return Err(VMError::ValueError(format!(
                            "could not convert string to float: '{}'",
                            s
                        )));
                    }
                }
            }
            Err(VMError::TypeError(format!(
                "float() argument must be a string or number, not {:?}",
                a
            )))
        }
        "bool" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("bool() takes 1 argument".into()))?;
            Ok(Value::from_bool(a.is_truthy()))
        }
        "type" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("type() takes 1 argument".into()))?;
            Ok(Value::from_str(a.type_name()))
        }
        "print" => {
            let mut out = std::io::stdout().lock();
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    write!(out, " ").ok();
                }
                write!(out, "{}", arg.display_string()).ok();
            }
            writeln!(out).ok();
            Ok(Value::NIL)
        }
        "min" => {
            if args.is_empty() {
                return Err(VMError::TypeError("min expected at least 1 argument".into()));
            }
            let mut best = args[0];
            for &v in &args[1..] {
                if native_cmp(v, best, |o| o.is_lt())?.as_bool() == Some(true) {
                    best = v;
                }
            }
            Ok(best)
        }
        "max" => {
            if args.is_empty() {
                return Err(VMError::TypeError("max expected at least 1 argument".into()));
            }
            let mut best = args[0];
            for &v in &args[1..] {
                if native_cmp(v, best, |o| o.is_gt())?.as_bool() == Some(true) {
                    best = v;
                }
            }
            Ok(best)
        }
        "list" => {
            if args.is_empty() {
                return Ok(Value::from_list(vec![]));
            }
            // list(iterable) -- convert iterable to list
            let a = &args[0];
            if a.is_native_list() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                let list = unsafe { a.as_native_list_ref().unwrap() };
                return Ok(Value::from_list(list.copy()));
            }
            if a.is_native_tuple() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
                let tuple = unsafe { a.as_native_tuple_ref().unwrap() };
                let items: Vec<Value> = tuple.as_slice().to_vec();
                for v in &items {
                    v.clone_refcount();
                }
                return Ok(Value::from_list(items));
            }
            if a.is_native_str() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                let s = unsafe { a.as_native_str_ref().unwrap() };
                let items: Vec<Value> = s.chars().map(|c| Value::from_string(c.to_string())).collect();
                return Ok(Value::from_list(items));
            }
            Err(VMError::TypeError("list() argument must be an iterable".into()))
        }
        "tuple" => {
            if args.is_empty() {
                return Ok(Value::from_tuple(vec![]));
            }
            let a = &args[0];
            if a.is_native_tuple() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
                let tuple = unsafe { a.as_native_tuple_ref().unwrap() };
                let items: Vec<Value> = tuple.as_slice().to_vec();
                for v in &items {
                    v.clone_refcount();
                }
                return Ok(Value::from_tuple(items));
            }
            if a.is_native_list() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                let list = unsafe { a.as_native_list_ref().unwrap() };
                return Ok(Value::from_tuple(list.copy()));
            }
            Err(VMError::TypeError("tuple() argument must be an iterable".into()))
        }
        "dict" => {
            if args.is_empty() {
                return Ok(Value::from_empty_dict());
            }
            // dict(iterable_of_pairs) - extract pairs from list/tuple of tuples/lists.
            // set_item does its own clone_refcount, so callers must not pre-increment.
            let iterable = &args[0];
            let dict = Value::from_empty_dict();
            // SAFETY: dict was just created by Value::from_empty_dict(), so the payload is a live Arc<NativeDict> owned here; the borrow does not outlive it.
            let d = unsafe { dict.as_native_dict_ref().unwrap() };

            // Collect items (as_slice_cloned / clone_refcount give owned refs).
            let items: Vec<Value> = if iterable.is_native_list() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                unsafe { iterable.as_native_list_ref().unwrap().as_slice_cloned() }
            } else if iterable.is_native_tuple() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
                let t = unsafe { iterable.as_native_tuple_ref().unwrap() };
                let s = t.as_slice();
                s.iter()
                    .map(|v| {
                        v.clone_refcount();
                        *v
                    })
                    .collect()
            } else {
                return Err(VMError::TypeError(
                    "dict(): argument must be a list or tuple of pairs".into(),
                ));
            };

            for (index, pair) in items.iter().enumerate() {
                let result = if pair.is_native_tuple() {
                    // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
                    let t = unsafe { pair.as_native_tuple_ref().unwrap() };
                    let s = t.as_slice();
                    if s.len() == 2 {
                        // as_slice borrows - set_item increments, pair.decref frees tuple+elements.
                        match s[0].to_key() {
                            Ok(key) => {
                                d.set_item(key, s[1]);
                                Ok(())
                            }
                            Err(err) => Err(err),
                        }
                    } else {
                        Err(VMError::TypeError(format!(
                            "dict(): expected 2-element tuple, got {}",
                            s.len()
                        )))
                    }
                } else if pair.is_native_list() {
                    // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                    let l = unsafe { pair.as_native_list_ref().unwrap() };
                    let s = l.as_slice_cloned();
                    let result = if s.len() == 2 {
                        match s[0].to_key() {
                            Ok(key) => {
                                d.set_item(key, s[1]);
                                Ok(())
                            }
                            Err(err) => Err(err),
                        }
                    } else {
                        Err(VMError::TypeError(format!(
                            "dict(): expected 2-element list, got {}",
                            s.len()
                        )))
                    };
                    for value in &s {
                        value.decref();
                    }
                    result
                } else {
                    Err(VMError::TypeError(
                        "dict(): iterable must yield pairs (tuples or lists)".into(),
                    ))
                };

                pair.decref();

                if let Err(err) = result {
                    for remaining in items.iter().skip(index + 1) {
                        remaining.decref();
                    }
                    return Err(err);
                }
            }
            Ok(dict)
        }
        "set" => {
            if args.is_empty() {
                return Ok(Value::from_set(indexmap::IndexSet::new()));
            }
            let a = &args[0];
            if a.is_native_list() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                let list = unsafe { a.as_native_list_ref().unwrap() };
                let items = list.as_slice_cloned();
                let mut set = indexmap::IndexSet::new();
                for v in &items {
                    set.insert(v.to_key()?);
                    v.decref();
                }
                return Ok(Value::from_set(set));
            }
            Err(VMError::TypeError("set() argument must be an iterable".into()))
        }
        "range" => {
            let (start, end, step) = match args.len() {
                1 => (
                    0,
                    args[0]
                        .as_int()
                        .ok_or_else(|| VMError::TypeError("range() requires int".into()))?,
                    1,
                ),
                2 => (
                    args[0]
                        .as_int()
                        .ok_or_else(|| VMError::TypeError("range() requires int".into()))?,
                    args[1]
                        .as_int()
                        .ok_or_else(|| VMError::TypeError("range() requires int".into()))?,
                    1,
                ),
                3 => (
                    args[0]
                        .as_int()
                        .ok_or_else(|| VMError::TypeError("range() requires int".into()))?,
                    args[1]
                        .as_int()
                        .ok_or_else(|| VMError::TypeError("range() requires int".into()))?,
                    args[2]
                        .as_int()
                        .ok_or_else(|| VMError::TypeError("range() requires int".into()))?,
                ),
                _ => return Err(VMError::TypeError("range() takes 1 to 3 arguments".into())),
            };
            if step == 0 {
                return Err(VMError::ValueError("range() step must not be zero".into()));
            }
            let mut items = Vec::new();
            let mut i = start;
            if step > 0 {
                while i < end {
                    items.push(Value::from_int(i));
                    i += step;
                }
            } else {
                while i > end {
                    items.push(Value::from_int(i));
                    i += step;
                }
            }
            Ok(Value::from_list(items))
        }
        "sorted" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("sorted() takes 1 argument".into()))?;
            if a.is_native_list() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                let list = unsafe { a.as_native_list_ref().unwrap() };
                let items = list.copy();
                let result = Value::from_list(items);
                // SAFETY: result was just created by Value::from_list(), so the payload is a live Arc<NativeList> owned here; the borrow does not outlive it.
                let rlist = unsafe { result.as_native_list_ref().unwrap() };
                rlist.sort()?;
                return Ok(result);
            }
            Err(VMError::TypeError("sorted() argument must be iterable".into()))
        }
        "reversed" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("reversed() takes 1 argument".into()))?;
            if a.is_native_list() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                let list = unsafe { a.as_native_list_ref().unwrap() };
                let mut items = list.copy();
                items.reverse();
                return Ok(Value::from_list(items));
            }
            if a.is_native_tuple() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
                let tuple = unsafe { a.as_native_tuple_ref().unwrap() };
                let mut items: Vec<Value> = tuple.as_slice().to_vec();
                for v in &items {
                    v.clone_refcount();
                }
                items.reverse();
                return Ok(Value::from_list(items));
            }
            Err(VMError::TypeError("reversed() argument must be a sequence".into()))
        }
        "sum" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("sum() takes 1 argument".into()))?;
            if a.is_native_list() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                let list = unsafe { a.as_native_list_ref().unwrap() };
                let items = list.as_slice_cloned();
                let mut total = Value::from_int(0);
                for v in &items {
                    total = native_add(total, *v)?;
                    v.decref();
                }
                return Ok(total);
            }
            Err(VMError::TypeError("sum() argument must be iterable".into()))
        }
        "any" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("any() takes 1 argument".into()))?;
            if a.is_native_list() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                let list = unsafe { a.as_native_list_ref().unwrap() };
                let items = list.as_slice_cloned();
                let result = items.iter().any(|v| v.is_truthy());
                for v in &items {
                    v.decref();
                }
                return Ok(Value::from_bool(result));
            }
            Err(VMError::TypeError("any() argument must be iterable".into()))
        }
        "all" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("all() takes 1 argument".into()))?;
            if a.is_native_list() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                let list = unsafe { a.as_native_list_ref().unwrap() };
                let items = list.as_slice_cloned();
                let result = items.iter().all(|v| v.is_truthy());
                for v in &items {
                    v.decref();
                }
                return Ok(Value::from_bool(result));
            }
            Err(VMError::TypeError("all() argument must be iterable".into()))
        }
        "enumerate" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("enumerate() takes 1 argument".into()))?;
            let start = if args.len() > 1 {
                args[1].as_int().unwrap_or(0)
            } else {
                0
            };
            if a.is_native_list() {
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                let list = unsafe { a.as_native_list_ref().unwrap() };
                let items = list.as_slice_cloned();
                let result: Vec<Value> = items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| Value::from_tuple(vec![Value::from_int(i as i64 + start), *v]))
                    .collect();
                return Ok(Value::from_list(result));
            }
            Err(VMError::TypeError("enumerate() argument must be iterable".into()))
        }
        "zip" => {
            if args.len() < 2 {
                return Err(VMError::TypeError("zip() takes at least 2 arguments".into()));
            }
            // Collect iterables as Vec<Vec<Value>>
            let mut iters: Vec<Vec<Value>> = Vec::new();
            for a in args {
                if a.is_native_list() {
                    // SAFETY: the tag was checked above, so the payload is a live Arc<NativeList> owned by the caller; the borrow does not outlive it.
                    let list = unsafe { a.as_native_list_ref().unwrap() };
                    iters.push(list.as_slice_cloned());
                } else if a.is_native_tuple() {
                    // SAFETY: the tag was checked above, so the payload is a live Arc<NativeTuple> owned by the caller; the borrow does not outlive it.
                    let tuple = unsafe { a.as_native_tuple_ref().unwrap() };
                    let items: Vec<Value> = tuple.as_slice().to_vec();
                    for v in &items {
                        v.clone_refcount();
                    }
                    iters.push(items);
                } else {
                    return Err(VMError::TypeError("zip() arguments must be iterables".into()));
                }
            }
            let min_len = iters.iter().map(|v| v.len()).min().unwrap_or(0);
            let mut result = Vec::with_capacity(min_len);
            for i in 0..min_len {
                let tuple_items: Vec<Value> = iters.iter().map(|iter| iter[i]).collect();
                // Refcounts already incremented by as_slice_cloned
                result.push(Value::from_tuple(tuple_items));
            }
            // Decref unused tail elements
            for iter_vals in &iters {
                for v in iter_vals.iter().skip(min_len) {
                    v.decref();
                }
            }
            Ok(Value::from_list(result))
        }
        // --- Batch 1: numerics + string utils ---
        "round" => {
            if args.is_empty() || args.len() > 2 {
                return Err(VMError::TypeError(format!(
                    "round() takes 1 or 2 arguments ({} given)",
                    args.len()
                )));
            }
            let a = args[0];
            let ndigits = if args.len() == 2 {
                args[1]
                    .as_int()
                    .ok_or_else(|| VMError::TypeError("ndigits must be an integer".into()))?
            } else {
                0
            };
            if let Some(f) = a.as_float() {
                if ndigits == 0 {
                    // Python tie-to-even (banker's rounding)
                    return Ok(Value::from_int(round_half_even(f) as i64));
                }
                let factor = 10f64.powi(ndigits as i32);
                return Ok(Value::from_float(round_half_even(f * factor) / factor));
            }
            if let Some(i) = a.as_int() {
                if ndigits >= 0 {
                    return Ok(Value::from_int(i));
                }
                let factor = 10i64.pow((-ndigits) as u32);
                return Ok(Value::from_int(
                    round_half_even(i as f64 / factor as f64) as i64 * factor,
                ));
            }
            Err(VMError::TypeError("round() requires a numeric value".into()))
        }
        "pow" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(VMError::TypeError(format!(
                    "pow() takes 2 or 3 arguments ({} given)",
                    args.len()
                )));
            }
            let result = crate::ops::arith::numeric_pow(args[0], args[1])?;
            if args.len() == 3 {
                // pow(base, exp, mod)
                let m = args[2]
                    .as_int()
                    .ok_or_else(|| VMError::TypeError("pow() 3rd argument must be an integer".into()))?;
                if m == 0 {
                    return Err(VMError::ValueError("pow() 3rd argument cannot be 0".into()));
                }
                let r = result
                    .as_int()
                    .ok_or_else(|| VMError::TypeError("pow() with 3 arguments requires integer result".into()))?;
                return Ok(Value::from_int(((r % m) + m) % m));
            }
            Ok(result)
        }
        "divmod" => {
            if args.len() != 2 {
                return Err(VMError::TypeError(format!(
                    "divmod() takes 2 arguments ({} given)",
                    args.len()
                )));
            }
            let (a, b) = (args[0], args[1]);
            if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
                if bi == 0 {
                    return Err(VMError::ZeroDivisionError("integer division or modulo by zero".into()));
                }
                let q = crate::ops::arith::i64_div_floor(ai, bi);
                let r = crate::ops::arith::i64_mod_floor(ai, bi);
                return Ok(Value::from_tuple(vec![Value::from_int(q), Value::from_int(r)]));
            }
            // BigInt divmod (before float fallback to keep exact precision)
            if (a.is_bigint() || a.as_int().is_some()) && (b.is_bigint() || b.as_int().is_some()) {
                if let (Some(ai), Some(bi)) = (crate::ops::arith::to_bigint(a), crate::ops::arith::to_bigint(b)) {
                    if bi == 0 {
                        return Err(VMError::ZeroDivisionError("integer division or modulo by zero".into()));
                    }
                    let (q, r) = ai.div_rem_floor(bi);
                    return Ok(Value::from_tuple(vec![
                        Value::from_bigint_or_demote(q),
                        Value::from_bigint_or_demote(r),
                    ]));
                }
            }
            if let (Some(af), Some(bf)) = (crate::ops::arith::to_f64(a), crate::ops::arith::to_f64(b)) {
                if bf == 0.0 {
                    return Err(VMError::ZeroDivisionError("float divmod() by zero".into()));
                }
                let q = (af / bf).floor();
                let r = af - q * bf;
                return Ok(Value::from_tuple(vec![Value::from_float(q), Value::from_float(r)]));
            }
            Err(VMError::TypeError("divmod() requires numeric arguments".into()))
        }
        "chr" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("chr() takes 1 argument".into()))?;
            let i = a
                .as_int()
                .ok_or_else(|| VMError::TypeError("chr() requires an integer".into()))?;
            let c = char::from_u32(i as u32)
                .ok_or_else(|| VMError::ValueError(format!("chr() arg not in range: {}", i)))?;
            Ok(Value::from_string(c.to_string()))
        }
        "ord" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("ord() takes 1 argument".into()))?;
            if !a.is_native_str() {
                return Err(VMError::TypeError("ord() requires a string argument".into()));
            }
            // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
            let s = unsafe { a.as_native_str_ref().unwrap() };
            let mut chars = s.chars();
            let c = chars
                .next()
                .ok_or_else(|| VMError::TypeError("ord() expected a character, got empty string".into()))?;
            if chars.next().is_some() {
                return Err(VMError::TypeError(format!(
                    "ord() expected a character, got string of length {}",
                    s.chars().count()
                )));
            }
            Ok(Value::from_int(c as i64))
        }
        "hex" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("hex() takes 1 argument".into()))?;
            let i = a
                .as_int()
                .ok_or_else(|| VMError::TypeError("hex() requires an integer".into()))?;
            if i < 0 {
                Ok(Value::from_string(format!("-0x{:x}", -i)))
            } else {
                Ok(Value::from_string(format!("0x{:x}", i)))
            }
        }
        "bin" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("bin() takes 1 argument".into()))?;
            let i = a
                .as_int()
                .ok_or_else(|| VMError::TypeError("bin() requires an integer".into()))?;
            if i < 0 {
                Ok(Value::from_string(format!("-0b{:b}", -i)))
            } else {
                Ok(Value::from_string(format!("0b{:b}", i)))
            }
        }
        "oct" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("oct() takes 1 argument".into()))?;
            let i = a
                .as_int()
                .ok_or_else(|| VMError::TypeError("oct() requires an integer".into()))?;
            if i < 0 {
                Ok(Value::from_string(format!("-0o{:o}", -i)))
            } else {
                Ok(Value::from_string(format!("0o{:o}", i)))
            }
        }
        "repr" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("repr() takes 1 argument".into()))?;
            Ok(Value::from_string(a.repr_string()))
        }
        // --- Batch 2: type introspection ---
        "hash" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("hash() takes 1 argument".into()))?;
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let key = a
                .to_key()
                .map_err(|_| VMError::TypeError(format!("unhashable type: '{}'", a.type_name())))?;
            let mut hasher = DefaultHasher::new();
            key.hash(&mut hasher);
            let h = hasher.finish() as i64;
            Ok(Value::try_from_int(h).unwrap_or_else(|| Value::from_bigint(Integer::from(h))))
        }
        "callable" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("callable() takes 1 argument".into()))?;
            let is_callable = a.is_vmfunc() || a.is_closure() || a.is_struct_type() || {
                // Only builtin names are callable strings, not arbitrary strings
                // SAFETY: the tag was checked above, so the payload is a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                a.is_native_str() && BUILTIN_NAMES.contains(&unsafe { a.as_native_str_ref().unwrap() })
            };
            Ok(Value::from_bool(is_callable))
        }
        "complex" => {
            let real = args.first().map_or(Ok(0.0), |v| {
                arith::to_f64(*v).ok_or_else(|| VMError::TypeError("complex() real must be a number".into()))
            })?;
            let imag = args.get(1).map_or(Ok(0.0), |v| {
                arith::to_f64(*v).ok_or_else(|| VMError::TypeError("complex() imag must be a number".into()))
            })?;
            Ok(Value::from_complex(real, imag))
        }
        _ => Err(VMError::NameError(format!("builtin '{}' not found", name))),
    }
}

// ---------------------------------------------------------------------------
// Native iterators
// ---------------------------------------------------------------------------

/// Character iterator for NativeStr.
struct StrCharIter {
    chars: Vec<char>,
    pos: usize,
}

impl ValueIter for StrCharIter {
    fn next_value(&mut self) -> VMResult<Option<Value>> {
        if self.pos < self.chars.len() {
            let ch = self.chars[self.pos];
            self.pos += 1;
            Ok(Some(Value::from_string(ch.to_string())))
        } else {
            Ok(None)
        }
    }
}

/// Build an iterator over pre-collected values (each already holding a
/// refcount). Lets the VM iterate dict/set keys it materialized through the
/// registry, which the host's `get_iter` cannot reach for struct keys.
pub(crate) fn vec_value_iter(items: Vec<Value>) -> Box<dyn ValueIter> {
    Box::new(VecIter { items, pos: 0 })
}

/// Generic iterator over a pre-collected Vec<Value>.
struct VecIter {
    items: Vec<Value>,
    pos: usize,
}

impl ValueIter for VecIter {
    fn next_value(&mut self) -> VMResult<Option<Value>> {
        if self.pos < self.items.len() {
            let v = self.items[self.pos];
            self.pos += 1;
            // Refcount already incremented during construction
            Ok(Some(v))
        } else {
            Ok(None)
        }
    }
}

impl Drop for VecIter {
    fn drop(&mut self) {
        // Decref unconsumed items
        for v in self.items.iter().skip(self.pos) {
            v.decref();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
