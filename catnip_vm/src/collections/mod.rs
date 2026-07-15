// FILE: catnip_vm/src/collections/mod.rs
//! Native collection types and ValueKey for hashable keys.

pub mod bytes;
pub mod dict;
pub mod list;
pub mod set;
pub mod tuple;

pub use self::bytes::NativeBytes;
pub use self::dict::NativeDict;
pub use self::list::NativeList;
pub use self::set::NativeSet;
pub use self::tuple::NativeTuple;

/// Clamp a slice index to [0, len] (Python-style).
/// Negative indices wrap around; out-of-range values clamp silently.
pub fn clamp_slice_index(index: i64, len: i64) -> usize {
    if index < 0 {
        let i = index + len;
        if i < 0 { 0 } else { i as usize }
    } else if index > len {
        len as usize
    } else {
        index as usize
    }
}

use crate::error::{VMError, VMResult};
use crate::value::{GmpInt, NativeString, TAG_NATIVESTR, Value};
use catnip_core::nanbox::{PAYLOAD_MASK, QNAN_BASE, TAG_BIGINT};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Hashable key for dict/set operations.
///
/// Python-compatible: `hash(1) == hash(1.0) == hash(True)`.
/// Cross-type numeric equality via canonical i64 normalization.
#[derive(Clone, Debug)]
pub enum ValueKey {
    Int(i64),
    Float(u64), // f64 bits
    Bool(bool),
    Nil,
    Symbol(u32),
    Str(Arc<NativeString>),
    BigInt(Arc<GmpInt>),
    Tuple(Arc<[ValueKey]>),
    Complex(u64, u64), // (real bits, imag bits) -- imag != 0.0
    /// Snapshot of a struct instance (and union payload variants, which are
    /// struct instances) hashed as a dict/set key. The fields are frozen at
    /// hash time -- the source instance is locked against mutation -- so the
    /// snapshot stays consistent with structural equality (`deep_eq`).
    ///
    /// `hash_override` carries the result of a custom `op_hash` when the type
    /// defines one; it feeds the hash only. Equality stays structural (on
    /// `type_id` and `fields`), so `a == b` keeps `deep_eq` semantics. Keeping
    /// `op_hash` consistent with that equality (`a == b` => `hash(a) == hash(b)`)
    /// is the user's responsibility, as in the PyO3 runtime.
    Struct {
        type_id: u32,
        fields: Arc<[ValueKey]>,
        hash_override: Option<i64>,
    },
}

/// Try to normalize a numeric ValueKey to i64 for cross-type hashing/equality.
fn numeric_as_i64(key: &ValueKey) -> Option<i64> {
    match key {
        ValueKey::Int(i) => Some(*i),
        ValueKey::Bool(b) => Some(*b as i64),
        ValueKey::Float(bits) => {
            let f = f64::from_bits(*bits);
            if f.is_finite() && f == f.trunc() && f.abs() < (i64::MAX as f64) {
                Some(f as i64)
            } else {
                None
            }
        }
        ValueKey::BigInt(n) => n.0.to_i64(),
        _ => None,
    }
}

fn is_numeric(key: &ValueKey) -> bool {
    matches!(
        key,
        ValueKey::Int(_) | ValueKey::Float(_) | ValueKey::Bool(_) | ValueKey::BigInt(_)
    )
}

impl Hash for ValueKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if is_numeric(self) {
            if let Some(i) = numeric_as_i64(self) {
                // All numeric types that represent the same integer hash identically
                0u8.hash(state);
                i.hash(state);
                return;
            }
        }
        // Non-normalizable: use type-specific hash
        match self {
            ValueKey::Float(bits) => {
                1u8.hash(state);
                bits.hash(state);
            }
            ValueKey::BigInt(n) => {
                // BigInt that doesn't fit i64
                2u8.hash(state);
                n.0.to_string_radix(16).hash(state);
            }
            ValueKey::Nil => 3u8.hash(state),
            ValueKey::Symbol(s) => {
                4u8.hash(state);
                s.hash(state);
            }
            ValueKey::Str(s) => {
                5u8.hash(state);
                s.as_str().hash(state);
            }
            ValueKey::Tuple(elems) => {
                6u8.hash(state);
                elems.hash(state);
            }
            ValueKey::Complex(r_bits, i_bits) => {
                7u8.hash(state);
                r_bits.hash(state);
                i_bits.hash(state);
            }
            ValueKey::Struct {
                type_id,
                fields,
                hash_override,
            } => {
                8u8.hash(state);
                type_id.hash(state);
                // A custom op_hash supersedes the structural field hash; without
                // one the fields drive the hash, matching structural equality.
                match hash_override {
                    Some(h) => h.hash(state),
                    None => fields.hash(state),
                }
            }
            // Int/Bool always handled by numeric_as_i64 above
            _ => unreachable!(),
        }
    }
}

