//! Free functions operating on VM Values: arithmetic, comparison, bitwise,
//! struct binary ops, and pattern matching. Extracted from the VM core module.

use super::*;

// --- Binary operations on NaN-boxed values ---

use catnip_vm::ops::errors;

// Generic bodies live in catnip_core::arith (Phase 5, step B), monomorphized
// for this crate's Value; ArithError maps into the PyO3-side VMError here.
pub(crate) use catnip_core::arith::{bigint_binop, eq_scalar, to_bigint};

#[inline]
fn map_arith(e: catnip_core::arith::ArithError) -> VMError {
    match e {
        catnip_core::arith::ArithError::Type(m) => VMError::TypeError(m.into()),
        catnip_core::arith::ArithError::ZeroDivision(m) => VMError::ZeroDivisionError(m.into()),
    }
}

/// Compare two Values in Rust when possible.
/// Returns None if the comparison requires Python (PyObject with custom __eq__).
#[inline]
pub(crate) fn eq_without_python(a: Value, b: Value) -> Option<bool> {
    // Bitwise identity for non-float, non-pyobj tags (a PyObject may have a
    // custom __eq__; floats: NaN != NaN).
    if a.bits() == b.bits() && !a.is_pyobj() && !a.is_float() {
        return Some(true);
    }
    eq_scalar(a, b)
}

macro_rules! arith_wrapper {
    ($local:ident, $core:ident) => {
        #[inline]
        pub(crate) fn $local(a: Value, b: Value) -> VMResult<Value> {
            catnip_core::arith::$core(a, b).map_err(map_arith)
        }
    };
}

arith_wrapper!(binary_add, numeric_add);
arith_wrapper!(binary_sub, numeric_sub);
arith_wrapper!(binary_mul, numeric_mul);
arith_wrapper!(binary_div, numeric_div);
arith_wrapper!(binary_floordiv, numeric_floordiv);
arith_wrapper!(binary_mod, numeric_mod);
arith_wrapper!(binary_pow, numeric_pow);
arith_wrapper!(compare_lt, numeric_lt);
arith_wrapper!(compare_le, numeric_le);
arith_wrapper!(compare_gt, numeric_gt);
arith_wrapper!(compare_ge, numeric_ge);

#[inline]
pub(crate) fn unary_neg(a: Value) -> VMResult<Value> {
    catnip_core::arith::numeric_neg(a).map_err(map_arith)
}

// --- Comparison operations ---

// --- Struct operator dispatch ---

/// Try to dispatch a binary operator on a struct instance.
/// Returns Some((code, closure, args)) if the struct has the method, None otherwise.
pub(crate) fn try_struct_binop(
    registry: &StructRegistry,
    py: Python<'_>,
    a: Value,
    b: Value,
    method_name: &str,
) -> Option<(Arc<CodeObject>, Option<NativeClosureScope>, Vec<Value>)> {
    let idx = a.as_struct_instance_idx()?;
    let type_id = registry.with_instance(idx, |inst| inst.type_id)?;
    let ty = registry.get_type(type_id)?;
    let func = ty.methods.get(method_name)?;
    let func_bound = func.bind(py);
    if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
        let r = vm_func.borrow();
        let code = Arc::clone(&r.vm_code.borrow(py).inner);
        let closure = r.native_closure.clone();
        drop(r);
        Some((code, closure, vec![a, b]))
    } else {
        None
    }
}

/// Try reverse dispatch: look up method on `b` (right operand) when `a` lacks it.
/// Args are passed as (b, a) - the struct stays as self.
pub(crate) fn try_struct_rbinop(
    registry: &StructRegistry,
    py: Python<'_>,
    a: Value,
    b: Value,
    method_name: &str,
) -> Option<(Arc<CodeObject>, Option<NativeClosureScope>, Vec<Value>)> {
    let idx = b.as_struct_instance_idx()?;
    let type_id = registry.with_instance(idx, |inst| inst.type_id)?;
    let ty = registry.get_type(type_id)?;
    let func = ty.methods.get(method_name)?;
    let func_bound = func.bind(py);
    if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
        let r = vm_func.borrow();
        let code = Arc::clone(&r.vm_code.borrow(py).inner);
        let closure = r.native_closure.clone();
        drop(r);
        Some((code, closure, vec![b, a]))
    } else {
        None
    }
}

