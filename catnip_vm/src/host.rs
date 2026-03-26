// FILE: catnip_vm/src/host.rs
//! VmHost trait and PureHost -- pure Rust, no PyO3.
//!
//! VmHost abstracts host operations so the VM dispatch loop doesn't
//! depend on Python. PureHost is a 100% Rust implementation with
//! native globals and builtin functions.

use crate::error::{VMError, VMResult};
use crate::ops::{arith, collection, string};
use crate::value::Value;
use indexmap::IndexMap;
use rug::Integer;
use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

/// Globals storage: shared reference to mutable ordered map.
pub type Globals = Rc<RefCell<IndexMap<String, Value>>>;

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
    fn store_global(&self, name: &str, value: Value);
    fn delete_global(&self, name: &str);
    fn has_global(&self, name: &str) -> bool;

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

/// Host backed by Rust-owned globals with native builtins.
pub struct PureHost {
    globals: Globals,
}

impl PureHost {
    /// Create a new PureHost with empty globals.
    pub fn new() -> Self {
        Self {
            globals: Rc::new(RefCell::new(IndexMap::new())),
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
        }
        host
    }

    /// Get a reference to globals.
    pub fn globals(&self) -> &Globals {
        &self.globals
    }
}

/// Builtin function names available as globals.
const BUILTIN_NAMES: &[&str] = &[
    "abs",
    "len",
    "str",
    "int",
    "float",
    "bool",
    "type",
    "print",
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
        self.globals.borrow_mut().insert(name.to_string(), value);
    }

    #[inline]
    fn delete_global(&self, name: &str) {
        self.globals.borrow_mut().swap_remove(name);
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
            let s = unsafe { obj.as_native_str_ref().unwrap().to_string() };
            Ok(Box::new(StrCharIter {
                chars: s.chars().collect(),
                pos: 0,
            }))
        } else if obj.is_native_list() {
            let list = unsafe { obj.as_native_list_ref().unwrap() };
            let items = list.as_slice_cloned();
            Ok(Box::new(VecIter { items, pos: 0 }))
        } else if obj.is_native_tuple() {
            let tuple = unsafe { obj.as_native_tuple_ref().unwrap() };
            let items: Vec<Value> = tuple.as_slice().to_vec();
            for v in &items {
                v.clone_refcount();
            }
            Ok(Box::new(VecIter { items, pos: 0 }))
        } else if obj.is_native_dict() {
            // Iterate over keys
            let dict = unsafe { obj.as_native_dict_ref().unwrap() };
            let keys = dict.keys();
            Ok(Box::new(VecIter { items: keys, pos: 0 }))
        } else if obj.is_native_set() {
            let set = unsafe { obj.as_native_set_ref().unwrap() };
            let vals = set.to_values();
            Ok(Box::new(VecIter { items: vals, pos: 0 }))
        } else if obj.is_native_bytes() {
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
            let name = unsafe { func.as_native_str_ref().unwrap() };
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
        // Collection method dispatch
        if let Some(result) = collection::call_method(obj, method, args)? {
            return Ok(result);
        }
        // String methods with arguments
        if obj.is_native_str() {
            let s = unsafe { obj.as_native_str_ref().unwrap() };
            return str_method_with_args(s, method, args);
        }
        Err(VMError::TypeError(format!(
            "'{}' has no method '{}'",
            obj.type_name(),
            method
        )))
    }

    fn obj_getattr(&self, obj: Value, name: &str) -> VMResult<Value> {
        // String methods (0-arg only -- called as attributes)
        if obj.is_native_str() {
            let s = unsafe { obj.as_native_str_ref().unwrap() };
            return str_method_dispatch(s, name);
        }
        Err(VMError::TypeError(format!(
            "'{}' has no attribute '{}'",
            obj.type_name(),
            name
        )))
    }

    fn obj_setattr(&self, obj: Value, name: &str, _val: Value) -> VMResult<()> {
        Err(VMError::TypeError(format!(
            "cannot set attribute '{}' on {:?}",
            name, obj
        )))
    }

    fn obj_getitem(&self, obj: Value, key: Value) -> VMResult<Value> {
        if obj.is_native_str() {
            let s = unsafe { obj.as_native_str_ref().unwrap() };
            if let Some(i) = key.as_int() {
                return string::str_getitem(s, i);
            }
            return Err(VMError::TypeError("string indices must be integers".to_string()));
        }
        if obj.is_native_list() {
            let list = unsafe { obj.as_native_list_ref().unwrap() };
            if let Some(i) = key.as_int() {
                return list.get(i);
            }
            return Err(VMError::TypeError("list indices must be integers".into()));
        }
        if obj.is_native_tuple() {
            let tuple = unsafe { obj.as_native_tuple_ref().unwrap() };
            if let Some(i) = key.as_int() {
                return tuple.get(i);
            }
            return Err(VMError::TypeError("tuple indices must be integers".into()));
        }
        if obj.is_native_dict() {
            let dict = unsafe { obj.as_native_dict_ref().unwrap() };
            let k = key.to_key()?;
            return dict.get_item(&k);
        }
        if obj.is_native_bytes() {
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
            let list = unsafe { obj.as_native_list_ref().unwrap() };
            if let Some(i) = key.as_int() {
                return list.set(i, val);
            }
            return Err(VMError::TypeError("list indices must be integers".into()));
        }
        if obj.is_native_dict() {
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
            let haystack = unsafe { container.as_native_str_ref().unwrap() };
            if item.is_native_str() {
                let needle = unsafe { item.as_native_str_ref().unwrap() };
                return Ok(string::str_contains(haystack, needle));
            }
            return Err(VMError::TypeError(
                "'in' requires string as left operand for string containment".to_string(),
            ));
        }
        if container.is_native_list() {
            let list = unsafe { container.as_native_list_ref().unwrap() };
            return Ok(list.contains(item));
        }
        if container.is_native_tuple() {
            let tuple = unsafe { container.as_native_tuple_ref().unwrap() };
            return Ok(tuple.contains(item));
        }
        if container.is_native_dict() {
            let dict = unsafe { container.as_native_dict_ref().unwrap() };
            let key = item.to_key()?;
            return Ok(dict.contains_key(&key));
        }
        if container.is_native_set() {
            let set = unsafe { container.as_native_set_ref().unwrap() };
            let key = item.to_key()?;
            return Ok(set.contains(&key));
        }
        if container.is_native_bytes() {
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
                let list = unsafe { arg.as_native_list_ref().unwrap() };
                let items = list.as_slice_cloned();
                let result = string::str_join(s, &items);
                for v in &items {
                    v.decref();
                }
                result
            } else if arg.is_native_tuple() {
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
                unsafe { args[0].as_native_str_ref().unwrap() }
            } else {
                return Err(VMError::TypeError("replace() arguments must be strings".into()));
            };
            let new = if args[1].is_native_str() {
                unsafe { args[1].as_native_str_ref().unwrap() }
            } else {
                return Err(VMError::TypeError("replace() arguments must be strings".into()));
            };
            Ok(string::str_replace(s, old, new))
        }
        "find" => {
            let sub = if let Some(a) = args.first() {
                if a.is_native_str() {
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
        let sa = unsafe { a.as_native_str_ref().unwrap() };
        let sb = unsafe { b.as_native_str_ref().unwrap() };
        return Ok(string::str_concat(sa, sb));
    }
    // List concat
    if a.is_native_list() && b.is_native_list() {
        let la = unsafe { a.as_native_list_ref().unwrap() };
        let lb = unsafe { b.as_native_list_ref().unwrap() };
        let mut items = la.as_slice_cloned();
        let items_b = lb.as_slice_cloned();
        items.extend(items_b);
        return Ok(Value::from_list(items));
    }
    // Tuple concat
    if a.is_native_tuple() && b.is_native_tuple() {
        let ta = unsafe { a.as_native_tuple_ref().unwrap() };
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
        let ba = unsafe { a.as_native_bytes_ref().unwrap() };
        let bb = unsafe { b.as_native_bytes_ref().unwrap() };
        let mut data = ba.as_bytes().to_vec();
        data.extend_from_slice(bb.as_bytes());
        return Ok(Value::from_bytes(data));
    }
    Err(VMError::TypeError("unsupported operand types for +".into()))
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
            let s = unsafe { a.as_native_str_ref().unwrap() };
            return Ok(string::str_repeat(s, n));
        }
    }
    if b.is_native_str() {
        if let Some(n) = a.as_int() {
            let s = unsafe { b.as_native_str_ref().unwrap() };
            return Ok(string::str_repeat(s, n));
        }
    }
    // List repeat: list * int
    if a.is_native_list() {
        if let Some(n) = b.as_int() {
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
    Err(VMError::TypeError("unsupported operand types for *".into()))
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
        let sa = unsafe { a.as_native_str_ref().unwrap() };
        let sb = unsafe { b.as_native_str_ref().unwrap() };
        return Ok(Value::from_bool(pred(sa.cmp(sb))));
    }
    Err(VMError::TypeError("unsupported comparison".into()))
}