impl PartialEq for ValueKey {
    fn eq(&self, other: &Self) -> bool {
        // Fast path: same variant
        match (self, other) {
            (ValueKey::Nil, ValueKey::Nil) => return true,
            (ValueKey::Symbol(a), ValueKey::Symbol(b)) => return a == b,
            (ValueKey::Str(a), ValueKey::Str(b)) => return a == b,
            (ValueKey::Tuple(a), ValueKey::Tuple(b)) => return a == b,
            (
                ValueKey::Struct {
                    type_id: at,
                    fields: af,
                    ..
                },
                ValueKey::Struct {
                    type_id: bt,
                    fields: bf,
                    ..
                },
            ) => return at == bt && af == bf,
            _ => {}
        }
        // Complex equality
        if let (ValueKey::Complex(ar, ai), ValueKey::Complex(br, bi)) = (self, other) {
            return ar == br && ai == bi;
        }
        // Numeric cross-type equality
        if is_numeric(self) && is_numeric(other) {
            match (numeric_as_i64(self), numeric_as_i64(other)) {
                (Some(a), Some(b)) => return a == b,
                (None, None) => {
                    // Both non-normalizable (non-integer floats or huge BigInts)
                    match (self, other) {
                        (ValueKey::Float(a), ValueKey::Float(b)) => {
                            return f64::from_bits(*a) == f64::from_bits(*b);
                        }
                        (ValueKey::BigInt(a), ValueKey::BigInt(b)) => return a.0 == b.0,
                        _ => return false,
                    }
                }
                _ => return false,
            }
        }
        false
    }
}

impl Eq for ValueKey {}

impl ValueKey {
    /// Convert back to Value.
    pub fn to_value(&self) -> Value {
        match self {
            ValueKey::Int(i) => Value::from_int(*i),
            ValueKey::Bool(b) => Value::from_bool(*b),
            ValueKey::Float(bits) => Value::from_float(f64::from_bits(*bits)),
            ValueKey::Nil => Value::NIL,
            ValueKey::Symbol(s) => Value::from_symbol(*s),
            ValueKey::Str(s) => {
                let arc = Arc::clone(s);
                let ptr = Arc::into_raw(arc) as u64;
                // Reconstruct Value with NativeStr tag
                Value::from_raw(QNAN_BASE | TAG_NATIVESTR | (ptr & PAYLOAD_MASK))
            }
            ValueKey::BigInt(n) => {
                let arc = Arc::clone(n);
                let ptr = Arc::into_raw(arc) as u64;
                Value::from_raw(QNAN_BASE | TAG_BIGINT | (ptr & PAYLOAD_MASK))
            }
            ValueKey::Tuple(elems) => {
                let values: Vec<Value> = elems.iter().map(|k| k.to_value()).collect();
                Value::from_tuple(values)
            }
            ValueKey::Complex(r_bits, i_bits) => Value::from_complex(f64::from_bits(*r_bits), f64::from_bits(*i_bits)),
            // Struct keys hold no live registry handle; rebuilding the instance
            // needs the registry, so materialization goes through
            // `PureVM::key_to_value`. Reaching here means a struct key escaped
            // that path -- a routing bug.
            ValueKey::Struct { .. } => {
                unreachable!("ValueKey::Struct must be materialized via PureVM::key_to_value")
            }
        }
    }
}

/// Context for turning struct instances into hashable keys. Abstracts the
/// registry queries plus the synchronous `op_hash` invocation, so the recursion
/// in `to_key_impl` can honor a custom `op_hash` at every nesting level: the
/// PyO3 runtime hashes each field through `hash()`, which dispatches to that
/// field's `op_hash`, and this keeps the pure runtime in step. Implemented by
/// the VM (`KeyBuilder`), which alone can run a method and reach the host.
/// `to_key` (no context) rejects structs, matching the host paths that have no
/// registry.
pub trait KeyCtx {
    /// True if the type defines a custom `op_eq` (=> unhashable).
    fn type_defines_op_eq(&self, type_id: u32) -> bool;
    /// Func-table index of the type's custom `op_hash`, if any.
    fn type_op_hash_func(&self, type_id: u32) -> Option<u32>;
    /// Type name for error messages.
    fn type_name(&self, type_id: u32) -> String;
    /// Run `op_hash` synchronously on the instance, returning its int result.
    fn call_op_hash(&mut self, func_idx: u32, instance: Value) -> VMResult<i64>;
}