/// Try to dispatch a unary operator on a struct instance.
pub(crate) fn try_struct_unaryop(
    registry: &StructRegistry,
    py: Python<'_>,
    a: Value,
    method_name: &str,
) -> Option<(Arc<CodeObject>, Option<NativeClosureScope>, Vec<Value>)> {
    let idx = a.as_struct_instance_idx()?;
    let type_id = registry.with_instance(idx, |inst| inst.type_id)?;
    let ty = registry.get_type(type_id)?;
    let func = ty.methods.get(method_name)?;
    let func_bound = func.bind(py);
    if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
        let r = vm_func.borrow();
        let code = Arc::clone(&r.vm_code.borrow(py).inner);
        let closure = r.native_closure.clone();
        drop(r);
        Some((code, closure, vec![a]))
    } else {
        None
    }
}

// --- Bitwise operations ---

#[inline]
pub(crate) fn bitwise_or(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_int(ai | bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x | y)) {
            return Ok(v);
        }
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_BITOR.into()))
}

#[inline]
pub(crate) fn bitwise_xor(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_int(ai ^ bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x ^ y)) {
            return Ok(v);
        }
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_BITXOR.into()))
}

#[inline]
pub(crate) fn bitwise_and(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_int(ai & bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x & y)) {
            return Ok(v);
        }
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_BITAND.into()))
}

#[inline]
pub(crate) fn bitwise_not(a: Value) -> VMResult<Value> {
    if let Some(i) = a.as_int() {
        return Ok(Value::from_int(!i));
    }
    if a.is_bigint() {
        // SAFETY: a is alive (caller owns the Value); is_bigint() checked the tag.
        let n = unsafe { a.as_bigint_ref().unwrap() };
        return Ok(Value::from_bigint_or_demote(Integer::from(!n)));
    }
    Err(VMError::TypeError(errors::ERR_BAD_UNARY_NOT.into()))
}

#[inline]
pub(crate) fn bitwise_lshift(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if bi >= 0 {
            if bi < 64 {
                if let Some(v) = Value::try_from_int(ai << bi) {
                    return Ok(v);
                }
            }
            // Overflow or large shift: promote to BigInt
            if let Ok(shift) = u32::try_from(bi) {
                return Ok(Value::from_bigint_or_demote(Integer::from(ai) << shift));
            }
        }
    }
    if a.is_bigint() || b.is_bigint() {
        if let (Some(ba), Some(bi)) = (to_bigint(a), b.as_int()) {
            if bi >= 0 {
                if let Ok(shift) = u32::try_from(bi) {
                    return Ok(Value::from_bigint_or_demote(ba << shift));
                }
            }
        }
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_LSHIFT.into()))
}

#[inline]
pub(crate) fn bitwise_rshift(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if (0..64).contains(&bi) {
            return Ok(Value::from_int(ai >> bi));
        }
    }
    if a.is_bigint() || b.is_bigint() {
        if let (Some(ba), Some(bi)) = (to_bigint(a), b.as_int()) {
            if bi >= 0 {
                if let Ok(shift) = u32::try_from(bi) {
                    return Ok(Value::from_bigint_or_demote(ba >> shift));
                }
            }
        }
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_RSHIFT.into()))
}

#[inline]
pub(crate) fn compare_eq(py: Python<'_>, a: Value, b: Value) -> VMResult<Value> {
    if let Some(r) = eq_without_python(a, b) {
        return Ok(Value::from_bool(r));
    }
    // For PyObjects (lists, strings, etc.), delegate to Python's ==
    inc(&PY_COMPARE_EQ_FALLBACKS);
    let py_a = a.to_pyobject(py);
    let py_b = b.to_pyobject(py);
    let result = py_a.bind(py).eq(&py_b).to_vm(py)?;
    Ok(Value::from_bool(result))
}

