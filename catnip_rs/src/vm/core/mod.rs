// FILE: catnip_rs/src/vm/core/mod.rs
//! Catnip Virtual Machine with O(1) dispatch via Rust match.
//!
//! Stack-based VM that executes bytecode without growing the Python stack.

use super::OpCode;
use super::enums::{CatnipEnumType, EnumRegistry};
use super::frame::{CodeObject, Frame, FramePool, NativeClosureScope, PyCodeObject, VMFunction, decref_frame_values};
use super::host::{BinaryOp, VmHost};
use super::iter::SeqIter;
use super::pattern::{VMPattern, VMPatternElement};
use super::py_interop::{
    PyResultExt, bitwise_binary_fallback, bitwise_unary_fallback, cast_tuple, convert_code_object,
    portabilize_struct_values, tuple_extract, tuple_get,
};
use super::structs::{
    CatnipStructType, MethodKey, StructField, StructMethods, StructParents, StructRegistry, StructType, StructTypeId,
    cascade_decref_fields,
};
use super::traits::{TraitDef, TraitField, TraitRegistry};
use super::value::resolve_symbol_by_name;
use super::value::{FuncSlot, FunctionTable, Value};
use crate::constants::*;
use crate::jit::builtin_dispatch::builtin_name_to_id;
use crate::jit::{HotLoopDetector, JITExecutor, Trace, TraceOp, TraceRecorder};
use catnip_core::symbols::{SymbolTable, qualified_name};
use catnip_vm::ops::errors;
use indexmap::IndexMap;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PySet, PyString, PyTuple};
use rug::Integer;
use std::collections::{HashMap, HashSet};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

/// Resolve a captured variable's integer value for JIT guard validation.
/// Searches: closure scope -> host globals -> VM globals.
///
/// The resolvers return owned values; a guard only reads the int, so whatever
/// they hand back is released here (a non-int resolution falls through, as
/// before, but must not leak its owned result).
#[inline]
fn resolve_jit_guard_value(
    py: Python<'_>,
    name: &str,
    closure: &Option<NativeClosureScope>,
    host: &(impl VmHost + ?Sized),
    globals: &IndexMap<String, Value>,
    struct_registry: &StructRegistry,
) -> Option<i64> {
    if let Some(ref closure) = closure {
        if let Some(v) = closure.resolve_with_py(py, name) {
            let as_int = v.as_int();
            decref_discard(struct_registry, v);
            if let Some(val) = as_int {
                return Some(val);
            }
        }
    }
    if let Some(v) = host.lookup_global(py, name).ok().flatten() {
        let as_int = v.as_int();
        decref_discard(struct_registry, v);
        if let Some(val) = as_int {
            return Some(val);
        }
    }
    globals.get(name).and_then(|v| v.as_int())
}

/// Resolve a name to its raw NaN-box bits for JIT function-identity guards.
/// Mirrors the LoadScope resolution order (captured chain -> VM globals ->
/// closure parent -> host) so the bits match what `record_func_guard` recorded;
/// a mismatch (function reassigned) makes the warm-start guard fail and the loop
/// falls back to the interpreter instead of running the stale inlined body.
#[inline]
fn resolve_jit_name_bits(
    py: Python<'_>,
    name: &str,
    closure: &Option<NativeClosureScope>,
    host: &(impl VmHost + ?Sized),
    globals: &IndexMap<String, Value>,
    struct_registry: &StructRegistry,
) -> Option<u64> {
    // The resolvers return owned values; the guard only compares identity
    // bits (read before release), so the owned result is discarded here.
    if let Some(ref closure) = closure {
        if let Some(v) = closure.resolve_captured_chain(name) {
            let bits = v.bits();
            decref_discard(struct_registry, v);
            return Some(bits);
        }
    }
    if let Some(v) = globals.get(name) {
        return Some(v.bits());
    }
    if let Some(ref closure) = closure {
        if let Some(v) = closure.resolve_with_py(py, name) {
            let bits = v.bits();
            decref_discard(struct_registry, v);
            return Some(bits);
        }
    }
    host.lookup_global(py, name).ok().flatten().map(|v| {
        let bits = v.bits();
        decref_discard(struct_registry, v);
        bits
    })
}

/// Voie A: release a discarded stack value's owned pyobj handle (no-op for
/// non-pyobj). Used at raw-pop consumer sites that must leave the separate
/// bigint/struct refcount discipline untouched.
#[inline]
fn decref_pyobj(val: Value) {
    if val.is_pyobj() {
        val.decref();
    }
}

/// Release a popped operand's BigInt/Complex or struct-instance ref; pyobj is
/// deliberately excluded. ONLY for arms whose Python fallback
/// (`call_binary_op`) consumes the pyobj refs but borrows everything else;
/// an arm with no consuming fallback must use `decref_discard` instead, or
/// its popped pyobj operands leak on the error path.
#[inline]
fn decref_non_pyobj(registry: &StructRegistry, val: Value) {
    if !val.is_pyobj() {
        decref_discard(registry, val);
    }
}

/// Release both popped operands of a binary arithmetic/comparison arm at its
/// single post-computation release point (errors included; the struct-overload
/// `continue` transfers instead and must skip this).
#[inline]
fn release_binop_operands(registry: &StructRegistry, a: Value, b: Value) {
    decref_non_pyobj(registry, a);
    decref_non_pyobj(registry, b);
}