// ---------------------------------------------------------------------------
// Builtin function dispatch
// ---------------------------------------------------------------------------

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
                let n = unsafe { a.as_bigint_ref().unwrap() };
                return Ok(Value::from_bigint_or_demote(n.clone().abs()));
            }
            Err(VMError::TypeError("bad operand type for abs()".into()))
        }
        "len" => {
            let a = args
                .first()
                .ok_or_else(|| VMError::TypeError("len() takes 1 argument".into()))?;
            if a.is_native_str() {
                let s = unsafe { a.as_native_str_ref().unwrap() };
                return Ok(Value::from_int(string::str_len(s) as i64));
            }
            if a.is_native_list() {
                let list = unsafe { a.as_native_list_ref().unwrap() };
                return Ok(Value::from_int(list.len() as i64));
            }
            if a.is_native_tuple() {
                let tuple = unsafe { a.as_native_tuple_ref().unwrap() };
                return Ok(Value::from_int(tuple.len() as i64));
            }
            if a.is_native_dict() {
                let dict = unsafe { a.as_native_dict_ref().unwrap() };
                return Ok(Value::from_int(dict.len() as i64));
            }
            if a.is_native_set() {
                let set = unsafe { a.as_native_set_ref().unwrap() };
                return Ok(Value::from_int(set.len() as i64));
            }
            if a.is_native_bytes() {
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
                let list = unsafe { a.as_native_list_ref().unwrap() };
                return Ok(Value::from_list(list.copy()));
            }
            if a.is_native_tuple() {
                let tuple = unsafe { a.as_native_tuple_ref().unwrap() };
                let items: Vec<Value> = tuple.as_slice().to_vec();
                for v in &items {
                    v.clone_refcount();
                }
                return Ok(Value::from_list(items));
            }
            if a.is_native_str() {
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
                let tuple = unsafe { a.as_native_tuple_ref().unwrap() };
                let items: Vec<Value> = tuple.as_slice().to_vec();
                for v in &items {
                    v.clone_refcount();
                }
                return Ok(Value::from_tuple(items));
            }
            if a.is_native_list() {
                let list = unsafe { a.as_native_list_ref().unwrap() };
                return Ok(Value::from_tuple(list.copy()));
            }
            Err(VMError::TypeError("tuple() argument must be an iterable".into()))
        }
        "dict" => {
            if args.is_empty() {
                return Ok(Value::from_empty_dict());
            }
            Err(VMError::TypeError("dict() takes no arguments in pure mode".into()))
        }
        "set" => {
            if args.is_empty() {
                return Ok(Value::from_set(indexmap::IndexSet::new()));
            }
            let a = &args[0];
            if a.is_native_list() {
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
                let list = unsafe { a.as_native_list_ref().unwrap() };
                let items = list.copy();
                let result = Value::from_list(items);
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
                let list = unsafe { a.as_native_list_ref().unwrap() };
                let mut items = list.copy();
                items.reverse();
                return Ok(Value::from_list(items));
            }
            if a.is_native_tuple() {
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
                    let list = unsafe { a.as_native_list_ref().unwrap() };
                    iters.push(list.as_slice_cloned());
                } else if a.is_native_tuple() {
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
mod tests {
    use super::*;

    #[test]
    fn test_pure_host_globals() {
        let host = PureHost::with_builtins();
        assert!(host.has_global("True"));
        assert!(host.has_global("False"));
        assert!(host.has_global("None"));
        assert!(!host.has_global("x"));

        host.store_global("x", Value::from_int(42));
        assert!(host.has_global("x"));
        assert_eq!(host.lookup_global("x").unwrap(), Some(Value::from_int(42)));

        host.delete_global("x");
        assert!(!host.has_global("x"));
    }

    #[test]
    fn test_binary_add_int() {
        let host = PureHost::new();
        let result = host
            .binary_op(BinaryOp::Add, Value::from_int(2), Value::from_int(3))
            .unwrap();
        assert_eq!(result.as_int(), Some(5));
    }

    #[test]
    fn test_binary_add_float() {
        let host = PureHost::new();
        let result = host
            .binary_op(BinaryOp::Add, Value::from_float(1.5), Value::from_float(2.5))
            .unwrap();
        assert!((result.as_float().unwrap() - 4.0).abs() < 1e-10);
    }

    #[test]
    fn test_binary_add_mixed() {
        let host = PureHost::new();
        let result = host
            .binary_op(BinaryOp::Add, Value::from_int(1), Value::from_float(2.5))
            .unwrap();
        assert!((result.as_float().unwrap() - 3.5).abs() < 1e-10);
    }

    #[test]
    fn test_binary_add_string() {
        let host = PureHost::new();
        let a = Value::from_str("hello");
        let b = Value::from_str(" world");
        let result = host.binary_op(BinaryOp::Add, a, b).unwrap();
        assert_eq!(unsafe { result.as_native_str_ref() }, Some("hello world"));
        a.decref();
        b.decref();
        result.decref();
    }

    #[test]
    fn test_binary_sub() {
        let host = PureHost::new();
        let result = host
            .binary_op(BinaryOp::Sub, Value::from_int(10), Value::from_int(3))
            .unwrap();
        assert_eq!(result.as_int(), Some(7));
    }

    #[test]
    fn test_binary_mul() {
        let host = PureHost::new();
        let result = host
            .binary_op(BinaryOp::Mul, Value::from_int(4), Value::from_int(5))
            .unwrap();
        assert_eq!(result.as_int(), Some(20));
    }

    #[test]
    fn test_binary_mul_string_repeat() {
        let host = PureHost::new();
        let s = Value::from_str("ab");
        let result = host.binary_op(BinaryOp::Mul, s, Value::from_int(3)).unwrap();
        assert_eq!(unsafe { result.as_native_str_ref() }, Some("ababab"));
        s.decref();
        result.decref();
    }

    #[test]
    fn test_binary_div() {
        let host = PureHost::new();
        let result = host
            .binary_op(BinaryOp::TrueDiv, Value::from_int(7), Value::from_int(2))
            .unwrap();
        assert!((result.as_float().unwrap() - 3.5).abs() < 1e-10);
    }

    #[test]
    fn test_binary_div_zero() {
        let host = PureHost::new();
        let result = host.binary_op(BinaryOp::TrueDiv, Value::from_int(1), Value::from_int(0));
        assert!(matches!(result, Err(VMError::ZeroDivisionError(_))));
    }

    #[test]
    fn test_binary_floordiv() {
        let host = PureHost::new();
        let result = host
            .binary_op(BinaryOp::FloorDiv, Value::from_int(7), Value::from_int(2))
            .unwrap();
        assert_eq!(result.as_int(), Some(3));

        // Python semantics: -7 // 2 == -4 (floor division)
        let result = host
            .binary_op(BinaryOp::FloorDiv, Value::from_int(-7), Value::from_int(2))
            .unwrap();
        assert_eq!(result.as_int(), Some(-4));
    }

    #[test]
    fn test_binary_mod() {
        let host = PureHost::new();
        let result = host
            .binary_op(BinaryOp::Mod, Value::from_int(7), Value::from_int(3))
            .unwrap();
        assert_eq!(result.as_int(), Some(1));

        // Python semantics: -7 % 3 == 2
        let result = host
            .binary_op(BinaryOp::Mod, Value::from_int(-7), Value::from_int(3))
            .unwrap();
        assert_eq!(result.as_int(), Some(2));
    }

    #[test]
    fn test_binary_pow() {
        let host = PureHost::new();
        let result = host
            .binary_op(BinaryOp::Pow, Value::from_int(2), Value::from_int(10))
            .unwrap();
        assert_eq!(result.as_int(), Some(1024));
    }

    #[test]
    fn test_comparison() {
        let host = PureHost::new();
        let t = |op: BinaryOp, a: i64, b: i64| -> bool {
            host.binary_op(op, Value::from_int(a), Value::from_int(b))
                .unwrap()
                .as_bool()
                .unwrap()
        };
        assert!(t(BinaryOp::Lt, 1, 2));
        assert!(!t(BinaryOp::Lt, 2, 1));
        assert!(t(BinaryOp::Le, 1, 1));
        assert!(t(BinaryOp::Gt, 3, 2));
        assert!(t(BinaryOp::Ge, 2, 2));
    }

    #[test]
    fn test_comparison_string() {
        let host = PureHost::new();
        let a = Value::from_str("abc");
        let b = Value::from_str("abd");
        let result = host.binary_op(BinaryOp::Lt, a, b).unwrap();
        assert_eq!(result.as_bool(), Some(true));
        a.decref();
        b.decref();
    }

    #[test]
    fn test_bigint_add() {
        let host = PureHost::new();
        let a = Value::from_bigint(Integer::from(1_u64 << 50));
        let b = Value::from_int(1);
        let result = host.binary_op(BinaryOp::Add, a, b).unwrap();
        assert!(result.is_bigint());
        let expected = Integer::from(1_u64 << 50) + Integer::from(1);
        assert_eq!(unsafe { result.as_bigint_ref() }, Some(&expected));
        a.decref();
        result.decref();
    }

    #[test]
    fn test_contains_string() {
        let host = PureHost::new();
        let haystack = Value::from_str("hello world");
        let needle = Value::from_str("world");
        assert!(host.contains_op(needle, haystack).unwrap());
        haystack.decref();
        needle.decref();
    }

    #[test]
    fn test_str_iter() {
        let host = PureHost::new();
        let s = Value::from_str("abc");
        let mut iter = host.get_iter(s).unwrap();
        let a = iter.next_value().unwrap().unwrap();
        assert_eq!(unsafe { a.as_native_str_ref() }, Some("a"));
        let b = iter.next_value().unwrap().unwrap();
        assert_eq!(unsafe { b.as_native_str_ref() }, Some("b"));
        let c = iter.next_value().unwrap().unwrap();
        assert_eq!(unsafe { c.as_native_str_ref() }, Some("c"));
        assert!(iter.next_value().unwrap().is_none());
        s.decref();
        a.decref();
        b.decref();
        c.decref();
    }

    #[test]
    fn test_str_getattr() {
        let host = PureHost::new();
        let s = Value::from_str("hello");
        let upper = host.obj_getattr(s, "upper").unwrap();
        assert_eq!(unsafe { upper.as_native_str_ref() }, Some("HELLO"));
        s.decref();
        upper.decref();
    }

    #[test]
    fn test_str_getitem() {
        let host = PureHost::new();
        let s = Value::from_str("hello");
        let ch = host.obj_getitem(s, Value::from_int(1)).unwrap();
        assert_eq!(unsafe { ch.as_native_str_ref() }, Some("e"));
        s.decref();
        ch.decref();
    }

    #[test]
    fn test_builtin_abs() {
        let result = call_builtin("abs", &[Value::from_int(-42)]).unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_builtin_len() {
        let s = Value::from_str("hello");
        let result = call_builtin("len", &[s]).unwrap();
        assert_eq!(result.as_int(), Some(5));
        s.decref();
    }

    #[test]
    fn test_builtin_str() {
        let result = call_builtin("str", &[Value::from_int(42)]).unwrap();
        assert_eq!(unsafe { result.as_native_str_ref() }, Some("42"));
        result.decref();
    }

    #[test]
    fn test_builtin_int_from_str() {
        let s = Value::from_str("42");
        let result = call_builtin("int", &[s]).unwrap();
        assert_eq!(result.as_int(), Some(42));
        s.decref();
    }

    #[test]
    fn test_builtin_float_from_str() {
        let s = Value::from_str("3.14");
        let result = call_builtin("float", &[s]).unwrap();
        assert!((result.as_float().unwrap() - 3.14).abs() < 1e-10);
        s.decref();
    }

    #[test]
    fn test_builtin_bool() {
        assert_eq!(
            call_builtin("bool", &[Value::from_int(0)]).unwrap().as_bool(),
            Some(false)
        );
        assert_eq!(
            call_builtin("bool", &[Value::from_int(1)]).unwrap().as_bool(),
            Some(true)
        );
    }

    #[test]
    fn test_builtin_type() {
        let result = call_builtin("type", &[Value::from_int(1)]).unwrap();
        assert_eq!(unsafe { result.as_native_str_ref() }, Some("int"));
        result.decref();

        let s = Value::from_str("x");
        let result = call_builtin("type", &[s]).unwrap();
        assert_eq!(unsafe { result.as_native_str_ref() }, Some("str"));
        s.decref();
        result.decref();
    }

    #[test]
    fn test_builtin_min_max() {
        let result = call_builtin("min", &[Value::from_int(3), Value::from_int(1), Value::from_int(2)]).unwrap();
        assert_eq!(result.as_int(), Some(1));

        let result = call_builtin("max", &[Value::from_int(3), Value::from_int(1), Value::from_int(2)]).unwrap();
        assert_eq!(result.as_int(), Some(3));
    }

    // --- Collection host tests ---

    #[test]
    fn test_list_iter() {
        let host = PureHost::new();
        let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2), Value::from_int(3)]);
        let mut iter = host.get_iter(list).unwrap();
        assert_eq!(iter.next_value().unwrap().unwrap(), Value::from_int(1));
        assert_eq!(iter.next_value().unwrap().unwrap(), Value::from_int(2));
        assert_eq!(iter.next_value().unwrap().unwrap(), Value::from_int(3));
        assert!(iter.next_value().unwrap().is_none());
        list.decref();
    }

    #[test]
    fn test_tuple_iter() {
        let host = PureHost::new();
        let tuple = Value::from_tuple(vec![Value::from_int(10), Value::from_int(20)]);
        let mut iter = host.get_iter(tuple).unwrap();
        assert_eq!(iter.next_value().unwrap().unwrap(), Value::from_int(10));
        assert_eq!(iter.next_value().unwrap().unwrap(), Value::from_int(20));
        assert!(iter.next_value().unwrap().is_none());
        tuple.decref();
    }

    #[test]
    fn test_dict_iter_keys() {
        let host = PureHost::new();
        let dict = Value::from_empty_dict();
        let d = unsafe { dict.as_native_dict_ref().unwrap() };
        d.set_item(crate::collections::ValueKey::Int(1), Value::from_int(10));
        d.set_item(crate::collections::ValueKey::Int(2), Value::from_int(20));
        let mut iter = host.get_iter(dict).unwrap();
        let k1 = iter.next_value().unwrap().unwrap();
        assert_eq!(k1.as_int(), Some(1));
        let k2 = iter.next_value().unwrap().unwrap();
        assert_eq!(k2.as_int(), Some(2));
        assert!(iter.next_value().unwrap().is_none());
        dict.decref();
    }

    #[test]
    fn test_list_getitem_setitem() {
        let host = PureHost::new();
        let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
        let v = host.obj_getitem(list, Value::from_int(0)).unwrap();
        assert_eq!(v, Value::from_int(1));
        host.obj_setitem(list, Value::from_int(0), Value::from_int(10)).unwrap();
        let v = host.obj_getitem(list, Value::from_int(0)).unwrap();
        assert_eq!(v, Value::from_int(10));
        list.decref();
    }

    #[test]
    fn test_dict_getitem_setitem() {
        let host = PureHost::new();
        let dict = Value::from_empty_dict();
        host.obj_setitem(dict, Value::from_int(1), Value::from_int(10)).unwrap();
        let v = host.obj_getitem(dict, Value::from_int(1)).unwrap();
        assert_eq!(v, Value::from_int(10));
        dict.decref();
    }

    #[test]
    fn test_contains_list() {
        let host = PureHost::new();
        let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2), Value::from_int(3)]);
        assert!(host.contains_op(Value::from_int(2), list).unwrap());
        assert!(!host.contains_op(Value::from_int(5), list).unwrap());
        list.decref();
    }

    #[test]
    fn test_contains_dict() {
        let host = PureHost::new();
        let dict = Value::from_empty_dict();
        let d = unsafe { dict.as_native_dict_ref().unwrap() };
        d.set_item(crate::collections::ValueKey::Int(1), Value::from_int(10));
        assert!(host.contains_op(Value::from_int(1), dict).unwrap());
        assert!(!host.contains_op(Value::from_int(2), dict).unwrap());
        dict.decref();
    }

    #[test]
    fn test_list_concat() {
        let host = PureHost::new();
        let a = Value::from_list(vec![Value::from_int(1)]);
        let b = Value::from_list(vec![Value::from_int(2)]);
        let result = host.binary_op(BinaryOp::Add, a, b).unwrap();
        assert!(result.is_native_list());
        let list = unsafe { result.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 2);
        a.decref();
        b.decref();
        result.decref();
    }

    #[test]
    fn test_list_repeat() {
        let host = PureHost::new();
        let a = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
        let result = host.binary_op(BinaryOp::Mul, a, Value::from_int(3)).unwrap();
        let list = unsafe { result.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 6);
        a.decref();
        result.decref();
    }

    #[test]
    fn test_builtin_len_collections() {
        let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
        let result = call_builtin("len", &[list]).unwrap();
        assert_eq!(result.as_int(), Some(2));
        list.decref();

        let tuple = Value::from_tuple(vec![Value::from_int(1)]);
        let result = call_builtin("len", &[tuple]).unwrap();
        assert_eq!(result.as_int(), Some(1));
        tuple.decref();

        let dict = Value::from_empty_dict();
        let result = call_builtin("len", &[dict]).unwrap();
        assert_eq!(result.as_int(), Some(0));
        dict.decref();
    }

    #[test]
    fn test_builtin_range() {
        let result = call_builtin("range", &[Value::from_int(5)]).unwrap();
        let list = unsafe { result.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 5);
        assert_eq!(list.get(0).unwrap(), Value::from_int(0));
        assert_eq!(list.get(4).unwrap(), Value::from_int(4));
        result.decref();
    }

    #[test]
    fn test_builtin_sorted() {
        let list = Value::from_list(vec![Value::from_int(3), Value::from_int(1), Value::from_int(2)]);
        let result = call_builtin("sorted", &[list]).unwrap();
        let sorted = unsafe { result.as_native_list_ref().unwrap() };
        assert_eq!(sorted.get(0).unwrap(), Value::from_int(1));
        assert_eq!(sorted.get(2).unwrap(), Value::from_int(3));
        list.decref();
        result.decref();
    }

    #[test]
    fn test_builtin_sum() {
        let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2), Value::from_int(3)]);
        let result = call_builtin("sum", &[list]).unwrap();
        assert_eq!(result.as_int(), Some(6));
        list.decref();
    }

    #[test]
    fn test_builtin_any_all() {
        let list = Value::from_list(vec![Value::from_int(0), Value::from_int(1)]);
        assert_eq!(call_builtin("any", &[list]).unwrap().as_bool(), Some(true));
        assert_eq!(call_builtin("all", &[list]).unwrap().as_bool(), Some(false));
        list.decref();

        let list2 = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
        assert_eq!(call_builtin("all", &[list2]).unwrap().as_bool(), Some(true));
        list2.decref();
    }

    #[test]
    fn test_builtin_enumerate() {
        let list = Value::from_list(vec![Value::from_str("a"), Value::from_str("b")]);
        let result = call_builtin("enumerate", &[list]).unwrap();
        let r = unsafe { result.as_native_list_ref().unwrap() };
        assert_eq!(r.len(), 2);
        let first = r.get(0).unwrap();
        assert!(first.is_native_tuple());
        let t = unsafe { first.as_native_tuple_ref().unwrap() };
        assert_eq!(t.get(0).unwrap(), Value::from_int(0));
        first.decref();
        list.decref();
        result.decref();
    }

    #[test]
    fn test_builtin_zip() {
        let a = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
        let b = Value::from_list(vec![Value::from_str("a"), Value::from_str("b"), Value::from_str("c")]);
        let result = call_builtin("zip", &[a, b]).unwrap();
        let r = unsafe { result.as_native_list_ref().unwrap() };
        assert_eq!(r.len(), 2); // min(2, 3)
        a.decref();
        b.decref();
        result.decref();
    }

    #[test]
    fn test_builtin_type_collections() {
        let list = Value::from_list(vec![]);
        let result = call_builtin("type", &[list]).unwrap();
        assert_eq!(unsafe { result.as_native_str_ref() }, Some("list"));
        list.decref();
        result.decref();

        let tuple = Value::from_tuple(vec![]);
        let result = call_builtin("type", &[tuple]).unwrap();
        assert_eq!(unsafe { result.as_native_str_ref() }, Some("tuple"));
        tuple.decref();
        result.decref();
    }

    #[test]
    fn test_call_method_str_split() {
        let host = PureHost::new();
        let s = Value::from_str("a,b,c");
        let sep = Value::from_str(",");
        let result = host.call_method(s, "split", &[sep]).unwrap();
        assert!(result.is_native_list());
        let list = unsafe { result.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 3);
        s.decref();
        sep.decref();
        result.decref();
    }

    #[test]
    fn test_call_method_list_append() {
        let host = PureHost::new();
        let list = Value::from_list(vec![]);
        host.call_method(list, "append", &[Value::from_int(42)]).unwrap();
        let l = unsafe { list.as_native_list_ref().unwrap() };
        assert_eq!(l.len(), 1);
        list.decref();
    }
}