#[inline]
pub(crate) fn compare_ne(py: Python<'_>, a: Value, b: Value) -> VMResult<Value> {
    if let Some(r) = eq_without_python(a, b) {
        return Ok(Value::from_bool(!r));
    }
    // For PyObjects, delegate to Python's !=
    inc(&PY_COMPARE_NE_FALLBACKS);
    let py_a = a.to_pyobject(py);
    let py_b = b.to_pyobject(py);
    let result = py_a.bind(py).ne(&py_b).to_vm(py)?;
    Ok(Value::from_bool(result))
}

/// Hand partially-collected owned bindings to the spill when a compound
/// pattern bails mid-way (a later item mismatches or errors); the op drains
/// the spill via decref_discard.
fn spill_bindings(bindings: Vec<(usize, Value)>, spill: &mut Vec<Value>) {
    spill.extend(bindings.into_iter().map(|(_, v)| v));
}

/// Match a VMPattern against a Value entirely in Rust (no Python boundary crossing).
/// Returns Some(bindings) with (slot, value) pairs on match, None on mismatch.
///
/// Ownership: the matcher BORROWS `value`; every returned binding owns its own
/// ref (Var clones the subject, struct fields are cloned, the star list is a
/// fresh value). Locally-owned intermediates -- tuple items, partial bindings
/// of a compound pattern that bails -- are pushed to `spill`, which the op
/// drains via decref_discard on every path (match, mismatch, error). The
/// registry stays shared (`&`): struct increfs go through the thread-local
/// path (clone_refcount); a `&mut` here would alias them.
#[allow(clippy::too_many_arguments)] // scope resolution + ownership spill, single call site
pub(crate) fn vm_match_pattern(
    py: Python<'_>,
    pattern: &VMPattern,
    value: Value,
    registry: &StructRegistry,
    globals: &IndexMap<String, Value>,
    host: &dyn crate::vm::host::VmHost,
    closure: &Option<NativeClosureScope>,
    spill: &mut Vec<Value>,
) -> PyResult<Option<Vec<(usize, Value)>>> {
    match pattern {
        VMPattern::Wildcard => Ok(Some(Vec::new())),
        VMPattern::Var(slot) => {
            // The binding takes its own ref; the subject stays owned by the op.
            value.clone_refcount();
            Ok(Some(vec![(*slot, value)]))
        }
        VMPattern::Literal(expected) => {
            // Fast path: compare in Rust for primitive and BigInt values.
            if let Some(eq) = eq_without_python(value, *expected) {
                return if eq { Ok(Some(Vec::new())) } else { Ok(None) };
            }
            // Pointer/bits equality still short-circuits for reference-like payloads.
            if value.bits() == expected.bits() {
                return Ok(Some(Vec::new()));
            }
            // Fallback: Python equality for strings, PyObj, etc.
            inc(&PY_PATTERN_LITERAL_EQ_FALLBACKS);
            let py_val = value.to_pyobject(py);
            let py_exp = expected.to_pyobject(py);
            if py_val.bind(py).eq(py_exp.bind(py))? {
                Ok(Some(Vec::new()))
            } else {
                Ok(None)
            }
        }
        VMPattern::Or(sub_patterns) => {
            for sub in sub_patterns {
                if let Some(bindings) = vm_match_pattern(py, sub, value, registry, globals, host, closure, spill)? {
                    return Ok(Some(bindings));
                }
            }
            Ok(None)
        }
        VMPattern::Tuple(elements) => {
            // Convert value to Python iterable then to items
            let py_val = value.to_pyobject(py);
            let py_bound = py_val.bind(py);
            let items: Vec<Value> = match py_bound.try_iter() {
                Ok(iter) => {
                    let mut v = Vec::new();
                    for item in iter {
                        v.push(Value::from_pyobject(py, &item?)?);
                    }
                    v
                }
                Err(_) => return Ok(None),
            };
            // The items are owned from_pyobject refs. The spill owns them for
            // the whole match -- sub-patterns borrow, bindings clone -- so an
            // item no binding consumes (wildcard/literal sub-pattern, star
            // coverage, mismatch) is still released by the op.
            spill.extend(items.iter().copied());

            // Sub-match or bail: on a mismatch or an error the owned bindings
            // already collected for earlier items are handed to the spill.
            macro_rules! sub_or_bail {
                ($sub:expr, $item:expr, $bindings:expr) => {
                    match vm_match_pattern(py, $sub, $item, registry, globals, host, closure, spill) {
                        Ok(Some(sub_bindings)) => $bindings.extend(sub_bindings),
                        Ok(None) => {
                            spill_bindings($bindings, spill);
                            return Ok(None);
                        }
                        Err(e) => {
                            spill_bindings($bindings, spill);
                            return Err(e);
                        }
                    }
                };
            }

            // Find star element if present
            let mut star_idx: Option<usize> = None;
            let mut non_star_count = 0;
            for (i, elem) in elements.iter().enumerate() {
                match elem {
                    VMPatternElement::Star(_) => {
                        if star_idx.is_some() {
                            return Ok(None); // Multiple stars
                        }
                        star_idx = Some(i);
                    }
                    VMPatternElement::Pattern(_) => non_star_count += 1,
                }
            }

            let mut bindings = Vec::new();

            if star_idx.is_none() {
                // No star: exact length required
                if items.len() != non_star_count {
                    return Ok(None);
                }
                for (i, elem) in elements.iter().enumerate() {
                    if let VMPatternElement::Pattern(sub) = elem {
                        sub_or_bail!(sub, items[i], bindings);
                    }
                }
            } else if let Some(star_pos) = star_idx {
                if items.len() < non_star_count {
                    return Ok(None);
                }

                let n_before = star_pos;
                let n_after = elements.len() - star_pos - 1;

                // Match before star
                let mut item_idx = 0;
                for elem in &elements[..star_pos] {
                    if let VMPatternElement::Pattern(sub) = elem {
                        sub_or_bail!(sub, items[item_idx], bindings);
                        item_idx += 1;
                    }
                }

                // Match after star (from end)
                let after_start = items.len() - n_after;
                for (i, elem) in elements[(star_pos + 1)..].iter().enumerate() {
                    if let VMPatternElement::Pattern(sub) = elem {
                        sub_or_bail!(sub, items[after_start + i], bindings);
                    }
                }

                // Bind star variable (a fresh owned list value)
                if let VMPatternElement::Star(slot) = &elements[star_pos] {
                    if *slot != usize::MAX {
                        let star_items: Vec<Py<PyAny>> =
                            items[n_before..after_start].iter().map(|v| v.to_pyobject(py)).collect();
                        let star_val =
                            PyList::new(py, &star_items).and_then(|l| Value::from_pyobject(py, &l.into_any()));
                        match star_val {
                            Ok(v) => bindings.push((*slot, v)),
                            Err(e) => {
                                spill_bindings(bindings, spill);
                                return Err(e);
                            }
                        }
                    }
                }
            }

            Ok(Some(bindings))
        }
        VMPattern::Struct {
            name,
            variant,
            field_slots,
        } => {
            let expected = match variant {
                Some(v) => qualified_name(name, v),
                None => name.clone(),
            };

            // Native struct path: direct field access via registry
            if let Some(idx) = value.as_struct_instance_idx() {
                let (type_id, fields) = registry.with_instance(idx, |i| (i.type_id, i.fields.clone())).unwrap();
                let ty = registry.get_type(type_id).unwrap();
                if ty.name != expected {
                    return Ok(None);
                }
                let mut bindings = Vec::new();
                for (field_name, slot) in field_slots {
                    match ty.field_index(field_name) {
                        Some(field_idx) => {
                            // Own the field: the struct instance keeps its own ref,
                            // so the binding (transferred into a slot by BindMatch,
                            // released at teardown) must hold an independent one. A
                            // borrowed copy would double-free against the instance.
                            let fval = fields[field_idx];
                            fval.clone_refcount();
                            bindings.push((*slot, fval));
                        }
                        None => {
                            spill_bindings(bindings, spill);
                            return Ok(None);
                        }
                    }
                }
                return Ok(Some(bindings));
            }

            // Python path (PyObject structs)
            let py_val = value.to_pyobject(py);
            let py_bound = py_val.bind(py);

            // CatnipStructProxy: use type_name field, not Python class name
            if let Ok(proxy) = py_bound.cast::<crate::vm::structs::CatnipStructProxy>() {
                let p = proxy.borrow();
                if p.type_name != expected {
                    return Ok(None);
                }
                let mut bindings = Vec::new();
                for (field_name, slot) in field_slots {
                    match p.field_names.iter().position(|n| n == field_name.as_str()) {
                        Some(i) => match Value::from_pyobject(py, p.field_values[i].bind(py)) {
                            Ok(val) => bindings.push((*slot, val)),
                            Err(e) => {
                                spill_bindings(bindings, spill);
                                return Err(e);
                            }
                        },
                        None => {
                            spill_bindings(bindings, spill);
                            return Ok(None);
                        }
                    }
                }
                return Ok(Some(bindings));
            }

            // Generic Python object path
            let value_type_name: String = py_bound.get_type().name()?.extract()?;
            if value_type_name != expected {
                return Ok(None);
            }

            let mut bindings = Vec::new();
            for (field_name, slot) in field_slots {
                let field_value = match py_bound.getattr(field_name.as_str()) {
                    Ok(v) => v,
                    Err(_) => {
                        spill_bindings(bindings, spill);
                        return Ok(None);
                    }
                };
                match Value::from_pyobject(py, &field_value) {
                    Ok(val) => bindings.push((*slot, val)),
                    Err(e) => {
                        spill_bindings(bindings, spill);
                        return Err(e);
                    }
                }
            }
            Ok(Some(bindings))
        }
        VMPattern::Enum {
            enum_name,
            variant_name,
        } => {
            // The enum type must be resolvable in scope, exactly like the AST
            // executor (which resolves `enum_name` as an identifier and raises
            // on a miss). Matching by interned qualified name alone would let a
            // pattern match a type that was never imported into scope -- a
            // silent divergence from the AST. Mirror the VM's LoadScope
            // resolution order (captured closure chain -> VM globals -> host
            // ctx globals): the closure chain reaches an enclosing module's
            // globals (a module method matching its own enum), self.globals
            // holds top-level aliases, the host holds wild imports. Raise
            // NameError so both executors agree.
            let in_scope = closure
                .as_ref()
                .is_some_and(|c| c.contains_with_py(py, enum_name.as_str()))
                || globals.contains_key(enum_name.as_str())
                || host.has_global(py, enum_name.as_str());
            if !in_scope {
                return Err(pyo3::exceptions::PyNameError::new_err(enum_name.clone()));
            }
            // Resolve the expected symbol by looking up "EnumName.variant" in the SymbolTable
            let qname = qualified_name(enum_name, variant_name);
            if let Some(expected_sym) = resolve_symbol_by_name(&qname) {
                let expected = Value::from_symbol(expected_sym);
                if value.to_raw() == expected.to_raw() {
                    Ok(Some(Vec::new()))
                } else {
                    Ok(None)
                }
            } else {
                // Fallback: compare via Python
                let py_value = value.to_pyobject(py);
                let expected_str = qname.into_pyobject(py).unwrap().into_any();
                if py_value.bind(py).eq(&expected_str)? {
                    Ok(Some(Vec::new()))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

/// Strict variant for assignment unpacking patterns.
/// Unlike `vm_match_pattern`, mismatches are reported as concrete unpacking errors.
///
/// Same ownership contract as `vm_match_pattern`: the matcher BORROWS `value`,
/// every returned binding owns its own ref (`Var` clones), and locally-owned
/// intermediates (decoded items, partial bindings of a failed compound
/// pattern) go to `spill`, drained by the op on every path including errors.
pub(crate) fn vm_match_assign_pattern(
    py: Python<'_>,
    pattern: &VMPattern,
    value: Value,
    registry: &StructRegistry,
    spill: &mut Vec<Value>,
) -> VMResult<Vec<(usize, Value)>> {
    match pattern {
        VMPattern::Var(slot) => {
            // The binding takes its own ref; the subject stays owned by the op.
            value.clone_refcount();
            Ok(vec![(*slot, value)])
        }
        VMPattern::Wildcard => Ok(Vec::new()),
        VMPattern::Tuple(elements) => {
            let py_val = value.to_pyobject(py);
            let py_bound = py_val.bind(py);
            let items: Vec<Value> = match py_bound.try_iter() {
                Ok(iter) => {
                    let mut v = Vec::new();
                    for item in iter {
                        match item.and_then(|i| Value::from_pyobject(py, &i)) {
                            Ok(val) => v.push(val),
                            Err(e) => {
                                // Items decoded so far are owned; abandon them
                                // to the spill before surfacing the error.
                                spill.extend(v);
                                return Err(e).to_vm(py);
                            }
                        }
                    }
                    v
                }
                Err(_) => {
                    let ty = py_bound
                        .get_type()
                        .name()
                        .map(|n| n.to_string())
                        .unwrap_or_else(|_| "value".to_string());
                    return Err(VMError::TypeError(format!("Cannot unpack non-iterable {}", ty)));
                }
            };
            // The spill owns the decoded items for the whole match (bindings
            // clone); the op releases them on every path -- including an item
            // only consumed by a star or a compound sub-pattern.
            spill.extend(items.iter().copied());

            // Sub-match or bail: on an unpacking error the owned bindings
            // already collected for earlier items are handed to the spill.
            macro_rules! sub_or_bail {
                ($sub:expr, $item:expr, $bindings:expr) => {
                    match vm_match_assign_pattern(py, $sub, $item, registry, spill) {
                        Ok(sub_bindings) => $bindings.extend(sub_bindings),
                        Err(e) => {
                            spill_bindings($bindings, spill);
                            return Err(e);
                        }
                    }
                };
            }

            let mut star_idx: Option<usize> = None;
            let mut non_star_count = 0usize;
            for (i, elem) in elements.iter().enumerate() {
                match elem {
                    VMPatternElement::Star(_) => {
                        if star_idx.is_some() {
                            return Err(VMError::RuntimeError("Cannot unpack assignment pattern".to_string()));
                        }
                        star_idx = Some(i);
                    }
                    VMPatternElement::Pattern(_) => non_star_count += 1,
                }
            }

            let mut bindings = Vec::new();
            if let Some(star_pos) = star_idx {
                if items.len() < non_star_count {
                    return Err(VMError::RuntimeError(format!(
                        "Not enough values to unpack: expected at least {}, got {}",
                        non_star_count,
                        items.len()
                    )));
                }
                let n_before = star_pos;
                let n_after = elements.len() - star_pos - 1;
                let after_start = items.len() - n_after;

                for (i, elem) in elements[..star_pos].iter().enumerate() {
                    if let VMPatternElement::Pattern(sub) = elem {
                        sub_or_bail!(sub, items[i], bindings);
                    }
                }

                for (i, elem) in elements[(star_pos + 1)..].iter().enumerate() {
                    if let VMPatternElement::Pattern(sub) = elem {
                        sub_or_bail!(sub, items[after_start + i], bindings);
                    }
                }

                if let VMPatternElement::Star(slot) = elements[star_pos] {
                    if slot != usize::MAX {
                        let star_items: Vec<Py<PyAny>> =
                            items[n_before..after_start].iter().map(|v| v.to_pyobject(py)).collect();
                        let star_val =
                            PyList::new(py, &star_items).and_then(|l| Value::from_pyobject(py, &l.into_any()));
                        match star_val {
                            Ok(v) => bindings.push((slot, v)),
                            Err(e) => {
                                spill_bindings(bindings, spill);
                                return Err(e).to_vm(py);
                            }
                        }
                    }
                }
                Ok(bindings)
            } else {
                if items.len() != non_star_count {
                    return Err(VMError::RuntimeError(format!(
                        "Cannot unpack {} values into {} variables",
                        items.len(),
                        non_star_count
                    )));
                }
                for (i, elem) in elements.iter().enumerate() {
                    if let VMPatternElement::Pattern(sub) = elem {
                        sub_or_bail!(sub, items[i], bindings);
                    }
                }
                Ok(bindings)
            }
        }
        VMPattern::Literal(_) | VMPattern::Or(_) | VMPattern::Struct { .. } | VMPattern::Enum { .. } => {
            // Assignment patterns compiled by VM should not produce these nodes.
            // Fallback to generic runtime mismatch.
            let _ = registry;
            Err(VMError::RuntimeError("Cannot unpack assignment pattern".to_string()))
        }
    }
}