/// Structural field-by-field equality of two struct instances; false when the
/// type ids differ. Borrow-only (compare_eq borrows): the Eq/Ne arms release
/// their popped operands at their single post-computation point, a raising
/// __eq__ included.
fn struct_fields_eq(registry: &StructRegistry, py: Python<'_>, idx_a: u32, idx_b: u32) -> VMResult<bool> {
    let (type_id_a, fields_a) = registry
        .with_instance(idx_a, |i| (i.type_id, i.fields.clone()))
        .unwrap();
    let (type_id_b, fields_b) = registry
        .with_instance(idx_b, |i| (i.type_id, i.fields.clone()))
        .unwrap();
    if type_id_a != type_id_b {
        return Ok(false);
    }
    for (fa, fb) in fields_a.iter().zip(fields_b.iter()) {
        let eq = compare_eq(py, *fa, *fb)?;
        if eq.as_bool() != Some(true) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Decref a Value being discarded (PopTop, StoreLocal overwrite, SetAttr old value).
/// Voie A: releases owned PyObj handles, plus BigInt (Arc) and Struct (registry
/// slot + field cascade).
/// Release a batch of owned operands. Error-path exits of the call opcodes:
/// popped args are in flight until consumed by a frame or a conversion, and a
/// `return Err`/`?` before that point would drop them raw.
fn release_operands(registry: &StructRegistry, vals: &[Value]) {
    for &v in vals {
        decref_discard(registry, v);
    }
}

/// Pop `num_defaults` default values off the stack and parse a field-spec
/// tuple -- (name, has_default[, annotation]) per field -- into StructFields.
/// Shared by MakeStruct and MakeTrait (which drops the annotation check).
/// The popped defaults are owned and move into the returned fields.
fn parse_field_specs(
    frame: &mut Frame,
    fields_info: &Bound<'_, PyAny>,
    num_defaults: usize,
) -> VMResult<Vec<StructField>> {
    // Read default values in stack order, then truncate
    let stack_len = frame.stack.len();
    let dstart = stack_len - num_defaults;
    let default_values: Vec<Value> = frame.stack[dstart..].to_vec();
    frame.stack.truncate(dstart);

    let fields_tuple = cast_tuple(fields_info)?;
    let mut fields = Vec::new();
    let mut default_idx = 0usize;
    for fi in fields_tuple.iter() {
        let pair = cast_tuple(&fi)?;
        let fname: String = tuple_extract(pair, 0)?;
        let has_default: bool = tuple_extract(pair, 1)?;
        let default_val = if has_default {
            let v = default_values[default_idx];
            default_idx += 1;
            v
        } else {
            Value::NIL
        };
        // Element 2 (when present) is the field's annotation text;
        // classify it into a runtime boundary check, like a param.
        let check = if pair.len() >= 3 {
            match tuple_extract::<String>(pair, 2) {
                Ok(text) => catnip_core::vm::opcode::ParamCheck::from_annotation(&text),
                Err(_) => catnip_core::vm::opcode::ParamCheck::None,
            }
        } else {
            catnip_core::vm::opcode::ParamCheck::None
        };
        fields.push(StructField {
            name: fname,
            has_default,
            default: default_val,
            check,
        });
    }
    Ok(fields)
}

/// Takes `&self`: the registry mutates through its interior `RefCell`, so a
/// pyobj field's `__del__` cascaded here may reenter the registry (dropping
/// another proxy) as a fresh shared borrow -- never a second `&mut` aliasing
/// this one. See `StructRegistry`'s field docs.
#[inline]
pub(crate) fn decref_discard(registry: &StructRegistry, val: Value) {
    if val.is_pyobj() {
        // Voie A (stack owned): every pyobj Value on the stack / in a slot owns
        // one ObjectTable handle ref, released here when the value is discarded.
        val.decref();
    } else if val.is_bigint() {
        val.decref_bigint();
    } else if val.is_complex() {
        val.decref();
    } else if val.is_struct_instance() {
        let idx = val.as_struct_instance_idx().unwrap();
        if let Some(fields) = registry.decref(idx) {
            cascade_decref_fields(registry, fields);
        }
    }
}

/// Release a frame's pending match bindings.
///
/// `vm_match_pattern` returns bindings that each own an independent ref (a
/// cloned subject ref, a cloned struct field, a freshly built star list).
/// BindMatch consumes them by CLONE (a guarded arm binds twice), so
/// match_bindings keeps its owned refs after binding; they must be released
/// when it is overwritten by the next match or when the frame is torn down, or
/// every capture-binding match leaks one ref.
#[inline]
fn release_match_bindings(registry: &StructRegistry, frame: &mut Frame) {
    if let Some(bindings) = frame.match_bindings.take() {
        for (_slot, val) in bindings {
            decref_discard(registry, val);
        }
    }
}

/// Check abstract struct guard. Returns Err if struct has unimplemented abstract methods.
#[inline]
fn check_abstract_guard(registry: &StructRegistry, type_id: StructTypeId) -> VMResult<()> {
    let ty = registry.get_type(type_id).unwrap();
    if !ty.abstract_methods.is_empty() {
        let mut names: Vec<&str> = ty.abstract_methods.iter().map(|k| k.name.as_str()).collect();
        names.sort();
        return Err(VMError::RuntimeError(format!(
            "cannot instantiate abstract struct '{}' (unimplemented: {})",
            ty.name,
            names.iter().map(|n| format!("'{}'", n)).collect::<Vec<_>>().join(", ")
        )));
    }
    Ok(())
}

/// Build an error message for a missing attribute on a struct, with "did you mean?" suggestion.
fn attr_error_msg(ty: &StructType, attr: &str) -> String {
    let candidates = ty.available_names();
    let candidates_ref: Vec<&str> = candidates.to_vec();
    let suggestions = catnip_tools::suggest::suggest_similar(attr, &candidates_ref, 1, 0.6);
    match catnip_tools::suggest::format_suggestion(&suggestions) {
        Some(hint) => format!("'{}' has no attribute '{}'. {}", ty.name, attr, hint),
        None => format!("'{}' has no attribute '{}'", ty.name, attr),
    }
}

/// Safely index into code.names with bounds check.
#[inline(always)]
fn get_name(code: &CodeObject, arg: u32) -> Result<&String, VMError> {
    let idx = arg as usize;
    code.names
        .get(idx)
        .ok_or_else(|| VMError::RuntimeError(format!("invalid name index {} (names len={})", idx, code.names.len())))
}

/// Best-effort runtime type name of a primitive, for boundary-check errors.
/// Needs `py` to recognise a `str` (NaN-boxed as a PyObject handle, unlike the
/// PureVM's native string).
fn primitive_type_name(py: Python<'_>, v: Value) -> &'static str {
    if v.is_bool() {
        "bool"
    } else if v.is_int() || v.is_bigint() {
        "int"
    } else if v.is_float() {
        "float"
    } else if v.is_nil() {
        "None"
    } else if v
        .as_pyobject(py)
        .is_some_and(|o| o.bind(py).is_instance_of::<PyString>())
    {
        "str"
    } else {
        "value"
    }
}

/// TH2-B step 0b boundary check + numeric-tower coercion for `CheckType`.
///
/// `code` is a `type_code::*` naming the declared param type. A value already of
/// that type passes through; a numeric-tower widening (`int`/`bool` → `float`,
/// `bool` → `int`, `bigint` → `float`) is coerced to the declared type; anything
/// else is a `TypeError`. Enforces `int`/`float`/`str`/`bool`/`None`; `str` has
/// no widening (only a `str` passes). Takes `py` because a `str` is a PyObject
/// handle. Kept semantically symmetric with the PureVM `boundary_coerce`.
fn boundary_coerce(py: Python<'_>, val: Value, code: u8) -> VMResult<Value> {
    use catnip_core::vm::opcode::type_code;
    let mismatch = || {
        VMError::TypeError(format!(
            "typed parameter expects '{}' but got '{}'",
            type_code::name(code),
            primitive_type_name(py, val)
        ))
    };
    // Scalar arms (int/float/bool/None, numeric-tower widening) are shared
    // with the twin VM: catnip_core::arith::coerce_scalar. Only the mismatch
    // message (per-crate primitive_type_name) and the heap codes stay here.
    use catnip_core::arith::ScalarCoerce;
    match catnip_core::arith::coerce_scalar(val, code) {
        ScalarCoerce::Ok(v) => return Ok(v),
        ScalarCoerce::HugeInt => {
            return Err(VMError::TypeError("int too large to convert to float".to_string()));
        }
        ScalarCoerce::Mismatch => return Err(mismatch()),
        ScalarCoerce::Unhandled => {}
    }
    match code {
        type_code::STR => {
            if val
                .as_pyobject(py)
                .is_some_and(|o| o.bind(py).is_instance_of::<PyString>())
            {
                Ok(val)
            } else {
                Err(mismatch())
            }
        }
        // Composites are enforced at the constructor level (params ignored), no
        // coercion. List/dict NaN-box as PyObject handles, so the check reads the
        // materialized object, mirroring the `str` arm.
        type_code::LIST => {
            if val
                .as_pyobject(py)
                .is_some_and(|o| o.bind(py).is_instance_of::<PyList>())
            {
                Ok(val)
            } else {
                Err(mismatch())
            }
        }
        type_code::DICT => {
            if val
                .as_pyobject(py)
                .is_some_and(|o| o.bind(py).is_instance_of::<PyDict>())
            {
                Ok(val)
            } else {
                Err(mismatch())
            }
        }
        _ => Ok(val),
    }
}

/// Classify a NaN-box `Value` into its [`PrimitiveClass`] for the shared union
/// membership test ([`catnip_core::vm::opcode::primitive_membership`]). The
/// numeric tower lives in core; this only maps the value's tags. The `str`,
/// `list`, and `dict` checks need `py` because the PyO3 `Value` NaN-boxes those
/// as PyObject handles. No coercion.
fn value_primitive_class(py: Python<'_>, val: Value) -> catnip_core::vm::opcode::PrimitiveClass {
    catnip_core::vm::opcode::PrimitiveClass {
        int_like: val.is_int() || val.is_bigint(),
        float_like: val.is_float(),
        str_like: val
            .as_pyobject(py)
            .is_some_and(|o| o.bind(py).is_instance_of::<PyString>()),
        bool_like: val.as_bool().is_some(),
        nil_like: val.is_nil(),
        list_like: val
            .as_pyobject(py)
            .is_some_and(|o| o.bind(py).is_instance_of::<PyList>()),
        set_like: val
            .as_pyobject(py)
            .is_some_and(|o| o.bind(py).is_instance_of::<PySet>()),
        dict_like: val
            .as_pyobject(py)
            .is_some_and(|o| o.bind(py).is_instance_of::<PyDict>()),
        tuple_like: val
            .as_pyobject(py)
            .is_some_and(|o| o.bind(py).is_instance_of::<PyTuple>()),
    }
}

/// `PyAny` -> [`PrimitiveClass`] for the composite element pass (the VM's
/// `list`/`dict` elements are Python objects). VM-local mirror of the AST
/// `value_primitive_class_py`, kept here so `vm/core` does not depend on the
/// `ast-executor`-gated `function` module.
fn value_primitive_class_pyany(value: &Bound<'_, PyAny>) -> catnip_core::vm::opcode::PrimitiveClass {
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

/// Whether a Python value is a member of nominal type `name` (struct proxy by
/// name / MRO / traits / tagged-union prefix, enum variant by enum name). VM-local
/// mirror of `value_is_member_of`, for the composite element pass.
fn value_is_member_of_pyany(value: &Bound<'_, PyAny>, py: Python<'_>, name: &str) -> bool {
    use crate::vm::enums::CatnipEnumVariant;
    use crate::vm::structs::CatnipStructProxy;
    if let Ok(proxy) = value.cast::<CatnipStructProxy>() {
        let p = proxy.borrow();
        if p.type_name == name {
            return true;
        }
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

/// Best-effort Catnip type name of a Python value for a composite boundary error
/// message. VM-local mirror of `nominal_value_type_name_ast`.
fn nominal_value_type_name_pyany(value: &Bound<'_, PyAny>) -> String {
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

/// Build an error message for a missing attribute on a Python object, via dir().
pub(crate) fn py_attr_error_msg(py_bound: &Bound<'_, PyAny>, attr: &str, original_msg: &str) -> String {
    if let Ok(dir_list) = py_bound.dir() {
        let candidates: Vec<String> = dir_list
            .iter()
            .filter_map(|item| item.extract::<String>().ok())
            .collect();
        let refs: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
        let suggestions = catnip_tools::suggest::suggest_similar(attr, &refs, 1, 0.6);
        if let Some(hint) = catnip_tools::suggest::format_suggestion(&suggestions) {
            let base = original_msg.strip_prefix("AttributeError: ").unwrap_or(original_msg);
            return format!("AttributeError: {base}. {hint}");
        }
    }
    original_msg.to_string()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VMFallbackStats {
    pub py_binary_div: u64,
    pub py_binary_floordiv: u64,
    pub py_binary_mod: u64,
    pub py_compare_eq: u64,
    pub py_compare_ne: u64,
    pub py_pattern_literal_eq: u64,
}

/// Global generation counter for StoreScope mutations across all VM instances.
/// Used to detect re-entrant modifications (VMFunction called from Python creates
/// a new VM that shares ctx_globals but not self.globals).
static GLOBALS_GEN: AtomicU64 = AtomicU64::new(0);

static PY_BINARY_DIV_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static PY_BINARY_FLOORDIV_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static PY_BINARY_MOD_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static PY_COMPARE_EQ_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static PY_COMPARE_NE_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static PY_PATTERN_LITERAL_EQ_FALLBACKS: AtomicU64 = AtomicU64::new(0);

#[inline]
fn inc(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

pub fn reset_vm_fallback_stats() {
    PY_BINARY_DIV_FALLBACKS.store(0, Ordering::Relaxed);
    PY_BINARY_FLOORDIV_FALLBACKS.store(0, Ordering::Relaxed);
    PY_BINARY_MOD_FALLBACKS.store(0, Ordering::Relaxed);
    PY_COMPARE_EQ_FALLBACKS.store(0, Ordering::Relaxed);
    PY_COMPARE_NE_FALLBACKS.store(0, Ordering::Relaxed);
    PY_PATTERN_LITERAL_EQ_FALLBACKS.store(0, Ordering::Relaxed);
}

pub fn get_vm_fallback_stats() -> VMFallbackStats {
    VMFallbackStats {
        py_binary_div: PY_BINARY_DIV_FALLBACKS.load(Ordering::Relaxed),
        py_binary_floordiv: PY_BINARY_FLOORDIV_FALLBACKS.load(Ordering::Relaxed),
        py_binary_mod: PY_BINARY_MOD_FALLBACKS.load(Ordering::Relaxed),
        py_compare_eq: PY_COMPARE_EQ_FALLBACKS.load(Ordering::Relaxed),
        py_compare_ne: PY_COMPARE_NE_FALLBACKS.load(Ordering::Relaxed),
        py_pattern_literal_eq: PY_PATTERN_LITERAL_EQ_FALLBACKS.load(Ordering::Relaxed),
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BigIntOpsBenchResult {
    pub bits: u32,
    pub iterations: usize,
    pub add_ns: f64,
    pub mul_ns: f64,
    pub floordiv_ns: f64,
    pub mod_ns: f64,
    pub div_ns: f64,
    pub fallback_delta: VMFallbackStats,
}

#[inline]
fn elapsed_ns_per_op(start: Instant, iterations: usize) -> f64 {
    start.elapsed().as_secs_f64() * 1_000_000_000.0 / iterations as f64
}

pub fn bench_bigint_ops(bits: u32, iterations: usize) -> BigIntOpsBenchResult {
    Python::attach(|_py| {
        let mut result = BigIntOpsBenchResult {
            bits,
            iterations,
            ..BigIntOpsBenchResult::default()
        };
        let a_big = (Integer::from(1_u8) << bits) + Integer::from(123_456_789_u64);
        let b_big = (Integer::from(1_u8) << bits.saturating_sub(1)) + Integer::from(987_654_321_u64);
        let a = Value::from_bigint(a_big);
        let b = Value::from_bigint(b_big);
        let before = get_vm_fallback_stats();

        let start = Instant::now();
        for _ in 0..iterations {
            let v = binary_add(a, b).expect("binary_add failed");
            black_box(v.bits());
            if v.is_bigint() {
                v.decref();
            }
        }
        result.add_ns = elapsed_ns_per_op(start, iterations);

        let start = Instant::now();
        for _ in 0..iterations {
            let v = binary_mul(a, b).expect("binary_mul failed");
            black_box(v.bits());
            if v.is_bigint() {
                v.decref();
            }
        }
        result.mul_ns = elapsed_ns_per_op(start, iterations);

        let start = Instant::now();
        for _ in 0..iterations {
            let v = binary_floordiv(a, b).expect("binary_floordiv failed");
            black_box(v.bits());
            if v.is_bigint() {
                v.decref();
            }
        }
        result.floordiv_ns = elapsed_ns_per_op(start, iterations);

        let start = Instant::now();
        for _ in 0..iterations {
            let v = binary_mod(a, b).expect("binary_mod failed");
            black_box(v.bits());
            if v.is_bigint() {
                v.decref();
            }
        }
        result.mod_ns = elapsed_ns_per_op(start, iterations);

        let start = Instant::now();
        for _ in 0..iterations {
            let v = binary_div(a, b).expect("binary_div failed");
            black_box(v.bits());
        }
        result.div_ns = elapsed_ns_per_op(start, iterations);

        let after = get_vm_fallback_stats();
        result.fallback_delta = VMFallbackStats {
            py_binary_div: after.py_binary_div.saturating_sub(before.py_binary_div),
            py_binary_floordiv: after.py_binary_floordiv.saturating_sub(before.py_binary_floordiv),
            py_binary_mod: after.py_binary_mod.saturating_sub(before.py_binary_mod),
            py_compare_eq: after.py_compare_eq.saturating_sub(before.py_compare_eq),
            py_compare_ne: after.py_compare_ne.saturating_sub(before.py_compare_ne),
            py_pattern_literal_eq: after.py_pattern_literal_eq.saturating_sub(before.py_pattern_literal_eq),
        };

        a.decref();
        b.decref();
        result
    })
}

mod error;
pub use error::*;

/// Tracking entry for an ND recursion frame pushed onto the VM stack.
struct NdRecurEntry {
    /// frame_stack.len() before the ND frame was pushed
    caller_depth: usize,
    /// Reference to the NDVmRecur for depth/cache updates on pop
    recur_py: Py<PyAny>,
    /// Cache key for memoization (None if disabled or unhashable)
    memo_key: Option<u64>,
}

impl VM {
    /// Report the Python references reachable only through the VM to the
    /// cyclic GC: per func-table slot its `code_py`/`context` and the
    /// captured pyobj handles of its closure (plus a PyGlobals closure
    /// terminal), and the VM globals' handles. Without this, a session that
    /// defines a function whose slot carries a context is an invisible
    /// cycle (PyPipeline -> VM -> func_table -> context -> globals -> ...)
    /// and the whole cluster leaks (measured: no release at all on
    /// teardown). Dedup-by-slot is inherited from `visit_obj_handles`.
    pub fn gc_traverse(&self, visit: &pyo3::gc::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        for slot in &self.func_table.slots {
            visit.call(&slot.code_py)?;
            if let Some(ref ctx) = slot.context {
                visit.call(ctx)?;
            }
            if let Some(ref closure) = slot.closure {
                closure.gc_traverse(visit)?;
            }
        }
        crate::vm::value::visit_obj_handles(self.globals.values().copied(), visit)?;
        Ok(())
    }

    /// Drop the references reported by `gc_traverse`. Only reached when the
    /// cluster is unreachable (no live frame shares the closures). Draining
    /// is idempotent with the real-death paths (`VM::Drop` mem::takes an
    /// already-empty map; the closure Drop drains an already-drained map).
    pub fn gc_clear(&mut self) {
        for slot in &mut self.func_table.slots {
            slot.context = None;
            if let Some(ref closure) = slot.closure {
                closure.gc_clear();
            }
        }
        let globals = std::mem::take(&mut self.globals);
        for (_, v) in globals {
            decref_discard(&self.struct_registry, v);
        }
    }
}

/// The globals map owns one ref per entry (StoreScope, the Halt sync, the
/// post-call resync and the type definitions all clone into it): they are
/// released at the VM's real destruction. `mem::take` makes a second pass a
/// no-op, and `decref_discard` takes the OBJECT_TABLE lock internally per
/// call -- no outer lock is held here (the CodeObject-pool lesson).
impl Drop for VM {
    fn drop(&mut self) {
        let globals = std::mem::take(&mut self.globals);
        for (_, v) in globals {
            decref_discard(&self.struct_registry, v);
        }
    }
}

/// Stack-based virtual machine for Catnip bytecode.
pub struct VM {
    /// Frame stack
    frame_stack: Vec<Frame>,
    /// Frame pool for reuse
    frame_pool: FramePool,
    /// Global variables (VM-owned)
    globals: IndexMap<String, Value>,
    /// Python context for name resolution fallback
    py_context: Option<Py<PyAny>>,
    /// Cached iter() builtin for GetIter
    cached_iter_fn: Option<Py<PyAny>>,
    /// Cached operator module for binary ops fallback
    cached_operator: Option<Py<PyAny>>,
    /// Cached operator.add for Add fallback
    cached_op_add: Option<Py<PyAny>>,
    /// Cached operator.sub for Sub fallback
    cached_op_sub: Option<Py<PyAny>>,
    /// Cached operator.mul for Mul fallback
    cached_op_mul: Option<Py<PyAny>>,
    /// Cached operator.truediv for Div fallback
    cached_op_truediv: Option<Py<PyAny>>,
    /// Cached operator.floordiv for FloorDiv fallback
    cached_op_floordiv: Option<Py<PyAny>>,
    /// Cached operator.mod for Mod fallback
    cached_op_mod: Option<Py<PyAny>>,
    /// Cached operator.pow for Pow fallback
    cached_op_pow: Option<Py<PyAny>>,
    /// Cached operator.lt for Lt fallback
    cached_op_lt: Option<Py<PyAny>>,
    /// Cached operator.le for Le fallback
    cached_op_le: Option<Py<PyAny>>,
    /// Cached operator.gt for Gt fallback
    cached_op_gt: Option<Py<PyAny>>,
    /// Cached operator.ge for Ge fallback
    cached_op_ge: Option<Py<PyAny>>,
    /// Cached operator.contains for In/NotIn
    cached_op_contains: Option<Py<PyAny>>,
    /// Cached NDTopos singleton for NdEmptyTopos
    cached_nd_topos: Option<Py<PyAny>>,
    /// Cached builtins.set for BuildSet
    cached_set_type: Option<Py<PyAny>>,
    /// Execution tracing enabled
    pub trace: bool,
    /// Profiling enabled
    pub profile: bool,
    /// Opcode counts for profiling
    pub profile_counts: HashMap<u8, u64>,
    /// Hot loop detector for JIT
    pub jit_detector: HotLoopDetector,
    /// Trace recorder (single-thread, no mutex needed)
    pub jit_recorder: TraceRecorder,
    /// JIT executor (behind Mutex for Sync - only used for compilation)
    pub jit: Mutex<Option<JITExecutor>>,
    /// JIT enabled flag
    pub jit_enabled: bool,
    /// Currently tracing flag
    jit_tracing: bool,
    /// Loop offset being traced
    jit_tracing_offset: usize,
    /// Function ID being traced (for function traces)
    jit_tracing_func_id: Option<String>,
    /// Frame stack depth when function tracing started (to detect when to stop)
    jit_tracing_depth: usize,
    /// Recursive call depth during tracing (suspend recording when > 0)
    jit_recursive_depth: usize,
    /// Pending trace: loop became hot, waiting for next iteration to start tracing
    jit_pending_trace: Option<usize>,
    /// Pending function trace: function became hot, waiting for next top-level call
    jit_pending_function_trace: Option<String>,
    /// Guard failed at this loop offset - skip JIT for one iteration
    jit_guard_failed: Option<usize>,
    /// Call stack for source-level stack traces
    call_stack: Vec<CallInfo>,
    /// Source code bytes (set before execution for error reporting)
    source: Option<Vec<u8>>,
    /// Source filename
    filename: String,
    /// Last error context (captured on VMError)
    pub last_error_context: Option<ErrorContext>,
    /// Debug callback (Python callable), called at breakpoints
    pub debug_callback: Option<Py<PyAny>>,
    /// Debug breakpoints (byte offsets in source)
    pub debug_breakpoints: HashSet<u32>,
    /// Current debug stepping mode
    pub debug_step_mode: DebugStepMode,
    /// Frame depth when stepping started (for step over/out)
    pub debug_step_depth: usize,
    /// Last byte offset where we paused (to avoid double-pause on same position)
    debug_last_paused_byte: Option<u32>,
    /// Native VM function table (grow-only, no refcounting)
    pub func_table: FunctionTable,
    /// Native struct type and instance registry
    pub struct_registry: StructRegistry,
    /// PyObject ptr -> StructTypeId, populated by MakeStruct
    struct_type_map: HashMap<usize, StructTypeId>,
    /// Trait registry for trait composition
    pub trait_registry: TraitRegistry,
    /// Enum type registry
    pub enum_registry: EnumRegistry,
    /// Symbol interning table (used by enums)
    pub symbol_table: SymbolTable,
    /// PyObject ptr -> enum_type_id, populated by MakeEnum
    enum_type_map: HashMap<usize, u32>,
    /// Stack of pre-existing global names at each module-level PushBlock
    block_globals_snapshot: Vec<Vec<String>>,
    /// Last source byte offset seen in dispatch loop (for error context)
    last_src_byte: u32,
    /// Memory limit in bytes (0 = disabled)
    memory_limit_bytes: u64,
    /// Instruction counter for periodic RSS checks
    instruction_count: u64,
    /// Interrupt flag (set by external signal to abort execution)
    interrupt_flag: Arc<AtomicBool>,
    /// ND recursion frame tracking stack
    nd_recur_stack: Vec<NdRecurEntry>,
    /// Loop offsets already checked against JIT trace cache (warm-start)
    jit_cache_checked: HashSet<usize>,
}

impl VM {
    /// Create a new VM.
    pub fn new() -> Self {
        Self {
            frame_stack: Vec::with_capacity(crate::constants::VM_FRAME_STACK_INIT),
            frame_pool: FramePool::default(),
            globals: IndexMap::new(),
            py_context: None,
            cached_iter_fn: None,
            cached_operator: None,
            cached_op_add: None,
            cached_op_sub: None,
            cached_op_mul: None,
            cached_op_truediv: None,
            cached_op_floordiv: None,
            cached_op_mod: None,
            cached_op_pow: None,
            cached_op_lt: None,
            cached_op_le: None,
            cached_op_gt: None,
            cached_op_ge: None,
            cached_op_contains: None,
            cached_nd_topos: None,
            cached_set_type: None,
            trace: false,
            profile: false,
            profile_counts: HashMap::new(),
            jit_detector: HotLoopDetector::new(JIT_THRESHOLD_DEFAULT),
            jit_recorder: TraceRecorder::new(),
            jit: Mutex::new(None), // Lazy init
            jit_enabled: false,    // Controlled by Python ConfigManager
            jit_tracing: false,
            jit_tracing_offset: 0,
            jit_tracing_func_id: None,
            jit_tracing_depth: 0,
            jit_recursive_depth: 0,
            jit_pending_trace: None,
            jit_pending_function_trace: None,
            jit_guard_failed: None,
            call_stack: Vec::new(),
            source: None,
            filename: "<input>".to_string(),
            last_error_context: None,
            debug_callback: None,
            debug_breakpoints: HashSet::new(),
            debug_step_mode: DebugStepMode::Disabled,
            debug_step_depth: 0,
            debug_last_paused_byte: None,
            func_table: FunctionTable::new(),
            struct_registry: StructRegistry::new(),
            struct_type_map: HashMap::new(),
            trait_registry: TraitRegistry::new(),
            enum_registry: EnumRegistry::new(),
            symbol_table: SymbolTable::new(),
            enum_type_map: HashMap::new(),
            block_globals_snapshot: Vec::new(),
            last_src_byte: 0,
            memory_limit_bytes: MEMORY_LIMIT_DEFAULT_MB * 1024 * 1024,
            instruction_count: 0,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            nd_recur_stack: Vec::new(),
            jit_cache_checked: HashSet::new(),
        }
    }

    /// Get a clone of the interrupt flag for external signal handlers.
    pub fn interrupt_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.interrupt_flag)
    }

    /// Enable JIT compilation with custom threshold.
    pub fn enable_jit_with_threshold(&mut self, threshold: u32) {
        self.jit_enabled = true;
        // Reset detector with new threshold
        self.jit_detector = HotLoopDetector::new(threshold);
        // Lazy init the JIT executor
        let mut jit = self.jit.lock().unwrap();
        if jit.is_none() {
            *jit = Some(JITExecutor::new(threshold));
        }
    }

    /// Enable JIT compilation.
    pub fn enable_jit(&mut self) {
        self.enable_jit_with_threshold(JIT_THRESHOLD_DEFAULT);
    }

    /// Disable JIT compilation.
    pub fn disable_jit(&mut self) {
        self.jit_enabled = false;
        // Reset JIT state to avoid stale traces when re-enabling
        self.jit_detector = HotLoopDetector::new(JIT_THRESHOLD_DEFAULT);
        self.jit_recorder = TraceRecorder::new();
        self.jit_tracing = false;
        self.jit_tracing_offset = 0;
        self.jit_guard_failed = None;
        // Clear compiled traces
        *self.jit.lock().unwrap() = None;
    }

    /// Handle frame pop for ND recursion: decrement depth, cache result.
    #[inline]
    fn handle_nd_frame_pop(&mut self, py: Python<'_>, result: Value) {
        if let Some(entry) = self.nd_recur_stack.last() {
            if self.frame_stack.len() == entry.caller_depth {
                let entry = self.nd_recur_stack.pop().unwrap();
                if let Ok(nd_recur) = entry.recur_py.bind(py).cast::<crate::nd::NDVmRecur>() {
                    let r = nd_recur.borrow();
                    let d = r.depth_cell().get();
                    if d > 0 {
                        r.depth_cell().set(d - 1);
                    }
                    if let Some(k) = entry.memo_key {
                        r.cache_ref().borrow_mut().insert(k, result.to_pyobject(py));
                    }
                }
            }
        }
    }

    /// Update the JIT executor's bytecode hash from a CodeObject (lazy, cached).
    #[inline]
    fn update_jit_bytecode_hash(&mut self, code: &CodeObject) {
        if !self.jit_enabled {
            return;
        }
        self.update_jit_bytecode_hash_value(code.bytecode_hash());
    }

    /// Set a pre-computed bytecode hash on the JIT executor and loop detector.
    #[inline]
    fn update_jit_bytecode_hash_value(&mut self, hash: u64) {
        if !self.jit_enabled {
            return;
        }
        // Key loop detection by (bytecode_hash, offset) too: a reused VM
        // (REPL/embedding) must re-JIT a second program's hot loop at the same
        // offset instead of treating it as already hot from the first program.
        self.jit_detector.set_bytecode_hash(hash);
        if let Ok(mut jit) = self.jit.lock() {
            if let Some(ref mut executor) = *jit {
                executor.set_bytecode_hash(hash);
            }
        }
    }

    /// Set memory limit in MB (0 = disabled).
    pub fn set_memory_limit(&mut self, mb: u64) {
        self.memory_limit_bytes = mb * 1024 * 1024;
    }

    /// Set the Python context for name resolution.
    pub fn set_context(&mut self, context: Py<PyAny>) {
        self.py_context = Some(context);
    }

    /// Borrow the Python context reference (used by `ContextHost::new()`).
    #[inline]
    pub fn py_context(&self) -> &Option<Py<PyAny>> {
        &self.py_context
    }

    /// Get the cached iter() builtin (used by `ContextHost::new()`).
    /// Panics if `ensure_builtins_cached` hasn't been called.
    #[inline]
    pub fn cached_iter_fn(&self) -> &Py<PyAny> {
        self.cached_iter_fn.as_ref().expect("iter_fn should be cached")
    }

    /// Get cached `operator.contains` ref (used by `ContextHost::new()`).
    /// Panics if `ensure_builtins_cached` hasn't been called.
    #[inline]
    pub fn cached_contains_fn(&self) -> &Py<PyAny> {
        self.cached_op_contains.as_ref().expect("contains_fn should be cached")
    }

    /// Get a cached operator ref by enum variant (used by `ContextHost::new()`).
    /// Panics if `ensure_builtins_cached` hasn't been called.
    #[inline]
    pub fn cached_op(&self, op: super::host::CachedOp) -> &Py<PyAny> {
        use super::host::CachedOp;
        match op {
            CachedOp::Add => self.cached_op_add.as_ref().unwrap(),
            CachedOp::Sub => self.cached_op_sub.as_ref().unwrap(),
            CachedOp::Mul => self.cached_op_mul.as_ref().unwrap(),
            CachedOp::TrueDiv => self.cached_op_truediv.as_ref().unwrap(),
            CachedOp::FloorDiv => self.cached_op_floordiv.as_ref().unwrap(),
            CachedOp::Mod => self.cached_op_mod.as_ref().unwrap(),
            CachedOp::Pow => self.cached_op_pow.as_ref().unwrap(),
            CachedOp::Lt => self.cached_op_lt.as_ref().unwrap(),
            CachedOp::Le => self.cached_op_le.as_ref().unwrap(),
            CachedOp::Gt => self.cached_op_gt.as_ref().unwrap(),
            CachedOp::Ge => self.cached_op_ge.as_ref().unwrap(),
        }
    }

    /// Set source code and filename for error reporting.
    pub fn set_source(&mut self, source: Vec<u8>, filename: String) {
        self.source = Some(source);
        self.filename = filename;
    }

    /// Get the last error context (if any).
    pub fn take_last_error_context(&mut self) -> Option<ErrorContext> {
        self.last_error_context.take()
    }

    /// Invoke debug callback with pre-collected data (avoids borrow conflicts).
    fn invoke_debug_callback(
        &mut self,
        py: Python<'_>,
        start_byte: u32,
        locals_data: &[(String, Value)],
        call_stack_data: &[(String, u32)],
    ) -> Result<DebugStepMode, VMError> {
        let cb = match &self.debug_callback {
            Some(cb) => cb.clone_ref(py),
            None => return Ok(DebugStepMode::Continue),
        };

        let locals_dict = PyDict::new(py);
        for (name, val) in locals_data {
            let _ = locals_dict.set_item(name, val.to_pyobject(py));
        }

        let call_stack = PyList::new(
            py,
            call_stack_data.iter().map(|(name, sb)| {
                PyTuple::new(
                    py,
                    [
                        name.clone().into_pyobject(py).unwrap().into_any().unbind(),
                        (*sb).into_pyobject(py).unwrap().into_any().unbind(),
                    ],
                )
                .unwrap()
                .into_any()
                .unbind()
            }),
        )
        .to_vm(py)?;

        let result = cb.call1(py, (start_byte, locals_dict, call_stack)).to_vm(py)?;
        let action_int: i32 = result.extract(py).unwrap_or(1);
        Ok(DebugStepMode::from_i32(action_int))
    }

    /// Capture error context from current VM state.
    fn capture_error_context(&mut self, error: &VMError) {
        let (error_type, message) = match error {
            VMError::NameError(s) => ("NameError".to_string(), s.clone()),
            VMError::AttributeError(s) => ("AttributeError".to_string(), s.clone()),
            VMError::TypeError(s) => ("TypeError".to_string(), s.clone()),
            VMError::ValueError(s) => ("ValueError".to_string(), s.clone()),
            VMError::IndexError(s) => ("IndexError".to_string(), s.clone()),
            VMError::KeyError(s) => ("KeyError".to_string(), s.clone()),
            VMError::ZeroDivisionError(s) => ("ZeroDivisionError".to_string(), s.clone()),
            VMError::RuntimeError(s) => ("RuntimeError".to_string(), s.clone()),
            VMError::MemoryLimitExceeded(s) => ("MemoryError".to_string(), s.clone()),
            VMError::UserException(info) => (info.type_name.clone(), info.message.clone()),
            VMError::StackUnderflow => ("RuntimeError".to_string(), "WeirdError: VM stack underflow".to_string()),
            VMError::FrameOverflow => (
                "RuntimeError".to_string(),
                "WeirdError: VM frame stack overflow".to_string(),
            ),
            VMError::Interrupted => ("KeyboardInterrupt".to_string(), "execution interrupted".to_string()),
            // Exit and control flow signals - no error context needed
            VMError::Exit(_) | VMError::Return(_) | VMError::Break | VMError::Continue => return,
        };

        // Use last_src_byte tracked in dispatch loop (always up-to-date)
        let start_byte = self.last_src_byte;

        // Snapshot the call stack
        let call_stack: Vec<(String, u32)> = self
            .call_stack
            .iter()
            .map(|ci| (ci.name.clone(), ci.call_start_byte))
            .collect();

        self.last_error_context = Some(ErrorContext {
            error_type,
            message,
            start_byte,
            call_stack,
        });
    }

    /// Cache builtins for fast access in dispatch loop.
    fn ensure_builtins_cached(&mut self, py: Python<'_>) -> PyResult<()> {
        if self.cached_iter_fn.is_none() {
            let builtins = py.import("builtins")?;
            self.cached_iter_fn = Some(builtins.getattr("iter")?.unbind());
            let op_mod = py.import("operator")?;
            self.cached_op_add = Some(op_mod.getattr("add")?.unbind());
            self.cached_op_sub = Some(op_mod.getattr("sub")?.unbind());
            self.cached_op_mul = Some(op_mod.getattr("mul")?.unbind());
            self.cached_op_truediv = Some(op_mod.getattr("truediv")?.unbind());
            self.cached_op_floordiv = Some(op_mod.getattr("floordiv")?.unbind());
            self.cached_op_mod = Some(op_mod.getattr("mod")?.unbind());
            self.cached_op_pow = Some(op_mod.getattr("pow")?.unbind());
            self.cached_op_lt = Some(op_mod.getattr("lt")?.unbind());
            self.cached_op_le = Some(op_mod.getattr("le")?.unbind());
            self.cached_op_gt = Some(op_mod.getattr("gt")?.unbind());
            self.cached_op_ge = Some(op_mod.getattr("ge")?.unbind());
            self.cached_op_contains = Some(op_mod.getattr("contains")?.unbind());
            self.cached_operator = Some(op_mod.unbind().into());
        }
        Ok(())
    }

    /// True when a compiled trace exists for the loop at `loop_offset`.
    fn jit_has_compiled(&self, loop_offset: usize) -> bool {
        let jit = self.jit.lock().unwrap();
        jit.as_ref().map(|e| e.has_compiled(loop_offset)).unwrap_or(false)
    }

    /// Try to load a compiled trace for `loop_offset` from the persistent
    /// cache. A hit pins the offset: a compiled offset never re-arms hot
    /// detection, otherwise guard failures (e.g. a type flip) re-trace and
    /// recompile mid-loop -- a codegen path with known latent bugs. The loop
    /// stays interpreted when guards reject; correctness over speed.
    fn jit_compile_from_cache(&mut self, loop_offset: usize) -> bool {
        let hit = {
            let mut jit = self.jit.lock().unwrap();
            jit.as_mut()
                .map(|e| e.try_compile_from_cache(loop_offset))
                .unwrap_or(false)
        };
        if hit {
            self.jit_detector.mark_compiled_offset(loop_offset);
        }
        hit
    }

    /// Warm start: one cache probe per loop offset per VM, on the first
    /// encounter of the loop's backward edge.
    fn jit_warm_start(&mut self, loop_offset: usize, label: &str) {
        if !self.jit_cache_checked.insert(loop_offset) {
            return;
        }
        if self.jit_compile_from_cache(loop_offset) && self.trace {
            eprintln!("[JIT] Warm-start: {} loop at {} loaded from cache", label, loop_offset);
        }
    }

    /// Compile a finished recording. Shared tail of every trace-stop site
    /// (while-loop Jump, ForRangeInt done/loop-back, ForRangeStep).
    fn jit_compile_finished_trace(&mut self, trace: Option<Trace>, label: &str) {
        let Some(t) = trace else { return };
        if self.trace {
            eprintln!(
                "[JIT] {} trace recorded: {} ops, {} iterations, int_only={}",
                label,
                t.ops.len(),
                t.iterations,
                t.is_int_only
            );
            for (i, op) in t.ops.iter().enumerate() {
                eprintln!("[JIT]   op[{}]: {:?}", i, op);
            }
        }
        if !t.is_compilable() {
            if self.trace {
                eprintln!("[JIT] {} trace has fallbacks, not compilable", label);
            }
            return;
        }
        let mut jit = self.jit.lock().unwrap();
        if let Some(ref mut executor) = *jit {
            match executor.compile_trace(t) {
                Ok(true) => {
                    if self.trace {
                        eprintln!("[JIT] {} trace compiled", label);
                    }
                }
                Ok(false) => {
                    if self.trace {
                        eprintln!("[JIT] {} trace not compilable", label);
                    }
                }
                Err(e) => {
                    if self.trace {
                        eprintln!("[JIT] {} compilation failed: {}", label, e);
                    }
                }
            }
        }
    }

    /// Record a loop-back edge; once the single-iteration budget is reached,
    /// stop recording and compile.
    fn jit_record_loop_back_and_maybe_compile(&mut self, ip: usize, label: &str) {
        self.jit_recorder.record_loop_back(ip);
        // A trace represents a single loop body.
        const TRACE_SINGLE_ITERATIONS: u32 = 1;
        if self.jit_recorder.iterations() >= TRACE_SINGLE_ITERATIONS {
            let trace = self.jit_recorder.stop();
            self.jit_tracing = false;
            self.jit_compile_finished_trace(trace, label);
        }
    }

    /// Validate every guard class for the compiled loop at `loop_offset` and,
    /// when all pass, execute the compiled trace and write the modified slots
    /// back into the frame (and the synced globals). Returns the trace's
    /// return code, or None when any guard rejects or no executor is
    /// installed -- the caller falls through to the interpreter. Post-exit
    /// control flow stays at the call-site: the while arm jumps back to the
    /// condition, the ForRangeInt arm handles the ret == -1 side exit.
    fn try_enter_jit_loop(
        &mut self,
        py: Python<'_>,
        frame: &mut Frame,
        host: &dyn super::host::VmHost,
        code: &CodeObject,
        loop_offset: usize,
    ) -> VMResult<Option<i64>> {
        let (guards, func_guards, func_slot_guards, loop_max_slot, slot_kinds) = {
            let jit = self.jit.lock().unwrap();
            let e = jit.as_ref();
            (
                e.and_then(|e| e.get_guards(loop_offset)).cloned(),
                e.and_then(|e| e.get_func_guards(loop_offset)).cloned(),
                e.and_then(|e| e.get_func_slot_guards(loop_offset)).cloned(),
                e.and_then(|e| e.get_loop_max_slot(loop_offset)),
                e.and_then(|e| e.get_slot_type_guards(loop_offset)).cloned(),
            )
        };

        // Scope-name guards: each guarded name must still resolve to its
        // trace-time value; passing values are seeded into the locals array.
        let mut guard_locals: Vec<(usize, i64)> = Vec::new();
        if let Some(ref guards) = guards {
            for (name, expected_value, slot) in guards {
                match resolve_jit_guard_value(
                    py,
                    name,
                    &frame.closure_scope,
                    host,
                    &self.globals,
                    &self.struct_registry,
                ) {
                    Some(val) if val == *expected_value => guard_locals.push((*slot, val)),
                    _ => return Ok(None),
                }
            }
        }

        // Function-identity guards: an inlined scope function must still
        // resolve to the same value, else the compiled loop would run a
        // stale inlined body.
        if let Some(ref fguards) = func_guards {
            for (name, expected_bits) in fguards {
                match resolve_jit_name_bits(
                    py,
                    name,
                    &frame.closure_scope,
                    host,
                    &self.globals,
                    &self.struct_registry,
                ) {
                    Some(bits) if bits == *expected_bits => {}
                    _ => return Ok(None),
                }
            }
        }

        // Local-slot function-identity guards: an inlined function held in a
        // frame local must still be the same function (read from frame.locals).
        if let Some(ref fsg) = func_slot_guards {
            for (slot, expected_bits) in fsg {
                match frame.locals.get(*slot) {
                    Some(v) if v.bits() == *expected_bits => {}
                    _ => return Ok(None),
                }
            }
        }

        for (i, v) in frame.locals.iter().enumerate() {
            if func_slot_guards
                .as_ref()
                .is_some_and(|fsg| fsg.iter().any(|(s, _)| *s == i))
            {
                continue;
            }
            // Typed contract for traced slots: compiled code unboxes each
            // guarded slot with its trace-time type (GuardFloat -> f64 bitcast,
            // GuardInt -> raw payload) and never re-checks at runtime --
            // entering with the wrong type yields silent garbage. Untraced
            // slots only need to keep heap types out (write-back would
            // overwrite them without a release).
            let ok = match slot_kinds
                .as_ref()
                .and_then(|sk| sk.iter().find(|(slot, _)| *slot == i))
            {
                Some((_, true)) => v.is_float(),
                Some((_, false)) => v.is_int() || v.is_bool() || v.is_nil(),
                None => v.is_int() || v.is_bool() || v.is_nil() || v.is_float(),
            };
            if !ok {
                return Ok(None);
            }
        }

        // Execute compiled code (pass NaN-boxed bits).
        let mut locals_i64: Vec<i64> = frame.locals.iter().map(|v| v.bits() as i64).collect();

        // Extend locals array so every slot codegen addresses is in bounds:
        // both guard slots and the loop's max trace.locals_used slot. The
        // warm-start path never extended frame.locals, so guard slots alone
        // undersize the array.
        let guard_max = guard_locals.iter().map(|(s, _)| *s).max();
        let need = loop_max_slot.into_iter().chain(guard_max).max();
        if let Some(max_slot) = need {
            if max_slot >= locals_i64.len() {
                locals_i64.resize(max_slot + 1, 0);
            }
        }

        // Copy guard values into locals array
        for (slot, value) in guard_locals {
            locals_i64[slot] = value;
        }

        // Snapshot pre-JIT values to detect which slots changed
        let snapshot: Vec<i64> = locals_i64.clone();

        let result = {
            let jit = self.jit.lock().unwrap();
            if let Some(ref executor) = *jit {
                // SAFETY: executor.execute jumps into the Cranelift-compiled
                // trace for loop_offset; locals_i64 was resized above to cover
                // every slot the trace addresses, so all codegen loads/stores
                // stay in bounds.
                unsafe { executor.execute(loop_offset, &mut locals_i64) }
            } else {
                None
            }
        };
        let Some(ret) = result else { return Ok(None) };

        // Restore only slots actually modified by JIT
        // (values are NaN-boxed by codegen)
        for (i, &val) in locals_i64.iter().enumerate() {
            if i < frame.locals.len() && val != snapshot[i] {
                let new_val = Value::from_raw_scalar(val as u64);
                let old = frame.locals[i];
                decref_discard(&self.struct_registry, old);
                frame.locals[i] = new_val;
                if i < code.varnames.len() {
                    if let Some(old) = host.store_global(py, &code.varnames[i], new_val)? {
                        decref_discard(&self.struct_registry, old);
                    }
                }
            }
        }
        Ok(Some(ret))
    }

    fn push_struct_init_frame(
        &mut self,
        py: Python<'_>,
        inst_val: Value,
        init_fn: Option<Py<PyAny>>,
        frame: &mut Frame,
    ) -> VMResult<bool> {
        let Some(init_fn) = init_fn else { return Ok(false) };
        let init_bound = init_fn.bind(py);
        let init_data = if let Ok(f) = init_bound.cast::<VMFunction>() {
            let r = f.borrow();
            let code = Arc::clone(&r.vm_code.borrow(py).inner);
            let cl = r.native_closure.clone();
            drop(r);
            Some((code, cl))
        } else if let Ok(vm_code) = init_bound.getattr("vm_code") {
            Some((convert_code_object(py, &vm_code).to_vm(py)?, None))
        } else {
            None
        };
        if let Some((new_code, native_closure)) = init_data {
            self.struct_registry.incref(inst_val.as_struct_instance_idx().unwrap());
            frame.push(inst_val);
            let mut new_frame = Frame::with_code(new_code);
            new_frame.set_local(0, inst_val);
            new_frame.closure_scope = native_closure;
            new_frame.discard_return = true;
            self.setup_super_proxy(py, inst_val, None, &mut new_frame)?;
            let old = std::mem::replace(frame, new_frame);
            self.frame_stack.push(old);
            return Ok(true);
        }
        Ok(false)
    }

    /// Setup a super proxy on a frame for method calls on struct instances.
    /// If `super_source_type` is Some, resolve super from that type's parent (chain resolution).
    /// Otherwise, resolve from the instance's own type.
    fn setup_super_proxy(
        &self,
        py: Python<'_>,
        inst_val: Value,
        super_source_type: Option<String>,
        frame: &mut Frame,
    ) -> VMResult<()> {
        // Resolve the instance's real type name
        let real_type_name = if let Some(idx) = inst_val.as_struct_instance_idx() {
            self.struct_registry
                .with_instance(idx, |inst| inst.type_id)
                .and_then(|type_id| self.struct_registry.get_type(type_id).map(|ty| ty.name.clone()))
        } else {
            let inst_py = inst_val.to_pyobject(py);
            let inst_bound = inst_py.bind(py);
            inst_bound
                .cast::<super::structs::CatnipStructProxy>()
                .ok()
                .map(|proxy| proxy.borrow().type_name.clone())
        };

        let Some(real_name) = real_type_name else {
            return Ok(());
        };

        let Some(real_type) = self.struct_registry.find_type_by_name(&real_name) else {
            return Ok(());
        };

        // Only types with parents need super
        if real_type.parent_names.is_empty() {
            return Ok(());
        }

        let mro = &real_type.mro;

        // Find position: super_source_type tells us which type's method we're in
        let start_pos = if let Some(ref source) = super_source_type {
            // Find source in MRO and skip past it
            mro.iter().position(|n| n == source).map(|p| p + 1).unwrap_or(1)
        } else {
            // Normal call from the struct's own method: skip self (pos 0)
            1
        };

        if start_pos >= mro.len() {
            return Ok(());
        }

        // Collect methods from MRO[start_pos:], first-wins, with provenance
        let mut methods: IndexMap<String, Py<PyAny>> = IndexMap::new();
        let mut method_sources: HashMap<String, String> = HashMap::new();
        for mro_type_name in &mro[start_pos..] {
            if let Some(ty) = self.struct_registry.find_type_by_name(mro_type_name) {
                for (k, v) in &ty.methods {
                    if !methods.contains_key(k) {
                        methods.insert(k.clone(), v.clone_ref(py));
                        method_sources.insert(k.clone(), mro_type_name.clone());
                    }
                }
            }
        }

        if !methods.is_empty() {
            let inst_py = inst_val.to_pyobject(py);
            let native_idx = inst_val.as_struct_instance_idx();
            let proxy = Py::new(
                py,
                super::structs::SuperProxy {
                    methods,
                    instance: inst_py,
                    method_sources,
                    native_instance_idx: native_idx,
                    native_registry_id: native_idx.map_or(0, |_| self.struct_registry.id()),
                },
            )
            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
            frame.super_proxy = Some(proxy.into_any());
        }
        Ok(())
    }

    /// Unwrap a BoundCatnipMethod callable: extract the inner function and
    /// prepend the bound instance to the argument list. Any other callable
    /// passes through unchanged. Shared by Call, CallKw and TailCall.
    ///
    /// Voie A: takes ownership of the popped `args`; on error they are
    /// released here (CallKw additionally releases its kw values at the call
    /// site). On success the prepended instance ref is owned by the returned
    /// args exactly once -- `bound_instance` is a bit-copy for
    /// setup_super_proxy, which only reads it.
    #[allow(clippy::type_complexity)]
    fn unwrap_bound_method<'py>(
        &self,
        py: Python<'py>,
        py_func_bound: &Bound<'py, PyAny>,
        args: Vec<Value>,
    ) -> VMResult<(Bound<'py, PyAny>, Vec<Value>, Option<Value>, Option<String>)> {
        let Ok(bound_method) = py_func_bound.cast::<crate::core::BoundCatnipMethod>() else {
            return Ok((py_func_bound.clone(), args, None, None));
        };
        let bm = bound_method.borrow();
        let actual_func = bm.func.bind(py).clone();
        // Use native struct index if available (avoids CatnipStructProxy round-trip)
        let instance_val = if let Some(idx) = bm
            .native_instance_idx
            .filter(|_| bm.native_registry_id == self.struct_registry.id())
        {
            // Native fast path only when the bound method belongs to
            // this VM's registry; a cross-VM method's idx would name an
            // unrelated slot, so fall back to the proxy round-trip.
            self.struct_registry.incref(idx);
            Value::from_struct_instance(idx)
        } else {
            match Value::from_pyobject(py, bm.instance.bind(py)) {
                Ok(v) => v,
                Err(e) => {
                    release_operands(&self.struct_registry, &args);
                    return Err(VMError::RuntimeError(e.to_string()));
                }
            }
        };
        let super_source_type = bm.super_source_type.clone();
        let mut new_args = Vec::with_capacity(args.len() + 1);
        new_args.push(instance_val);
        new_args.extend_from_slice(&args);
        Ok((actual_func, new_args, Some(instance_val), super_source_type))
    }

    /// Convert owned args to Python handles for a direct call to a Python
    /// callable, prepending the host context when the callable requests it
    /// (pass_context). Shared by the Python branches of Call, CallKw and
    /// TailCall.
    ///
    /// Voie A: the popped args are owned and `to_pyobject` only reads them
    /// (each returned entry holds its own ref) -- they are released here,
    /// before the caller's fallible call chain, which would otherwise leak
    /// them on a `?` exit. Nothing may read `args` after this returns.
    fn build_python_call_args(
        &self,
        py: Python<'_>,
        host: &dyn super::host::VmHost,
        actual_func: &Bound<'_, PyAny>,
        args: &[Value],
    ) -> VMResult<Vec<Py<PyAny>>> {
        let pass_context = actual_func
            .getattr("pass_context")
            .map(|attr| attr.is_truthy().unwrap_or(false))
            .unwrap_or(false);

        let mut args_py: Vec<Py<PyAny>> = Vec::with_capacity(args.len() + usize::from(pass_context));

        if pass_context {
            if let Some(ref ctx) = host.context() {
                args_py.push(ctx.clone_ref(py));
            } else {
                release_operands(&self.struct_registry, args);
                return Err(VMError::RuntimeError(
                    "Function requires context but VM has no context available".to_string(),
                ));
            }
        }

        for arg in args {
            args_py.push(arg.to_pyobject(py));
        }
        release_operands(&self.struct_registry, args);
        Ok(args_py)
    }

    /// Build the (instance, static, abstract) method maps from a compiled
    /// method list -- (name, code_obj_or_None[, is_static]) per entry --
    /// closing each body over the current frame. Shared by MakeStruct and
    /// MakeTrait.
    #[allow(clippy::type_complexity)]
    fn collect_method_maps(
        &self,
        py: Python<'_>,
        host: &dyn super::host::VmHost,
        frame: &Frame,
        methods: &Bound<'_, PyAny>,
    ) -> VMResult<(
        IndexMap<String, Py<PyAny>>,
        IndexMap<String, Py<PyAny>>,
        HashSet<MethodKey>,
    )> {
        let mut instance_map: IndexMap<String, Py<PyAny>> = IndexMap::new();
        let mut static_map: IndexMap<String, Py<PyAny>> = IndexMap::new();
        let mut abstract_set: HashSet<MethodKey> = HashSet::new();
        for method_result in methods.try_iter().map_err(|e| VMError::RuntimeError(e.to_string()))? {
            let method_pair = method_result.map_err(|e| VMError::RuntimeError(e.to_string()))?;
            let pair = cast_tuple(&method_pair)?;
            let method_name: String = tuple_extract(pair, 0)?;

            // Read is_static flag (3rd element, defaults to false)
            let is_static: bool = if pair.len() > 2 {
                tuple_extract(pair, 2).unwrap_or(false)
            } else {
                false
            };

            let code_obj = tuple_get(pair, 1)?;

            // Abstract method: code_obj is None
            if code_obj.is_none() {
                abstract_set.insert(MethodKey {
                    name: method_name,
                    kind: if is_static {
                        super::structs::MethodKind::Static
                    } else {
                        super::structs::MethodKind::Instance
                    },
                });
                continue;
            }

            let captured = {
                let mut cap: IndexMap<String, Value> = IndexMap::new();
                if let Some(ref code) = frame.code {
                    for (lname, &slot_idx) in &code.slotmap {
                        // See MakeFunction: only suppress capture of a global
                        // homonym at the top-level frame; a nested-frame slot
                        // is a real local that shadows the global.
                        if frame.closure_scope.is_none() && host.has_global(py, lname) {
                            continue;
                        }
                        let val = frame.get_local(slot_idx);
                        if !val.is_nil() && !val.is_invalid() {
                            cap.insert(lname.clone(), val);
                        }
                    }
                }
                portabilize_struct_values(py, &mut cap, &self.struct_registry);
                cap
            };

            let parent = host.build_closure_parent(py, frame.closure_scope.as_ref());
            let native_scope = NativeClosureScope::new(captured, parent);
            let context_for_func = host.context().as_ref().map(|c| c.clone_ref(py));
            let code_py: Py<PyCodeObject> = code_obj
                .cast::<PyCodeObject>()
                .map_err(|e| VMError::TypeError(format!("Expected CodeObject: {e}")))?
                .clone()
                .unbind();
            let func = Py::new(
                py,
                VMFunction::create_native(py, code_py, Some(native_scope), context_for_func),
            )
            .map_err(|e| VMError::RuntimeError(e.to_string()))?;

            if is_static {
                static_map.insert(method_name, func.into_any());
            } else {
                instance_map.insert(method_name, func.into_any());
            }
        }
        Ok((instance_map, static_map, abstract_set))
    }

    /// Execute a code object and return the result.
    pub fn execute(&mut self, py: Python<'_>, code: Arc<CodeObject>, args: &[Value]) -> VMResult<Value> {
        self.execute_with_closure(py, code, args, None)
    }

    /// Execute a code object with an optional native closure scope.
    pub fn execute_with_closure(
        &mut self,
        py: Python<'_>,
        code: Arc<CodeObject>,
        args: &[Value],
        closure_scope: Option<NativeClosureScope>,
    ) -> VMResult<Value> {
        // Mark a dispatch loop as active on this thread so a nested import can
        // tell a live parent VM from a stale leftover func_table pointer.
        let _depth_guard = super::value::VmDepthGuard::enter();

        // Set bytecode hash for JIT trace cache and reset warm-start tracking
        self.update_jit_bytecode_hash(&code);
        self.jit_cache_checked.clear();

        // Create initial frame
        let mut frame = Frame::with_code(code);
        frame.bind_args(py, &self.struct_registry, args, None);
        frame.closure_scope = closure_scope;

        self.frame_stack.push(frame);

        // Clear previous error context
        self.last_error_context = None;
        self.call_stack.clear();

        // Install thread-local pointers for Value conversions.
        // Save previous pointers so re-entrant VM calls (e.g. import) restore them.
        let prev_sym = super::value::save_symbol_table();
        let prev_enum = super::value::save_enum_registry();
        super::value::set_struct_registry(&self.struct_registry as *const _);
        super::value::set_func_table(&self.func_table as *const _);
        super::value::set_symbol_table(&self.symbol_table as *const _ as *mut _);
        super::value::set_enum_registry(&self.enum_registry as *const _ as *mut _);

        // Run dispatch loop
        let result = match self.run(py) {
            Ok(v) => v,
            Err(e) => {
                self.capture_error_context(&e);
                self.nd_recur_stack.clear();
                while let Some(frame) = self.frame_stack.pop() {
                    self.frame_pool.free(frame, &self.struct_registry);
                }
                super::value::restore_symbol_table(prev_sym);
                super::value::restore_enum_registry(prev_enum);
                return Err(e);
            }
        };

        // Clean up
        while let Some(frame) = self.frame_stack.pop() {
            self.frame_pool.free(frame, &self.struct_registry);
        }

        super::value::restore_symbol_table(prev_sym);
        super::value::restore_enum_registry(prev_enum);
        Ok(result)
    }

    /// Execute a code object with a custom VmHost (no Python Context needed).
    pub fn execute_with_host(
        &mut self,
        py: Python<'_>,
        code: Arc<CodeObject>,
        args: &[Value],
        host: &dyn super::host::VmHost,
        closure_scope: Option<NativeClosureScope>,
    ) -> VMResult<Value> {
        // Mark a dispatch loop as active on this thread (see VmDepthGuard).
        let _depth_guard = super::value::VmDepthGuard::enter();

        // Set bytecode hash for JIT trace cache and reset warm-start tracking.
        // Mirrors execute_with_closure: without it the trace cache key collapses
        // to hash 0, so loops sharing an offset collide across programs and a
        // stale trace miscompiles the current loop.
        self.update_jit_bytecode_hash(&code);
        self.jit_cache_checked.clear();

        let mut frame = Frame::with_code(code);
        frame.bind_args(py, &self.struct_registry, args, None);
        frame.closure_scope = closure_scope;

        self.frame_stack.push(frame);
        self.last_error_context = None;
        self.call_stack.clear();

        let prev_sym = super::value::save_symbol_table();
        let prev_enum = super::value::save_enum_registry();
        super::value::set_struct_registry(&self.struct_registry as *const _);
        super::value::set_func_table(&self.func_table as *const _);
        super::value::set_symbol_table(&self.symbol_table as *const _ as *mut _);
        super::value::set_enum_registry(&self.enum_registry as *const _ as *mut _);

        let result = match self.run_with_host(py, host) {
            Ok(v) => v,
            Err(e) => {
                self.capture_error_context(&e);
                self.nd_recur_stack.clear();
                while let Some(frame) = self.frame_stack.pop() {
                    self.frame_pool.free(frame, &self.struct_registry);
                }
                super::value::restore_symbol_table(prev_sym);
                super::value::restore_enum_registry(prev_enum);
                return Err(e);
            }
        };

        while let Some(frame) = self.frame_stack.pop() {
            self.frame_pool.free(frame, &self.struct_registry);
        }

        super::value::restore_symbol_table(prev_sym);
        super::value::restore_enum_registry(prev_enum);
        Ok(result)
    }

    /// Get globals as a HashMap reference for syncing back to Python.
    pub fn get_globals(&self) -> &IndexMap<String, Value> {
        &self.globals
    }

    /// Main dispatch loop with the default ContextHost.
    fn run(&mut self, py: Python<'_>) -> VMResult<Value> {
        // Cache builtins once at start of execution
        self.ensure_builtins_cached(py).to_vm(py)?;

        // Build host: owns ctx_globals, operator refs, py_context, and iter_fn
        let host = super::host::ContextHost::new(py, self);
        self.dispatch(py, &host)
    }

    /// Main dispatch loop with a custom host.
    fn run_with_host(&mut self, py: Python<'_>, host: &dyn super::host::VmHost) -> VMResult<Value> {
        self.dispatch(py, host)
    }

    /// Outer dispatch loop with exception unwinding.
    /// Call a VMFunction (by func_table index) synchronously in the CURRENT VM
    /// and return its result -- the broadcast-callback counterpart of PureVM's
    /// `run_sync`. Runs the callee to completion on an isolated `frame_stack`
    /// AND `call_stack`, so a per-element broadcast call neither disturbs the
    /// outer dispatch nor pays the fresh child VM + `clone_from_parent` of
    /// `VMFunction::__call__` (the O(N^2) source). `args` are MOVED into the
    /// callee frame (`bind_args` copies the bits); the caller must not release
    /// them afterwards.
    fn call_vmfunc_sync(
        &mut self,
        idx: u32,
        args: &[Value],
        py: Python<'_>,
        host: &dyn super::host::VmHost,
    ) -> VMResult<Value> {
        let (code, closure) = {
            let slot = self.func_table.get(idx).ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "invalid function index {idx} (table has {} entries)",
                    self.func_table.slots.len()
                ))
            })?;
            (Arc::clone(&slot.code), slot.closure.clone())
        };
        let mut new_frame = self.frame_pool.alloc_with_code(code);
        new_frame.bind_args(py, &self.struct_registry, args, None);
        new_frame.closure_scope = closure;
        // Isolate both stacks: the callee's internal calls balance within the
        // empty call_stack, so its final Return sees an empty call_stack (no
        // spurious pop of an outer entry) and an empty frame_stack (dispatch
        // returns its value). Restored on every path, including error.
        let saved_frames = std::mem::take(&mut self.frame_stack);
        let saved_calls = std::mem::take(&mut self.call_stack);
        // Also isolate nd_recur_stack: `handle_nd_frame_pop` tests
        // `frame_stack.len()` against a `caller_depth` recorded on the OUTER
        // stack, so an outer ND recursion (`~~`) in flight must not observe this
        // isolated sub-dispatch's frame depths and misfire (pop an ND entry early
        // or cache a wrong memo value).
        let saved_nd = std::mem::take(&mut self.nd_recur_stack);
        // Isolate the JIT trace state for the same reason: `jit_tracing_depth`
        // is measured against `frame_stack.len()` (just isolated), and a loop
        // trace carries no `func_id` so a Return never finalizes it. A trace
        // opened inside this sub-dispatch would otherwise leak `jit_tracing =
        // true` back to the outer dispatch (polluting whatever it records next),
        // and an outer trace in flight would swallow this callback's opcodes.
        // Any trace started here is dropped on restore. JIT-only, memory-safe
        // either way (a polluted trace deopts on its guards). The detector stays
        // shared: it holds hit counters, not open-trace state.
        let saved_jit_tracing = std::mem::take(&mut self.jit_tracing);
        let saved_jit_offset = std::mem::take(&mut self.jit_tracing_offset);
        let saved_jit_func_id = std::mem::take(&mut self.jit_tracing_func_id);
        let saved_jit_depth = std::mem::take(&mut self.jit_tracing_depth);
        let saved_jit_recorder = std::mem::replace(&mut self.jit_recorder, TraceRecorder::new());
        self.frame_stack.push(new_frame);
        let result = self.dispatch(py, host);
        self.frame_stack = saved_frames;
        self.call_stack = saved_calls;
        self.nd_recur_stack = saved_nd;
        self.jit_tracing = saved_jit_tracing;
        self.jit_tracing_offset = saved_jit_offset;
        self.jit_tracing_func_id = saved_jit_func_id;
        self.jit_tracing_depth = saved_jit_depth;
        self.jit_recorder = saved_jit_recorder;
        result
    }

    /// In-VM fast path for `list.[(x) => ...]`: map a VMFunction callback over a
    /// flat list without the fresh-child-VM-per-element of `broadcast_map` ->
    /// `VMFunction::__call__` (the O(N^2) `clone_from_parent`). Struct elements
    /// are deep-copied for (5,1) isolation (mutations at any depth stay private,
    /// source untouched); the result carries the mutated copy, and no transplant
    /// /materialization runs (so the pass-through leak cannot arise). Returns
    /// `None` (fall back to the generic path) when the target is not a flat list
    /// -- a nested list/tuple element needs the recursive `broadcast_map`.
    fn try_broadcast_map_in_vm(
        &mut self,
        py: Python<'_>,
        host: &dyn super::host::VmHost,
        operator_val: Value,
        target_val: &Value,
    ) -> VMResult<Value> {
        let idx = operator_val.as_vmfunc_idx();
        let target_py = target_val.to_pyobject(py);
        let result = self.broadcast_map_in_vm(py, host, idx, target_py.bind(py))?;
        Value::from_pyobject(py, result.bind(py)).to_vm(py)
    }

    /// Recursive in-VM broadcast map, mirroring `core::broadcast::ops::broadcast_map`
    /// EXACTLY (scalar leaf; list/tuple/other-iterable recurse and preserve the
    /// container; non-iterable non-scalar = struct leaf) but invoking the VMFunc
    /// leaf callback in the current VM (`map_leaf_in_vm`) instead of
    /// `VMFunction::__call__`. So a NESTED list/tuple of structs isolates at every
    /// depth ((5,1)) and stays O(N) -- closing the flat-vs-nested divergence the
    /// flat-only fast path left. No mid-recursion bail (a partially-run callback
    /// with side effects must not be replayed by a fallback).
    fn broadcast_map_in_vm(
        &mut self,
        py: Python<'_>,
        host: &dyn super::host::VmHost,
        idx: u32,
        target: &Bound<'_, PyAny>,
    ) -> VMResult<Py<PyAny>> {
        let scalar = target
            .get_type()
            .name()
            .ok()
            .and_then(|n| n.to_str().map(|s| s.to_string()).ok())
            .map(|n| matches!(n.as_str(), "int" | "float" | "str" | "bool" | "NoneType"))
            .unwrap_or(false);
        if scalar {
            return self.map_leaf_in_vm(py, host, idx, target);
        }
        // Exact type (not isinstance), matching the reference `broadcast_map`:
        // a subclass of tuple must route through the other-iterable branch (-> list),
        // not the tuple branch, or the result container type diverges from AST.
        let is_list = target.is_exact_instance_of::<pyo3::types::PyList>();
        let is_tuple = target.is_exact_instance_of::<pyo3::types::PyTuple>();
        if is_list || is_tuple {
            let out = pyo3::types::PyList::empty(py);
            for item in target.try_iter().to_vm(py)? {
                let mapped = self.broadcast_map_in_vm(py, host, idx, &item.to_vm(py)?)?;
                out.append(mapped).to_vm(py)?;
            }
            return if is_tuple {
                Ok(pyo3::types::PyTuple::new(py, &out).to_vm(py)?.into_any().unbind())
            } else {
                Ok(out.into_any().unbind())
            };
        }
        match target.try_iter() {
            Ok(iter) => {
                let out = pyo3::types::PyList::empty(py);
                for item in iter {
                    let mapped = self.broadcast_map_in_vm(py, host, idx, &item.to_vm(py)?)?;
                    out.append(mapped).to_vm(py)?;
                }
                Ok(out.into_any().unbind())
            }
            Err(_) => self.map_leaf_in_vm(py, host, idx, target),
        }
    }

    /// Invoke the VMFunc leaf callback in the current VM on one element: a struct
    /// element is deep-copied first (`deep_snapshot`, (5,1) isolation), any other
    /// value passes through. The in-VM counterpart of `broadcast_map`'s
    /// `func.call1((leaf_arg(..),))`.
    fn map_leaf_in_vm(
        &mut self,
        py: Python<'_>,
        host: &dyn super::host::VmHost,
        idx: u32,
        leaf: &Bound<'_, PyAny>,
    ) -> VMResult<Py<PyAny>> {
        let elem = Value::from_pyobject(py, leaf).to_vm(py)?;
        let arg = if elem.is_struct_instance() {
            let snap = self
                .struct_registry
                .deep_snapshot(elem.as_struct_instance_idx().unwrap());
            decref_discard(&self.struct_registry, elem);
            Value::from_struct_instance(snap)
        } else {
            elem
        };
        let res = self.call_vmfunc_sync(idx, &[arg], py, host)?;
        let py_res = res.to_pyobject(py);
        decref_discard(&self.struct_registry, res);
        Ok(py_res)
    }

    fn dispatch(&mut self, py: Python<'_>, host: &dyn super::host::VmHost) -> VMResult<Value> {
        let mut frame = match self.frame_stack.pop() {
            Some(f) => f,
            None => return Ok(Value::NIL),
        };
        'outer: loop {
            match self.dispatch_inner(&mut frame, py, host) {
                Ok(val) => {
                    // Full release of the outermost frame, same as every Return
                    // path: leftover heap locals (module-level globals keep
                    // their slot ref until here -- the Halt sync clones its own)
                    // leaked one BigInt/Complex/struct ref per run under the
                    // old pyobj-only release. free() also drains the block-stack
                    // snapshots and pending match bindings.
                    self.frame_pool.free(frame, &self.struct_registry);
                    return Ok(val);
                }
                Err(err) => {
                    // Try to unwind to an exception handler
                    if self.unwind_exception(&mut frame, &err) {
                        continue 'outer;
                    }
                    // No handler found. Handle Return specially (frame pop).
                    if let VMError::Return(val) = err {
                        if let Some(caller) = self.frame_stack.pop() {
                            let discard = frame.discard_return;
                            let old = std::mem::replace(&mut frame, caller);
                            // Full release (pyobj + bigint/complex/struct):
                            // leftover heap locals leaked one ref per return
                            // under the pyobj-only Voie A (see the Return
                            // opcode handler).
                            self.frame_pool.free(old, &self.struct_registry);
                            self.handle_nd_frame_pop(py, val);
                            if discard {
                                // discard_return: don't push result to caller
                            } else {
                                frame.push(val);
                            }
                            continue 'outer;
                        }
                        self.frame_pool.free(frame, &self.struct_registry);
                        return Ok(val);
                    }
                    // Error path: frames may have unbalanced refcounts, use free for cleanup
                    while let Some(f) = self.frame_stack.pop() {
                        self.frame_pool.free(f, &self.struct_registry);
                    }
                    self.frame_pool.free(frame, &self.struct_registry);
                    return Err(err);
                }
            }
        }
    }

    /// Inner dispatch loop. Returns Ok on clean exit, Err on any signal/exception.
    fn dispatch_inner(&mut self, frame: &mut Frame, py: Python<'_>, host: &dyn super::host::VmHost) -> VMResult<Value> {
        #[allow(unused_assignments)]
        let mut last_result = Value::NIL;

        loop {
            let code = match &frame.code {
                Some(c) => c.clone(),
                None => return Ok(Value::NIL),
            };
            // SAFETY: code Arc is kept alive by the frame (never replaced during execution).
            // Raw pointer avoids atomic refcount on every instruction fetch.
            let code: &CodeObject = unsafe { &*Arc::as_ptr(&code) };

            // Check if we've reached the end of bytecode
            if frame.ip >= code.instructions.len() {
                let result = if !frame.stack.is_empty() {
                    frame.pop()
                } else {
                    Value::NIL
                };
                if let Some(caller) = self.frame_stack.pop() {
                    let discard = frame.discard_return;
                    let old = std::mem::replace(frame, caller);
                    // Full release (pyobj + bigint/complex/struct): the
                    // 'balanced by opcodes' claim only held for overwrites,
                    // not for the final state of leftover heap locals.
                    self.frame_pool.free(old, &self.struct_registry);
                    self.handle_nd_frame_pop(py, result);
                    if !discard {
                        frame.push(result);
                    }
                    continue;
                }
                return Ok(result);
            }

            // Fetch instruction + source position
            let instr = code.instructions[frame.ip];
            let _current_src_byte = code.line_table.get(frame.ip).copied().unwrap_or(0);
            self.last_src_byte = _current_src_byte;
            frame.ip += 1;

            // Periodic checks (every ~65k instructions)
            self.instruction_count = self.instruction_count.wrapping_add(1);
            if self.instruction_count & MEMORY_CHECK_INTERVAL == 0 {
                // Interrupt check (Ctrl+C from REPL)
                if self.interrupt_flag.load(Ordering::Relaxed) {
                    self.interrupt_flag.store(false, Ordering::Relaxed);
                    return Err(VMError::Interrupted);
                }
                // RSS memory guard
                if self.memory_limit_bytes > 0 {
                    if let Some(rss) = super::memory::get_rss_bytes() {
                        if rss > self.memory_limit_bytes {
                            let rss_mb = rss / (1024 * 1024);
                            let limit_mb = self.memory_limit_bytes / (1024 * 1024);
                            return Err(VMError::MemoryLimitExceeded(format!(
                                "memory limit exceeded ({rss_mb} MB / {limit_mb} MB)\n\
                                 Increase: catnip -o memory:{}\n\
                                 Disable:  catnip -o memory:0",
                                limit_mb * 2
                            )));
                        }
                    }
                }
            }

            if self.trace {
                eprintln!(
                    "[TRACE] ip={} {:?} arg={} stack_len={}",
                    frame.ip - 1,
                    instr.op,
                    instr.arg,
                    frame.stack.len()
                );
            }

            // Feed the ObjectTable trace ledger the current opcode (leak
            // hunts): one relaxed load when the trace is off.
            if crate::vm::value::table_trace::active() {
                crate::vm::value::table_trace::set_ctx(format!(
                    "{:?} ip={} depth={}",
                    instr.op,
                    frame.ip - 1,
                    self.call_stack.len()
                ));
            }

            if self.profile {
                *self.profile_counts.entry(instr.op as u8).or_insert(0) += 1;
            }

            // Debug hook: determine if we should pause (before dispatch)
            let debug_should_pause = if self.debug_callback.is_some() {
                // Clear last_paused tracking when we move to a new source position
                if self.debug_last_paused_byte != Some(_current_src_byte) {
                    self.debug_last_paused_byte = None;
                }
                let is_step = matches!(
                    self.debug_step_mode,
                    DebugStepMode::StepInto | DebugStepMode::StepOver | DebugStepMode::StepOut
                );
                match instr.op {
                    OpCode::Breakpoint => {
                        // Always pause on explicit breakpoint()
                        self.debug_last_paused_byte != Some(_current_src_byte)
                    }
                    _ if is_step => match self.debug_step_mode {
                        DebugStepMode::StepInto => true,
                        DebugStepMode::StepOver => self.call_stack.len() <= self.debug_step_depth,
                        DebugStepMode::StepOut => self.call_stack.len() < self.debug_step_depth,
                        _ => false,
                    },
                    _ => {
                        // Byte-offset breakpoints: skip if same position as last pause
                        self.debug_breakpoints.contains(&_current_src_byte)
                            && self.debug_last_paused_byte != Some(_current_src_byte)
                    }
                }
            } else {
                false
            };

            // JIT trace recording (skip opcodes handled specially)
            // Only record if not inside a recursive call (jit_recursive_depth == 0)
            if self.jit_tracing
                && self.jit_recursive_depth == 0
                && instr.op != OpCode::ForRangeInt
                && instr.op != OpCode::LoadScope
                && instr.op != OpCode::StoreScope
            {
                let ip = frame.ip - 1;
                // f-local inlining: a LoadLocal of a function being called is elided
                // from the trace and replaced by a slot-keyed identity guard, instead
                // of pushing the function value (which would emit a bogus GuardFloat
                // and leave a function on the JIT stack). The inliner rebuilds the
                // callee body from the registry; the function never reaches codegen.
                // Gated on loop tracing. If the loaded vmfunc is NOT actually called,
                // the trace's stack ends up unbalanced -> codegen rejects -> fallback.
                let elide_func_load = instr.op == OpCode::LoadLocal
                    && self.jit_tracing_func_id.is_none()
                    && (instr.arg as usize) < frame.locals.len()
                    && frame.locals[instr.arg as usize].is_vmfunc();
                if elide_func_load {
                    let slot = instr.arg as usize;
                    self.jit_recorder
                        .record_func_slot_guard(slot, frame.locals[slot].bits());
                } else {
                    // Classify the operand the trace will guard on. Booleans count
                    // as ints (True=1, False=0). The observed value is the top of
                    // stack for a binary op, or the loaded slot for LoadLocal; other
                    // opcodes observe nothing.
                    let observed = match instr.op {
                        // Binary ops: check top of stack
                        OpCode::Add
                        | OpCode::Sub
                        | OpCode::Mul
                        | OpCode::Div
                        | OpCode::Mod
                        | OpCode::Lt
                        | OpCode::Le
                        | OpCode::Gt
                        | OpCode::Ge
                        | OpCode::Eq
                        | OpCode::Ne => frame.stack.last().copied(),
                        // LoadLocal: check the value being loaded
                        OpCode::LoadLocal => {
                            let slot = instr.arg as usize;
                            (slot < frame.locals.len()).then(|| frame.locals[slot])
                        }
                        _ => None,
                    };
                    // A non-numeric operand (BigInt, struct, pyobj, nil, a function
                    // not elided above) cannot be JIT-compiled: it would push a bogus
                    // GuardFloat that types the slot F64 while integer arithmetic
                    // yields I64 -> Cranelift def_var panic. Abort the trace; the loop
                    // stays interpreted. A genuine float keeps the existing GuardFloat
                    // path (float codegen is unreachable -> trace rejected -> fallback).
                    if matches!(observed, Some(v) if !v.is_int() && !v.is_bool() && !v.is_float()) {
                        self.jit_recorder.abort();
                        self.jit_tracing = false;
                        self.jit_tracing_func_id = None;
                    } else {
                        let is_int_value = observed.is_none_or(|v| v.is_int() || v.is_bool());
                        if !self.jit_recorder.record_opcode(instr.op, instr.arg, is_int_value, ip) {
                            // Trace was aborted (e.g. exception opcodes) -- reset tracing state
                            self.jit_tracing = false;
                            self.jit_tracing_func_id = None;
                        }
                    }
                }
            }

            // Dispatch via match - compiles to jump table
            match instr.op {
                // --- Stack operations ---
                OpCode::LoadConst => {
                    let idx = instr.arg as usize;
                    let value = if idx < code.constants.len() {
                        code.constants[idx]
                    } else {
                        Value::NIL
                    };
                    // Record constant value for JIT (only if not suspended)
                    if self.jit_tracing && self.jit_recursive_depth == 0 {
                        let ip = frame.ip - 1;
                        if let Some(i) = value.as_int() {
                            self.jit_recorder.record_const_int(i, ip);
                        } else if let Some(f) = value.as_float() {
                            self.jit_recorder.record_const_float(f, ip);
                        } else if let Some(b) = value.as_bool() {
                            // Treat booleans as ints for JIT (0 or 1)
                            self.jit_recorder.record_const_int(if b { 1 } else { 0 }, ip);
                        } else {
                            // Other constants (None, strings, etc.) - record as 0 to balance stack
                            // These will likely prevent compilation (fallback to interpreter)
                            self.jit_recorder.record_const_int(0, ip);
                        }
                    }
                    // Incref: const is shared with stack (Voie A: own the pyobj too)
                    value.clone_refcount_bigint();
                    value.clone_refcount_pyobj();
                    if value.is_struct_instance() {
                        self.struct_registry.incref(value.as_struct_instance_idx().unwrap());
                    }
                    frame.push(value);
                }

                OpCode::LoadLocal => {
                    let value = frame.get_local(instr.arg as usize);
                    value.clone_refcount_bigint();
                    value.clone_refcount_pyobj();
                    if value.is_struct_instance() {
                        self.struct_registry.incref(value.as_struct_instance_idx().unwrap());
                    }
                    frame.push(value);
                }

                OpCode::StoreLocal => {
                    let value = frame.pop();
                    let old = frame.get_local(instr.arg as usize);
                    decref_discard(&self.struct_registry, old);
                    frame.set_local(instr.arg as usize, value);
                }

                OpCode::LoadScope => {
                    let name = get_name(code, instr.arg)?;
                    let resolved_value: Value;

                    // 0. Check super proxy (for extends parent method access)
                    if name == "super" {
                        if let Some(ref proxy) = frame.super_proxy {
                            let value = Value::from_pyobject(py, proxy.bind(py))
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            frame.push(value);
                            continue;
                        }
                    }

                    // 1. Check captured vars of this closure and every enclosing
                    //    closure (lexical scope), before globals: an enclosing
                    //    binding shadows a global homonym (LEGB). Pure Rust, no GIL.
                    if let Some(ref closure) = frame.closure_scope {
                        if let Some(value) = closure.resolve_captured_chain(name) {
                            // Resolver returns a fully owned value: push as-is.
                            resolved_value = value;
                            frame.push(resolved_value);
                            if self.jit_tracing {
                                if let Some(int_val) = resolved_value.as_int() {
                                    let ip = frame.ip - 1;
                                    self.jit_recorder.record_load_scope(name, int_val, ip);
                                } else if resolved_value.is_vmfunc() && self.jit_tracing_func_id.is_none() {
                                    // Loop trace: a scope-resolved function that may be
                                    // inlined into the loop. Guard its identity (NaN-box
                                    // bits) so a reassignment falls back to the interpreter
                                    // instead of running the stale inlined body.
                                    self.jit_recorder.record_func_guard(name, resolved_value.bits());
                                }
                            }
                            continue;
                        }
                    }
                    // 2. Check VM globals (Rust HashMap, O(1), always in sync)
                    if let Some(&value) = self.globals.get(name.as_str()) {
                        resolved_value = value;
                        resolved_value.clone_refcount_bigint();
                        resolved_value.clone_refcount_pyobj();
                        if resolved_value.is_struct_instance() {
                            self.struct_registry
                                .incref(resolved_value.as_struct_instance_idx().unwrap());
                        }
                        frame.push(resolved_value);
                        if self.jit_tracing {
                            if let Some(int_val) = resolved_value.as_int() {
                                let ip = frame.ip - 1;
                                self.jit_recorder.record_load_scope(name, int_val, ip);
                            } else if resolved_value.is_vmfunc() && self.jit_tracing_func_id.is_none() {
                                // Loop trace: guard the identity of a scope-resolved
                                // function that may be inlined (see the other LoadScope
                                // branches) so a reassignment falls back gracefully.
                                self.jit_recorder.record_func_guard(name, resolved_value.bits());
                            }
                        }
                        continue;
                    }
                    // 3. Check closure parent chain (may hit PyGlobals)
                    if let Some(ref closure) = frame.closure_scope {
                        if let Some(value) = closure.resolve_with_py(py, name) {
                            // Resolver returns a fully owned value: push as-is.
                            resolved_value = value;
                            frame.push(resolved_value);
                            if self.jit_tracing {
                                if let Some(int_val) = resolved_value.as_int() {
                                    let ip = frame.ip - 1;
                                    self.jit_recorder.record_load_scope(name, int_val, ip);
                                } else if resolved_value.is_vmfunc() && self.jit_tracing_func_id.is_none() {
                                    // Loop trace: a scope-resolved function that may be
                                    // inlined into the loop. Guard its identity (NaN-box
                                    // bits) so a reassignment falls back to the interpreter
                                    // instead of running the stale inlined body.
                                    self.jit_recorder.record_func_guard(name, resolved_value.bits());
                                }
                            }
                            continue;
                        }
                    }
                    // 4. Fallback to ctx_globals (Python builtins, modules)
                    if let Some(value) = host.lookup_global(py, name)? {
                        // lookup_global returns a fully owned value: push as-is.
                        resolved_value = value;
                        frame.push(resolved_value);
                        if self.jit_tracing {
                            if let Some(int_val) = resolved_value.as_int() {
                                let ip = frame.ip - 1;
                                self.jit_recorder.record_load_scope(name, int_val, ip);
                            } else if resolved_value.is_vmfunc() && self.jit_tracing_func_id.is_none() {
                                // Symmetry with the other LoadScope branches and with
                                // resolve_jit_name_bits (which resolves via host too): a
                                // function resolved here also gets an identity guard. A
                                // host value is normally a Python callable (not a vmfunc),
                                // so this rarely fires, but it keeps "inlined => guarded".
                                self.jit_recorder.record_func_guard(name, resolved_value.bits());
                            }
                        }
                        continue;
                    }
                    return Err(VMError::NameError(name.to_owned()));
                }

                OpCode::StoreScope => {
                    let name = get_name(code, instr.arg)?;

                    // Check slotmap before recording
                    let slot_idx = code.slotmap.get(name.as_str()).copied();

                    // Record StoreScope during tracing (BEFORE pop, while value is on stack)
                    // Pass the existing slot from slotmap if available
                    let trace_slot = if self.jit_tracing {
                        let ip = frame.ip - 1;
                        self.jit_recorder.record_store_scope(name, ip, slot_idx)
                    } else {
                        None
                    };

                    let value = frame.pop();

                    // During tracing, also store to the trace slot to keep frame.locals synchronized
                    // Track whether we already wrote to the local slot (to avoid double-decref)
                    let mut local_slot_written = false;
                    if let Some(slot) = trace_slot {
                        if slot >= frame.locals.len() {
                            frame.locals.resize(slot + 1, Value::NIL);
                        }
                        let old = frame.get_local(slot);
                        decref_discard(&self.struct_registry, old);
                        frame.set_local(slot, value);
                        local_slot_written = true;
                    } else if self.jit_enabled {
                        // When JIT is enabled (but not currently tracing), still sync frame.locals
                        // using the slotmap so that JIT code can read correct values
                        if let Some(slot) = slot_idx {
                            if slot >= frame.locals.len() {
                                frame.locals.resize(slot + 1, Value::NIL);
                            }
                            let old = frame.get_local(slot);
                            decref_discard(&self.struct_registry, old);
                            frame.set_local(slot, value);
                            local_slot_written = true;
                        }
                    }

                    // 1. Closure membership test FIRST, transfer LAST: the
                    // popped ref has exactly ONE consumer, and set_with_py
                    // consumes it on success (owned-in-on-true) -- so every
                    // other store below takes its OWN ref BEFORE the transfer
                    // (wip/GLOBALS_OWNERSHIP.md; the PyGlobals arm releases
                    // the handle, so touching `value` after a true is a
                    // use-after-free).
                    let in_closure = frame
                        .closure_scope
                        .as_ref()
                        .is_some_and(|c| c.contains_with_py(py, name.as_str()));

                    // 2. Store to local slot if name is in slotmap
                    // Skip if already written above (same slot) to avoid double-decref.
                    // Also skip if the slot already holds `value` (StoreLocal ran before
                    // StoreScope in the DupTop;StoreLocal;StoreScope sequence).
                    if !local_slot_written {
                        if let Some(idx) = slot_idx {
                            let old = frame.get_local(idx);
                            if old.bits() != value.bits() {
                                decref_discard(&self.struct_registry, old);
                                if in_closure {
                                    // The popped ref goes to the closure below:
                                    // the slot takes its own.
                                    value.clone_refcount();
                                }
                                frame.set_local(idx, value);
                            } else if !in_closure {
                                // StoreLocal;LoadLocal;StoreScope: the popped
                                // ref duplicates the one already parked in this
                                // slot (LoadLocal increfs). Release it, or every
                                // top-level pyobj reassignment leaks one ref.
                                // (in_closure: the popped ref transfers below.)
                                decref_discard(&self.struct_registry, value);
                            }
                        } else if !in_closure {
                            // No slot and no closure consumer: release the
                            // popped ref (defensive -- every compiler emission
                            // creates a slot today). The globals stores below
                            // take their own refs.
                            decref_discard(&self.struct_registry, value);
                        }
                    }

                    // Keep VM globals in sync for module-level vars modified via
                    // closures. The clone is guarded by the entry's existence:
                    // an unconditional clone with no else-insert orphaned one
                    // ref per assignment to a name this VM never stored.
                    let mut stored_in_closure = false;
                    if in_closure {
                        if let Some(&old_global) = self.globals.get(name.as_str()) {
                            decref_discard(&self.struct_registry, old_global);
                            value.clone_refcount();
                            if let Some(existing) = self.globals.get_mut(name.as_str()) {
                                *existing = value;
                            }
                        }
                        // Signal change even if this VM doesn't own the var
                        // (re-entrant VMs write to ctx_globals via closure chain)
                        GLOBALS_GEN.fetch_add(1, Ordering::Relaxed);

                        // The transfer, LAST: consumes the popped ref on true.
                        // On false (set_item failure, contains/set divergence)
                        // the ref was not consumed: release it here.
                        if let Some(ref closure) = frame.closure_scope {
                            stored_in_closure = closure.set_with_py(py, name.as_str(), value, &self.struct_registry);
                        }
                        if !stored_in_closure {
                            decref_discard(&self.struct_registry, value);
                            stored_in_closure = true;
                        }
                    }

                    // 3. Store to globals for name resolution (if not in closure)
                    if !stored_in_closure {
                        // Decref old value in globals (BigInt/Struct refcount)
                        if let Some(&old_global) = self.globals.get(name.as_str()) {
                            decref_discard(&self.struct_registry, old_global);
                        }
                        // Incref new value going into globals (separate ownership)
                        value.clone_refcount();
                        if let Some(existing) = self.globals.get_mut(name.as_str()) {
                            *existing = value;
                        } else {
                            self.globals.insert(name.clone(), value);
                        }
                        GLOBALS_GEN.fetch_add(1, Ordering::Relaxed);
                        // Also sync to Python context.globals immediately
                        // so closures created later can access these values.
                        // The displaced host entry is released with the
                        // registry in hand (struct-aware).
                        if let Some(old) = host.store_global(py, name.as_str(), value)? {
                            decref_discard(&self.struct_registry, old);
                        }
                    }
                }

                OpCode::LoadGlobal => {
                    let name = get_name(code, instr.arg)?;
                    if let Some(&value) = self.globals.get(name.as_str()) {
                        value.clone_refcount_bigint();
                        value.clone_refcount_pyobj();
                        if value.is_struct_instance() {
                            self.struct_registry.incref(value.as_struct_instance_idx().unwrap());
                        }
                        frame.push(value);
                    } else if let Some(value) = host.lookup_global(py, name.as_str())? {
                        // lookup_global returns a fully owned value: push as-is.
                        frame.push(value);
                    } else {
                        return Err(VMError::NameError(name.to_owned()));
                    }

                    // JIT: pure builtins are handled in the Call handler (record_builtin)
                    // Other globals: emit Fallback (trace not compilable)
                    if self.jit_tracing && self.jit_recursive_depth == 0 {
                        let is_pure_builtin = JIT_PURE_BUILTINS.contains(&name.as_str());
                        if !is_pure_builtin {
                            let ip = frame.ip - 1;
                            self.jit_recorder.record_fallback(OpCode::LoadGlobal, ip);
                        }
                    }
                }

                // --- Stack manipulation ---
                OpCode::PopTop => {
                    let val = frame.pop();
                    decref_discard(&self.struct_registry, val);
                }

                OpCode::DupTop => {
                    let value = frame.peek();
                    value.clone_refcount();
                    frame.push(value);
                }

                OpCode::RotTwo => {
                    let len = frame.stack.len();
                    if len >= 2 {
                        frame.stack.swap(len - 1, len - 2);
                    }
                }

                // --- Arithmetic (binary) ---
                OpCode::Add => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match binary_add(a, b) {
                        Err(VMError::TypeError(_)) => {
                            // Struct operator overload (stays in VM); the popped
                            // operands transfer into the overload frame's locals.
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_add")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_add"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            // Fallback to Python for strings, lists, etc.
                            host.binary_op(py, BinaryOp::Add, a, b)
                        }
                        r => r,
                    };
                    // binary_* borrows its operands and the Python fallback
                    // consumes only the pyobj refs (call_binary_op): release the
                    // popped BigInt/Complex/struct refs on every non-transfer
                    // path, errors included -- without this, every chained
                    // BigInt intermediate leaked its whole allocation.
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                // TH4 canal A: typed arithmetic. Operands are a proven int/float
                // runtime fact, so skip the struct-overload lookup and the type
                // dispatch. Refcount handling mirrors the Add arm exactly.
                OpCode::AddInt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = binary_add(a, b);
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }
                OpCode::AddFloat => {
                    let b = frame.pop();
                    let a = frame.pop();
                    // Proven-float fast path: both operands are unboxed
                    // doubles, nothing to release. Only the fallback can see
                    // heap operands (an int-typed value grown into BigInt).
                    match (a.as_float(), b.as_float()) {
                        (Some(x), Some(y)) => frame.push(Value::from_float(x + y)),
                        _ => {
                            let result = binary_add(a, b);
                            release_binop_operands(&self.struct_registry, a, b);
                            frame.push(result?);
                        }
                    }
                }
                OpCode::SubInt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = binary_sub(a, b);
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }
                OpCode::SubFloat => {
                    let b = frame.pop();
                    let a = frame.pop();
                    match (a.as_float(), b.as_float()) {
                        (Some(x), Some(y)) => frame.push(Value::from_float(x - y)),
                        _ => {
                            let result = binary_sub(a, b);
                            release_binop_operands(&self.struct_registry, a, b);
                            frame.push(result?);
                        }
                    }
                }
                OpCode::MulInt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = binary_mul(a, b);
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }
                OpCode::MulFloat => {
                    let b = frame.pop();
                    let a = frame.pop();
                    match (a.as_float(), b.as_float()) {
                        (Some(x), Some(y)) => frame.push(Value::from_float(x * y)),
                        _ => {
                            let result = binary_mul(a, b);
                            release_binop_operands(&self.struct_registry, a, b);
                            frame.push(result?);
                        }
                    }
                }
                // True division always yields a float; the fast path divides
                // directly but defers the zero check to binary_div.
                OpCode::DivFloat => {
                    let b = frame.pop();
                    let a = frame.pop();
                    match (a.as_float(), b.as_float()) {
                        (Some(x), Some(y)) if y != 0.0 => frame.push(Value::from_float(x / y)),
                        _ => {
                            let result = binary_div(a, b);
                            release_binop_operands(&self.struct_registry, a, b);
                            frame.push(result?);
                        }
                    }
                }

                OpCode::Sub => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match binary_sub(a, b) {
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_sub")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_sub"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            host.binary_op(py, BinaryOp::Sub, a, b)
                        }
                        r => r,
                    };
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                OpCode::Mul => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match binary_mul(a, b) {
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_mul")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_mul"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            // Fallback to Python for string * int, etc.
                            host.binary_op(py, BinaryOp::Mul, a, b)
                        }
                        r => r,
                    };
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                OpCode::Div => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_div")
                        .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_div"))
                    {
                        let mut new_frame = Frame::with_code(code);
                        for (i, arg) in args.iter().enumerate() {
                            new_frame.set_local(i, *arg);
                        }
                        new_frame.closure_scope = closure;
                        {
                            let old = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old);
                        }
                        continue;
                    }
                    let result = match binary_div(a, b) {
                        Err(VMError::TypeError(_)) => {
                            inc(&PY_BINARY_DIV_FALLBACKS);
                            host.binary_op(py, BinaryOp::TrueDiv, a, b)
                        }
                        r => r,
                    };
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                OpCode::FloorDiv => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some((code, closure, args)) =
                        try_struct_binop(&self.struct_registry, py, a, b, "op_floordiv")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_floordiv"))
                    {
                        let mut new_frame = Frame::with_code(code);
                        for (i, arg) in args.iter().enumerate() {
                            new_frame.set_local(i, *arg);
                        }
                        new_frame.closure_scope = closure;
                        {
                            let old = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old);
                        }
                        continue;
                    }
                    let result = match binary_floordiv(a, b) {
                        Err(VMError::TypeError(_)) => {
                            inc(&PY_BINARY_FLOORDIV_FALLBACKS);
                            host.binary_op(py, BinaryOp::FloorDiv, a, b)
                        }
                        r => r,
                    };
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                OpCode::Mod => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_mod")
                        .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_mod"))
                    {
                        let mut new_frame = Frame::with_code(code);
                        for (i, arg) in args.iter().enumerate() {
                            new_frame.set_local(i, *arg);
                        }
                        new_frame.closure_scope = closure;
                        {
                            let old = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old);
                        }
                        continue;
                    }
                    let result = match binary_mod(a, b) {
                        Err(VMError::TypeError(_)) => {
                            inc(&PY_BINARY_MOD_FALLBACKS);
                            host.binary_op(py, BinaryOp::Mod, a, b)
                        }
                        r => r,
                    };
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                OpCode::Pow => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match binary_pow(a, b) {
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_pow")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_pow"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            host.binary_op(py, BinaryOp::Pow, a, b)
                        }
                        r => r,
                    };
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                // --- Arithmetic (unary) ---
                OpCode::Neg => {
                    let a = frame.pop();
                    let result = match unary_neg(a) {
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_unaryop(&self.struct_registry, py, a, "op_neg")
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            // __neg__ borrows via to_pyobject; capture the error
                            // so the popped operand is released on that path too.
                            let py_a = a.to_pyobject(py);
                            py_a.call_method0(py, "__neg__")
                                .and_then(|r| Value::from_pyobject(py, r.bind(py)))
                                .to_vm(py)
                        }
                        r => r,
                    };
                    decref_discard(&self.struct_registry, a);
                    frame.push(result?);
                }

                OpCode::Pos => {
                    let a = frame.peek();
                    if a.as_struct_instance_idx().is_some() {
                        frame.pop();
                        if let Some((code, closure, args)) = try_struct_unaryop(&self.struct_registry, py, a, "op_pos")
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                        // The operand was popped: release it before the error
                        // propagates, or every caught `+struct` leaks its slot.
                        decref_discard(&self.struct_registry, a);
                        return Err(VMError::TypeError("bad operand type for unary +: struct".to_string()));
                    }
                    // No-op for native numbers
                }

                // --- Bitwise ---
                OpCode::BOr => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match bitwise_or(a, b) {
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_bor")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_bor"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            // Fallback to Python for numpy arrays, sets, and
                            // types with custom __or__. Mirrors the Add arm.
                            bitwise_binary_fallback(py, "or_", a, b)
                        }
                        r => r,
                    };
                    // The fallback consumes pyobj operand refs (call_binary_op,
                    // Voie A); release only the leftover bigint/struct refs here.
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                OpCode::BXor => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match bitwise_xor(a, b) {
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_bxor")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_bxor"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            // Fallback to Python for numpy arrays, sets, and
                            // types with custom __xor__. Mirrors the Add arm.
                            bitwise_binary_fallback(py, "xor", a, b)
                        }
                        r => r,
                    };
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                OpCode::BAnd => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match bitwise_and(a, b) {
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_band")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_band"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            // Fallback to Python for numpy arrays, sets, and
                            // types with custom __and__. Mirrors the Add arm.
                            bitwise_binary_fallback(py, "and_", a, b)
                        }
                        r => r,
                    };
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                OpCode::BNot => {
                    let a = frame.pop();
                    let result = match bitwise_not(a) {
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_unaryop(&self.struct_registry, py, a, "op_bnot")
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            // Fallback to Python for numpy arrays and types with
                            // a custom __invert__. Borrows `a`, released below.
                            bitwise_unary_fallback(py, a)
                        }
                        r => r,
                    };
                    decref_discard(&self.struct_registry, a);
                    frame.push(result?);
                }

                OpCode::LShift => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match bitwise_lshift(a, b) {
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_lshift")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_lshift"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            Err(VMError::TypeError("unsupported operand type(s) for <<".to_string()))
                        }
                        r => r,
                    };
                    release_operands(&self.struct_registry, &[a, b]);
                    frame.push(result?);
                }

                OpCode::RShift => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match bitwise_rshift(a, b) {
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_rshift")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_rshift"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            Err(VMError::TypeError("unsupported operand type(s) for >>".to_string()))
                        }
                        r => r,
                    };
                    release_operands(&self.struct_registry, &[a, b]);
                    frame.push(result?);
                }

                // --- Comparison ---
                OpCode::Lt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if a.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_lt")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_gt"))
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    let result = match compare_lt(a, b) {
                        Err(VMError::TypeError(_)) => host.binary_op(py, BinaryOp::Lt, a, b),
                        r => r,
                    };
                    // The fallback consumes only the pyobj refs: release the
                    // popped numeric/struct refs on every non-transfer path.
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                OpCode::Le => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if a.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_le")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_ge"))
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    let result = match compare_le(a, b) {
                        Err(VMError::TypeError(_)) => host.binary_op(py, BinaryOp::Le, a, b),
                        r => r,
                    };
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                OpCode::Gt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if a.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_gt")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_lt"))
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    let result = match compare_gt(a, b) {
                        Err(VMError::TypeError(_)) => host.binary_op(py, BinaryOp::Gt, a, b),
                        r => r,
                    };
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                OpCode::Ge => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if a.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_ge")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_le"))
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    let result = match compare_ge(a, b) {
                        Err(VMError::TypeError(_)) => host.binary_op(py, BinaryOp::Ge, a, b),
                        r => r,
                    };
                    release_binop_operands(&self.struct_registry, a, b);
                    frame.push(result?);
                }

                OpCode::Eq => {
                    let b = frame.pop();
                    let a = frame.pop();
                    // Try op_eq method first
                    if a.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_eq")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_eq"))
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    // Fallback: structural equality for structs. compare_eq
                    // borrows (to_pyobject clones): release the popped operands
                    // on every path, a raising __eq__ included.
                    let result =
                        if let (Some(idx_a), Some(idx_b)) = (a.as_struct_instance_idx(), b.as_struct_instance_idx()) {
                            struct_fields_eq(&self.struct_registry, py, idx_a, idx_b).map(Value::from_bool)
                        } else {
                            compare_eq(py, a, b)
                        };
                    release_operands(&self.struct_registry, &[a, b]);
                    frame.push(result?);
                }

                OpCode::Ne => {
                    let b = frame.pop();
                    let a = frame.pop();
                    // Try op_ne method first
                    if a.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_ne")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_ne"))
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    // Fallback: structural inequality for structs. Same
                    // ownership as Eq: borrow-only helpers, release both popped
                    // operands on every path including a raising __ne__.
                    let result =
                        if let (Some(idx_a), Some(idx_b)) = (a.as_struct_instance_idx(), b.as_struct_instance_idx()) {
                            struct_fields_eq(&self.struct_registry, py, idx_a, idx_b).map(|eq| Value::from_bool(!eq))
                        } else {
                            compare_ne(py, a, b)
                        };
                    release_operands(&self.struct_registry, &[a, b]);
                    frame.push(result?);
                }

                // --- Membership ---
                OpCode::In => {
                    let b = frame.pop(); // container
                    let a = frame.pop(); // item
                    // Try op_in method on container
                    if b.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, b, a, "op_in")
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    // contains_op borrows: release the popped operands on every
                    // path, a raising __contains__ included.
                    let result = host.contains_op(py, a, b);
                    decref_discard(&self.struct_registry, a);
                    decref_discard(&self.struct_registry, b);
                    frame.push(result?);
                }
                OpCode::NotIn => {
                    let b = frame.pop(); // container
                    let a = frame.pop(); // item
                    // Dispatch op_not_in on struct (mirrors Eq/Ne pattern)
                    if b.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) =
                            try_struct_binop(&self.struct_registry, py, b, a, "op_not_in")
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    let result = host.contains_op(py, a, b);
                    decref_discard(&self.struct_registry, a);
                    decref_discard(&self.struct_registry, b);
                    let negated = Value::from_bool(!result?.is_truthy_py(py));
                    frame.push(negated);
                }

                // --- Identity ---
                OpCode::Is => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = a.is_identical(py, b);
                    frame.push(Value::from_bool(result));
                    decref_discard(&self.struct_registry, a);
                    decref_discard(&self.struct_registry, b);
                }
                OpCode::IsNot => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = !a.is_identical(py, b);
                    frame.push(Value::from_bool(result));
                    decref_discard(&self.struct_registry, a);
                    decref_discard(&self.struct_registry, b);
                }

                // --- Logic ---
                OpCode::Not => {
                    let a = frame.pop();
                    let result = Value::from_bool(!a.is_truthy_py(py));
                    frame.push(result);
                    decref_discard(&self.struct_registry, a);
                }

                // --- Control flow ---
                OpCode::Jump => {
                    let target = instr.arg as usize;
                    let is_backward = target < frame.ip;

                    // Backward jump = loop, check for JIT opportunities
                    if is_backward && self.jit_enabled {
                        let loop_offset = target;
                        let is_for_range_header = frame
                            .code
                            .as_ref()
                            .and_then(|code| code.instructions.get(loop_offset))
                            .map(|instr| instr.op == OpCode::ForRangeInt)
                            .unwrap_or(false);

                        if !is_for_range_header {
                            // Check if we have compiled code for this loop
                            if !self.jit_tracing
                                && self.jit_has_compiled(loop_offset)
                                && self.try_enter_jit_loop(py, frame, host, code, loop_offset)?.is_some()
                            {
                                if self.trace {
                                    eprintln!("[JIT] Executed compiled trace for while loop at {}", loop_offset);
                                }
                                // Loop completed, jump to condition check
                                frame.ip = target;
                                continue;
                            }
                            // If guards didn't pass, fall through to interpreter

                            // If we're tracing and jumping back to loop start
                            if self.jit_tracing && self.jit_tracing_offset == loop_offset {
                                self.jit_record_loop_back_and_maybe_compile(frame.ip - 1, "while");
                            } else if !self.jit_tracing {
                                // Warm-start: check trace cache on first encounter
                                self.jit_warm_start(loop_offset, "while");

                                // Not tracing, check if loop becomes hot
                                if self.jit_detector.record_loop_header(loop_offset) {
                                    // Try cache first - skip recording if trace already cached
                                    if self.jit_compile_from_cache(loop_offset) {
                                        if self.trace {
                                            eprintln!("[JIT] While loop at offset {} compiled from cache", loop_offset);
                                        }
                                    } else {
                                        // Cache miss - start tracing
                                        let num_locals = frame.locals.len();
                                        self.jit_recorder.start(loop_offset, num_locals);
                                        self.jit_tracing = true;
                                        // A trace begins at call-nesting depth 0. Reset
                                        // defensively so a depth leaked by a previous trace
                                        // (a callee that exited without a Return opcode --
                                        // native-compiled shortcut, exception unwind) cannot
                                        // suspend recording of this one.
                                        self.jit_recursive_depth = 0;
                                        self.jit_tracing_offset = loop_offset;

                                        if self.trace {
                                            eprintln!("[JIT] Started tracing while loop at offset {}", loop_offset);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    frame.ip = target;
                }

                OpCode::JumpIfFalse => {
                    let cond = frame.pop();
                    let took_jump = !cond.is_truthy_py(py);
                    decref_discard(&self.struct_registry, cond);
                    if took_jump {
                        frame.ip = instr.arg as usize;
                    }
                    // Record for JIT after execution (we now know if we jumped)
                    // Only record if not suspended in recursive call
                    if self.jit_tracing && self.jit_recursive_depth == 0 {
                        let ip = frame.ip.saturating_sub(1);
                        self.jit_recorder.record_conditional_jump(took_jump, true, ip);
                    }
                }

                OpCode::JumpIfTrue => {
                    let cond = frame.pop();
                    let took_jump = cond.is_truthy_py(py);
                    decref_discard(&self.struct_registry, cond);
                    if took_jump {
                        frame.ip = instr.arg as usize;
                    }
                    // Record for JIT after execution
                    if self.jit_tracing {
                        let ip = frame.ip.saturating_sub(1);
                        self.jit_recorder.record_conditional_jump(took_jump, false, ip);
                    }
                }

                OpCode::JumpIfFalseOrPop => {
                    let cond = frame.peek();
                    if !cond.is_truthy_py(py) {
                        frame.ip = instr.arg as usize;
                    } else {
                        frame.pop();
                        decref_discard(&self.struct_registry, cond);
                    }
                }

                OpCode::JumpIfTrueOrPop => {
                    let cond = frame.peek();
                    if cond.is_truthy_py(py) {
                        frame.ip = instr.arg as usize;
                    } else {
                        frame.pop();
                        decref_discard(&self.struct_registry, cond);
                    }
                }

                // --- Iteration ---
                OpCode::GetIter => {
                    let obj = frame.pop();
                    let py_obj = obj.to_pyobject(py);
                    decref_discard(&self.struct_registry, obj);
                    let py_obj_bound = py_obj.bind(py);

                    if let Ok(list) = py_obj_bound.cast::<PyList>() {
                        let iter = Py::new(py, SeqIter::from_list(list).to_vm(py)?).to_vm(py)?;
                        frame.push(Value::from_owned_pyobject(iter.into_any()));
                        continue;
                    }

                    if let Ok(tuple) = py_obj_bound.cast::<PyTuple>() {
                        let iter = Py::new(py, SeqIter::from_tuple(tuple).to_vm(py)?).to_vm(py)?;
                        frame.push(Value::from_owned_pyobject(iter.into_any()));
                        continue;
                    }

                    // Fallback: use host's iter() builtin
                    let iterator = host.get_iter(py, py_obj_bound)?;
                    frame.push(Value::from_owned_pyobject(iterator.unbind()));
                }

                OpCode::ForIter => {
                    // TOS is the iterator. Try to get next item.
                    // If exhausted, jump to end of loop (arg is jump target).
                    let iter_val = frame.peek();
                    let py_iter = iter_val.to_pyobject(py);
                    let py_iter_bound = py_iter.bind(py);

                    if let Ok(iter_ref) = py_iter_bound.cast::<SeqIter>() {
                        let mut iter = iter_ref.borrow_mut();
                        match iter.next_value(py).to_vm(py)? {
                            Some(value) => frame.push(value),
                            None => {
                                frame.pop();
                                // `py_iter` keeps an independent ref alive, so releasing
                                // the iterator handle here cannot drop the borrowed object.
                                decref_pyobj(iter_val);
                                frame.ip = instr.arg as usize;
                            }
                        }
                        continue;
                    }

                    // Fallback: use CPython tp_iternext directly (avoids sentinel + clone_ref)
                    // Safety: verify tp_iternext is non-NULL to avoid segfault on
                    // corrupted stack or objects that slipped past GetIter
                    let tp_iternext = unsafe { (*(*py_iter_bound.as_ptr()).ob_type).tp_iternext };
                    if tp_iternext.is_none() {
                        let type_name = py_iter_bound
                            .get_type()
                            .name()
                            .map(|n| n.to_string())
                            .unwrap_or_else(|_| "?".to_string());
                        return Err(VMError::RuntimeError(format!(
                            "ForIter: object of type '{type_name}' is not a valid iterator (NULL tp_iternext)"
                        )));
                    }
                    // SAFETY: tp_iternext was checked non-NULL above, so py_iter is a valid
                    // iterator; its pointer is borrowed for this GIL scope (py held) and
                    // PyIter_Next returns either a new owned reference or NULL.
                    let next_ptr = unsafe { pyo3::ffi::PyIter_Next(py_iter_bound.as_ptr()) };
                    if next_ptr.is_null() {
                        // NULL = exhausted (StopIteration) or real error
                        if let Some(err) = PyErr::take(py) {
                            if !err.is_instance_of::<pyo3::exceptions::PyStopIteration>(py) {
                                return Err(VMError::RuntimeError(err.to_string()));
                            }
                        }
                        frame.pop();
                        decref_pyobj(iter_val);
                        frame.ip = instr.arg as usize;
                    } else {
                        // SAFETY: next_ptr is the non-NULL new reference returned by
                        // PyIter_Next above; from_owned_ptr takes ownership of that
                        // reference under the held GIL.
                        let result = unsafe { pyo3::Bound::from_owned_ptr(py, next_ptr) };
                        let value = Value::from_pyobject(py, &result).to_vm(py)?;
                        frame.push(value);
                    }
                }

                // --- Function calls ---
                OpCode::Call => {
                    let nargs = instr.arg as usize;
                    // Read args in order from stack, then pop all + function
                    let stack_len = frame.stack.len();
                    let args_start = stack_len - nargs;

                    // Peek at the function (below args) to try the fast path
                    // before allocating a Vec for args
                    let func_pos = args_start - 1;
                    let func = frame.stack[func_pos];

                    // FAST PATH: native VM function (skip PyO3 boundary + avoid Vec alloc)
                    if func.is_vmfunc() {
                        let idx = func.as_vmfunc_idx();
                        let (new_code, native_closure) = {
                            let slot = self.func_table.get(idx).ok_or_else(|| {
                                VMError::RuntimeError(format!(
                                    "invalid function index {idx} (table has {} entries)",
                                    self.func_table.slots.len()
                                ))
                            })?;
                            (Arc::clone(&slot.code), slot.closure.clone())
                        };
                        let func_id = new_code.func_id();

                        // JIT trace recording
                        if self.jit_recorder.is_recording() {
                            let ip = frame.ip - 1;
                            let is_recursive = self.jit_tracing_func_id.as_ref() == Some(&func_id);
                            if is_recursive {
                                if self.jit_recursive_depth == 0 {
                                    self.jit_recorder.record_call(&func_id, nargs, new_code.is_pure, ip);
                                }
                                self.jit_recursive_depth += 1;
                            } else {
                                self.jit_recorder.record_call(&func_id, nargs, new_code.is_pure, ip);
                                // Loop trace: suspend recording of the callee body
                                // (the call is one CallPure; the inliner rebuilds
                                // the body from the registry). Without this the body
                                // is traced into the loop -- Exit mid-trace + slot
                                // collisions. Gated on loop tracing so function-trace
                                // content is unchanged. The matching decrement is
                                // in the Return opcode handler; a leaked depth (a
                                // callee exiting without a Return -- native shortcut,
                                // exception unwind) is cleared at the next trace start.
                                if self.jit_tracing_func_id.is_none() {
                                    self.jit_recursive_depth += 1;
                                }
                            }
                        }

                        // JIT pure function registration + hot detection
                        if self.jit_enabled {
                            if new_code.is_pure {
                                if let Ok(mut jit) = self.jit.lock() {
                                    if let Some(ref mut executor) = *jit {
                                        crate::jit::executor::register_pure_function(
                                            executor,
                                            func_id.clone(),
                                            &new_code,
                                        );
                                    }
                                }
                            }
                            self.jit_detector.record_call_internal(&func_id);
                        }

                        // Setup new frame - copy args directly from caller stack
                        let jit_hash = new_code.bytecode_hash();
                        let call_start_byte = _current_src_byte;
                        let fn_name = new_code.name.clone();
                        let has_varargs = new_code.vararg_idx >= 0;
                        // Snapshot args into inline buffer, then release frame borrow
                        // to access self.frame_pool
                        let mut arg_buf = [Value::NIL; 8];
                        let use_pool = !has_varargs && nargs <= 8;
                        if use_pool {
                            arg_buf[..nargs].copy_from_slice(&frame.stack[args_start..(nargs + args_start)]);
                            frame.stack.truncate(func_pos);
                        }
                        // Allocate frame: pool (fast) or new (fallback)
                        let mut new_frame = if use_pool {
                            self.frame_pool.alloc_with_code(new_code)
                        } else {
                            Frame::with_code(new_code)
                        };
                        if use_pool {
                            let nparams = new_frame.locals.len().min(nargs);
                            new_frame.locals[..nparams].copy_from_slice(&arg_buf[..nparams]);
                            // Voie A: excess args are accepted and discarded --
                            // release their moved refs (the buffer owns them).
                            for &a in &arg_buf[nparams..nargs] {
                                decref_discard(&self.struct_registry, a);
                            }
                            // Fill defaults for missing args
                            if let Some(ref fc) = new_frame.code {
                                let code_nargs = fc.nargs;
                                let ndefaults = fc.defaults.len();
                                if ndefaults > 0 && nargs < code_nargs {
                                    let default_start = code_nargs.saturating_sub(ndefaults);
                                    for i in nargs.max(default_start)..code_nargs {
                                        let default_idx = i - default_start;
                                        if default_idx < ndefaults {
                                            let val = fc.defaults[default_idx];
                                            val.clone_refcount();
                                            new_frame.locals[i] = val;
                                        }
                                    }
                                }
                            }
                        } else {
                            // Varargs or >8 args: use bind_args with Vec
                            let args: Vec<Value> = frame.stack[args_start..args_start + nargs].to_vec();
                            frame.stack.truncate(func_pos);
                            new_frame.bind_args(py, &self.struct_registry, &args, None);
                        }
                        new_frame.closure_scope = native_closure;
                        if self.jit_enabled {
                            if let Ok(mut jit) = self.jit.lock() {
                                if let Some(ref mut executor) = *jit {
                                    executor.set_bytecode_hash(jit_hash);
                                }
                            }
                        }
                        self.call_stack.push(CallInfo {
                            name: fn_name,
                            call_start_byte,
                        });
                        {
                            let old = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old);
                        }
                        continue;
                    }

                    // SLOW PATH: pop args into Vec
                    let args: Vec<Value> = frame.stack[args_start..].to_vec();
                    frame.stack.truncate(args_start);
                    // Pop function
                    frame.pop();

                    // SLOW PATH: PyObject (struct instantiation, bound methods, Python callables)
                    let py_func = func.to_pyobject(py);
                    // Voie A: the callable handle was popped off the stack; `py_func`
                    // now holds an independent ref, so release the owned handle here.
                    // Every slow-path sub-case operates on `py_func`, not on `func`.
                    decref_pyobj(func);
                    let py_func_bound = py_func.bind(py);

                    // ND recursion fast path: push frame instead of creating new VM
                    if let Ok(nd_recur) = py_func_bound.cast::<crate::nd::NDVmRecur>() {
                        let r = nd_recur.borrow();
                        if let Some(code) = r.vm_code_arc().cloned() {
                            // Depth guard
                            let depth = r.depth_cell().get();
                            if depth >= ND_MAX_RECURSION_DEPTH {
                                drop(r);
                                crate::nd::set_nd_abort();
                                release_operands(&self.struct_registry, &args);
                                return Err(VMError::RuntimeError("maximum ND recursion depth exceeded".to_string()));
                            }

                            // Memo cache check
                            let key = if r.is_memoize() && !args.is_empty() {
                                let py_val = args[0].to_pyobject(py);
                                py_val.bind(py).hash().ok().map(|h| h as u64)
                            } else {
                                None
                            };
                            let cache_hit = if let Some(k) = key {
                                let guard = r.cache_ref().borrow();
                                guard.get(&k).map(|c| c.clone_ref(py))
                            } else {
                                None
                            };
                            if let Some(cached) = cache_hit {
                                drop(r);
                                // The memoized result replaces the call: the popped
                                // args are consumed by nothing else, so release them
                                // on both exits.
                                let value = match Value::from_pyobject(py, cached.bind(py)) {
                                    Ok(v) => v,
                                    Err(e) => {
                                        release_operands(&self.struct_registry, &args);
                                        return Err(VMError::RuntimeError(e.to_string()));
                                    }
                                };
                                release_operands(&self.struct_registry, &args);
                                frame.push(value);
                                continue;
                            }

                            // Increment depth
                            r.depth_cell().set(depth + 1);
                            let closure = r.vm_closure_ref().cloned();
                            drop(r);

                            // Build args: [value, recur] - inject self as 2nd arg
                            let recur_value = match Value::from_pyobject(py, py_func_bound) {
                                Ok(v) => v,
                                Err(e) => {
                                    // Restore the depth taken above (nothing will pop
                                    // it) and release the in-flight args.
                                    nd_recur.borrow().depth_cell().set(depth);
                                    release_operands(&self.struct_registry, &args);
                                    return Err(VMError::RuntimeError(e.to_string()));
                                }
                            };
                            let mut lambda_args = Vec::with_capacity(args.len() + 1);
                            lambda_args.extend_from_slice(&args);
                            lambda_args.push(recur_value);

                            let caller_depth = self.frame_stack.len();
                            let mut new_frame = Frame::with_code(code);
                            new_frame.bind_args(py, &self.struct_registry, &lambda_args, None);
                            new_frame.closure_scope = closure;

                            self.nd_recur_stack.push(NdRecurEntry {
                                caller_depth,
                                recur_py: py_func.clone_ref(py),
                                memo_key: key,
                            });

                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                        // No vm_code: fall through to Python slow path
                    }

                    // Native struct instantiation (fast path)
                    {
                        let ptr = py_func_bound.as_ptr() as usize;
                        if let Some(&type_id) = self.struct_type_map.get(&ptr) {
                            // Error exits release the in-flight popped args (the
                            // callable handle was already released above).
                            if let Err(e) = check_abstract_guard(&self.struct_registry, type_id) {
                                release_operands(&self.struct_registry, &args);
                                return Err(e);
                            }
                            // Extract type info before mutable borrow
                            let (num_fields, min_args, type_name, defaults, init_func) = {
                                let ty = self.struct_registry.get_type(type_id).unwrap();
                                let nf = ty.fields.len();
                                let ma = ty.fields.iter().filter(|f| !f.has_default).count();
                                let tn = ty.name.clone();
                                let defs: Vec<Value> = ty.fields.iter().map(|f| f.default).collect();
                                let init = ty.methods.get("init").map(|f| f.clone_ref(py));
                                (nf, ma, tn, defs, init)
                            };
                            if nargs < min_args {
                                release_operands(&self.struct_registry, &args);
                                return Err(VMError::TypeError(format!(
                                    "{}() missing {} required argument(s)",
                                    type_name,
                                    min_args - nargs
                                )));
                            }
                            if nargs > num_fields {
                                release_operands(&self.struct_registry, &args);
                                return Err(VMError::TypeError(format!(
                                    "{}() takes {} argument(s) but {} were given",
                                    type_name, num_fields, nargs
                                )));
                            }
                            let mut field_values = args;
                            // Voie A: defaults are shared from the type; each instance
                            // owns its field refs, so incref the filled defaults to
                            // balance the cascade decref at instance teardown.
                            field_values.extend(defaults.iter().take(num_fields).skip(nargs).map(|&d| {
                                d.clone_refcount();
                                d
                            }));
                            if let Err(e) = self.enforce_field_types(py, type_id, &mut field_values) {
                                // field_values holds every in-flight ref exactly once
                                // (moved args + incref'd surviving defaults).
                                release_operands(&self.struct_registry, &field_values);
                                return Err(e);
                            }
                            let idx = self.struct_registry.create_instance(type_id, field_values);
                            let inst_val = Value::from_struct_instance(idx);

                            match self.push_struct_init_frame(py, inst_val, init_func, frame) {
                                Ok(true) => continue,
                                Ok(false) => {}
                                Err(e) => {
                                    // The freshly created instance has no owner yet.
                                    decref_discard(&self.struct_registry, inst_val);
                                    return Err(e);
                                }
                            }
                            frame.push(inst_val);
                            continue;
                        }
                    }

                    // Unwrap BoundCatnipMethod: extract inner func, prepend instance to args
                    let (actual_func, args, bound_instance, super_source_type) =
                        self.unwrap_bound_method(py, py_func_bound, args)?;
                    let nargs = args.len();

                    // Check if this is a VMFunction (fast Rust cast, then fallback)
                    let vm_func_data: Option<(Arc<CodeObject>, Option<NativeClosureScope>)> =
                        if let Ok(vm_func) = actual_func.cast::<VMFunction>() {
                            let vm_ref = vm_func.borrow();
                            let code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                            let closure = vm_ref.native_closure.clone();
                            drop(vm_ref);
                            Some((code, closure))
                        } else if let Ok(vm_code) = actual_func.getattr("vm_code") {
                            match convert_code_object(py, &vm_code) {
                                Ok(c) => Some((c, None)),
                                Err(e) => {
                                    release_operands(&self.struct_registry, &args);
                                    return Err(VMError::RuntimeError(e.to_string()));
                                }
                            }
                        } else {
                            None
                        };
                    if let Some((new_code, native_closure)) = vm_func_data {
                        let func_id = new_code.func_id();

                        // Register pure function for JIT inlining
                        if new_code.is_pure && self.jit_enabled {
                            let mut jit = self.jit.lock().unwrap();
                            if let Some(ref mut executor) = *jit {
                                crate::jit::executor::register_pure_function(executor, func_id.clone(), &new_code);
                            }
                        }

                        // JIT: Handle recursive calls - check BEFORE recording
                        if self.jit_recorder.is_recording() {
                            let ip = frame.ip - 1; // Call instruction was just executed

                            // Check if this is a recursive call (calling the function being traced)
                            let is_recursive_call = if let Some(ref tracing_func_id) = self.jit_tracing_func_id {
                                &func_id == tracing_func_id
                            } else {
                                false
                            };

                            if is_recursive_call {
                                // Only record the FIRST CallSelf (when depth=0)
                                // Then increment depth to suspend further recording
                                if self.jit_recursive_depth == 0 {
                                    self.jit_recorder.record_call(&func_id, nargs, new_code.is_pure, ip);
                                }
                                self.jit_recursive_depth += 1;
                            } else {
                                // Non-recursive call - record normally
                                self.jit_recorder.record_call(&func_id, nargs, new_code.is_pure, ip);
                                // Loop trace: suspend recording of the callee body
                                // (one CallPure; the inliner rebuilds it). Gated on
                                // loop tracing so function-trace content is unchanged.
                                // The matching decrement is in the Return handler; a
                                // leaked depth is cleared at the next trace start.
                                if self.jit_tracing_func_id.is_none() {
                                    self.jit_recursive_depth += 1;
                                }
                            }
                        }

                        // JIT: Check if function is already compiled
                        let mut use_compiled = false;
                        if self.jit_enabled {
                            // Compiled functions take native i64 args and return a raw
                            // scalar: anything that isn't a plain int (float, bool, nil,
                            // heap value) has no representation on this path --
                            // as_int().unwrap_or(0) below would silently truncate it to 0.
                            // Fall back to the interpreter for those calls.
                            if self.jit_detector.is_compiled_internal(&func_id) && args.iter().all(|a| a.is_int()) {
                                // Function is compiled - try to use native code
                                let jit = self.jit.lock().unwrap();
                                if let Some(ref executor) = *jit {
                                    if let Some((compiled_fn, max_slot, fn_guards)) =
                                        executor.get_compiled_function(&func_id)
                                    {
                                        // Call compiled native code via locals array
                                        // Setup locals array with enough space for all used slots
                                        let array_size = (max_slot + 1).max(new_code.nlocals);
                                        let mut locals_array: Vec<i64> = vec![0; array_size];

                                        // Copy arguments to first N slots (native i64, not NaN-boxed)
                                        for (i, arg) in args.iter().enumerate() {
                                            if i < array_size {
                                                locals_array[i] = arg.as_int().unwrap_or(0);
                                            }
                                        }

                                        // Populate captured variable slots from name_guards
                                        let fn_guards = fn_guards.to_vec();
                                        let mut guards_passed = true;
                                        for (name, expected_value, slot) in &fn_guards {
                                            // Resolve current value of captured variable
                                            let current_value = resolve_jit_guard_value(
                                                py,
                                                name,
                                                &frame.closure_scope,
                                                host,
                                                &self.globals,
                                                &self.struct_registry,
                                            );

                                            match current_value {
                                                Some(val) if val == *expected_value => {
                                                    if *slot < locals_array.len() {
                                                        locals_array[*slot] = val;
                                                    }
                                                }
                                                _ => {
                                                    // Guard failed: fall back to interpreter
                                                    guards_passed = false;
                                                    break;
                                                }
                                            }
                                        }

                                        if !guards_passed {
                                            // Guard failed, skip to interpreter path
                                        } else {
                                            // Call compiled function with locals pointer and depth=0
                                            // Phase 3: Initial call starts at depth 0
                                            // Safety: locals_array has enough elements for all used slots
                                            let result_raw = unsafe { compiled_fn(locals_array.as_mut_ptr(), 0) };

                                            // Check for guard failure (-1 = side exit needed)
                                            if result_raw == -1 {
                                                // Guard failure: fall back to interpreter
                                                if self.trace {
                                                    eprintln!(
                                                        "[JIT] Guard failure in compiled function {}, falling back to interpreter",
                                                        func_id
                                                    );
                                                }
                                                use_compiled = false;
                                            } else {
                                                // Normal return: push result value
                                                if self.trace {
                                                    eprintln!(
                                                        "[JIT] Called compiled function: {}, result_raw = {:#x}",
                                                        func_id, result_raw
                                                    );
                                                }

                                                let result_value = Value::from_raw_scalar(result_raw as u64);
                                                frame.push(result_value);
                                                use_compiled = true;
                                            }
                                        }
                                    }
                                }
                            } else {
                                // Check if this function has a pending trace from previous hot detection
                                if let Some(ref pending_func_id) = self.jit_pending_function_trace.clone() {
                                    if pending_func_id == &func_id {
                                        // Check if this is a top-level call (not recursive)
                                        let is_recursive = self.frame_stack.iter().any(|f| {
                                            if let Some(ref code) = f.code {
                                                code.name == new_code.name
                                            } else {
                                                false
                                            }
                                        });

                                        if !is_recursive && !self.jit_tracing {
                                            // Start tracing this top-level call
                                            self.jit_recorder.start_function(
                                                func_id.clone(),
                                                new_code.nlocals,
                                                new_code.nargs,
                                            );
                                            self.jit_tracing = true;
                                            self.jit_recursive_depth = 0;
                                            self.jit_tracing_func_id = Some(func_id.clone());
                                            self.jit_tracing_depth = self.frame_stack.len() + 2; // Depth after frame push (current frame not on stack)
                                            self.jit_pending_function_trace = None; // Clear pending

                                            if self.trace {
                                                eprintln!(
                                                    "[JIT] Started tracing function '{}' (params: {}) [pending → top-level]",
                                                    new_code.name, new_code.nargs
                                                );
                                            }
                                        }
                                    }
                                }

                                // Track function calls for profiling
                                let became_hot = self.jit_detector.record_call_internal(&func_id);

                                if became_hot {
                                    if self.trace {
                                        eprintln!("[JIT] Function '{}' became hot (id: {})", new_code.name, func_id);
                                    }

                                    // Check if this is a recursive call (function already in call stack)
                                    let is_recursive = self.frame_stack.iter().any(|f| {
                                        if let Some(ref code) = f.code {
                                            code.name == new_code.name
                                        } else {
                                            false
                                        }
                                    });

                                    // If became hot during recursive call, schedule tracing for next top-level call
                                    if is_recursive {
                                        if !self.jit_tracing {
                                            self.jit_pending_function_trace = Some(func_id.clone());
                                            if self.trace {
                                                eprintln!(
                                                    "[JIT] Function became hot during recursion, pending trace for next top-level call"
                                                );
                                            }
                                        }
                                    } else {
                                        // Top-level call - start tracing
                                        if !self.jit_tracing {
                                            self.jit_recorder.start_function(
                                                func_id.clone(),
                                                new_code.nlocals,
                                                new_code.nargs,
                                            );
                                            self.jit_tracing = true;
                                            self.jit_recursive_depth = 0;
                                            self.jit_tracing_func_id = Some(func_id);
                                            self.jit_tracing_depth = self.frame_stack.len() + 2; // Depth after frame push (current frame not on stack)

                                            if self.trace {
                                                eprintln!(
                                                    "[JIT] Started tracing function '{}' (params: {}) [top-level]",
                                                    new_code.name, new_code.nargs
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // If not using compiled code, execute via interpreter
                        if !use_compiled {
                            // Track call for stack traces
                            let call_start_byte = _current_src_byte;
                            let fn_name = new_code.name.clone();
                            let jit_hash = new_code.bytecode_hash();

                            // Create and setup new frame
                            let mut new_frame = Frame::with_code(new_code);
                            new_frame.bind_args(py, &self.struct_registry, &args, None);
                            new_frame.closure_scope = native_closure;

                            // Setup super proxy if this is a bound method call on a struct with parent_methods
                            if let Some(inst_val) = bound_instance {
                                if let Err(e) = self.setup_super_proxy(py, inst_val, super_source_type, &mut new_frame)
                                {
                                    // bind_args moved the args into this frame.
                                    decref_frame_values(&new_frame, &self.struct_registry);
                                    return Err(e);
                                }
                            }

                            self.update_jit_bytecode_hash_value(jit_hash);
                            self.call_stack.push(CallInfo {
                                name: fn_name,
                                call_start_byte,
                            });
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                        }
                        continue;
                    } else {
                        // Regular Python function - call directly
                        let args_py = self.build_python_call_args(py, host, &actual_func, &args)?;

                        // JIT: record builtin pure calls as native ops
                        if self.jit_tracing && self.jit_recursive_depth == 0 {
                            if let Ok(qualname) =
                                actual_func.getattr("__qualname__").and_then(|n| n.extract::<String>())
                            {
                                let ip = frame.ip - 1;
                                let recorded = match (qualname.as_str(), nargs) {
                                    // Native builtins (Cranelift codegen)
                                    ("abs", 1) => {
                                        self.jit_recorder.record_builtin(TraceOp::AbsInt, ip);
                                        true
                                    }
                                    ("min", 2) => {
                                        self.jit_recorder.record_builtin(TraceOp::MinInt, ip);
                                        true
                                    }
                                    ("max", 2) => {
                                        self.jit_recorder.record_builtin(TraceOp::MaxInt, ip);
                                        true
                                    }
                                    ("round", 1) => {
                                        self.jit_recorder.record_builtin(TraceOp::RoundInt, ip);
                                        true
                                    }
                                    ("int", 1) => {
                                        self.jit_recorder.record_builtin(TraceOp::IntCastInt, ip);
                                        true
                                    }
                                    ("bool", 1) => {
                                        self.jit_recorder.record_builtin(TraceOp::BoolInt, ip);
                                        true
                                    }
                                    // Callback builtins (extern C dispatch)
                                    (name, n) => {
                                        if let Some(bid) = builtin_name_to_id(name) {
                                            self.jit_recorder.record_builtin(
                                                TraceOp::CallBuiltinPure {
                                                    builtin_id: bid,
                                                    num_args: n as u8,
                                                },
                                                ip,
                                            );
                                            true
                                        } else {
                                            false
                                        }
                                    }
                                };
                                if !recorded {
                                    self.jit_recorder.record_fallback(OpCode::Call, ip);
                                }
                            }
                        }

                        let gen_before = GLOBALS_GEN.load(Ordering::Relaxed);
                        let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                        let result = actual_func.call1(args_tuple).to_vm(py)?;
                        let value = Value::from_pyobject(py, &result).to_vm(py)?;
                        frame.push(value);

                        // Sync globals back to local slots after Python call.
                        // Skip if no globals were mutated (most builtins don't re-enter VM).
                        // Uses static GLOBALS_GEN to detect re-entrant VMs that share ctx_globals.
                        if GLOBALS_GEN.load(Ordering::Relaxed) != gen_before {
                            if let Some(ref code) = frame.code {
                                let updates: Vec<(String, usize, Value)> = code
                                    .slotmap
                                    .iter()
                                    .filter_map(|(name, &slot_idx)| {
                                        // Skip native-tagged values (would lose tag through Python round-trip)
                                        let current = frame.get_local(slot_idx);
                                        if current.has_native_tag() {
                                            return None;
                                        }
                                        host.lookup_global(py, name.as_str())
                                            .ok()
                                            .flatten()
                                            .map(|v| (name.clone(), slot_idx, v))
                                    })
                                    .collect();
                                for (name, slot_idx, v) in updates {
                                    // Same accounting as StoreScope: the old
                                    // local's ref is released before the slot
                                    // is overwritten (set_local writes without
                                    // releasing), and self.globals takes its
                                    // OWN ref of the resynced value (two
                                    // owning slots, two refs). Skipping either
                                    // side leaked the old handle's refs and
                                    // double-released the new one.
                                    let old_local = frame.get_local(slot_idx);
                                    decref_discard(&self.struct_registry, old_local);
                                    frame.set_local(slot_idx, v);
                                    // Keep self.globals in sync for subsequent LoadScope
                                    if let Some(&old_global) = self.globals.get(name.as_str()) {
                                        decref_discard(&self.struct_registry, old_global);
                                        v.clone_refcount();
                                        if let Some(existing) = self.globals.get_mut(name.as_str()) {
                                            *existing = v;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                OpCode::CallKw => {
                    // Decode: (nargs << 8) | nkw
                    const NARGS_SHIFT: u32 = 8;
                    const NKW_MASK: u32 = 0xFF;
                    let nargs = (instr.arg >> NARGS_SHIFT) as usize;
                    let nkw = (instr.arg & NKW_MASK) as usize;

                    // Pop kw_names tuple
                    let kw_names_val = frame.pop();
                    let kw_names = kw_names_val.to_pyobject(py);
                    // Voie A: the popped names-tuple handle is owned; `kw_names`
                    // holds its own ref.
                    decref_pyobj(kw_names_val);
                    let kw_names_bound = kw_names.bind(py);
                    let kw_names_tuple = kw_names_bound
                        .cast::<PyTuple>()
                        .map_err(|_| VMError::TypeError("expected tuple for kw_names".into()))?;

                    // Read kwargs + args in stack order, then truncate
                    let stack_len = frame.stack.len();
                    let total = nargs + nkw;
                    let start = stack_len - total;
                    let args: Vec<Value> = frame.stack[start..start + nargs].to_vec();
                    let kw_values: Vec<Value> = frame.stack[start + nargs..].to_vec();
                    frame.stack.truncate(start);

                    // Pop function
                    let func = frame.pop();

                    // Native struct instantiation with kwargs (fast path)
                    {
                        let py_func_tmp = func.to_pyobject(py);
                        let ptr = py_func_tmp.bind(py).as_ptr() as usize;
                        if let Some(&type_id) = self.struct_type_map.get(&ptr) {
                            // Error exits release the in-flight owned values: the
                            // callable handle, the popped args and kw values.
                            if let Err(e) = check_abstract_guard(&self.struct_registry, type_id) {
                                decref_pyobj(func);
                                release_operands(&self.struct_registry, &args);
                                release_operands(&self.struct_registry, &kw_values);
                                return Err(e);
                            }
                            // Extract type info before mutable borrow
                            let (type_name, field_defaults, field_info, init_func) = {
                                let ty = self.struct_registry.get_type(type_id).unwrap();
                                let tn = ty.name.clone();
                                let defs: Vec<(Value, bool)> =
                                    ty.fields.iter().map(|f| (f.default, f.has_default)).collect();
                                let fi: Vec<(String, bool)> =
                                    ty.fields.iter().map(|f| (f.name.clone(), f.has_default)).collect();
                                let init = ty.methods.get("init").map(|f| f.clone_ref(py));
                                (tn, defs, fi, init)
                            };

                            // Start with defaults (bit-copies; the surviving ones
                            // take their own ref below, once overwrites are known)
                            let mut field_values: Vec<Value> = field_defaults
                                .iter()
                                .map(|(def, has)| if *has { *def } else { Value::NIL })
                                .collect();
                            let mut filled = vec![false; field_values.len()];

                            // Place positional args (owned, moved into the fields)
                            for (i, val) in args.iter().enumerate() {
                                field_values[i] = *val;
                                filled[i] = true;
                            }

                            // Place keyword args by name (owned, moved into the fields)
                            for (i, val) in kw_values.iter().enumerate() {
                                let kw_name: String = match kw_names_tuple.get_item(i).and_then(|it| it.extract()) {
                                    Ok(n) => n,
                                    Err(e) => {
                                        decref_pyobj(func);
                                        release_operands(&self.struct_registry, &args);
                                        release_operands(&self.struct_registry, &kw_values);
                                        return Err(VMError::RuntimeError(e.to_string()));
                                    }
                                };
                                match field_info.iter().position(|(n, _)| n == &kw_name) {
                                    Some(idx) => {
                                        if filled[idx] {
                                            // Mirrors Python; also keeps the move
                                            // contract single-owner (a silent
                                            // overwrite would leak the displaced ref).
                                            decref_pyobj(func);
                                            release_operands(&self.struct_registry, &args);
                                            release_operands(&self.struct_registry, &kw_values);
                                            return Err(VMError::TypeError(format!(
                                                "{}() got multiple values for argument '{}'",
                                                type_name, kw_name
                                            )));
                                        }
                                        field_values[idx] = *val;
                                        filled[idx] = true;
                                    }
                                    None => {
                                        decref_pyobj(func);
                                        release_operands(&self.struct_registry, &args);
                                        release_operands(&self.struct_registry, &kw_values);
                                        return Err(VMError::TypeError(format!(
                                            "{}() got an unexpected keyword argument '{}'",
                                            type_name, kw_name
                                        )));
                                    }
                                }
                            }

                            // Voie A: defaults are shared from the type; each instance
                            // owns its field refs, so the defaults that survived the
                            // overwrites take their own ref (mirrors the Call opcode) --
                            // the instance teardown cascade releases exactly one.
                            for (i, val) in field_values.iter().enumerate() {
                                if !filled[i] && field_defaults[i].1 {
                                    val.clone_refcount();
                                }
                            }

                            // Validate no missing required fields
                            for (i, (fname, has_default)) in field_info.iter().enumerate() {
                                if !has_default && field_values[i].is_nil() && i >= nargs {
                                    // field_values now holds every in-flight ref
                                    // exactly once (placed args/kw + incref'd
                                    // surviving defaults; empty slots are NIL).
                                    decref_pyobj(func);
                                    release_operands(&self.struct_registry, &field_values);
                                    return Err(VMError::TypeError(format!(
                                        "{}() missing required argument: '{}'",
                                        type_name, fname
                                    )));
                                }
                            }

                            if let Err(e) = self.enforce_field_types(py, type_id, &mut field_values) {
                                decref_pyobj(func);
                                release_operands(&self.struct_registry, &field_values);
                                return Err(e);
                            }
                            let inst_idx = self.struct_registry.create_instance(type_id, field_values);
                            let inst_val = Value::from_struct_instance(inst_idx);
                            // Voie A: release the callable handle on the struct fast path
                            // (py_func_tmp keeps the type object alive across the continue).
                            decref_pyobj(func);
                            match self.push_struct_init_frame(py, inst_val, init_func, frame) {
                                Ok(true) => continue,
                                Ok(false) => {}
                                Err(e) => {
                                    // The freshly created instance has no owner yet.
                                    decref_discard(&self.struct_registry, inst_val);
                                    return Err(e);
                                }
                            }
                            frame.push(inst_val);
                            continue;
                        }
                    }

                    let py_func = func.to_pyobject(py);
                    // Voie A: slow path uses py_func; release the callable handle.
                    decref_pyobj(func);
                    let py_func_bound = py_func.bind(py);

                    // Unwrap BoundCatnipMethod: extract inner func, prepend instance to args
                    // (the helper releases `args` on error; the kw values are
                    // still owned here -- released below, after the dict build)
                    let (actual_func_kw, args, bound_instance_kw, super_source_type_kw) =
                        match self.unwrap_bound_method(py, py_func_bound, args) {
                            Ok(unwrapped) => unwrapped,
                            Err(e) => {
                                release_operands(&self.struct_registry, &kw_values);
                                return Err(e);
                            }
                        };

                    // Build kwargs dict
                    let kwargs_dict = PyDict::new(py);
                    for (i, val) in kw_values.iter().enumerate() {
                        let entry = kw_names_tuple
                            .get_item(i)
                            .and_then(|name| kwargs_dict.set_item(name, val.to_pyobject(py)));
                        if let Err(e) = entry {
                            release_operands(&self.struct_registry, &args);
                            release_operands(&self.struct_registry, &kw_values);
                            return Err(VMError::RuntimeError(e.to_string()));
                        }
                    }
                    // Voie A: the popped kw values are owned and the dict holds
                    // independent refs -- release them here for BOTH branches
                    // below (bind_args reads the dict, not these Values).
                    for &kwv in &kw_values {
                        decref_discard(&self.struct_registry, kwv);
                    }

                    // Check if VMFunction (fast Rust cast, then fallback)
                    let vm_func_data_kw: Option<(Arc<CodeObject>, Option<NativeClosureScope>)> =
                        if let Ok(vm_func) = actual_func_kw.cast::<VMFunction>() {
                            let vm_ref = vm_func.borrow();
                            let code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                            let closure = vm_ref.native_closure.clone();
                            drop(vm_ref);
                            Some((code, closure))
                        } else if let Ok(vm_code) = actual_func_kw.getattr("vm_code") {
                            match convert_code_object(py, &vm_code) {
                                Ok(c) => Some((c, None)),
                                Err(e) => {
                                    release_operands(&self.struct_registry, &args);
                                    return Err(VMError::RuntimeError(e.to_string()));
                                }
                            }
                        } else {
                            None
                        };
                    if let Some((new_code, native_closure)) = vm_func_data_kw {
                        let fn_name = new_code.name.clone();
                        let call_start_byte = _current_src_byte;
                        let mut new_frame = Frame::with_code(new_code);
                        new_frame.bind_args(py, &self.struct_registry, &args, Some(&kwargs_dict));
                        new_frame.closure_scope = native_closure;

                        // Setup super proxy for bound method calls
                        if let Some(inst_val) = bound_instance_kw {
                            if let Err(e) = self.setup_super_proxy(py, inst_val, super_source_type_kw, &mut new_frame) {
                                // bind_args moved the args into this frame.
                                decref_frame_values(&new_frame, &self.struct_registry);
                                return Err(e);
                            }
                        }

                        self.call_stack.push(CallInfo {
                            name: fn_name,
                            call_start_byte,
                        });
                        {
                            let old = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old);
                        }
                        continue;
                    } else {
                        // Python function - call with kwargs
                        let args_py = self.build_python_call_args(py, host, &actual_func_kw, &args)?;

                        let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                        let result = actual_func_kw.call(args_tuple, Some(&kwargs_dict)).to_vm(py)?;
                        let value = Value::from_pyobject(py, &result).to_vm(py)?;
                        frame.push(value);
                    }
                }

                OpCode::TailCall => {
                    // TCO: reuse current frame instead of creating a new one
                    let nargs = instr.arg as usize;
                    let stack_len = frame.stack.len();
                    let args_start = stack_len - nargs;
                    let args: Vec<Value> = frame.stack[args_start..].to_vec();
                    frame.stack.truncate(args_start);
                    let func = frame.pop();

                    // Native struct instantiation (fast path) - same as Call
                    {
                        let py_func_tmp = func.to_pyobject(py);
                        let ptr = py_func_tmp.bind(py).as_ptr() as usize;
                        if let Some(&type_id) = self.struct_type_map.get(&ptr) {
                            // Error exits release the in-flight popped args AND the
                            // callable handle (unlike Call, it is still owned here --
                            // the happy path releases it just before the continue).
                            if let Err(e) = check_abstract_guard(&self.struct_registry, type_id) {
                                decref_pyobj(func);
                                release_operands(&self.struct_registry, &args);
                                return Err(e);
                            }
                            let (num_fields, min_args, type_name, defaults, init_func) = {
                                let ty = self.struct_registry.get_type(type_id).unwrap();
                                let nf = ty.fields.len();
                                let ma = ty.fields.iter().filter(|f| !f.has_default).count();
                                let tn = ty.name.clone();
                                let defs: Vec<Value> = ty.fields.iter().map(|f| f.default).collect();
                                let init = ty.methods.get("init").map(|f| f.clone_ref(py));
                                (nf, ma, tn, defs, init)
                            };
                            if nargs < min_args {
                                decref_pyobj(func);
                                release_operands(&self.struct_registry, &args);
                                return Err(VMError::TypeError(format!(
                                    "{}() missing {} required argument(s)",
                                    type_name,
                                    min_args - nargs
                                )));
                            }
                            if nargs > num_fields {
                                decref_pyobj(func);
                                release_operands(&self.struct_registry, &args);
                                return Err(VMError::TypeError(format!(
                                    "{}() takes {} argument(s) but {} were given",
                                    type_name, num_fields, nargs
                                )));
                            }
                            let mut field_values = args;
                            // Voie A: defaults are shared from the type; each instance
                            // owns its field refs, so incref the filled defaults to
                            // balance the cascade decref at instance teardown.
                            field_values.extend(defaults.iter().take(num_fields).skip(nargs).map(|&d| {
                                d.clone_refcount();
                                d
                            }));
                            if let Err(e) = self.enforce_field_types(py, type_id, &mut field_values) {
                                decref_pyobj(func);
                                // Moved args + incref'd surviving defaults, each once.
                                release_operands(&self.struct_registry, &field_values);
                                return Err(e);
                            }
                            let idx = self.struct_registry.create_instance(type_id, field_values);
                            let inst_val = Value::from_struct_instance(idx);

                            // Voie A: release the callable handle on the struct fast path
                            // (py_func_tmp keeps the type object alive across the continue).
                            decref_pyobj(func);
                            match self.push_struct_init_frame(py, inst_val, init_func, frame) {
                                Ok(true) => continue,
                                Ok(false) => {}
                                Err(e) => {
                                    // The freshly created instance has no owner yet.
                                    decref_discard(&self.struct_registry, inst_val);
                                    return Err(e);
                                }
                            }
                            frame.push(inst_val);
                            continue;
                        }
                    }

                    let py_func = func.to_pyobject(py);
                    // Voie A: slow path uses py_func; release the callable handle.
                    decref_pyobj(func);
                    let py_func_bound = py_func.bind(py);

                    // Unwrap BoundCatnipMethod: extract inner func, prepend instance to args
                    let (actual_func, args, bound_instance, super_source_type) =
                        self.unwrap_bound_method(py, py_func_bound, args)?;
                    let nargs = args.len();

                    // VMFunction detection (fast Rust cast, then fallback)
                    let tco_data: Option<(Arc<CodeObject>, Option<NativeClosureScope>)> =
                        if let Ok(vm_func) = actual_func.cast::<VMFunction>() {
                            let vm_ref = vm_func.borrow();
                            let code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                            let closure = vm_ref.native_closure.clone();
                            drop(vm_ref);
                            Some((code, closure))
                        } else if let Ok(vm_code) = actual_func.getattr("vm_code") {
                            match convert_code_object(py, &vm_code) {
                                Ok(c) => Some((c, None)),
                                Err(e) => {
                                    release_operands(&self.struct_registry, &args);
                                    return Err(VMError::RuntimeError(e.to_string()));
                                }
                            }
                        } else {
                            None
                        };
                    if let Some((new_code, tco_closure)) = tco_data {
                        // VMFunction - reuse frame for TCO

                        // 1. Release the reused frame's old locals, then resize.
                        // A reused frame's slots own independent handles, so a
                        // bare NIL overwrite (or a truncating resize) leaked one
                        // ref per pyobj/bigint/struct local surviving into a tail
                        // call. Releasing here is balanced, not a double-free with
                        // the globals map: StoreScope and push_block take
                        // independent refcounts (the aliasing hazard pop_block
                        // used to exhibit is closed). Decref over the OLD length
                        // before the resize so a shrink cannot drop live slots.
                        for i in 0..frame.locals.len() {
                            decref_discard(&self.struct_registry, frame.locals[i]);
                        }
                        let nlocals = new_code.nlocals;
                        frame.locals.clear();
                        frame.locals.resize(nlocals, Value::NIL);

                        // 3. Rebind args with varargs handling
                        let vararg_idx = new_code.vararg_idx;
                        if vararg_idx >= 0 {
                            let vararg_idx_usize = vararg_idx as usize;
                            // Args before vararg
                            frame.locals[..args.len().min(vararg_idx_usize)]
                                .copy_from_slice(&args[..args.len().min(vararg_idx_usize)]);
                            // Collect excess into vararg slot (store PyList directly, skip type detection)
                            if args.len() > vararg_idx_usize {
                                let excess: Vec<Py<PyAny>> = args[vararg_idx_usize..]
                                    .iter()
                                    .map(|v: &Value| v.to_pyobject(py))
                                    .collect();
                                // Voie A: the excess originals live on only through
                                // the PyList (independent refs) -- release them.
                                for &a in &args[vararg_idx_usize..] {
                                    decref_discard(&self.struct_registry, a);
                                }
                                let list = PyList::new(py, excess).map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                frame.locals[vararg_idx_usize] = Value::from_owned_pyobject(list.unbind().into_any());
                            } else {
                                let empty = PyList::empty(py);
                                frame.locals[vararg_idx_usize] = Value::from_owned_pyobject(empty.unbind().into_any());
                            }
                        } else {
                            // No varargs - direct rebind
                            for (i, arg) in args.into_iter().enumerate() {
                                if i < nlocals {
                                    frame.locals[i] = arg;
                                } else {
                                    // Voie A: accepted-and-discarded excess arg.
                                    decref_discard(&self.struct_registry, arg);
                                }
                            }
                        }

                        // 4. Fill defaults for remaining params
                        let nparams = new_code.nargs;
                        let ndefaults = new_code.defaults.len();
                        if ndefaults > 0 {
                            let default_start = nparams.saturating_sub(ndefaults);
                            for i in nargs.max(default_start)..nparams {
                                let default_idx = i - default_start;
                                if default_idx < ndefaults {
                                    let val = new_code.defaults[default_idx];
                                    val.clone_refcount();
                                    frame.locals[i] = val;
                                }
                            }
                        }

                        // 5. Reset frame state. Release block_stack entries whose
                        // values now hold independent refcounts (taken at push_block),
                        // then clear -- the jump to ip=0 abandons the body without
                        // running PopBlock, otherwise entries pile up per iteration.
                        frame.ip = 0;
                        frame.stack.clear();
                        for (_slot_start, saved) in frame.block_stack.drain(..) {
                            for val in saved {
                                decref_discard(&self.struct_registry, val);
                            }
                        }
                        release_match_bindings(&self.struct_registry, frame);
                        frame.closure_scope = tco_closure;

                        // Setup super proxy for bound method calls. The reused
                        // frame may carry a stale proxy from the previous call:
                        // clear it first, setup_super_proxy only writes when the
                        // instance's type actually has parent methods.
                        frame.super_proxy = None;
                        if let Some(inst_val) = bound_instance {
                            self.setup_super_proxy(py, inst_val, super_source_type, frame)?;
                        }

                        // Replace code object
                        frame.code = Some(new_code);
                        // Continue to restart dispatch with new code
                        continue;
                    } else {
                        // Python callable - call directly
                        let args_py = self.build_python_call_args(py, host, &actual_func, &args)?;

                        let args_tuple = PyTuple::new(py, args_py).map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        let result = actual_func
                            .call1(args_tuple)
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        let value = Value::from_pyobject(py, &result).to_vm(py)?;
                        frame.push(value);
                    }
                }

                // Fused GetAttr + Call: resolve method on obj, call directly.
                // Eliminates BoundCatnipMethod allocation for struct method calls.
                OpCode::CallMethod => {
                    use super::{CALL_ARGS_MASK, CALL_ARGS_SHIFT};
                    let nargs = (instr.arg & CALL_ARGS_MASK) as usize;
                    let name_idx = (instr.arg >> CALL_ARGS_SHIFT) as usize;
                    let method_name = &code.names[name_idx];

                    // Stack: [obj, arg1, arg2, ...argN]
                    let stack_len = frame.stack.len();
                    let args_start = stack_len - nargs;
                    let args: Vec<Value> = frame.stack[args_start..].to_vec();
                    frame.stack.truncate(args_start);
                    let obj = frame.pop();

                    if let Some(idx) = obj.as_struct_instance_idx() {
                        let (type_id, fields) = self
                            .struct_registry
                            .with_instance(idx, |i| (i.type_id, i.fields.clone()))
                            .ok_or_else(|| VMError::RuntimeError(format!("invalid struct instance index {idx}")))?;
                        let ty = self
                            .struct_registry
                            .get_type(type_id)
                            .ok_or_else(|| VMError::RuntimeError(format!("invalid struct type index {type_id}")))?;

                        // Check field first (callable field, no self binding)
                        if let Some(field_idx) = ty.field_index(method_name) {
                            let field_val = fields[field_idx];
                            // Call as regular function (no self)
                            let py_func = field_val.to_pyobject(py);
                            let py_func_bound = py_func.bind(py);
                            if let Ok(vm_func) = py_func_bound.cast::<VMFunction>() {
                                let vm_ref = vm_func.borrow();
                                let new_code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                                let closure = vm_ref.native_closure.clone();
                                drop(vm_ref);
                                let mut new_frame = Frame::with_code(new_code);
                                new_frame.bind_args(py, &self.struct_registry, &args, None);
                                new_frame.closure_scope = closure;
                                // The owned receiver is not passed (no self); the
                                // callee's code/closure are independent clones, so
                                // releasing it before the body is safe.
                                decref_discard(&self.struct_registry, obj);
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            } else {
                                // Python callable field. The receiver is owned and
                                // not passed (no self binding): release it, plus the
                                // owned args once `args_py` holds independent refs.
                                let args_py: Vec<Py<PyAny>> = args.iter().map(|v| v.to_pyobject(py)).collect();
                                decref_discard(&self.struct_registry, obj);
                                for &arg in &args {
                                    decref_discard(&self.struct_registry, arg);
                                }
                                let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                                let result = py_func_bound.call1(args_tuple).to_vm(py)?;
                                let value = Value::from_pyobject(py, &result).to_vm(py)?;
                                frame.push(value);
                                continue;
                            }
                        }

                        // Method lookup (with self binding). The handle is cloned so
                        // the registry borrow ends -- the Python sub-branch releases
                        // owned values through `&self.struct_registry`.
                        if let Some(func) = ty.methods.get(method_name.as_str()).map(|f| f.clone_ref(py)) {
                            let func_bound = func.bind(py);
                            // Prepend self to args
                            let mut all_args = Vec::with_capacity(nargs + 1);
                            all_args.push(obj);
                            all_args.extend_from_slice(&args);

                            if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
                                let vm_ref = vm_func.borrow();
                                let new_code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                                let closure = vm_ref.native_closure.clone();
                                drop(vm_ref);
                                let mut new_frame = Frame::with_code(new_code);
                                new_frame.bind_args(py, &self.struct_registry, &all_args, None);
                                new_frame.closure_scope = closure;
                                // Setup super proxy
                                if let Err(e) = self.setup_super_proxy(py, obj, None, &mut new_frame) {
                                    // bind_args moved all_args (receiver included).
                                    decref_frame_values(&new_frame, &self.struct_registry);
                                    return Err(e);
                                }
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            } else {
                                // Python method: all_args (receiver included) are
                                // owned; `args_py` holds independent refs.
                                let args_py: Vec<Py<PyAny>> = all_args.iter().map(|v| v.to_pyobject(py)).collect();
                                for &arg in &all_args {
                                    decref_discard(&self.struct_registry, arg);
                                }
                                let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                                let result = func_bound.call1(args_tuple).to_vm(py)?;
                                let value = Value::from_pyobject(py, &result).to_vm(py)?;
                                frame.push(value);
                                continue;
                            }
                        }

                        // Static method (no self binding): the owned receiver is not
                        // passed, so both sub-branches release it.
                        if let Some(func) = ty.static_methods.get(method_name.as_str()).map(|f| f.clone_ref(py)) {
                            let func_bound = func.bind(py);
                            if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
                                let vm_ref = vm_func.borrow();
                                let new_code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                                let closure = vm_ref.native_closure.clone();
                                drop(vm_ref);
                                let mut new_frame = Frame::with_code(new_code);
                                new_frame.bind_args(py, &self.struct_registry, &args, None);
                                new_frame.closure_scope = closure;
                                decref_discard(&self.struct_registry, obj);
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            } else {
                                let args_py: Vec<Py<PyAny>> = args.iter().map(|v| v.to_pyobject(py)).collect();
                                decref_discard(&self.struct_registry, obj);
                                for &arg in &args {
                                    decref_discard(&self.struct_registry, arg);
                                }
                                let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                                let result = func_bound.call1(args_tuple).to_vm(py)?;
                                let value = Value::from_pyobject(py, &result).to_vm(py)?;
                                frame.push(value);
                                continue;
                            }
                        }

                        // Reachable (typo in a method name): the popped receiver
                        // and args are in flight. Build the message first -- `ty`
                        // borrows the registry the releases mutate.
                        let msg = attr_error_msg(ty, method_name);
                        decref_discard(&self.struct_registry, obj);
                        release_operands(&self.struct_registry, &args);
                        return Err(VMError::RuntimeError(msg));
                    } else if let Some(func) = obj.as_symbol().and_then(|sym_id| {
                        // Union method on a nullary variant: dispatch natively
                        // so `self` keeps its symbol id in this VM's table
                        // (the Python boundary would re-intern it elsewhere).
                        let qualified = super::value::resolve_symbol(sym_id)?;
                        let methods = super::value::union_nullary_methods_for(&qualified)?;
                        methods.get(method_name.as_str()).map(|f| f.clone_ref(py))
                    }) {
                        let func_bound = func.bind(py);
                        let mut all_args = Vec::with_capacity(nargs + 1);
                        all_args.push(obj);
                        all_args.extend_from_slice(&args);
                        if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
                            let vm_ref = vm_func.borrow();
                            let new_code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                            let closure = vm_ref.native_closure.clone();
                            drop(vm_ref);
                            let mut new_frame = Frame::with_code(new_code);
                            new_frame.bind_args(py, &self.struct_registry, &all_args, None);
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        } else {
                            // Voie A: all_args are owned (the symbol receiver's
                            // decref is a no-op); `args_py` holds independent refs.
                            let args_py: Vec<Py<PyAny>> = all_args.iter().map(|v| v.to_pyobject(py)).collect();
                            for &arg in &all_args {
                                decref_discard(&self.struct_registry, arg);
                            }
                            let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                            let result = func_bound.call1(args_tuple).to_vm(py)?;
                            let value = Value::from_pyobject(py, &result).to_vm(py)?;
                            frame.push(value);
                            continue;
                        }
                    } else {
                        // PyObject fallback: getattr + call
                        let py_obj = obj.to_pyobject(py);
                        let py_bound = py_obj.bind(py);
                        // Voie A: the popped receiver is owned and `py_obj` holds an
                        // independent ref; nothing below reads `obj` again, so one
                        // release covers every sub-branch of this fallback.
                        decref_discard(&self.struct_registry, obj);

                        // Native plugin object: route method calls to the plugin's
                        // method callback (mirrors catnip_vm::host), GIL released.
                        if let Ok(po) = py_bound.cast::<crate::loader::native_plugin::NativePluginObject>() {
                            let (handle, cbs) = po
                                .borrow()
                                .handle_and_callbacks()
                                .ok_or_else(|| VMError::RuntimeError("invalid plugin object".into()))?;
                            let method_fn = cbs.method.ok_or_else(|| {
                                VMError::RuntimeError(format!("plugin object has no method '{method_name}'"))
                            })?;
                            let mut vm_args: Vec<catnip_vm::Value> = Vec::with_capacity(args.len());
                            for a in &args {
                                let pa = a.to_pyobject(py);
                                match crate::vm::py_interop::convert_py_to_vm_value(pa.bind(py)) {
                                    Ok(v) => vm_args.push(v),
                                    Err(e) => {
                                        // Already-converted catnip_vm values own
                                        // their refs; the original args too.
                                        for v in &vm_args {
                                            v.decref();
                                        }
                                        release_operands(&self.struct_registry, &args);
                                        return Err(VMError::RuntimeError(e.to_string()));
                                    }
                                }
                            }
                            // Voie A: the owned args were converted into independent
                            // catnip_vm values; release the originals.
                            for &arg in &args {
                                decref_discard(&self.struct_registry, arg);
                            }
                            let method_owned = method_name.clone();
                            let bits = py.detach(|| {
                                catnip_vm::plugin::call_plugin_method(handle, method_fn, &method_owned, &vm_args, &cbs)
                                    .map(|v| v.bits())
                                    .map_err(|e| e.to_string())
                            });
                            for a in &vm_args {
                                a.decref();
                            }
                            let bits = bits.map_err(VMError::RuntimeError)?;
                            let pyres = crate::vm::py_interop::vm_value_to_py(py, catnip_vm::Value::from_raw(bits))
                                .to_vm(py)?;
                            let value = Value::from_pyobject(py, pyres.bind(py)).to_vm(py)?;
                            frame.push(value);
                            continue;
                        }

                        // Check struct marker type (static methods)
                        let ptr = py_bound.as_ptr() as usize;
                        if let Some(&type_id) = self.struct_type_map.get(&ptr) {
                            // Owned clone so the registry borrow ends (the Python
                            // sub-branch releases args through `&mut struct_registry`).
                            let func = self
                                .struct_registry
                                .get_type(type_id)
                                .unwrap()
                                .static_methods
                                .get(method_name.as_str())
                                .map(|f| f.clone_ref(py));
                            if let Some(func) = func {
                                let func_bound = func.bind(py);
                                if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
                                    let vm_ref = vm_func.borrow();
                                    let new_code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                                    let closure = vm_ref.native_closure.clone();
                                    drop(vm_ref);
                                    let mut new_frame = Frame::with_code(new_code);
                                    new_frame.bind_args(py, &self.struct_registry, &args, None);
                                    new_frame.closure_scope = closure;
                                    {
                                        let old = std::mem::replace(frame, new_frame);
                                        self.frame_stack.push(old);
                                    }
                                    continue;
                                } else {
                                    // Python static method (Voie A: owned args released)
                                    let args_py: Vec<Py<PyAny>> = args.iter().map(|v| v.to_pyobject(py)).collect();
                                    for &arg in &args {
                                        decref_discard(&self.struct_registry, arg);
                                    }
                                    let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                                    let result = func_bound.call1(args_tuple).to_vm(py)?;
                                    let value = Value::from_pyobject(py, &result).to_vm(py)?;
                                    frame.push(value);
                                    continue;
                                }
                            }
                            let msg = attr_error_msg(self.struct_registry.get_type(type_id).unwrap(), method_name);
                            release_operands(&self.struct_registry, &args);
                            return Err(VMError::RuntimeError(msg));
                        }

                        // Check SuperProxy: resolve method and call with self
                        if let Ok(sp) = py_bound.cast::<super::structs::SuperProxy>() {
                            let sp_ref = sp.borrow();
                            if let Some(func) = sp_ref.methods.get(method_name.as_str()) {
                                let func_clone = func.clone_ref(py);
                                let inst_py = sp_ref.instance.clone_ref(py);
                                let native_idx = sp_ref.native_instance_idx;
                                let native_registry_id = sp_ref.native_registry_id;
                                let source_type = sp_ref
                                    .method_sources
                                    .get(method_name.as_str())
                                    .cloned()
                                    .unwrap_or_default();
                                drop(sp_ref);
                                let func_bound = func_clone.bind(py);
                                // Build args with self prepended. Native fast path only
                                // when the proxy belongs to this VM's registry; a cross-VM
                                // idx would name an unrelated slot.
                                let inst_val = if let Some(idx) =
                                    native_idx.filter(|_| native_registry_id == self.struct_registry.id())
                                {
                                    self.struct_registry.incref(idx);
                                    Value::from_struct_instance(idx)
                                } else {
                                    Value::from_pyobject(py, inst_py.bind(py)).to_vm(py)?
                                };
                                let mut all_args = Vec::with_capacity(nargs + 1);
                                all_args.push(inst_val);
                                all_args.extend_from_slice(&args);
                                if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
                                    let vm_ref = vm_func.borrow();
                                    let new_code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                                    let closure = vm_ref.native_closure.clone();
                                    drop(vm_ref);
                                    let mut new_frame = Frame::with_code(new_code);
                                    new_frame.bind_args(py, &self.struct_registry, &all_args, None);
                                    new_frame.closure_scope = closure;
                                    // Setup super chain for parent of parent
                                    if let Err(e) =
                                        self.setup_super_proxy(py, inst_val, Some(source_type), &mut new_frame)
                                    {
                                        decref_frame_values(&new_frame, &self.struct_registry);
                                        return Err(e);
                                    }
                                    {
                                        let old = std::mem::replace(frame, new_frame);
                                        self.frame_stack.push(old);
                                    }
                                    continue;
                                } else {
                                    // Voie A: all_args (prepended self included) are owned.
                                    let args_py: Vec<Py<PyAny>> = all_args.iter().map(|v| v.to_pyobject(py)).collect();
                                    for &arg in &all_args {
                                        decref_discard(&self.struct_registry, arg);
                                    }
                                    let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                                    let result = func_bound.call1(args_tuple).to_vm(py)?;
                                    let value = Value::from_pyobject(py, &result).to_vm(py)?;
                                    frame.push(value);
                                    continue;
                                }
                            }
                            release_operands(&self.struct_registry, &args);
                            return Err(VMError::RuntimeError(format!("super has no method '{}'", method_name)));
                        }

                        // General Python getattr + call
                        let method = match py_bound.getattr(method_name.as_str()) {
                            Ok(m) => m,
                            Err(e) => {
                                let msg = e.to_string();
                                // Reachable (typo in a method name): args in flight,
                                // the receiver was released at the fallback head.
                                release_operands(&self.struct_registry, &args);
                                return Err(VMError::RuntimeError(py_attr_error_msg(py_bound, method_name, &msg)));
                            }
                        };
                        // Inline VMFunction calls (avoid VMFunction.__call__ which
                        // creates a fresh VM without the parent's enum/symbol tables).
                        if let Ok(vm_func) = method.cast::<VMFunction>() {
                            let vm_ref = vm_func.borrow();
                            let new_code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                            let closure = vm_ref.native_closure.clone();
                            drop(vm_ref);
                            let mut new_frame = Frame::with_code(new_code);
                            new_frame.bind_args(py, &self.struct_registry, &args, None);
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                        // Voie A: owned args released once `args_py` holds its refs.
                        let args_py: Vec<Py<PyAny>> = args.iter().map(|v| v.to_pyobject(py)).collect();
                        for &arg in &args {
                            decref_discard(&self.struct_registry, arg);
                        }
                        let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                        let result = method.call1(args_tuple).to_vm(py)?;
                        let value = Value::from_pyobject(py, &result).to_vm(py)?;
                        frame.push(value);
                    }
                }

                OpCode::Return => {
                    // If handler_stack has Finally, handle inline (don't exit dispatch_inner)
                    if !frame.handler_stack.is_empty() {
                        let val = frame.pop();
                        let err = VMError::Return(val);
                        if self.try_unwind_to_handler(frame, &err) {
                            continue;
                        }
                        // No Finally handler, fall through to normal return
                        // Recover the value from the error
                        if let VMError::Return(v) = err {
                            frame.push(v);
                        }
                    }

                    // Decrement recursive depth BEFORE processing return
                    // This ensures we resume tracing at the correct point
                    if self.jit_recursive_depth > 0 {
                        self.jit_recursive_depth -= 1;
                    }

                    // Store the return value and discard flag before releasing frame borrow
                    last_result = frame.pop();
                    let discard = frame.discard_return;

                    // Pop call stack entry (if any)
                    if !self.call_stack.is_empty() {
                        self.call_stack.pop();
                    }

                    // Check if we should finalize function trace
                    // We finalize when returning to the depth where tracing started
                    // current frame is NOT on frame_stack, so depth = frame_stack.len() + 1
                    let current_depth = self.frame_stack.len() + 1;
                    let should_finalize_trace = self.jit_tracing
                        && self.jit_tracing_func_id.is_some()
                        && current_depth == self.jit_tracing_depth;

                    // Pop caller from frame_stack and replace current frame
                    if let Some(caller) = self.frame_stack.pop() {
                        let old = std::mem::replace(frame, caller);
                        // Full release of the frame's leftover values (stack
                        // residue + locals): pyobj handles AND bigint/complex/
                        // struct refs. The old Voie A contract released pyobj
                        // only, leaking one Arc/registry ref per leftover
                        // heap local on every function return (the 'balanced
                        // by opcodes' claim only holds for overwrites, not
                        // for the final state of the slots).
                        self.frame_pool.free(old, &self.struct_registry);
                        self.handle_nd_frame_pop(py, last_result);

                        // Push result to caller (unless init whose return is discarded)
                        if !discard {
                            frame.push(last_result);
                        }

                        // Restore caller's bytecode hash for JIT
                        if self.jit_enabled {
                            if let Some(ref caller_code) = frame.code {
                                self.update_jit_bytecode_hash_value(caller_code.bytecode_hash());
                            }
                        }
                    } else {
                        // No caller: return from top-level, let outer dispatch handle it
                        return Err(VMError::Return(last_result));
                    }

                    // Finalize trace if needed (after frame is popped)
                    if should_finalize_trace {
                        if let Some(mut trace) = self.jit_recorder.stop() {
                            if trace.is_compilable() {
                                let func_id = self.jit_tracing_func_id.take().unwrap();

                                // Phase 4.1: Optimize tail calls before compilation
                                trace.optimize_tail_calls();

                                if self.trace {
                                    eprintln!(
                                        "[JIT] Function trace complete: {} ops, params: {}",
                                        trace.ops.len(),
                                        trace.num_params
                                    );
                                }

                                // Compile the function trace to native code
                                let mut jit = self.jit.lock().unwrap();
                                if let Some(ref mut executor) = *jit {
                                    match executor.compile_function_trace(&trace) {
                                        Ok((compiled_fn, max_slot, name_guards)) => {
                                            if self.trace {
                                                eprintln!(
                                                    "[JIT] Function compiled successfully: {} (max_slot: {}, guards: {})",
                                                    func_id,
                                                    max_slot,
                                                    name_guards.len()
                                                );
                                            }
                                            // Store compiled function with max slot info and guards
                                            executor.store_compiled_function(
                                                func_id.clone(),
                                                compiled_fn,
                                                max_slot,
                                                name_guards,
                                            );
                                            // Mark as compiled in detector
                                            self.jit_detector.mark_compiled_internal(&func_id);
                                        }
                                        Err(e) => {
                                            if self.trace {
                                                eprintln!("[JIT] Function compilation failed: {}", e);
                                            }
                                        }
                                    }
                                }
                            } else if self.trace {
                                eprintln!("[JIT] Function trace not compilable");
                            }
                        }

                        self.jit_tracing = false;
                        self.jit_tracing_func_id = None;
                    }

                    // Sync globals back to caller's local slots ONLY for module frame.
                    // Function frames use LoadScope (resolves from closure chain),
                    // so they don't need sync. Syncing to function frames would
                    // overwrite locals with stale ctx_globals values.
                    if self.frame_stack.is_empty() {
                        let updates: Vec<(usize, Value)> = if let Some(ref code) = frame.code {
                            code.slotmap
                                .iter()
                                .filter_map(|(name, &slot_idx)| self.globals.get(name.as_str()).map(|&v| (slot_idx, v)))
                                .collect()
                        } else {
                            Vec::new()
                        };
                        for (slot_idx, v) in updates {
                            // The slot takes its OWN ref of the map's value and
                            // releases the previous local (set_local overwrites
                            // without releasing) -- wip/GLOBALS_OWNERSHIP.md.
                            let old = frame.get_local(slot_idx);
                            decref_discard(&self.struct_registry, old);
                            v.clone_refcount();
                            frame.set_local(slot_idx, v);
                        }
                    }
                    continue;
                }

                OpCode::MakeFunction => {
                    // Pop code object and create VMFunction. to_pyobject clones
                    // its own Python ref: release the popped const handle, or
                    // its refcount grows by one per closure creation.
                    let code_val = frame.pop();
                    let code_obj = code_val.to_pyobject(py);
                    decref_pyobj(code_val);

                    // Build native captured HashMap (no Python boundary crossing)
                    let mut captured: IndexMap<String, Value> = IndexMap::new();
                    if let Some(ref code) = frame.code {
                        for (name, &slot_idx) in &code.slotmap {
                            // Module-level vars are reached live via the parent chain, not
                            // frozen. But a slot in a nested function frame is a real local
                            // (param/local) that shadows any global homonym, so it must be
                            // captured; only suppress capture at the top-level frame, where
                            // slots coincide with module globals.
                            if frame.closure_scope.is_none() && self.globals.contains_key(name.as_str()) {
                                continue;
                            }
                            let val = frame.get_local(slot_idx);
                            if !val.is_nil() && !val.is_invalid() {
                                // The captured map OWNS its entries
                                // (wip/GLOBALS_OWNERSHIP.md): without this ref,
                                // the overwrite release in set/set_with_py
                                // (mutable closures) would free the parent
                                // frame's slot ref under it. The final ref of
                                // each entry is released only if the scope is
                                // drained -- a bounded pin per closure, traced
                                // in the ownership map.
                                val.clone_refcount();
                                captured.insert(name.clone(), val);
                            }
                        }
                    }
                    portabilize_struct_values(py, &mut captured, &self.struct_registry);

                    // Build parent: native chain or PyGlobals terminal
                    let parent = host.build_closure_parent(py, frame.closure_scope.as_ref());

                    let native_scope = NativeClosureScope::new(captured, parent);
                    let context_for_func = host.context().as_ref().map(|c| c.clone_ref(py));

                    let code_py: Py<PyCodeObject> = code_obj
                        .bind(py)
                        .cast::<PyCodeObject>()
                        .map_err(|e| VMError::TypeError(format!("Expected CodeObject: {e}")))?
                        .clone()
                        .unbind();

                    let code = Arc::clone(&code_py.borrow(py).inner);
                    let idx = self.func_table.insert(FuncSlot {
                        code,
                        closure: Some(native_scope),
                        code_py,
                        context: context_for_func,
                    });

                    // arg = name_idx + 1: bind the function under its own name
                    // in its closure so recursive references resolve (let-rec)
                    if instr.arg > 0 {
                        let self_name = frame
                            .code
                            .as_ref()
                            .and_then(|c| c.names.get((instr.arg - 1) as usize))
                            .cloned();
                        if let Some(name) = self_name {
                            if let Some(slot) = self.func_table.get(idx) {
                                if let Some(ref closure) = slot.closure {
                                    closure.insert_captured(&name, Value::from_vmfunc(idx));
                                }
                            }
                        }
                    }

                    frame.push(Value::from_vmfunc(idx));
                }

                OpCode::PatchClosure => {
                    // Letrec group patch: closure_of(target)[names[arg]] = value.
                    // No-op when target is not a function (e.g. a sibling slot
                    // not populated yet because its branch never ran).
                    let value = frame.pop();
                    let target = frame.pop();
                    let mut consumed = false;
                    if target.is_vmfunc() && value.is_vmfunc() {
                        let name = frame
                            .code
                            .as_ref()
                            .and_then(|c| c.names.get(instr.arg as usize))
                            .cloned();
                        if let Some(name) = name {
                            if let Some(slot) = self.func_table.get(target.as_vmfunc_idx()) {
                                if let Some(ref closure) = slot.closure {
                                    closure.insert_captured(&name, value);
                                    consumed = true;
                                }
                            }
                        }
                    }
                    if !consumed {
                        decref_discard(&self.struct_registry, value);
                    }
                    decref_discard(&self.struct_registry, target);
                }

                // --- Collection literals ---
                OpCode::BuildList => {
                    let n = instr.arg as usize;
                    let stack_len = frame.stack.len();
                    let start = stack_len - n;
                    let items: Vec<Py<PyAny>> = frame.stack[start..].iter().map(|v| v.to_pyobject(py)).collect();
                    // Voie A: `items` clone_ref'd each element; release the owned
                    // element refs (pyobj handle, BigInt/Complex Arc, struct slot)
                    // before truncating them off the stack.
                    release_operands(&self.struct_registry, &frame.stack[start..]);
                    frame.stack.truncate(start);
                    let list = PyList::new(py, items).unwrap();
                    frame.push(Value::from_owned_pyobject(list.unbind().into_any()));
                }

                OpCode::BuildTuple => {
                    let n = instr.arg as usize;
                    let stack_len = frame.stack.len();
                    let start = stack_len - n;
                    let items: Vec<Py<PyAny>> = frame.stack[start..].iter().map(|v| v.to_pyobject(py)).collect();
                    // Voie A: `items` clone_ref'd each element; release the owned
                    // element refs (pyobj handle, BigInt/Complex Arc, struct slot)
                    // before truncating them off the stack.
                    release_operands(&self.struct_registry, &frame.stack[start..]);
                    frame.stack.truncate(start);
                    let tuple = PyTuple::new(py, items).unwrap();
                    frame.push(Value::from_owned_pyobject(tuple.unbind().into_any()));
                }

                OpCode::BuildSet => {
                    let n = instr.arg as usize;
                    let stack_len = frame.stack.len();
                    let start = stack_len - n;
                    let items: Vec<Py<PyAny>> = frame.stack[start..].iter().map(|v| v.to_pyobject(py)).collect();
                    // Voie A: `items` clone_ref'd each element; release the owned
                    // element refs (pyobj handle, BigInt/Complex Arc, struct slot)
                    // before truncating them off the stack.
                    release_operands(&self.struct_registry, &frame.stack[start..]);
                    frame.stack.truncate(start);
                    let set_type = match &self.cached_set_type {
                        Some(cached) => cached.bind(py).clone(),
                        None => {
                            let st = py.import("builtins").to_vm(py)?.getattr("set").to_vm(py)?;
                            self.cached_set_type = Some(st.unbind());
                            self.cached_set_type.as_ref().unwrap().bind(py).clone()
                        }
                    };
                    let py_list = PyList::new(py, items).to_vm(py)?;
                    let py_set = set_type.call1((py_list,)).to_vm(py)?;
                    frame.push(Value::from_owned_pyobject(py_set.unbind()));
                }

                OpCode::BuildDict => {
                    let n = instr.arg as usize;
                    let dict = PyDict::new(py);
                    for _ in 0..n {
                        let value_v = frame.pop();
                        let key_v = frame.pop();
                        let value = value_v.to_pyobject(py);
                        let key = key_v.to_pyobject(py);
                        // Voie A: release the owned key/value refs (the dict now
                        // holds its own refs via the clone_ref above).
                        decref_discard(&self.struct_registry, value_v);
                        decref_discard(&self.struct_registry, key_v);
                        // Propagate hashing errors (unhashable keys, failing op_hash)
                        // so dict literals do not silently drop entries.
                        dict.set_item(key, value).to_vm(py)?;
                    }
                    frame.push(Value::from_owned_pyobject(dict.unbind().into_any()));
                }

                OpCode::BuildSlice => {
                    // Build slice(start, stop[, step])
                    const SLICE_ARGS_MIN: usize = 2;
                    const SLICE_ARGS_MAX: usize = 3;
                    let n = instr.arg as usize;
                    let stack_len = frame.stack.len();
                    let start = stack_len - n;
                    let items: Vec<Py<PyAny>> = frame.stack[start..].iter().map(|v| v.to_pyobject(py)).collect();
                    // Voie A: `items` clone_ref'd each element; release the owned
                    // element refs (pyobj handle, BigInt/Complex Arc, struct slot)
                    // before truncating them off the stack.
                    release_operands(&self.struct_registry, &frame.stack[start..]);
                    frame.stack.truncate(start);

                    // Create slice object
                    let slice_type = py.get_type::<pyo3::types::PySlice>();
                    let slice = if n == SLICE_ARGS_MIN {
                        slice_type.call1((&items[0], &items[1])).to_vm(py)?
                    } else if n == SLICE_ARGS_MAX {
                        slice_type.call1((&items[0], &items[1], &items[2])).to_vm(py)?
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "BUILD_SLICE expects 2 or 3 args, got {}",
                            n
                        )));
                    };
                    frame.push(Value::from_owned_pyobject(slice.unbind()));
                }

                // --- Attribute/item access ---
                OpCode::GetAttr => {
                    let attr_name = get_name(code, instr.arg)?;
                    let obj = frame.pop();

                    if let Some(idx) = obj.as_struct_instance_idx() {
                        let (type_id, fields) = self
                            .struct_registry
                            .with_instance(idx, |i| (i.type_id, i.fields.clone()))
                            .unwrap();
                        let ty = self.struct_registry.get_type(type_id).unwrap();
                        match ty.field_index(attr_name) {
                            Some(field_idx) => {
                                let val = fields[field_idx];
                                // ty borrow ends here (NLL: val and type_id are Copy)
                                // The field becomes an independent reference: bump its
                                // refcount before the receiver is dropped below, or the
                                // cascade decref of a temporary instance would free a
                                // PyObject field still live on the stack (UAF on
                                // `S([1,2,3]).items`). Struct uses the local registry.
                                val.clone_refcount_bigint();
                                val.clone_refcount_pyobj();
                                if val.is_struct_instance() {
                                    self.struct_registry.incref(val.as_struct_instance_idx().unwrap());
                                }
                                frame.push(val);
                                decref_discard(&self.struct_registry, obj);
                            }
                            None => {
                                // Look up method in StructType
                                let ty = self.struct_registry.get_type(type_id).unwrap();
                                if let Some(func) = ty.methods.get(attr_name.as_str()) {
                                    let func_clone = func.clone_ref(py);
                                    // ty borrow ends (NLL: func_clone is owned)
                                    let proxy = obj.to_pyobject(py);
                                    let bound = Py::new(
                                        py,
                                        crate::core::BoundCatnipMethod {
                                            func: func_clone,
                                            instance: proxy,
                                            super_source_type: None,
                                            native_instance_idx: Some(idx),
                                            native_registry_id: self.struct_registry.id(),
                                        },
                                    )
                                    .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                    frame.push(Value::from_owned_pyobject(bound.into_any()));
                                    decref_discard(&self.struct_registry, obj);
                                } else if let Some(func) = ty.static_methods.get(attr_name.as_str()) {
                                    let func_clone = func.clone_ref(py);
                                    frame.push(Value::from_owned_pyobject(func_clone));
                                    decref_discard(&self.struct_registry, obj);
                                } else {
                                    let msg = attr_error_msg(ty, attr_name);
                                    decref_discard(&self.struct_registry, obj);
                                    return Err(VMError::RuntimeError(msg));
                                }
                            }
                        }
                    } else {
                        let py_obj = obj.to_pyobject(py);
                        let py_bound = py_obj.bind(py);
                        // Check if this is a struct marker type (for static methods)
                        let ptr = py_bound.as_ptr() as usize;
                        let result = if let Some(&type_id) = self.struct_type_map.get(&ptr) {
                            let ty = self.struct_registry.get_type(type_id).unwrap();
                            match ty.static_methods.get(attr_name.as_str()) {
                                Some(func) => Value::from_pyobject(py, func.bind(py)).to_vm(py),
                                None => Err(VMError::RuntimeError(attr_error_msg(ty, attr_name))),
                            }
                        } else if let Some(&enum_type_id) = self.enum_type_map.get(&ptr) {
                            let ety = self.enum_registry.get_type(enum_type_id).unwrap();
                            match ety.variant_symbol(attr_name) {
                                Some(sym_id) => Ok(Value::from_symbol(sym_id)),
                                None => Err(VMError::RuntimeError(format!(
                                    "enum '{}' has no variant '{}'",
                                    ety.name, attr_name
                                ))),
                            }
                        } else if let Ok(etype) = py_bound.cast::<CatnipEnumType>() {
                            // CatnipEnumType from an imported module that isn't
                            // yet registered in our enum_type_map: lazily
                            // register it, then resolve the variant.
                            let et = etype.borrow();
                            let type_id =
                                self.enum_registry
                                    .register(&et.name, &et.variant_names, &mut self.symbol_table);
                            self.enum_type_map.insert(ptr, type_id);
                            let ety = self.enum_registry.get_type(type_id).unwrap();
                            match ety.variant_symbol(attr_name) {
                                Some(sym_id) => Ok(Value::from_symbol(sym_id)),
                                None => Err(VMError::RuntimeError(format!(
                                    "enum '{}' has no variant '{}'",
                                    ety.name, attr_name
                                ))),
                            }
                        } else {
                            host.obj_getattr(py, obj, attr_name)
                        };
                        // Voie A: release the receiver ref on every path, the
                        // failing accesses included (every sub-path captured
                        // py_obj; obj_getattr already used obj).
                        decref_discard(&self.struct_registry, obj);
                        frame.push(result?);
                    }
                }

                OpCode::SetAttr => {
                    let attr_name = get_name(code, instr.arg)?;
                    let value = frame.pop();
                    let obj = frame.pop();

                    if let Some(idx) = obj.as_struct_instance_idx() {
                        let type_id = self.struct_registry.with_instance(idx, |i| i.type_id).unwrap();
                        // Refuse mutation once the instance has been hashed,
                        // otherwise dict/set lookups would silently break.
                        if self.struct_registry.is_frozen(idx) {
                            let ty_name = self.struct_registry.get_type(type_id).unwrap().name.clone();
                            decref_discard(&self.struct_registry, value);
                            decref_discard(&self.struct_registry, obj);
                            return Err(VMError::RuntimeError(format!(
                                "cannot mutate '{ty_name}' after it has been hashed (used as dict/set key)"
                            )));
                        }
                        let ty = self.struct_registry.get_type(type_id).unwrap();
                        match ty.field_index(attr_name) {
                            Some(field_idx) => {
                                // ty borrow ends (NLL: field_idx is Copy). Return the
                                // displaced value from the closure and release it after
                                // the borrow drops -- a pyobj __del__ must not run under
                                // the with_instance_mut borrow.
                                let old = self
                                    .struct_registry
                                    .with_instance_mut(idx, |inst| {
                                        let old = inst.fields[field_idx];
                                        inst.fields[field_idx] = value;
                                        old
                                    })
                                    .unwrap();
                                decref_discard(&self.struct_registry, old);
                                decref_discard(&self.struct_registry, obj);
                            }
                            None => {
                                // The popped value was never transferred into a
                                // field: release it with the receiver.
                                let msg = attr_error_msg(ty, attr_name);
                                decref_discard(&self.struct_registry, value);
                                decref_discard(&self.struct_registry, obj);
                                return Err(VMError::RuntimeError(msg));
                            }
                        }
                    } else {
                        // obj_setattr borrows (to_pyobject clones): release the
                        // popped receiver and value on every path.
                        let r = host.obj_setattr(py, obj, attr_name, value);
                        decref_discard(&self.struct_registry, obj);
                        decref_discard(&self.struct_registry, value);
                        r?;
                    }
                }

                OpCode::GetItem => {
                    if instr.arg == 1 {
                        // Fused slice mode: stack has [obj, start, stop, step]
                        let step = frame.pop();
                        let stop = frame.pop();
                        let start = frame.pop();
                        let obj = frame.pop();
                        let slice_type = py.get_type::<pyo3::types::PySlice>();
                        let py_start = start.to_pyobject(py);
                        let py_stop = stop.to_pyobject(py);
                        let py_step = step.to_pyobject(py);
                        let slice = slice_type.call1((&py_start, &py_stop, &py_step)).to_vm(py)?;
                        let index = Value::from_owned_pyobject(slice.unbind());
                        let value = host.obj_getitem(py, obj, index);
                        // Voie A: release receiver, slice components (BigInt
                        // bounds included), and slice handle -- errors too.
                        release_operands(&self.struct_registry, &[obj, start, stop, step]);
                        index.decref();
                        frame.push(value?);
                    } else {
                        let index = frame.pop();
                        let obj = frame.pop();
                        let value = host.obj_getitem(py, obj, index);
                        // Voie A: release receiver and index refs, errors too.
                        release_operands(&self.struct_registry, &[obj, index]);
                        frame.push(value?);
                    }
                }

                OpCode::SetItem => {
                    let value = frame.pop();
                    let index = frame.pop();
                    let obj = frame.pop();
                    let r = host.obj_setitem(py, obj, index, value);
                    // Voie A: container holds its own refs; release the popped
                    // refs (pyobj, BigInt/Complex, struct) on every path.
                    release_operands(&self.struct_registry, &[obj, index, value]);
                    r?;
                }

                // --- Block/scope ---
                OpCode::PushBlock => {
                    let is_module_block = instr.arg & 0x8000_0000 != 0;
                    let slot_start = (instr.arg & 0x7FFF_FFFF) as usize;
                    frame.push_block(slot_start);
                    // Snapshot pre-existing global names for cleanup at PopBlock
                    if is_module_block {
                        if let Some(ref code) = frame.code {
                            let existing: Vec<String> = code.varnames[slot_start..]
                                .iter()
                                .filter(|n| self.globals.contains_key(n.as_str()) || host.has_global(py, n.as_str()))
                                .cloned()
                                .collect();
                            self.block_globals_snapshot.push(existing);
                        }
                    }
                }

                OpCode::PopBlock => {
                    // arg=1: module-level block, clean block-local names from globals
                    if instr.arg == 1 {
                        if let Some(&(slot_start, _)) = frame.block_stack.last() {
                            if let Some(ref code) = frame.code {
                                // Pop the globals snapshot saved at PushBlock
                                let pre_existing = self.block_globals_snapshot.pop();
                                for slot in slot_start..code.varnames.len() {
                                    let name = &code.varnames[slot];
                                    // Only clean names not in globals before the block
                                    let existed_before =
                                        pre_existing.as_ref().is_some_and(|names| names.contains(name));
                                    if !existed_before {
                                        if let Some(old) = self.globals.swap_remove(name) {
                                            decref_discard(&self.struct_registry, old);
                                        }
                                        if let Some(old) = host.delete_global(py, name.as_str())? {
                                            decref_discard(&self.struct_registry, old);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    frame.pop_block(&self.struct_registry);
                }

                // --- Control signals ---
                OpCode::Break => {
                    let err = VMError::Break;
                    if self.try_unwind_to_handler(frame, &err) {
                        continue;
                    }
                    return Err(err);
                }

                OpCode::Continue => {
                    let err = VMError::Continue;
                    if self.try_unwind_to_handler(frame, &err) {
                        continue;
                    }
                    return Err(err);
                }

                // --- Broadcasting ---
                OpCode::Broadcast => {
                    // Decode flags: bit 0 = is_filter, bit 1 = has_operand, bits 2-3 = ND type
                    const FLAG_FILTER: u32 = 1;
                    const FLAG_OPERAND: u32 = 2;
                    const FLAG_ND_RECURSION: u32 = 4;
                    const FLAG_ND_MAP: u32 = 8;
                    let flags = instr.arg;
                    let is_filter = (flags & FLAG_FILTER) != 0;
                    let has_operand = (flags & FLAG_OPERAND) != 0;
                    let is_nd_recursion = (flags & FLAG_ND_RECURSION) != 0;
                    let is_nd_map = (flags & FLAG_ND_MAP) != 0;

                    // Handle ND operations specially (delegated to host for parallelism)
                    // Ownership: the host helpers borrow (to_pyobject clones);
                    // the popped operands are released here on every path.
                    if is_nd_recursion || is_nd_map {
                        let lambda_val = frame.pop();
                        let target_val = frame.pop();
                        let lambda_py = lambda_val.to_pyobject(py);
                        let target_py = target_val.to_pyobject(py);
                        let target_bound = target_py.bind(py);
                        let lambda_bound = lambda_py.bind(py);

                        let result_py = if is_nd_recursion {
                            host.broadcast_nd_recursion(py, target_bound, lambda_bound)
                        } else {
                            host.broadcast_nd_map(py, target_bound, lambda_bound)
                        };
                        release_operands(&self.struct_registry, &[lambda_val, target_val]);

                        let value = Value::from_pyobject(py, result_py?.bind(py)).to_vm(py)?;
                        frame.push(value);
                    } else {
                        // Regular broadcast: pop operand (if present), operator, target
                        let operand_val = if has_operand { Some(frame.pop()) } else { None };
                        let operator_val = frame.pop();
                        let target_val = frame.pop();

                        // FAST PATH: in-VM map of a VMFunc callback (recursive,
                        // mirroring broadcast_map for nested lists/tuples) --
                        // avoids the fresh child VM + clone_from_parent per
                        // element (the O(N^2) source), deep-copies struct
                        // elements for (5,1) isolation at every depth, and needs
                        // no transplant/materialize. Falls back for filter,
                        // operand forms, non-VMFunc operators, or while the JIT
                        // is recording (the sub-dispatch would pollute the trace).
                        if !is_filter && !has_operand && operator_val.is_vmfunc() && !self.jit_recorder.is_recording() {
                            // Release operands UNCONDITIONALLY, then propagate a
                            // callback error -- a bare `?` here would skip the
                            // release and leak the target list (+ pin its struct
                            // elements) on every erroring broadcast, unbounded on
                            // a long-lived pipeline that catches the error. Mirrors
                            // the slow path below.
                            let result = self.try_broadcast_map_in_vm(py, host, operator_val, &target_val);
                            release_operands(&self.struct_registry, &[operator_val, target_val]);
                            frame.push(result?);
                        } else {
                            let operand = operand_val.map(|v| v.to_pyobject(py));
                            let operator = operator_val.to_pyobject(py);
                            let target = target_val.to_pyobject(py);
                            let target_bound = target.bind(py);
                            let operator_bound = operator.bind(py);
                            let result = host.apply_broadcast(
                                py,
                                target_bound,
                                operator_bound,
                                operand.as_ref().map(|o| o.bind(py)),
                                is_filter,
                            );
                            if let Some(v) = operand_val {
                                decref_discard(&self.struct_registry, v);
                            }
                            release_operands(&self.struct_registry, &[operator_val, target_val]);
                            let value = Value::from_pyobject(py, result?.bind(py)).to_vm(py)?;
                            frame.push(value);
                        }
                    }
                }

                // --- Pattern matching ---
                OpCode::MatchPattern => {
                    return Err(VMError::RuntimeError(errors::ERR_LEGACY_MATCH.into()));
                }

                OpCode::MatchPatternVM => {
                    // Native path: pre-compiled VMPattern, no Python boundary crossing
                    let pat_idx = instr.arg as usize;
                    let value = frame.pop();
                    // Release a previous arm's bindings before overwriting them.
                    release_match_bindings(&self.struct_registry, frame);
                    let pattern = frame.code.as_ref().and_then(|c| c.patterns.get(pat_idx)).cloned();
                    // The matcher borrows the subject; every binding owns its
                    // own ref (Var clones). Locally-owned intermediates (tuple
                    // items, a failed arm's partial bindings) come back through
                    // the spill. Both the subject copy (DupTop) and the spill
                    // are released unconditionally -- match, mismatch or error.
                    let mut spill: Vec<Value> = Vec::new();
                    let matched = match pattern {
                        Some(ref pat) => vm_match_pattern(
                            py,
                            pat,
                            value,
                            &self.struct_registry,
                            &self.globals,
                            host,
                            &frame.closure_scope,
                            &mut spill,
                        ),
                        None => Ok(None),
                    };
                    for v in spill {
                        decref_discard(&self.struct_registry, v);
                    }
                    decref_discard(&self.struct_registry, value);
                    match matched.to_vm(py)? {
                        Some(bindings) => {
                            frame.match_bindings = Some(bindings);
                            frame.push(Value::TRUE);
                        }
                        None => frame.push(Value::NIL),
                    }
                }

                OpCode::MatchAssignPatternVM => {
                    // Strict assignment-pattern matching:
                    // on mismatch, raise unpacking error (type/runtime) with details.
                    let pat_idx = instr.arg as usize;
                    let value = frame.pop();
                    release_match_bindings(&self.struct_registry, frame);
                    let pattern = frame.code.as_ref().and_then(|c| c.patterns.get(pat_idx)).cloned();
                    // Same ownership contract as MatchPatternVM: the matcher
                    // borrows the subject (bindings clone), owned locals come
                    // back through the spill, and the DupTop copy is released
                    // unconditionally -- including on the unpacking-error path.
                    let mut spill: Vec<Value> = Vec::new();
                    let matched = match pattern {
                        Some(ref pat) => vm_match_assign_pattern(py, pat, value, &self.struct_registry, &mut spill),
                        None => Err(VMError::RuntimeError("Invalid assignment pattern index".to_string())),
                    };
                    for v in spill {
                        decref_discard(&self.struct_registry, v);
                    }
                    decref_discard(&self.struct_registry, value);
                    frame.match_bindings = Some(matched?);
                    frame.push(Value::TRUE);
                }

                OpCode::BindMatch => {
                    // clone (not take): a guarded arm binds twice from the same
                    // match_bindings -- once inside the guard's push_block/pop_block,
                    // once for the body (see compile_match). Each binding takes its
                    // OWN refcount so the slot can be decref'd independently; the
                    // owned refs still held by match_bindings are released when it is
                    // overwritten (next MatchPatternVM) or the frame is torn down
                    // (release_match_bindings). Cloning without that release was the
                    // leak: one ref per capture-binding match.
                    if let Some(bindings) = frame.match_bindings.clone() {
                        frame.pop(); // pop the sentinel TRUE
                        for (slot, val) in bindings {
                            let old = frame.get_local(slot);
                            decref_discard(&self.struct_registry, old);
                            val.clone_refcount();
                            frame.set_local(slot, val);
                        }
                    }
                }

                OpCode::JumpIfNone => {
                    let value = frame.pop();
                    if value.is_nil() {
                        frame.ip = instr.arg as usize;
                    }
                }

                OpCode::JumpIfNotNoneOrPop => {
                    let cond = frame.peek();
                    if !cond.is_nil() {
                        frame.ip = instr.arg as usize;
                    } else {
                        frame.pop();
                    }
                }

                OpCode::ToBool => {
                    let value = frame.pop();
                    frame.push(Value::from_bool(value.is_truthy()));
                    // is_truthy borrows; the popped operand (an and/or branch
                    // value) leaked one ref per heap condition without this.
                    decref_discard(&self.struct_registry, value);
                }

                // Typed-parameter boundary (TH2-B step 0b): check + numeric-tower
                // coercion so an annotated param IS its declared type before any
                // specialized opcode reads it. Kept symmetric with the PureVM.
                // On a type error the popped operand is dropped, so release its
                // refcount before unwinding.
                OpCode::CheckType => {
                    let value = frame.pop();
                    match boundary_coerce(py, value, instr.arg as u8) {
                        Ok(coerced) => frame.push(coerced),
                        Err(e) => {
                            decref_discard(&self.struct_registry, value);
                            return Err(e);
                        }
                    }
                }

                // Nominal-type boundary: an annotated param must be an instance of
                // its declared struct/enum/union type, with subtyping (MRO + traits).
                // No coercion; an unknown type name at runtime is inert (no-op). On a
                // type error the popped value is dropped, so release its refcount
                // before unwinding. Mirrors the PureVM CheckNominal arm.
                OpCode::CheckNominal => {
                    let name = get_name(code, instr.arg)?;
                    let value = frame.pop();
                    match self.check_nominal(py, value, name) {
                        Ok(()) => frame.push(value),
                        Err(e) => {
                            decref_discard(&self.struct_registry, value);
                            return Err(e);
                        }
                    }
                }

                // Type-union boundary (`int | str`, `Point | None`): accept if the
                // value satisfies any member (no coercion). On a type error the
                // popped value is dropped, so release its refcount before unwinding.
                OpCode::CheckUnion => {
                    let members = &code.union_checks[instr.arg as usize];
                    let value = frame.pop();
                    match self.check_union(py, value, members) {
                        Ok(()) => frame.push(value),
                        Err(e) => {
                            decref_discard(&self.struct_registry, value);
                            return Err(e);
                        }
                    }
                }

                // Composite boundary (`list[T]`, `dict[K, V]`): check the container
                // tag (params carried, not yet enforced). On a type error the popped
                // value is dropped, so release its refcount before unwinding.
                OpCode::CheckComposite => {
                    let spec = &code.composite_checks[instr.arg as usize];
                    let value = frame.pop();
                    match self.check_composite(py, value, spec) {
                        Ok(()) => frame.push(value),
                        Err(e) => {
                            decref_discard(&self.struct_registry, value);
                            return Err(e);
                        }
                    }
                }

                // Generic-nominal boundary (`Option[int]`): union membership + the
                // parametric payload substitution. On a type error the popped value
                // is dropped, so release its refcount before unwinding.
                OpCode::CheckGeneric => {
                    let spec = &code.generic_checks[instr.arg as usize];
                    let value = frame.pop();
                    match self.check_generic(py, value, spec) {
                        Ok(()) => frame.push(value),
                        Err(e) => {
                            decref_discard(&self.struct_registry, value);
                            return Err(e);
                        }
                    }
                }

                // Function-type boundary (`(int) -> int`, FT3): callable +
                // declared-arity acceptance; `instr.arg` IS the arity.
                OpCode::CheckCallable => {
                    let value = frame.pop();
                    match self.check_callable(py, value, instr.arg) {
                        Ok(()) => frame.push(value),
                        Err(e) => {
                            decref_discard(&self.struct_registry, value);
                            return Err(e);
                        }
                    }
                }

                // --- Process ---
                OpCode::Exit => {
                    // arg encodes: 0 = no argument (default 0), 1 = pop code from stack
                    let code = if instr.arg == 1 {
                        let v = frame.pop();
                        v.as_int().map(|n| n as i32).unwrap_or(1)
                    } else {
                        0
                    };
                    return Err(VMError::Exit(code));
                }

                // --- Unpacking ---
                OpCode::UnpackSequence => {
                    let n = instr.arg as usize;
                    let seq = frame.pop();
                    let py_seq = seq.to_pyobject(py);
                    let py_seq_bound = py_seq.bind(py);

                    // Convert to list to get items. The popped subject is owned
                    // by this op and consumed by nothing below (py_seq holds its
                    // own Python ref): release it on every exit.
                    let items: Vec<Py<PyAny>> = match py_seq_bound.try_iter() {
                        Ok(iter) => match iter.map(|item| item.map(|i| i.unbind())).collect::<PyResult<Vec<_>>>() {
                            Ok(v) => v,
                            Err(e) => {
                                decref_discard(&self.struct_registry, seq);
                                return Err(e).to_vm(py);
                            }
                        },
                        Err(_) => {
                            let ty = py_seq_bound
                                .get_type()
                                .name()
                                .map(|n| n.to_string())
                                .unwrap_or_else(|_| "value".to_string());
                            decref_discard(&self.struct_registry, seq);
                            return Err(VMError::TypeError(format!("Cannot unpack non-iterable {}", ty)));
                        }
                    };
                    decref_discard(&self.struct_registry, seq);

                    if items.len() != n {
                        return Err(VMError::RuntimeError(format!(
                            "Cannot unpack {} values into {} variables",
                            items.len(),
                            n
                        )));
                    }

                    // Push items in reverse order (so first item ends on top)
                    for item in items.into_iter().rev() {
                        let val = Value::from_pyobject(py, item.bind(py)).to_vm(py)?;
                        frame.push(val);
                    }
                }

                OpCode::UnpackEx => {
                    // Extended unpacking: *rest syntax
                    // arg encodes (before << 8) | after
                    let before = ((instr.arg >> 8) & 0xFF) as usize;
                    let after = (instr.arg & 0xFF) as usize;

                    let seq = frame.pop();
                    let py_seq = seq.to_pyobject(py);
                    let py_seq_bound = py_seq.bind(py);

                    // Convert to list. Same subject ownership as UnpackSequence:
                    // released on every exit, nothing below consumes it.
                    let items: Vec<Py<PyAny>> = match py_seq_bound.try_iter() {
                        Ok(iter) => match iter.map(|item| item.map(|i| i.unbind())).collect::<PyResult<Vec<_>>>() {
                            Ok(v) => v,
                            Err(e) => {
                                decref_discard(&self.struct_registry, seq);
                                return Err(e).to_vm(py);
                            }
                        },
                        Err(_) => {
                            let ty = py_seq_bound
                                .get_type()
                                .name()
                                .map(|n| n.to_string())
                                .unwrap_or_else(|_| "value".to_string());
                            decref_discard(&self.struct_registry, seq);
                            return Err(VMError::TypeError(format!("Cannot unpack non-iterable {}", ty)));
                        }
                    };
                    decref_discard(&self.struct_registry, seq);

                    let total_fixed = before + after;
                    if items.len() < total_fixed {
                        return Err(VMError::RuntimeError(format!(
                            "Not enough values to unpack (expected at least {}, got {})",
                            total_fixed,
                            items.len()
                        )));
                    }

                    // Split: before items, middle (rest), after items
                    let rest_len = items.len() - total_fixed;
                    let before_items = &items[..before];
                    let rest_items = &items[before..before + rest_len];
                    let after_items = &items[before + rest_len..];

                    // Push in reverse order: after, rest (as list), before
                    for item in after_items.iter().rev() {
                        let val = Value::from_pyobject(py, item.bind(py)).to_vm(py)?;
                        frame.push(val);
                    }

                    // Create list for rest
                    let rest_py: Vec<Py<PyAny>> = rest_items.iter().map(|item| item.clone_ref(py)).collect();
                    let rest_list = PyList::new(py, rest_py).to_vm(py)?;
                    frame.push(Value::from_owned_pyobject(rest_list.unbind().into_any()));

                    for item in before_items.iter().rev() {
                        let val = Value::from_pyobject(py, item.bind(py)).to_vm(py)?;
                        frame.push(val);
                    }
                }

                // --- Optimized iteration ---
                OpCode::ForRangeInt => {
                    // Optimized range loop condition check
                    // Replaces: LoadLocal + LoadLocal + GE/LE + JumpIfTrue (4 opcodes -> 1)
                    use super::{
                        FOR_RANGE_JUMP_MASK, FOR_RANGE_SLOT_I_SHIFT, FOR_RANGE_SLOT_MASK, FOR_RANGE_SLOT_STOP_SHIFT,
                        FOR_RANGE_STEP_SIGN_SHIFT,
                    };

                    let slot_i = (instr.arg >> FOR_RANGE_SLOT_I_SHIFT) as usize;
                    let slot_stop = ((instr.arg >> FOR_RANGE_SLOT_STOP_SHIFT) & FOR_RANGE_SLOT_MASK) as usize;
                    let step_positive = ((instr.arg >> FOR_RANGE_STEP_SIGN_SHIFT) & 1) == 0;
                    let jump_offset = (instr.arg & FOR_RANGE_JUMP_MASK) as usize;

                    let i = frame.get_local(slot_i);
                    let stop = frame.get_local(slot_stop);

                    // Fast path: both are ints
                    let done = match (i.as_int(), stop.as_int()) {
                        (Some(i_val), Some(stop_val)) => {
                            if step_positive {
                                i_val >= stop_val
                            } else {
                                i_val <= stop_val
                            }
                        }
                        _ => true, // Fallback: treat as done
                    };

                    if done {
                        // If we were tracing this loop, finish tracing
                        if self.jit_tracing && self.jit_tracing_offset == frame.ip - 1 {
                            let ip = frame.ip - 1;
                            self.jit_recorder
                                .record_opcode(OpCode::ForRangeInt, instr.arg, true, ip);
                            let trace = self.jit_recorder.stop();
                            self.jit_tracing = false;
                            self.jit_compile_finished_trace(trace, "for-range");
                        }
                        frame.ip += jump_offset;
                    } else if self.jit_enabled {
                        let loop_offset = frame.ip - 1;

                        // Check if we have compiled code for this loop
                        if !self.jit_tracing {
                            // Skip JIT if guard just failed for this loop
                            if self.jit_guard_failed == Some(loop_offset) {
                                self.jit_guard_failed = None;
                                // Fall through to interpreter
                            } else if self.jit_has_compiled(loop_offset) {
                                if let Some(ret) = self.try_enter_jit_loop(py, frame, host, code, loop_offset)? {
                                    // ret = 0: loop completed normally
                                    // ret = -1: guard failure (side exit)
                                    let guard_failed = ret == -1;

                                    if self.trace {
                                        eprintln!(
                                            "[JIT] Executed compiled trace for loop at {} (guard_failed={})",
                                            loop_offset, guard_failed
                                        );
                                    }
                                    if guard_failed {
                                        // Guard failed: reset IP to ForRangeInt to re-check condition
                                        // Also set flag to skip JIT on next iteration
                                        frame.ip = loop_offset;
                                        self.jit_guard_failed = Some(loop_offset);
                                    } else {
                                        // Loop completed normally, skip to end of loop
                                        frame.ip += jump_offset;
                                    }
                                    continue;
                                }
                                // If guards didn't pass, fall through to interpreter
                            }
                        }

                        // If we're tracing and back at loop header, record loop back
                        if self.jit_tracing && self.jit_tracing_offset == loop_offset {
                            self.jit_record_loop_back_and_maybe_compile(frame.ip - 1, "for-range");
                        } else if self.jit_tracing && self.jit_tracing_offset != loop_offset {
                            // Tracing a different loop - nested loop encountered
                            // Record the nested ForRangeInt as part of outer trace
                            let ip = frame.ip - 1;
                            self.jit_recorder
                                .record_opcode(OpCode::ForRangeInt, instr.arg, true, ip);
                        } else if !self.jit_tracing {
                            // Check if we have a pending trace for this loop
                            if self.jit_pending_trace == Some(loop_offset) {
                                // Start tracing now (beginning of a full iteration)
                                self.jit_pending_trace = None;

                                let num_locals = frame.locals.len();
                                self.jit_recorder.start(loop_offset, num_locals);

                                // Extract loop bounds from ForRangeInt arg for Jump classification
                                let jump_offset = (instr.arg & 0x7FFF) as usize;
                                let loop_start = frame.ip - 1;
                                let loop_end = loop_start + jump_offset;
                                self.jit_recorder.set_loop_bounds(loop_start, loop_end);

                                self.jit_tracing = true;
                                // Reset call-nesting depth (see the while-loop start).
                                self.jit_recursive_depth = 0;
                                self.jit_tracing_offset = loop_offset;

                                // Record ForRangeInt as FIRST op
                                let ip = frame.ip - 1;
                                self.jit_recorder
                                    .record_opcode(OpCode::ForRangeInt, instr.arg, true, ip);

                                if self.trace {
                                    eprintln!(
                                        "[JIT] Starting trace at {} (bounds: {} - {})",
                                        loop_offset, loop_start, loop_end
                                    );
                                }
                            } else {
                                // Warm-start: check trace cache on first encounter
                                self.jit_warm_start(loop_offset, "for-range");

                                if self.jit_detector.record_loop_header(loop_offset) {
                                    // Loop just became hot - try cache first
                                    if self.jit_compile_from_cache(loop_offset) {
                                        // Don't schedule tracing, compiled code will be picked up next iteration
                                        if self.trace {
                                            eprintln!("[JIT] ForRange loop at {} compiled from cache", loop_offset);
                                        }
                                    } else {
                                        // Cache miss - schedule tracing for next iteration
                                        self.jit_pending_trace = Some(loop_offset);
                                        if self.trace {
                                            eprintln!(
                                                "[JIT] Hot loop detected at {}, will trace next iteration",
                                                loop_offset
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                OpCode::ForRangeStep => {
                    // Fused increment + backward jump for range loops
                    // arg = (slot_i << 24) | (step_i8 << 16) | jump_target
                    use super::{
                        FOR_RANGE_SLOT_I_SHIFT, FOR_RANGE_STEP_BYTE_MASK, FOR_RANGE_STEP_JUMP_MASK,
                        FOR_RANGE_STEP_SHIFT,
                    };
                    let slot_i = (instr.arg >> FOR_RANGE_SLOT_I_SHIFT) as usize;
                    let step = ((instr.arg >> FOR_RANGE_STEP_SHIFT) & FOR_RANGE_STEP_BYTE_MASK) as i8 as i64;
                    let jump_target = (instr.arg & FOR_RANGE_STEP_JUMP_MASK) as usize;

                    let i_val = frame.get_local(slot_i).as_int().unwrap_or(0);
                    frame.set_local(slot_i, Value::from_int(i_val + step));

                    // JIT: this replaces the backward Jump, so handle loop-back tracing
                    if self.jit_tracing && self.jit_tracing_offset == jump_target {
                        let ip = frame.ip - 1;
                        self.jit_recorder
                            .record_opcode(OpCode::ForRangeStep, instr.arg, true, ip);
                        self.jit_record_loop_back_and_maybe_compile(ip, "for-range-step");
                    } else if self.jit_tracing {
                        // Tracing a different loop - record as part of outer trace
                        let ip = frame.ip - 1;
                        self.jit_recorder
                            .record_opcode(OpCode::ForRangeStep, instr.arg, true, ip);
                    }

                    frame.ip = jump_target;
                }

                // --- Special ---
                OpCode::Nop => {}
                OpCode::Breakpoint => {} // handled by debug hook above

                OpCode::TypeOf => {
                    let val = frame.pop();
                    let type_str: &str = if val.is_bool() {
                        "bool"
                    } else if val.is_int() {
                        "int"
                    } else if val.is_float() {
                        "float"
                    } else if val.is_nil() {
                        "nil"
                    } else if val.is_symbol() {
                        // Enum variant -> declaring enum type name. Union nullary
                        // variant -> declaring union name parsed from the qualified
                        // symbol "Union.variant": unions live in the symbol table,
                        // not the enum registry (mirrors the pure VM's TypeOf).
                        let sym_idx = val.as_symbol().unwrap();
                        if let Some((type_id, _)) = self.enum_registry.lookup_symbol(sym_idx) {
                            &self.enum_registry.get_type(type_id).unwrap().name
                        } else if let Some((owner, _)) =
                            self.symbol_table.resolve(sym_idx).and_then(|s| s.split_once('.'))
                        {
                            owner
                        } else {
                            "symbol"
                        }
                    } else if val.is_bigint() {
                        "int"
                    } else if val.is_vmfunc() {
                        "function"
                    } else if val.is_struct_instance() {
                        let idx = val.as_struct_instance_idx().unwrap();
                        let name = self
                            .struct_registry
                            .with_instance(idx, |inst| inst.type_id)
                            .and_then(|type_id| self.struct_registry.get_type(type_id).map(|ty| ty.name.clone()))
                            .unwrap_or_else(|| "object".to_string());
                        let py_str = PyString::intern(py, &name);
                        // The popped operand owns its ref on every exit of this arm.
                        decref_discard(&self.struct_registry, val);
                        frame.push(Value::from_pyobject(py, py_str.as_any()).to_vm(py)?);
                        continue;
                    } else if val.is_pyobj() {
                        let obj = val.to_pyobject(py);
                        let obj_bound = obj.bind(py);
                        if obj_bound.is_instance_of::<pyo3::types::PyBool>() {
                            "bool"
                        } else if obj_bound.is_instance_of::<pyo3::types::PyInt>() {
                            "int"
                        } else if obj_bound.is_instance_of::<pyo3::types::PyFloat>() {
                            "float"
                        } else if obj_bound.is_instance_of::<PyString>() {
                            "string"
                        } else if obj_bound.is_instance_of::<PyList>() {
                            "list"
                        } else if obj_bound.is_instance_of::<PyTuple>() {
                            "tuple"
                        } else if obj_bound.is_instance_of::<PyDict>() {
                            "dict"
                        } else if obj_bound.is_instance_of::<pyo3::types::PySet>()
                            || obj_bound.is_instance_of::<pyo3::types::PyFrozenSet>()
                        {
                            "set"
                        } else if obj_bound.is_none() {
                            "nil"
                        } else if let Ok(proxy) = obj_bound.cast::<super::structs::CatnipStructProxy>() {
                            let name = proxy.borrow().type_name.clone();
                            let py_str = PyString::intern(py, &name);
                            decref_discard(&self.struct_registry, val);
                            frame.push(Value::from_pyobject(py, py_str.as_any()).to_vm(py)?);
                            continue;
                        } else if obj_bound.is_callable() {
                            "function"
                        } else {
                            let class_name: String = obj_bound
                                .get_type()
                                .qualname()
                                .and_then(|n| n.extract())
                                .unwrap_or_else(|_| "object".to_string());
                            // Catnip convention: lowercase type names
                            let catnip_name = class_name.to_ascii_lowercase();
                            let py_str = PyString::new(py, &catnip_name);
                            decref_discard(&self.struct_registry, val);
                            frame.push(Value::from_pyobject(py, py_str.as_any()).to_vm(py)?);
                            continue;
                        }
                    } else {
                        "object"
                    };
                    let py_str = PyString::intern(py, type_str);
                    decref_discard(&self.struct_registry, val);
                    frame.push(Value::from_pyobject(py, py_str.as_any()).to_vm(py)?);
                }

                OpCode::Globals => {
                    let dict = PyDict::new(py);
                    for (k, v) in self.globals.iter() {
                        dict.set_item(k, v.to_pyobject(py))
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    }
                    host.collect_globals(py, &dict)?;
                    let result =
                        Value::from_pyobject(py, dict.as_any()).map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    frame.push(result);
                }

                OpCode::Locals => {
                    let dict = PyDict::new(py);
                    // Inside function: frame.locals + code.varnames + closure captures.
                    // At module level (code.name == "<module>"), fall through to globals.
                    let is_module = code.name == "<module>" || code.name.is_empty();
                    if is_module {
                        // Module level: locals() == globals()
                        for (k, v) in self.globals.iter() {
                            dict.set_item(k, v.to_pyobject(py))
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        }
                        host.collect_globals(py, &dict)?;
                    } else {
                        for (i, name) in code.varnames.iter().enumerate() {
                            if i < frame.locals.len() {
                                let val = frame.locals[i];
                                if !val.is_nil() && val.bits() != Value::INVALID.bits() {
                                    dict.set_item(name, val.to_pyobject(py))
                                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                }
                            }
                        }
                        if let Some(ref closure) = frame.closure_scope {
                            closure
                                .dump_into_dict(py, &dict)
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        }
                    }
                    let result =
                        Value::from_pyobject(py, dict.as_any()).map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    frame.push(result);
                }

                OpCode::MakeStruct => {
                    let const_idx = instr.arg as usize;
                    let struct_info_val = code.constants[const_idx];
                    let struct_info_py = struct_info_val.to_pyobject(py);
                    let info_tuple = struct_info_py
                        .bind(py)
                        .cast::<PyTuple>()
                        .map_err(|e| VMError::RuntimeError(format!("MakeStruct: bad constant: {e}")))?;

                    let name: String = tuple_extract(info_tuple, 0)?;
                    let fields_info = tuple_get(info_tuple, 1)?;
                    let num_defaults: usize = tuple_extract(info_tuple, 2)?;
                    // Detect format: new format has implements tuple at index 3
                    // New: (name, fields, num_defaults, implements, bases_tuple_or_None, [methods])
                    // Legacy: (name, fields, num_defaults, [methods_list])
                    let mut implements_list: Vec<String> = Vec::new();
                    let mut base_names: Vec<String> = Vec::new();
                    let mut methods_idx: Option<usize> = None;

                    if info_tuple.len() > 3 {
                        let item3 = tuple_get(info_tuple, 3)?;
                        // New format: item3 is a tuple (implements list)
                        if item3.is_instance_of::<PyTuple>() {
                            for imp in item3.try_iter().map_err(|e| VMError::RuntimeError(e.to_string()))? {
                                let imp = imp.map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                implements_list
                                    .push(imp.extract().map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?);
                            }
                            // item4 = bases tuple or None
                            if info_tuple.len() > 4 {
                                let item4 = tuple_get(info_tuple, 4)?;
                                if !item4.is_none() {
                                    if item4.is_instance_of::<PyTuple>() {
                                        for b in item4.try_iter().map_err(|e| VMError::RuntimeError(e.to_string()))? {
                                            let b = b.map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                            base_names.push(
                                                b.extract().map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?,
                                            );
                                        }
                                    } else if let Ok(base) = item4.extract::<String>() {
                                        // Legacy single base string
                                        base_names.push(base);
                                    }
                                }
                            }
                            // item5 = methods
                            if info_tuple.len() > 5 {
                                methods_idx = Some(5);
                            }
                        } else if let Ok(base) = item3.extract::<String>() {
                            // Legacy: item3 is base name string
                            base_names.push(base);
                            if info_tuple.len() > 4 {
                                methods_idx = Some(4);
                            }
                        } else {
                            // Legacy: item3 is methods list
                            methods_idx = Some(3);
                        }
                    }

                    // Read defaults off the stack and parse fields
                    let native_fields = parse_field_specs(frame, &fields_info, num_defaults)?;

                    // Build methods map if present
                    let (methods_map, static_methods_map, own_abstract) = if let Some(midx) = methods_idx {
                        let methods = tuple_get(info_tuple, midx)?;
                        self.collect_method_maps(py, host, frame, &methods)?
                    } else {
                        (IndexMap::new(), IndexMap::new(), HashSet::new())
                    };

                    // Phase 1: extends(B, C, ...) merges parent fields+methods via C3 MRO.
                    let (mut merged_fields, mut merged_methods, mut merged_static, struct_mro) =
                        if !base_names.is_empty() {
                            // Compute C3 MRO (fallback to built-in exception hierarchy)
                            let struct_mro = super::mro::c3_linearize(&name, &base_names, |n| {
                                self.struct_registry
                                    .find_type_by_name(n)
                                    .map(|ty| ty.mro.clone())
                                    .or_else(|| catnip_core::exception::ExceptionKind::from_name(n).map(|k| k.mro()))
                            })
                            .map_err(VMError::RuntimeError)?;

                            // Merge fields following MRO (first-seen wins, skip self)
                            let mut seen_fields: HashSet<String> = HashSet::new();
                            let mut mro_fields: Vec<StructField> = Vec::new();
                            for mro_type_name in struct_mro.iter().skip(1) {
                                if let Some(ty) = self.struct_registry.find_type_by_name(mro_type_name) {
                                    for f in &ty.fields {
                                        if seen_fields.insert(f.name.clone()) {
                                            let f = f.clone();
                                            // Each type owns one ref per heap default
                                            // (released at registry Drop), so the
                                            // inherited copy takes its own -- mirrors
                                            // the clone_ref on inherited methods below.
                                            if f.has_default && !f.default.is_struct_instance() {
                                                f.default.clone_refcount();
                                            }
                                            mro_fields.push(f);
                                        }
                                    }
                                }
                            }

                            // Check child doesn't redefine inherited fields
                            for child_field in &native_fields {
                                if seen_fields.contains(&child_field.name) {
                                    return Err(VMError::RuntimeError(format!(
                                        "Struct '{}' redefines inherited field '{}'",
                                        name, child_field.name
                                    )));
                                }
                            }
                            mro_fields.extend(native_fields);

                            // Merge methods following MRO (first-seen wins, skip self)
                            let mut inherited_methods: IndexMap<String, Py<PyAny>> = IndexMap::new();
                            let mut inherited_static: IndexMap<String, Py<PyAny>> = IndexMap::new();
                            for mro_type_name in struct_mro.iter().skip(1) {
                                if let Some(ty) = self.struct_registry.find_type_by_name(mro_type_name) {
                                    for (k, v) in &ty.methods {
                                        if !inherited_methods.contains_key(k) {
                                            inherited_methods.insert(k.clone(), v.clone_ref(py));
                                        }
                                    }
                                    for (k, v) in &ty.static_methods {
                                        if !inherited_static.contains_key(k) {
                                            inherited_static.insert(k.clone(), v.clone_ref(py));
                                        }
                                    }
                                }
                            }

                            // Child overrides win
                            for (mname, mfunc) in methods_map {
                                inherited_methods.insert(mname, mfunc);
                            }
                            for (mname, mfunc) in static_methods_map {
                                inherited_static.insert(mname, mfunc);
                            }

                            (mro_fields, inherited_methods, inherited_static, struct_mro)
                        } else {
                            let mro = vec![name.clone()];
                            (native_fields, methods_map, static_methods_map, mro)
                        };

                    // Phase 2: implements(T1, T2, ...) resolves trait composition.
                    let mut trait_mro = Vec::new();
                    let mut trait_abstract: HashSet<MethodKey> = HashSet::new();
                    if !implements_list.is_empty() {
                        let struct_method_names: HashSet<String> = merged_methods.keys().cloned().collect();
                        let resolved = self
                            .trait_registry
                            .resolve_for_struct(py, &implements_list, &struct_method_names)
                            .map_err(VMError::RuntimeError)?;

                        trait_mro = resolved.linearization;
                        trait_abstract = resolved.abstract_methods;

                        // Prepend trait fields (before struct fields)
                        let struct_field_names: HashSet<String> =
                            merged_fields.iter().map(|f| f.name.clone()).collect();
                        let mut trait_fields_to_prepend = Vec::new();
                        for tf in resolved.fields {
                            if !struct_field_names.contains(&tf.name) {
                                trait_fields_to_prepend.push(StructField {
                                    name: tf.name,
                                    has_default: tf.has_default,
                                    default: tf.default,
                                    // Trait fields carry no enforced type annotation in v1.
                                    check: catnip_core::vm::opcode::ParamCheck::None,
                                });
                            }
                        }
                        if !trait_fields_to_prepend.is_empty() {
                            trait_fields_to_prepend.extend(merged_fields);
                            merged_fields = trait_fields_to_prepend;
                        }

                        // Merge trait methods (struct override > trait)
                        for (mname, mcallable) in resolved.methods {
                            if !merged_methods.contains_key(&mname) {
                                merged_methods.insert(mname, mcallable);
                            }
                        }

                        // Merge trait static methods (struct override > trait)
                        for (mname, mcallable) in resolved.static_methods {
                            if !merged_static.contains_key(&mname) {
                                merged_static.insert(mname, mcallable);
                            }
                        }
                    }

                    // Collect all abstract methods (own + inherited)
                    let mut final_abstract = own_abstract.clone();

                    // From parents (extends) - collect from all parents in MRO
                    for parent_name in &base_names {
                        if let Some(parent_type) = self.struct_registry.find_type_by_name(parent_name) {
                            for key in &parent_type.abstract_methods {
                                final_abstract.insert(key.clone());
                            }
                        }
                    }

                    // From traits (implements)
                    for key in trait_abstract {
                        final_abstract.insert(key);
                    }

                    // Remove methods that have concrete implementations
                    final_abstract.retain(|key| match key.kind {
                        super::structs::MethodKind::Instance => !merged_methods.contains_key(&key.name),
                        super::structs::MethodKind::Static => !merged_static.contains_key(&key.name),
                    });

                    // Concrete struct with unresolved abstracts => error
                    if own_abstract.is_empty() && !final_abstract.is_empty() {
                        let mut names: Vec<&str> = final_abstract.iter().map(|k| k.name.as_str()).collect();
                        names.sort();
                        return Err(VMError::RuntimeError(format!(
                            "struct '{}' must implement abstract method(s): {}",
                            name,
                            names.iter().map(|n| format!("'{}'", n)).collect::<Vec<_>>().join(", ")
                        )));
                    }

                    // Build full MRO: struct_mro (from C3) + trait_mro
                    let mut mro = struct_mro;
                    mro.extend(trait_mro);

                    let type_id = self.struct_registry.register_type_with_parents(
                        name.clone(),
                        merged_fields,
                        StructMethods {
                            instance: merged_methods,
                            statics: merged_static,
                            abstract_methods: final_abstract,
                        },
                        StructParents {
                            implements: implements_list,
                            mro,
                            parent_names: base_names,
                        },
                    );

                    // Build a callable CatnipStructType for Python-side access
                    let ty = self.struct_registry.get_type(type_id).unwrap();
                    let field_names: Vec<String> = ty.fields.iter().map(|f| f.name.clone()).collect();
                    let field_defaults: Vec<Option<Py<PyAny>>> = ty
                        .fields
                        .iter()
                        .map(|f| {
                            if f.has_default {
                                Some(f.default.to_pyobject(py))
                            } else {
                                None
                            }
                        })
                        .collect();
                    let methods_py: IndexMap<String, Py<PyAny>> =
                        ty.methods.iter().map(|(k, v)| (k.clone(), v.clone_ref(py))).collect();
                    let static_py: IndexMap<String, Py<PyAny>> = ty
                        .static_methods
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone_ref(py)))
                        .collect();
                    let init_fn = ty.methods.get("init").map(|f| f.clone_ref(py));
                    let parent_names_py = ty.parent_names.clone();
                    let mro_py = ty.mro.clone();
                    let implements_py = ty.implements.clone();
                    let abstract_py = ty.abstract_methods.clone();

                    let struct_type_obj = Py::new(
                        py,
                        CatnipStructType {
                            name: name.clone(),
                            field_names,
                            field_defaults,
                            // VM-side proxy type: field-type enforcement happens on
                            // the native StructType, so this carries no checks/ctx.
                            field_checks: Vec::new(),
                            field_templates: Vec::new(),
                            ctx_weakref: None,
                            methods: methods_py,
                            static_methods: static_py,
                            init_fn,
                            parent_names: parent_names_py,
                            mro: mro_py,
                            implements: implements_py,
                            abstract_methods: abstract_py,
                        },
                    )
                    .map_err(|e| VMError::RuntimeError(e.to_string()))?;

                    let marker_ptr = struct_type_obj.as_ptr();
                    self.struct_type_map.insert(marker_ptr as usize, type_id);

                    // One owned handle: store_global takes host.globals' own
                    // ref internally, VM.globals keeps this one.
                    let val = Value::from_owned_pyobject(struct_type_obj.into_any());

                    // Store in context.globals for Python-side access
                    if let Some(old) = host.store_global(py, &name, val)? {
                        decref_discard(&self.struct_registry, old);
                    }
                    // Also store in VM globals for scope resolution; a
                    // redefinition purges the old marker entry then releases
                    // the overwritten ref (pinning instead would retain the
                    // whole context cluster through the type's methods).
                    if let Some(old) = self.globals.insert(name, val) {
                        self.release_redefined_type(py, old);
                    }
                }

                OpCode::MakeTrait => {
                    let const_idx = instr.arg as usize;
                    let trait_info_val = code.constants[const_idx];
                    let trait_info_py = trait_info_val.to_pyobject(py);
                    let info_tuple = trait_info_py
                        .bind(py)
                        .cast::<PyTuple>()
                        .map_err(|e| VMError::RuntimeError(format!("MakeTrait: bad constant: {e}")))?;

                    // (name, extends_tuple, fields_info, num_defaults, [methods])
                    let name: String = tuple_extract(info_tuple, 0)?;

                    let extends_obj = tuple_get(info_tuple, 1)?;
                    let mut extends: Vec<String> = Vec::new();
                    for e in extends_obj
                        .try_iter()
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?
                    {
                        let e = e.map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        extends.push(e.extract().map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?);
                    }

                    let fields_info = tuple_get(info_tuple, 2)?;
                    let num_defaults: usize = tuple_extract(info_tuple, 3)?;

                    let has_methods = info_tuple.len() > 4;

                    // Read defaults off the stack and parse fields
                    // (trait fields carry no enforced annotation check)
                    let trait_fields: Vec<TraitField> = parse_field_specs(frame, &fields_info, num_defaults)?
                        .into_iter()
                        .map(|f| TraitField {
                            name: f.name,
                            has_default: f.has_default,
                            default: f.default,
                        })
                        .collect();

                    // Build method callables (same pattern as MakeStruct)
                    let (method_bodies, trait_static_methods, abstract_methods) = if has_methods {
                        let methods = tuple_get(info_tuple, 4)?;
                        self.collect_method_maps(py, host, frame, &methods)?
                    } else {
                        (IndexMap::new(), IndexMap::new(), HashSet::new())
                    };

                    // Register trait
                    let trait_def = TraitDef::new(
                        name,
                        extends,
                        trait_fields,
                        method_bodies,
                        abstract_methods,
                        trait_static_methods,
                    );
                    self.trait_registry.register_trait(trait_def);
                }

                OpCode::MakeEnum => {
                    let const_idx = instr.arg as usize;
                    let enum_info_val = code.constants[const_idx];
                    let enum_info_py = enum_info_val.to_pyobject(py);
                    let info_tuple = enum_info_py
                        .bind(py)
                        .cast::<PyTuple>()
                        .map_err(|e| VMError::RuntimeError(format!("MakeEnum: bad constant: {e}")))?;

                    let name: String = tuple_extract(info_tuple, 0)?;
                    let variants_obj = tuple_get(info_tuple, 1)?;
                    let variants_tuple = cast_tuple(&variants_obj)?;

                    let mut variant_names: Vec<String> = Vec::new();
                    for v in variants_tuple.iter() {
                        let vname: String = v.extract().map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;
                        variant_names.push(vname);
                    }

                    let type_id = self
                        .enum_registry
                        .register(&name, &variant_names, &mut self.symbol_table);

                    // Create a Python marker object for the enum type and store as global
                    let enum_type_obj = Py::new(py, CatnipEnumType::new(name.clone(), type_id, &variant_names))
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    let marker_ptr = enum_type_obj.as_ptr() as usize;
                    self.enum_type_map.insert(marker_ptr, type_id);

                    let val = Value::from_owned_pyobject(enum_type_obj.into_any());
                    if let Some(old) = host.store_global(py, &name, val)? {
                        decref_discard(&self.struct_registry, old);
                    }
                    if let Some(old) = self.globals.insert(name, val) {
                        self.release_redefined_type(py, old);
                    }
                }
                OpCode::MakeUnion => {
                    use crate::vm::unions::build_union_type;
                    let const_idx = instr.arg as usize;
                    let info_val = code.constants[const_idx];
                    let info_py = info_val.to_pyobject(py);
                    let info_tuple = info_py
                        .bind(py)
                        .cast::<PyTuple>()
                        .map_err(|e| VMError::RuntimeError(format!("MakeUnion: bad constant: {e}")))?;
                    if info_tuple.len() < 3 {
                        return Err(VMError::RuntimeError(
                            "MakeUnion: expected (name, type_params, variants) tuple".into(),
                        ));
                    }

                    let name: String = tuple_extract(info_tuple, 0)?;

                    // Type parameters tuple
                    let type_params_obj = tuple_get(info_tuple, 1)?;
                    let type_params_tuple = cast_tuple(&type_params_obj)?;
                    let mut type_params: Vec<String> = Vec::new();
                    for tp in type_params_tuple.iter() {
                        type_params.push(tp.extract().map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?);
                    }

                    // Variants tuple: each entry is
                    // (variant_name, field_names_tuple, field_types_tuple).
                    let variants_obj = tuple_get(info_tuple, 2)?;
                    let variants_tuple = cast_tuple(&variants_obj)?;
                    let mut variants: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
                    for variant in variants_tuple.iter() {
                        let pair = variant
                            .cast::<PyTuple>()
                            .map_err(|e| VMError::RuntimeError(format!("MakeUnion: bad variant: {e}")))?;
                        if pair.len() < 2 {
                            return Err(VMError::RuntimeError("MakeUnion: variant tuple too small".into()));
                        }
                        let variant_name: String = pair
                            .get_item(0)
                            .and_then(|v| v.extract())
                            .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;
                        let fields_any = pair
                            .get_item(1)
                            .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;
                        let fields_tuple = fields_any
                            .cast::<PyTuple>()
                            .map_err(|e| VMError::RuntimeError(format!("MakeUnion: bad fields: {e}")))?;
                        let mut field_names: Vec<String> = Vec::new();
                        for f in fields_tuple.iter() {
                            field_names.push(f.extract().map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?);
                        }
                        // Field types (3rd element, parallel to names; empty string =
                        // unannotated). Absent on a legacy 2-tuple -> all empty.
                        let mut field_types: Vec<String> = Vec::new();
                        if let Ok(types_any) = pair.get_item(2) {
                            if let Ok(types_tuple) = types_any.cast::<PyTuple>() {
                                for t in types_tuple.iter() {
                                    field_types.push(t.extract().unwrap_or_default());
                                }
                            }
                        }
                        variants.push((variant_name, field_names, field_types));
                    }

                    // Eagerly intern nullary variant symbols so the qualified
                    // name resolves anywhere, including child VMs that clone this
                    // table (broadcast lambdas). Without this the symbol is only
                    // interned lazily on the first to_vm crossing, so a variant
                    // built inside a callback registers into the throwaway child
                    // table and the parent never learns its id (the round-trip
                    // back demotes it to a raw int).
                    for (variant_name, field_names, _) in &variants {
                        if field_names.is_empty() {
                            self.symbol_table
                                .intern(&catnip_core::symbols::qualified_name(&name, variant_name));
                        }
                    }

                    // Optional 4th element: methods list of (name, CodeObject)
                    // pairs. Materialized as VMFunctions like struct methods.
                    let mut methods: IndexMap<String, Py<PyAny>> = IndexMap::new();
                    if info_tuple.len() > 3 {
                        let methods_obj = tuple_get(info_tuple, 3)?;
                        for method_result in methods_obj
                            .try_iter()
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?
                        {
                            let method_pair = method_result.map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let pair = cast_tuple(&method_pair)?;
                            let method_name: String = tuple_extract(pair, 0)?;
                            let code_obj = tuple_get(pair, 1)?;
                            let code_py: Py<PyCodeObject> = code_obj
                                .cast::<PyCodeObject>()
                                .map_err(|e| VMError::TypeError(format!("Expected CodeObject: {e}")))?
                                .clone()
                                .unbind();
                            let parent = host.build_closure_parent(py, frame.closure_scope.as_ref());
                            let native_scope = NativeClosureScope::new(IndexMap::new(), parent);
                            let context_for_func = host.context().as_ref().map(|c| c.clone_ref(py));
                            let func = Py::new(
                                py,
                                VMFunction::create_native(py, code_py, Some(native_scope), context_for_func),
                            )
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            methods.insert(method_name, func.into_any());
                        }
                    }

                    // Register payload variants as native struct types so
                    // `Union.Variant(...)` builds a native instance via the fast
                    // path, instead of round-tripping through the Python `__call__`
                    // (which orphans an ObjectTable handle on the variant type and
                    // pins the union's methods -> Context). Methods are shared by
                    // all variants; each concrete payload field carries its type
                    // check (type-parameter fields stay inert, see below).
                    let mut variant_type_ids: HashMap<String, StructTypeId> = HashMap::new();
                    for (variant_name, field_names, field_types) in &variants {
                        if field_names.is_empty() {
                            continue; // nullary -> TAG_SYMBOL, not a struct
                        }
                        let qualified = qualified_name(&name, variant_name);
                        // Payload-field templates for the generic-nominal boundary
                        // (`Option[int]`), classified against the union's type params.
                        let templates: Vec<catnip_core::vm::opcode::FieldTemplate> = field_names
                            .iter()
                            .enumerate()
                            .map(|(i, _)| {
                                let ftext = field_types.get(i).map(String::as_str).filter(|s| !s.is_empty());
                                catnip_core::vm::opcode::compute_field_template(&type_params, ftext)
                            })
                            .collect();
                        // A concrete field (`A(x: int)`) is enforced at construction,
                        // mirroring struct fields; a type-parameter field (`Some(value: T)`)
                        // is inert here (`T` binds at the use-site generic boundary).
                        let fields: Vec<StructField> = field_names
                            .iter()
                            .enumerate()
                            .map(|(i, fname)| StructField {
                                name: fname.clone(),
                                has_default: false,
                                default: Value::NIL,
                                check: templates[i].construction_check(),
                            })
                            .collect();
                        let variant_methods: IndexMap<String, Py<PyAny>> =
                            methods.iter().map(|(k, v)| (k.clone(), v.clone_ref(py))).collect();
                        let type_id = self.struct_registry.register_type(
                            qualified.clone(),
                            fields,
                            variant_methods,
                            Vec::new(),
                            vec![qualified],
                        );
                        self.struct_registry.set_variant_templates(type_id, templates);
                        variant_type_ids.insert(variant_name.clone(), type_id);
                    }

                    // Payload variants are constructed natively (the struct types
                    // registered above, with enforced field checks) on the plain
                    // `Call` path. A *fused* `U.A(...)` (CallMethod) instead falls
                    // through to the proxy's Python `__call__`, so the proxy also
                    // needs the ctx weakref to enforce there, mirroring op_struct.
                    let ctx_weakref = host.context().as_ref().and_then(|ctx| {
                        py.import("weakref")
                            .and_then(|w| w.getattr("ref"))
                            .and_then(|r| r.call1((ctx.bind(py),)))
                            .map(|w| w.unbind())
                            .ok()
                    });
                    let union_obj = build_union_type(py, &name, type_params, &variants, methods, ctx_weakref)
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;

                    // Map each payload variant's marker pointer to its registry id.
                    for (variant_name, ptr) in union_obj.borrow(py).payload_variant_ptrs() {
                        if let Some(&type_id) = variant_type_ids.get(&variant_name) {
                            self.struct_type_map.insert(ptr, type_id);
                        }
                    }

                    let val = Value::from_owned_pyobject(union_obj.into_any());
                    if let Some(old) = host.store_global(py, &name, val)? {
                        decref_discard(&self.struct_registry, old);
                    }
                    if let Some(old) = self.globals.insert(name, val) {
                        self.release_redefined_type(py, old);
                    }
                    // Statement-list compilers emit PopTop after the
                    // declaration, mirroring the AST executor where
                    // op_union returns py.None(). Push NIL here so the
                    // following PopTop has something to discard --
                    // otherwise a `union { ... }\nexpr` program would
                    // underflow the VM stack.
                    frame.push(Value::NIL);
                }

                // --- Exception handling ---
                OpCode::SetupExcept => {
                    frame.handler_stack.push(catnip_core::exception::Handler {
                        handler_type: catnip_core::exception::HandlerType::Except,
                        target_addr: instr.arg as usize,
                        stack_depth: frame.stack.len(),
                        block_depth: frame.block_stack.len(),
                    });
                }
                OpCode::SetupFinally => {
                    frame.handler_stack.push(catnip_core::exception::Handler {
                        handler_type: catnip_core::exception::HandlerType::Finally,
                        target_addr: instr.arg as usize,
                        stack_depth: frame.stack.len(),
                        block_depth: frame.block_stack.len(),
                    });
                }
                OpCode::PopHandler => {
                    frame.handler_stack.pop();
                }
                OpCode::CheckExcMatch => {
                    let const_val = code.constants[instr.arg as usize];
                    let py_obj = const_val.to_pyobject(py);
                    let type_name_to_match: String = py_obj.bind(py).str().map(|s| s.to_string()).unwrap_or_default();
                    let matches = if let Some(exc_info) = frame.active_exception_stack.last() {
                        exc_info.matches(&type_name_to_match)
                    } else {
                        false
                    };
                    frame.push(Value::from_bool(matches));
                }
                OpCode::LoadException => {
                    if instr.arg == 1 {
                        // ExcInfo mode: push (exc_type_class, exc_instance, None) tuple
                        if let Some(exc_info) = frame.active_exception_stack.last() {
                            let builtins = py.import("builtins").unwrap();
                            let exc_type = builtins
                                .getattr(exc_info.type_name.as_str())
                                .unwrap_or_else(|_| builtins.getattr("RuntimeError").unwrap());
                            let exc_val = exc_type
                                .call1((&exc_info.message,))
                                .unwrap_or_else(|_| py.None().into_bound(py));
                            let tuple = pyo3::types::PyTuple::new(
                                py,
                                &[exc_type.unbind().into_any(), exc_val.unbind().into_any(), py.None()],
                            )
                            .unwrap();
                            frame.push(Value::from_owned_pyobject(tuple.unbind().into_any()));
                        } else {
                            let tuple = pyo3::types::PyTuple::new(py, &[py.None(), py.None(), py.None()]).unwrap();
                            frame.push(Value::from_owned_pyobject(tuple.unbind().into_any()));
                        }
                    } else if let Some(exc_info) = frame.active_exception_stack.last() {
                        let py_str = PyString::new(py, &exc_info.message);
                        frame.push(Value::from_owned_pyobject(py_str.unbind().into_any()));
                    } else {
                        frame.push(Value::NIL);
                    }
                }
                OpCode::Raise => {
                    if instr.arg == 1 {
                        // Bare raise: re-raise preserving full MRO
                        if let Some(exc_info) = frame.active_exception_stack.last().cloned() {
                            return Err(VMError::UserException(exc_info));
                        } else {
                            return Err(VMError::RuntimeError(errors::ERR_NO_ACTIVE_EXCEPTION.into()));
                        }
                    } else {
                        // raise expr: pop value, detect exception type
                        let val = frame.pop();
                        let err = if let Some(inst_idx) = val.as_struct_instance_idx() {
                            // Struct instance: get real type name from registry
                            // (to_pyobject wraps in CatnipStruct proxy, losing the name)
                            let type_name = self
                                .struct_registry
                                .with_instance(inst_idx, |inst| inst.type_id)
                                .and_then(|type_id| self.struct_registry.get_type(type_id))
                                .map(|ty| ty.name.clone())
                                .unwrap_or_else(|| "RuntimeError".to_string());
                            let msg = self
                                .struct_registry
                                .with_instance(inst_idx, |inst| inst.fields.first().copied())
                                .flatten()
                                .map(|v| {
                                    let obj = v.to_pyobject(py);
                                    obj.bind(py).str().map(|s| s.to_string()).unwrap_or_default()
                                })
                                .unwrap_or_default();
                            let mro = self
                                .struct_registry
                                .find_type_by_name(&type_name)
                                .map(|ty| ty.mro.clone())
                                .unwrap_or_else(|| vec![type_name.clone(), "Exception".to_string()]);
                            VMError::UserException(catnip_core::exception::ExceptionInfo::new(type_name, msg, mro))
                        } else {
                            // Python object: detect type from Python introspection
                            let py_obj = val.to_pyobject(py);
                            let bound = py_obj.bind(py);
                            let msg = bound.str().map(|s| s.to_string()).unwrap_or_default();
                            let type_name = bound.get_type().name().map(|n| n.to_string()).unwrap_or_default();
                            if let Some(kind) = catnip_core::exception::ExceptionKind::from_name(&type_name) {
                                // Known exception: dedicated variant when available,
                                // UserException for group types that would lose identity
                                let test_err = VMError::from_exception_info(&type_name, &msg);
                                if matches!(&test_err, VMError::RuntimeError(_)) && type_name != "RuntimeError" {
                                    VMError::UserException(catnip_core::exception::ExceptionInfo::from_kind(kind, msg))
                                } else {
                                    test_err
                                }
                            } else {
                                VMError::RuntimeError(msg)
                            }
                        };
                        decref_discard(&self.struct_registry, val);
                        return Err(err);
                    }
                }
                OpCode::ResumeUnwind => {
                    if let Some(pending) = frame.pending_unwind.take() {
                        match pending {
                            catnip_core::exception::PendingUnwind::Exception(info) => {
                                return Err(VMError::UserException(info));
                            }
                            catnip_core::exception::PendingUnwind::Return => {
                                let val = frame.pop();
                                return Err(VMError::Return(val));
                            }
                            catnip_core::exception::PendingUnwind::Break => {
                                return Err(VMError::Break);
                            }
                            catnip_core::exception::PendingUnwind::Continue => {
                                return Err(VMError::Continue);
                            }
                        }
                    } else if let Some(exc_info) = frame.active_exception_stack.last().cloned() {
                        // Fallback: re-raise from active exception (except no-match -> finally case)
                        return Err(VMError::UserException(exc_info));
                    }
                    // No pending unwind: finally on happy path, just continue
                }
                OpCode::ClearException => {
                    frame.active_exception_stack.pop();
                }

                // --- ND Operations ---
                OpCode::NdEmptyTopos => {
                    // Get cached NDTopos singleton or create it
                    if self.cached_nd_topos.is_none() {
                        let nd_module = py.import(PY_MOD_ND).map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        let nd_topos_class = nd_module
                            .getattr("NDTopos")
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        let instance = nd_topos_class
                            .call_method0("instance")
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        self.cached_nd_topos = Some(instance.unbind());
                    }

                    let instance = self.cached_nd_topos.as_ref().unwrap();
                    let value = Value::from_pyobject(py, instance.bind(py))
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    frame.push(value);
                }

                OpCode::NdRecursion => {
                    let form = instr.arg;

                    if form == 0 {
                        // Combinator: pop lambda, pop seed
                        // Ownership: the host helper borrows (to_pyobject clones);
                        // the popped operands are released here on every path.
                        let lambda_val = frame.pop();
                        let seed_val = frame.pop();
                        let lambda_py = lambda_val.to_pyobject(py);
                        let seed_py = seed_val.to_pyobject(py);

                        let result = host.execute_nd_recursion(py, seed_py.bind(py), lambda_py.bind(py));
                        release_operands(&self.struct_registry, &[lambda_val, seed_val]);
                        let value = Value::from_pyobject(py, result?.bind(py))
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        frame.push(value);
                    } else {
                        // Declaration: pop lambda, wrap in NDDeclaration
                        let lambda_val = frame.pop();
                        let lambda_py = lambda_val.to_pyobject(py);
                        release_operands(&self.struct_registry, &[lambda_val]);
                        if let Some(ctx) = host.context() {
                            let decl = Py::new(py, crate::nd::NDDeclaration::new(lambda_py, ctx.clone_ref(py)))
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let value = Value::from_pyobject(py, decl.into_any().bind(py))
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            frame.push(value);
                        } else {
                            // Standalone mode: wrap in NDVmDecl so f(seed) calls lambda(seed, f)
                            let decl = Py::new(py, crate::nd::NDVmDecl::new(lambda_py))
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let value = Value::from_pyobject(py, decl.into_any().bind(py))
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            frame.push(value);
                        }
                    }
                }

                OpCode::NdMap => {
                    let form = instr.arg;

                    if form == 0 {
                        // Applicative: pop func, pop data
                        // Ownership: the host helper borrows (to_pyobject clones);
                        // the popped operands are released here on every path.
                        let func_val = frame.pop();
                        let data_val = frame.pop();
                        let func_py = func_val.to_pyobject(py);
                        let data_py = data_val.to_pyobject(py);

                        let result = host.execute_nd_map(py, data_py.bind(py), func_py.bind(py));
                        release_operands(&self.struct_registry, &[func_val, data_val]);
                        let value = Value::from_pyobject(py, result?.bind(py))
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        frame.push(value);
                    } else {
                        // Lift: pop func, push back
                        let func_val = frame.pop();
                        frame.push(func_val);
                    }
                }

                OpCode::MatchFail => {
                    let msg_idx = instr.arg as usize;
                    let msg = code.constants[msg_idx].to_pyobject(py);
                    let msg_str: String = msg.bind(py).extract().unwrap_or_default();
                    return Err(VMError::RuntimeError(msg_str));
                }

                // --- String formatting ---
                OpCode::FormatValue => {
                    // flags = (conv << 1) | has_spec
                    let flags = instr.arg;
                    let has_spec = (flags & 1) != 0;
                    let conv = (flags >> 1) & 3;

                    // Keep the popped Values: to_pyobject clones its own Python
                    // ref, so the operand handles must be released on every
                    // path (a raising __format__ included) or each f-string
                    // interpolation of a heap value leaks one ref.
                    let spec_val = if has_spec { Some(frame.pop()) } else { None };
                    let spec_obj = match spec_val {
                        Some(v) => v.to_pyobject(py),
                        None => "".into_pyobject(py).unwrap().into_any().unbind(),
                    };
                    let value_val = frame.pop();
                    let value = value_val.to_pyobject(py);

                    let result = (|| -> VMResult<Py<PyAny>> {
                        let builtins = py.import("builtins").to_vm(py)?;

                        // Apply conversion: 0=none, 1=str, 2=repr, 3=ascii
                        let converted = match conv {
                            1 => builtins
                                .getattr("str")
                                .to_vm(py)?
                                .call1((value.bind(py),))
                                .to_vm(py)?
                                .unbind(),
                            2 => builtins
                                .getattr("repr")
                                .to_vm(py)?
                                .call1((value.bind(py),))
                                .to_vm(py)?
                                .unbind(),
                            3 => builtins
                                .getattr("ascii")
                                .to_vm(py)?
                                .call1((value.bind(py),))
                                .to_vm(py)?
                                .unbind(),
                            _ => value.clone_ref(py),
                        };

                        Ok(builtins
                            .getattr("format")
                            .to_vm(py)?
                            .call1((converted.bind(py), spec_obj.bind(py)))
                            .to_vm(py)?
                            .unbind())
                    })();
                    decref_discard(&self.struct_registry, value_val);
                    if let Some(v) = spec_val {
                        decref_discard(&self.struct_registry, v);
                    }
                    frame.push(Value::from_owned_pyobject(result?));
                }

                OpCode::BuildString => {
                    let n = instr.arg as usize;
                    let stack_len = frame.stack.len();
                    let start = stack_len - n;

                    let mut buf = String::with_capacity(n * 16);
                    for i in start..stack_len {
                        let py_obj = frame.stack[i].to_pyobject(py);
                        let s: String = py_obj.bind(py).extract().unwrap_or_default();
                        buf.push_str(&s);
                    }
                    // The pieces are FormatValue outputs or string consts:
                    // pyobj by construction, so the pyobj-only release is
                    // sufficient here (the sibling Build* arms release all
                    // heap kinds because their elements are arbitrary).
                    for &piece in &frame.stack[start..] {
                        decref_pyobj(piece);
                    }
                    frame.stack.truncate(start);

                    let py_str = PyString::new(py, &buf);
                    frame.push(Value::from_owned_pyobject(py_str.unbind().into_any()));
                }

                OpCode::Halt => {
                    last_result = if frame.stack.is_empty() {
                        Value::NIL
                    } else {
                        frame.pop()
                    };
                    // Sync locals to globals for module-level code
                    // Only sync variables that were stored via STORE_NAME (already in globals)
                    // Loop variables (stored via STORE_LOCAL only) are not synced
                    let sync_data: Vec<(String, Value)> = if let Some(code) = &frame.code {
                        code.slotmap
                            .iter()
                            .filter_map(|(name, &slot)| {
                                // Only sync if this name was already stored via STORE_NAME
                                if !self.globals.contains_key(name) {
                                    return None;
                                }
                                if slot < frame.locals.len() {
                                    let val = frame.locals[slot];
                                    if !val.is_nil() && !val.is_invalid() {
                                        Some((name.clone(), val))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    // Check if main frame (current frame is not on stack)
                    let is_main_frame = self.frame_stack.is_empty();
                    if is_main_frame {
                        for (name, val) in sync_data {
                            // The map takes its OWN ref: the slot's ref is
                            // released by the frame-pool free right after the
                            // run returns, so copying without recounting would
                            // leave the map holding a dead handle (double
                            // release once the map is drained). The overwritten
                            // entry releases hers -- a same-handle overwrite
                            // nets to zero.
                            val.clone_refcount();
                            if let Some(old) = self.globals.insert(name, val) {
                                decref_discard(&self.struct_registry, old);
                            }
                        }
                    }
                    return Ok(last_result);
                }
            }

            // Post-dispatch debug pause: instruction was already executed
            if debug_should_pause {
                self.debug_last_paused_byte = Some(_current_src_byte);
                let locals_data: Vec<(String, Value)> = if let Some(ref code) = frame.code {
                    code.slotmap
                        .iter()
                        .filter_map(|(name, &slot)| {
                            if slot < frame.locals.len() {
                                Some((name.clone(), frame.locals[slot]))
                            } else {
                                None
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let call_stack_snapshot: Vec<(String, u32)> = self
                    .call_stack
                    .iter()
                    .map(|ci| (ci.name.clone(), ci.call_start_byte))
                    .collect();
                let depth = self.call_stack.len();
                // frame no longer used past this point
                self.debug_step_mode = DebugStepMode::Disabled;
                let action = self.invoke_debug_callback(py, _current_src_byte, &locals_data, &call_stack_snapshot)?;
                self.debug_step_mode = action;
                if action == DebugStepMode::StepOver || action == DebugStepMode::StepOut {
                    self.debug_step_depth = depth;
                }
                continue;
            }
        }
    }

    // --- Exception unwinding ---

    /// Try to unwind to a handler in the current frame, or walk up the call stack.
    fn unwind_exception(&mut self, frame: &mut Frame, err: &VMError) -> bool {
        // Try current frame
        if self.try_unwind_to_handler(frame, err) {
            return true;
        }
        // For catchable exceptions, walk up the call stack
        if err.is_catchable() {
            while let Some(caller) = self.frame_stack.pop() {
                let old = std::mem::replace(frame, caller);
                self.frame_pool.free(old, &self.struct_registry);
                if self.try_unwind_to_handler(frame, err) {
                    return true;
                }
            }
        }
        false
    }

    /// Try to find and activate a handler in the current frame.
    fn try_unwind_to_handler(&mut self, frame: &mut Frame, err: &VMError) -> bool {
        while let Some(handler) = frame.handler_stack.last() {
            match handler.handler_type {
                catnip_core::exception::HandlerType::Except => {
                    if err.is_catchable() {
                        let handler = frame.handler_stack.pop().unwrap();
                        if let Some(info) = err.exception_info() {
                            frame.active_exception_stack.push(info);
                        }
                        for &v in &frame.stack[handler.stack_depth..] {
                            decref_discard(&self.struct_registry, v);
                        }
                        frame.stack.truncate(handler.stack_depth);
                        for (_slot_start, saved) in frame.block_stack.drain(handler.block_depth..) {
                            for val in saved {
                                decref_discard(&self.struct_registry, val);
                            }
                        }
                        frame.ip = handler.target_addr;
                        return true;
                    }
                    // Control flow signal: skip Except handler
                    frame.handler_stack.pop();
                }
                catnip_core::exception::HandlerType::Finally => {
                    let handler = frame.handler_stack.pop().unwrap();
                    frame.pending_unwind = Some(err.to_pending_unwind());
                    for &v in &frame.stack[handler.stack_depth..] {
                        decref_discard(&self.struct_registry, v);
                    }
                    frame.stack.truncate(handler.stack_depth);
                    for (_slot_start, saved) in frame.block_stack.drain(handler.block_depth..) {
                        for val in saved {
                            decref_discard(&self.struct_registry, val);
                        }
                    }
                    // For Return, save the value on the stack
                    if let VMError::Return(val) = err {
                        frame.push(*val);
                    }
                    frame.ip = handler.target_addr;
                    return true;
                }
            }
        }
        false
    }

    /// Boundary check for a nominal-typed param (`CheckNominal`). Returns
    /// `Ok(())` when `val` is an instance of the type named `name` -- with
    /// subtyping (MRO + implemented traits) and tagged-union membership -- or
    /// when `name` is not a known nominal type at runtime, in which case the
    /// annotation is inert. Returns a `TypeError` when `name` is a known nominal
    /// but `val` is not a member. No coercion: a nominal value is never rewritten.
    fn check_nominal(&self, py: Python<'_>, val: Value, name: &str) -> VMResult<()> {
        if self.value_is_nominal_member(py, val, name) {
            return Ok(());
        }
        if self.name_is_known_nominal(py, name) {
            return Err(VMError::TypeError(format!(
                "typed parameter expects '{}' but got '{}'",
                name,
                self.nominal_value_type_name(py, val)
            )));
        }
        Ok(())
    }

    /// Boundary check for a type-union param (`CheckUnion`). Accepts `val` when it
    /// satisfies any member: a primitive member by the numeric tower (no coercion,
    /// [`primitive_membership`]), a nominal member by the same subtyping rule as
    /// [`Self::value_is_nominal_member`]. Raises a `TypeError` naming the union when
    /// no member matches. Mirrors the PureVM `check_union`.
    fn check_union(&self, py: Python<'_>, val: Value, members: &[catnip_core::vm::opcode::ParamCheck]) -> VMResult<()> {
        use catnip_core::vm::opcode::{ParamCheck, format_union_members, primitive_membership};
        // A nominal member whose name is unknown at runtime can't be proven absent
        // (forward ref, conditionally-defined type), so -- like `CheckNominal` --
        // we stay inert rather than reject a possibly-valid value.
        let class = value_primitive_class(py, val);
        let mut unknown_nominal = false;
        for m in members {
            match m {
                ParamCheck::Primitive(code) => {
                    if primitive_membership(*code, &class) {
                        return Ok(());
                    }
                }
                ParamCheck::Nominal(name) => {
                    if self.value_is_nominal_member(py, val, name) {
                        return Ok(());
                    }
                    if !self.name_is_known_nominal(py, name) {
                        unknown_nominal = true;
                    }
                }
                // A composite member is checked in full (container + parameters),
                // mirroring `check_composite`.
                ParamCheck::Composite { .. } => {
                    if self.check_composite(py, val, m).is_ok() {
                        return Ok(());
                    }
                }
                // A generic-nominal member (`Option[int]`): a member of the union
                // with a matching payload accepts; a member with a mismatched
                // payload is not this alternative; an unknown union name keeps the
                // whole check inert, like `Nominal`.
                ParamCheck::Generic { name, .. } => {
                    if self.value_is_nominal_member(py, val, name) {
                        if self.check_generic(py, val, m).is_ok() {
                            return Ok(());
                        }
                    } else if !self.name_is_known_nominal(py, name) {
                        unknown_nominal = true;
                    }
                }
                // A function-type member (`None | (int) -> int`): full
                // callability + arity acceptance, mirroring the prologue check.
                ParamCheck::Callable { arity } => {
                    if self.check_callable(py, val, *arity).is_ok() {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        if unknown_nominal {
            return Ok(());
        }
        Err(VMError::TypeError(format!(
            "typed parameter expects '{}' but got '{}'",
            format_union_members(members),
            self.nominal_value_type_name(py, val)
        )))
    }

    /// Boundary check for a composite param (`CheckComposite`). In this VM a
    /// `list`/`set`/`dict`/`tuple` is a Python object, so the check runs on the
    /// `PyAny` rep (a nested composite element reuses the same path). Mirrors the
    /// PureVM `check_composite` and the AST `composite_check_py`.
    fn check_composite(&self, py: Python<'_>, val: Value, spec: &catnip_core::vm::opcode::ParamCheck) -> VMResult<()> {
        let obj = val.to_pyobject(py);
        self.check_composite_py(py, obj.bind(py), spec)
    }

    /// Composite boundary check on a Python value: container tag + recursive
    /// element/key/value checks. No coercion, read-only (PyO3 manages element
    /// refcounts).
    fn check_composite_py(
        &self,
        py: Python<'_>,
        value: &Bound<'_, PyAny>,
        spec: &catnip_core::vm::opcode::ParamCheck,
    ) -> VMResult<()> {
        use catnip_core::vm::opcode::{ParamCheck, format_param_check, primitive_membership, type_code};
        let ParamCheck::Composite { head, params } = spec else {
            return Ok(());
        };
        if !primitive_membership(*head, &value_primitive_class_pyany(value)) {
            return Err(VMError::TypeError(format!(
                "typed parameter expects '{}' but got '{}'",
                format_param_check(spec),
                nominal_value_type_name_pyany(value)
            )));
        }
        let enforced = |p: &ParamCheck| !matches!(p, ParamCheck::None);
        match *head {
            type_code::LIST => {
                if let Some(elem) = params.first().filter(|p| enforced(p)) {
                    if let Ok(list) = value.cast::<PyList>() {
                        for it in list.iter() {
                            if !self.value_satisfies_py(py, &it, elem) {
                                return Err(VMError::TypeError(format!(
                                    "typed parameter expects '{}' but an element has the wrong type",
                                    format_param_check(spec)
                                )));
                            }
                        }
                    }
                }
            }
            type_code::SET => {
                if let Some(elem) = params.first().filter(|p| enforced(p)) {
                    if let Ok(set) = value.cast::<PySet>() {
                        for it in set.iter() {
                            if !self.value_satisfies_py(py, &it, elem) {
                                return Err(VMError::TypeError(format!(
                                    "typed parameter expects '{}' but an element has the wrong type",
                                    format_param_check(spec)
                                )));
                            }
                        }
                    }
                }
            }
            type_code::TUPLE => {
                // Positional: `params.len()` is the enforced arity, position `i`
                // is checked against `params[i]`. A bare `tuple` checks only the
                // container.
                if !params.is_empty() {
                    if let Ok(tuple) = value.cast::<PyTuple>() {
                        if tuple.len() != params.len() {
                            return Err(VMError::TypeError(format!(
                                "typed parameter expects '{}' but got a tuple of length {}",
                                format_param_check(spec),
                                tuple.len()
                            )));
                        }
                        for (it, p) in tuple.iter().zip(params.iter()) {
                            if !self.value_satisfies_py(py, &it, p) {
                                return Err(VMError::TypeError(format!(
                                    "typed parameter expects '{}' but an element has the wrong type",
                                    format_param_check(spec)
                                )));
                            }
                        }
                    }
                }
            }
            type_code::DICT if params.len() == 2 => {
                let (kc, vc) = (&params[0], &params[1]);
                if let Ok(dict) = value.cast::<PyDict>() {
                    for (k, v) in dict.iter() {
                        if enforced(kc) && !self.value_satisfies_py(py, &k, kc) {
                            return Err(VMError::TypeError(format!(
                                "typed parameter expects '{}' but a key has the wrong type",
                                format_param_check(spec)
                            )));
                        }
                        if enforced(vc) && !self.value_satisfies_py(py, &v, vc) {
                            return Err(VMError::TypeError(format!(
                                "typed parameter expects '{}' but a value has the wrong type",
                                format_param_check(spec)
                            )));
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Whether a single Python value satisfies a `ParamCheck` (per element of a
    /// composite). Mirrors the PureVM `value_satisfies` on the `PyAny` rep.
    /// Read-only validation of a constructor field value against its annotation.
    /// Mirrors [`Self::value_satisfies_py`] but rejects up front a bigint that
    /// would overflow `f64` for a `float` slot -- the one input [`boundary_coerce`]
    /// fails on -- so [`Self::enforce_field_types`] can validate first and coerce
    /// second without a fallible step after a refcount mutation.
    /// Function-type boundary (FT3). Callable + declared-arity acceptance
    /// (the shared `callable_arity_accepts` rule, so executors cannot drift):
    /// a VM function's arity is read off its CodeObject; every other value is
    /// checked through Python via `check_callable_pyany` (a `VMFunction`
    /// wrapper and a `CatnipStructType` constructor are introspectable there;
    /// any other callable passes on callability alone).
    /// Parameter/return types are NOT checked (observable only at calls).
    fn check_callable(&self, py: Python<'_>, val: Value, arity: u32) -> VMResult<()> {
        if val.is_vmfunc() && !val.is_invalid() {
            let slot = self
                .func_table
                .get(val.as_vmfunc_idx())
                .ok_or_else(|| VMError::RuntimeError("invalid function index".into()))?;
            return Self::vmfunc_code_accepts(&slot.code, arity);
        }
        let obj = val.to_pyobject(py);
        self.check_callable_pyany(obj.bind(py), arity)
    }

    /// Release the globals entry overwritten by a type redefinition, after
    /// purging the marker-pointer maps keyed by the OLD object's address --
    /// a stale entry would dangle once the object dies. Live instances of
    /// the old type keep working through the Python slow path (the marker
    /// maps are fast paths with a correct fallback).
    fn release_redefined_type(&mut self, py: Python<'_>, old: Value) {
        if old.is_pyobj() {
            let old_ptr = old.to_pyobject(py).as_ptr() as usize;
            self.struct_type_map.remove(&old_ptr);
            self.enum_type_map.remove(&old_ptr);
        }
        decref_discard(&self.struct_registry, old);
    }

    /// Arity acceptance for a VM function's CodeObject (shared by the Value
    /// and PyAny paths).
    fn vmfunc_code_accepts(code: &CodeObject, arity: u32) -> VMResult<()> {
        use catnip_core::vm::opcode::callable_arity_accepts;
        let arity = arity as usize;
        let real_defaults = code.defaults.iter().filter(|v| !v.is_nil()).count();
        let has_vararg = code.vararg_idx >= 0;
        let fixed = if has_vararg {
            code.vararg_idx as usize
        } else {
            code.nargs
        };
        let (required, accepts) = callable_arity_accepts(fixed, has_vararg, real_defaults, arity);
        if accepts {
            Ok(())
        } else {
            Err(VMError::TypeError(format!(
                "typed parameter expects a callable taking {arity} argument(s) but the function requires {required}"
            )))
        }
    }

    /// Function-type boundary on a `PyAny`: a `VMFunction` wrapper resolves
    /// its CodeObject through the func table; a `CatnipStructType` constructor
    /// checks its field range; any other callable passes on callability alone
    /// (arity not introspectable without `inspect`).
    fn check_callable_pyany(&self, bound: &Bound<'_, PyAny>, arity: u32) -> VMResult<()> {
        use catnip_core::vm::opcode::callable_arity_accepts;
        if let Ok(f) = bound.cast::<crate::vm::frame::VMFunction>() {
            if let Some(idx) = f.borrow().func_table_idx {
                if let Some(slot) = self.func_table.get(idx) {
                    return Self::vmfunc_code_accepts(&slot.code, arity);
                }
            }
            return Ok(());
        }
        if let Ok(st) = bound.cast::<crate::vm::structs::CatnipStructType>() {
            let st = st.borrow();
            let fixed = st.field_names.len();
            let defaults = st.field_defaults.iter().filter(|d| d.is_some()).count().min(fixed);
            let (required, accepts) = callable_arity_accepts(fixed, false, defaults, arity as usize);
            if accepts {
                return Ok(());
            }
            return Err(VMError::TypeError(format!(
                "typed parameter expects a callable taking {arity} argument(s) but the constructor requires {required}"
            )));
        }
        if bound.is_callable() {
            Ok(())
        } else {
            Err(VMError::TypeError(format!(
                "typed parameter expects a callable taking {arity} argument(s) but got a non-callable value"
            )))
        }
    }

    fn field_value_ok(&self, py: Python<'_>, val: Value, check: &catnip_core::vm::opcode::ParamCheck) -> bool {
        use catnip_core::vm::opcode::{ParamCheck, type_code};
        // Dispatch to the same `&self` helpers the opcode prologue uses, so nominal
        // subtyping (the `extends` chain / mro) resolves through the registry --
        // `value_satisfies_py` only inspects the value and would miss it. Read-only:
        // `boundary_coerce` mutates a refcount only for bigint -> float, handled
        // first so pass 1 never mutates.
        match check {
            ParamCheck::None => true,
            ParamCheck::Primitive(type_code::FLOAT) if val.is_bigint() => {
                // SAFETY: is_bigint() guards the Arc<Integer> payload.
                unsafe { val.as_bigint_ref() }.unwrap().to_f64().is_finite()
            }
            ParamCheck::Primitive(code) => boundary_coerce(py, val, *code).is_ok(),
            ParamCheck::Nominal(name) => self.check_nominal(py, val, name).is_ok(),
            ParamCheck::Union(members) => self.check_union(py, val, members).is_ok(),
            ParamCheck::Composite { .. } => self.check_composite(py, val, check).is_ok(),
            ParamCheck::Generic { .. } => self.check_generic(py, val, check).is_ok(),
            ParamCheck::Callable { arity } => self.check_callable(py, val, *arity).is_ok(),
        }
    }

    /// Enforce the declared field types at struct construction. Two passes mirror
    /// the PureVM constructor: (1) validate every field read-only, so a mismatch
    /// returns `Err` before any mutation (the surrounding handler releases the
    /// operands, as for every other construction error); (2) coerce primitive
    /// fields (numeric tower). Validated first, pass 2 cannot fail mid-coercion,
    /// so a bigint replaced by a float (the only refcount mutation) is never on an
    /// error path. `field_values` is indexed in lockstep with `ty.fields`.
    fn enforce_field_types(&self, py: Python<'_>, type_id: StructTypeId, field_values: &mut [Value]) -> VMResult<()> {
        use catnip_core::vm::opcode::ParamCheck;
        {
            let ty = self.struct_registry.get_type(type_id).unwrap();
            for (i, f) in ty.fields.iter().enumerate() {
                if matches!(f.check, ParamCheck::None) {
                    continue;
                }
                let val = field_values[i];
                if !self.field_value_ok(py, val, &f.check) {
                    return Err(VMError::TypeError(format!(
                        "field '{}' of '{}' expects '{}' but got '{}'",
                        f.name,
                        ty.name,
                        catnip_core::vm::opcode::format_param_check(&f.check),
                        self.nominal_value_type_name(py, val)
                    )));
                }
            }
        }
        let ty = self.struct_registry.get_type(type_id).unwrap();
        for (i, f) in ty.fields.iter().enumerate() {
            if let ParamCheck::Primitive(code) = &f.check {
                field_values[i] = boundary_coerce(py, field_values[i], *code).unwrap_or(field_values[i]);
            }
        }
        Ok(())
    }

    fn value_satisfies_py(
        &self,
        py: Python<'_>,
        value: &Bound<'_, PyAny>,
        check: &catnip_core::vm::opcode::ParamCheck,
    ) -> bool {
        use catnip_core::vm::opcode::{ParamCheck, primitive_membership};
        match check {
            ParamCheck::None => true,
            ParamCheck::Primitive(code) => primitive_membership(*code, &value_primitive_class_pyany(value)),
            ParamCheck::Nominal(name) => {
                value_is_member_of_pyany(value, py, name) || !self.name_is_known_nominal(py, name)
            }
            ParamCheck::Union(members) => members.iter().any(|m| self.value_satisfies_py(py, value, m)),
            ParamCheck::Composite { .. } => self.check_composite_py(py, value, check).is_ok(),
            ParamCheck::Generic { .. } => self.check_generic_py(py, value, check).is_ok(),
            // A PyAny element under a function-type check: same introspection
            // as the prologue (VMFunction/constructor arity when readable).
            ParamCheck::Callable { arity } => self.check_callable_pyany(value, *arity).is_ok(),
        }
    }

    /// Boundary check for a generic-nominal param (`CheckGeneric`, `Option[int]`).
    /// Delegates to the `PyAny` form (a union variant is a `CatnipStructProxy` in
    /// this VM), mirroring `check_composite` -> `check_composite_py`.
    fn check_generic(&self, py: Python<'_>, val: Value, spec: &catnip_core::vm::opcode::ParamCheck) -> VMResult<()> {
        let obj = val.to_pyobject(py);
        self.check_generic_py(py, obj.bind(py), spec)
    }

    /// Generic-nominal boundary check on a Python value: union membership (same
    /// rule as `CheckNominal`, an unknown union name is inert) plus the parametric
    /// payload substitution. A payload variant is a `CatnipStructProxy` carrying
    /// `field_values`; its per-field [`FieldTemplate`]s come from the AST-built
    /// `CatnipStructType` (`struct_type`) or, for a native-backed proxy, the native
    /// struct registry (via `native_instance_idx`). Each `Param(k)` field is
    /// checked against `args[k]`, each `Fixed` against its own check, via
    /// [`Self::value_satisfies_py`]. A nullary variant (an enum-variant object) has
    /// no payload and passes. No coercion, read-only.
    fn check_generic_py(
        &self,
        py: Python<'_>,
        value: &Bound<'_, PyAny>,
        spec: &catnip_core::vm::opcode::ParamCheck,
    ) -> VMResult<()> {
        use crate::vm::structs::CatnipStructProxy;
        use catnip_core::vm::opcode::{FieldTemplate, ParamCheck, format_param_check};
        let ParamCheck::Generic { name, args } = spec else {
            return Ok(());
        };
        // Union membership (mirror `check_nominal`).
        if !value_is_member_of_pyany(value, py, name) {
            if self.name_is_known_nominal(py, name) {
                return Err(VMError::TypeError(format!(
                    "typed parameter expects '{}' but got '{}'",
                    format_param_check(spec),
                    nominal_value_type_name_pyany(value)
                )));
            }
            return Ok(()); // unknown union -> inert
        }
        // Only a payload variant (a proxy) has fields to substitute; a nullary
        // variant object carries nothing.
        let Ok(proxy) = value.cast::<CatnipStructProxy>() else {
            return Ok(());
        };
        let p = proxy.borrow();
        // Templates: AST-built proxies carry the CatnipStructType; native-backed
        // proxies resolve them from the native registry via their instance index.
        let templates: Vec<FieldTemplate> = if let Some(ref st) = p.struct_type {
            st.bind(py).borrow().field_templates.clone()
        } else if let Some(idx) = p.native_instance_idx {
            match self
                .struct_registry
                .with_instance(idx, |inst| inst.type_id)
                .and_then(|tid| self.struct_registry.variant_templates(tid).map(<[_]>::to_vec))
            {
                Some(t) => t,
                None => return Ok(()),
            }
        } else {
            return Ok(()); // no templates -> membership-only
        };
        for (i, tmpl) in templates.iter().enumerate() {
            let Some(fval) = p.field_values.get(i) else { break };
            let required: Option<&ParamCheck> = match tmpl {
                FieldTemplate::Param(k) => args.get(*k),
                FieldTemplate::Fixed(c) => Some(c),
            };
            if let Some(check) = required {
                if !matches!(check, ParamCheck::None) && !self.value_satisfies_py(py, fval.bind(py), check) {
                    return Err(VMError::TypeError(format!(
                        "typed parameter expects '{}' but a payload field has the wrong type",
                        format_param_check(spec)
                    )));
                }
            }
        }
        Ok(())
    }

    /// True if the struct named `child` has `name` in its transitive `extends`
    /// chain. Walks struct parents only -- never traits -- so it stays correct
    /// even when a struct and a trait share a name. Single inheritance keeps the
    /// chain acyclic.
    fn struct_extends(&self, child: &str, name: &str) -> bool {
        let parents = match self.struct_registry.find_type_by_name(child) {
            Some(ty) => ty.parent_names.clone(),
            None => return false,
        };
        parents.iter().any(|p| p == name || self.struct_extends(p, name))
    }

    /// True if `val` is a member of the nominal type `name`. Covers native
    /// struct instances (name, extends chain, direct traits, tagged-union payload
    /// prefix), enum/union nullary symbols, struct proxies, and enum-variant
    /// PyObjects.
    fn value_is_nominal_member(&self, py: Python<'_>, val: Value, name: &str) -> bool {
        // Native struct instance: exact name, MRO, implemented traits, or a
        // tagged-union payload variant whose type name is "Union.Variant".
        if let Some(idx) = val.as_struct_instance_idx() {
            let type_id = match self.struct_registry.with_instance(idx, |inst| inst.type_id) {
                Some(type_id) => type_id,
                None => return false,
            };
            if let Some(ty) = self.struct_registry.get_type(type_id) {
                // A struct ancestor (via `extends`) or a direct trait
                // (`implements`). The native mro mixes struct ancestors with the
                // trait linearization, so resolve struct ancestry through the
                // extends chain instead -- correct even when a struct and a trait
                // share a name -- and keep traits direct-only (no transitive trait
                // subtyping), for parity with AST/PureVM.
                return ty.name == name
                    || ty.implements.iter().any(|n| n == name)
                    || ty.name.split_once('.').map(|(u, _)| u) == Some(name)
                    || self.struct_extends(&ty.name, name);
            }
            return false;
        }
        // Enum / union nullary symbol: resolve to "Type.variant", split on '.',
        // compare the type prefix to `name`.
        if let Some(sym) = val.as_symbol() {
            if let Some(full) = self.symbol_table.resolve(sym) {
                let tyname = full.split_once('.').map(|(t, _)| t).unwrap_or(full);
                return tyname == name;
            }
        }
        // Python-side representations: struct proxy or enum-variant object.
        let obj = val.to_pyobject(py);
        let bound = obj.bind(py);
        // CatnipStructProxy: exact name or tagged-union payload prefix, then
        // subtyping via the type back-reference. The proxy's CatnipStructType mro
        // is struct-only (AST-built), so a plain ancestor match is correct; direct
        // traits come from `implements`.
        if let Ok(proxy) = bound.cast::<crate::vm::structs::CatnipStructProxy>() {
            let p = proxy.borrow();
            if p.type_name == name || p.type_name.split_once('.').map(|(u, _)| u) == Some(name) {
                return true;
            }
            if let Some(ref st) = p.struct_type {
                let st = st.bind(py).borrow();
                return st.implements.iter().any(|n| n == name)
                    || st.parent_names.iter().any(|n| n == name)
                    || st.mro.iter().any(|n| n == name);
            }
            return false;
        }
        // CatnipEnumVariant object: compare its declaring enum name.
        if let Ok(variant) = bound.cast::<crate::vm::enums::CatnipEnumVariant>() {
            return variant.borrow().enum_name == name;
        }
        false
    }

    /// True if `name` is a struct, enum, tagged union, or trait defined at
    /// runtime. Decides whether a non-member is a type error (known nominal) or
    /// an inert annotation (unknown name -> no-op). A trait annotation accepts a
    /// struct that implements it (via `value_is_nominal_member`); recognizing the
    /// trait here makes a non-implementer a type error rather than a no-op.
    /// Globals are inspected through a borrowed `&Value`, so no refcount is
    /// touched here.
    fn name_is_known_nominal(&self, py: Python<'_>, name: &str) -> bool {
        if self.struct_registry.find_type_by_name(name).is_some()
            || self.enum_registry.find_by_name(name).is_some()
            || self.trait_registry.find_trait(name).is_some()
        {
            return true;
        }
        // Union types live in the globals as a CatnipUnionType namespace object.
        if let Some(&value) = self.globals.get(name) {
            if value.is_pyobj() {
                let obj = value.to_pyobject(py);
                if obj.bind(py).cast::<crate::vm::unions::CatnipUnionType>().is_ok() {
                    return true;
                }
            }
        }
        false
    }

    /// Best-effort runtime type name of `val` for a boundary error message: the
    /// nominal type name for a struct/enum/union value, else a generic name.
    fn nominal_value_type_name(&self, py: Python<'_>, val: Value) -> String {
        if let Some(idx) = val.as_struct_instance_idx() {
            if let Some(name) = self
                .struct_registry
                .with_instance(idx, |inst| inst.type_id)
                .and_then(|type_id| self.struct_registry.get_type(type_id).map(|ty| ty.name.clone()))
            {
                return name;
            }
        }
        if let Some(sym) = val.as_symbol() {
            if let Some(full) = self.symbol_table.resolve(sym) {
                return full.to_string();
            }
        }
        let obj = val.to_pyobject(py);
        let bound = obj.bind(py);
        if let Ok(proxy) = bound.cast::<crate::vm::structs::CatnipStructProxy>() {
            return proxy.borrow().type_name.clone();
        }
        if let Ok(variant) = bound.cast::<crate::vm::enums::CatnipEnumVariant>() {
            let v = variant.borrow();
            return qualified_name(&v.enum_name, &v.variant_name);
        }
        bound
            .get_type()
            .name()
            .ok()
            .and_then(|n| n.extract::<String>().ok())
            .unwrap_or_else(|| "object".to_string())
    }
}

impl Default for VM {
    fn default() -> Self {
        Self::new()
    }
}

mod value_ops;
pub(crate) use value_ops::*;

#[cfg(test)]
mod tests;
