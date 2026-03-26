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
            _ => {}
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
        }
    }
}

impl Value {
    /// Convert Value to a hashable ValueKey. Errors if unhashable (list, dict, set).
    pub fn to_key(self) -> VMResult<ValueKey> {
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
            unsafe { Arc::increment_strong_count(ptr) };
            let arc = unsafe { Arc::from_raw(ptr) };
            return Ok(ValueKey::Str(arc));
        }
        if self.is_bigint() {
            let ptr = (self.to_raw() & PAYLOAD_MASK) as *const GmpInt;
            unsafe { Arc::increment_strong_count(ptr) };
            let arc = unsafe { Arc::from_raw(ptr) };
            return Ok(ValueKey::BigInt(arc));
        }
        if self.is_native_tuple() {
            let t = unsafe { self.as_native_tuple_ref().unwrap() };
            let keys: VMResult<Vec<ValueKey>> = t.as_slice().iter().map(|v| v.to_key()).collect();
            return Ok(ValueKey::Tuple(keys?.into()));
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
        } else if self.is_vmfunc() {
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