impl Value {
    /// Convert Value to a hashable ValueKey without registry access. Errors if
    /// unhashable (list, dict, set, or any struct -- struct keys need a
    /// `KeyCtx`, see `to_key_ctx`).
    pub fn to_key(self) -> VMResult<ValueKey> {
        self.to_key_impl(None)
    }

    /// Like `to_key`, but with a `KeyCtx` so struct instances (and union payload
    /// variants) can be hashed: the fields are snapshotted into the key and the
    /// source instance is frozen against mutation. A struct whose type defines a
    /// custom `op_eq` is rejected as unhashable (the "op_eq without op_hash"
    /// rule); a custom `op_hash` is honored at every nesting level.
    pub fn to_key_ctx(self, ctx: &mut dyn KeyCtx) -> VMResult<ValueKey> {
        self.to_key_impl(Some(ctx))
    }

    // The trait-object lifetime `'a` is decoupled from the reference lifetime so
    // `ctx.as_deref_mut()` reborrows can be released between loop iterations.
    fn to_key_impl<'a>(self, mut ctx: Option<&mut (dyn KeyCtx + 'a)>) -> VMResult<ValueKey> {
        if let Some(i) = self.as_int() {
            return Ok(ValueKey::Int(i));
        }
        if let Some(b) = self.as_bool() {
            return Ok(ValueKey::Bool(b));
        }
        if self.is_nil() {
            return Ok(ValueKey::Nil);
        }
        if let Some(f) = self.as_float() {
            return Ok(ValueKey::Float(f.to_bits()));
        }
        if let Some(s) = self.as_symbol() {
            return Ok(ValueKey::Symbol(s));
        }
        if self.is_native_str() {
            let ptr = (self.to_raw() & PAYLOAD_MASK) as *const NativeString;
            // SAFETY: the is_native_str tag guarantees ptr is a live Arc<NativeString>
            // payload (from Arc::into_raw); the increment accounts for the extra owner
            // materialized by from_raw below, leaving the NaN-box's own reference intact.
            unsafe { Arc::increment_strong_count(ptr) };
            // SAFETY: ptr came from Arc::into_raw on the same NativeString and the strong
            // count was just incremented, so reconstructing one Arc here is balanced.
            let arc = unsafe { Arc::from_raw(ptr) };
            return Ok(ValueKey::Str(arc));
        }
        if self.is_bigint() {
            let ptr = (self.to_raw() & PAYLOAD_MASK) as *const GmpInt;
            // SAFETY: the is_bigint tag guarantees ptr is a live Arc<GmpInt> payload
            // (from Arc::into_raw); the increment accounts for the extra owner
            // materialized by from_raw below, leaving the NaN-box's own reference intact.
            unsafe { Arc::increment_strong_count(ptr) };
            // SAFETY: ptr came from Arc::into_raw on the same GmpInt and the strong count
            // was just incremented, so reconstructing one Arc here is balanced.
            let arc = unsafe { Arc::from_raw(ptr) };
            return Ok(ValueKey::BigInt(arc));
        }
        if self.is_complex() {
            // SAFETY: the is_complex tag was checked above, so the payload holds the
            // two f64 parts; reading them does not outlive `self`.
            let (r, i) = unsafe { self.as_complex_parts().unwrap() };
            if i == 0.0 {
                // Pure real complex hashes like float for cross-type equality
                return Ok(ValueKey::Float(r.to_bits()));
            }
            return Ok(ValueKey::Complex(r.to_bits(), i.to_bits()));
        }
        if self.is_native_tuple() {
            // SAFETY: the is_native_tuple tag was checked above, so the payload is a live
            // Arc<NativeTuple> owned by `self`; the borrow does not outlive it.
            let t = unsafe { self.as_native_tuple_ref().unwrap() };
            let slice = t.as_slice();
            let mut keys = Vec::with_capacity(slice.len());
            for v in slice {
                keys.push(v.to_key_impl(ctx.as_deref_mut())?);
            }
            return Ok(ValueKey::Tuple(keys.into()));
        }
        // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
        if let Some(cell) = unsafe { self.as_struct_ref() } {
            // Without a context (host paths with no registry) a struct stays
            // unhashable, like the catch-all below. type_id / fields / freeze come
            // straight from the cell; only the type-level queries need the ctx.
            let ctx = match ctx {
                Some(c) => c,
                None => return Err(VMError::TypeError(format!("unhashable type: '{}'", self.type_name()))),
            };
            let type_id = cell.type_id;
            if ctx.type_defines_op_eq(type_id) {
                return Err(VMError::TypeError(format!(
                    "unhashable type: '{}' (defines op_eq without op_hash)",
                    ctx.type_name(type_id)
                )));
            }
            // Snapshot fields first (honors a nested op_hash), then the
            // instance's own op_hash, then lock it against mutation.
            let mut fields = Vec::with_capacity(cell.field_count());
            for v in cell.field_values() {
                fields.push(v.to_key_impl(Some(&mut *ctx))?);
            }
            let hash_override = match ctx.type_op_hash_func(type_id) {
                Some(f) => Some(ctx.call_op_hash(f, self)?),
                None => None,
            };
            cell.frozen.set(true);
            return Ok(ValueKey::Struct {
                type_id,
                fields: fields.into(),
                hash_override,
            });
        }
        Err(VMError::TypeError(format!("unhashable type: '{}'", self.type_name())))
    }

    /// Return the type name string for error messages.
    pub fn type_name(&self) -> &'static str {
        if self.is_int() || self.is_bigint() {
            "int"
        } else if self.is_float() {
            "float"
        } else if self.is_bool() {
            "bool"
        } else if self.is_nil() {
            "NoneType"
        } else if self.is_native_str() {
            "str"
        } else if self.is_vmfunc() || self.is_closure() {
            "function"
        } else if self.is_native_list() {
            "list"
        } else if self.is_native_dict() {
            "dict"
        } else if self.is_native_tuple() {
            "tuple"
        } else if self.is_native_set() {
            "set"
        } else if self.is_native_bytes() {
            "bytes"
        } else if self.is_struct_instance() {
            "struct"
        } else if self.is_struct_type() {
            "type"
        } else if self.is_symbol() {
            "enum"
        } else if self.is_module() {
            "module"
        } else if self.is_enum_type() {
            "type"
        } else if self.is_union_type() {
            "union"
        } else if self.is_complex() {
            "complex"
        } else if self.is_meta() {
            "meta"
        } else if self.is_extended() {
            "extended"
        } else {
            "unknown"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valuekey_int_bool_equality() {
        assert_eq!(ValueKey::Int(1), ValueKey::Bool(true));
        assert_eq!(ValueKey::Int(0), ValueKey::Bool(false));
        assert_ne!(ValueKey::Int(2), ValueKey::Bool(true));
    }

    #[test]
    fn test_valuekey_int_float_equality() {
        assert_eq!(ValueKey::Int(1), ValueKey::Float(1.0_f64.to_bits()));
        assert_eq!(ValueKey::Int(0), ValueKey::Float(0.0_f64.to_bits()));
        assert_ne!(ValueKey::Int(1), ValueKey::Float(1.5_f64.to_bits()));
    }

    #[test]
    fn test_valuekey_hash_consistency() {
        use std::collections::hash_map::DefaultHasher;

        fn hash_key(k: &ValueKey) -> u64 {
            let mut h = DefaultHasher::new();
            k.hash(&mut h);
            h.finish()
        }

        // hash(1) == hash(True) == hash(1.0)
        assert_eq!(hash_key(&ValueKey::Int(1)), hash_key(&ValueKey::Bool(true)));
        assert_eq!(
            hash_key(&ValueKey::Int(1)),
            hash_key(&ValueKey::Float(1.0_f64.to_bits()))
        );
        assert_eq!(hash_key(&ValueKey::Int(0)), hash_key(&ValueKey::Bool(false)));
        assert_eq!(
            hash_key(&ValueKey::Int(0)),
            hash_key(&ValueKey::Float(0.0_f64.to_bits()))
        );
    }

    #[test]
    fn test_valuekey_unhashable() {
        let list = Value::from_list(vec![Value::from_int(1)]);
        assert!(list.to_key().is_err());
        list.decref();
    }

    #[test]
    fn test_valuekey_roundtrip() {
        let v = Value::from_int(42);
        let k = v.to_key().unwrap();
        let v2 = k.to_value();
        assert_eq!(v, v2);
    }
}
