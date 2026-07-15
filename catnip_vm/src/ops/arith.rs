// FILE: catnip_vm/src/ops/arith.rs
//! Numeric arithmetic and comparison over the pure `Value`.
//!
//! The generic bodies live in `catnip_core::arith` (Phase 5, step B), shared
//! with catnip_rs; this module monomorphizes them for the pure Value and maps
//! `ArithError` into the crate's `VMError`. `eq_native` adds the pure-side
//! heap extension (NativeStr) on top of the shared scalar core.

use crate::error::{VMError, VMResult};
use crate::value::Value;
use catnip_core::arith::{self as core_arith, ArithError};

// Shared helpers and promotion utilities: generic, usable as-is.
pub use catnip_core::arith::{
    bigint_binop, bigint_cmp, complex_pow as complex_pow_generic, eq_scalar, i64_div_floor, i64_mod_floor, to_bigint,
    to_complex, to_f64,
};

#[inline]
fn map_arith(e: ArithError) -> VMError {
    match e {
        ArithError::Type(m) => VMError::TypeError(m.into()),
        ArithError::ZeroDivision(m) => VMError::ZeroDivisionError(m.into()),
    }
}

/// Complex exponentiation via polar form (De Moivre's theorem).
pub fn complex_pow(ar: f64, ai: f64, br: f64, bi: f64) -> VMResult<Value> {
    core_arith::complex_pow::<Value>(ar, ai, br, bi).map_err(map_arith)
}

/// Compare two Values in Rust when possible.
/// Returns None if the comparison requires Python (PyObject with custom __eq__).
#[inline]
pub fn eq_native(a: Value, b: Value) -> Option<bool> {
    // Bitwise identity for non-float tags.
    // Floats excluded: NaN != NaN (IEEE 754).
    if a.bits() == b.bits() && !a.is_float() {
        return Some(true);
    }
    if let Some(r) = eq_scalar(a, b) {
        return Some(r);
    }
    // NativeStr equality (pure-side heap extension)
    if a.is_native_str() && b.is_native_str() {
        // SAFETY: both operands were checked as native strings just above, so
        // each payload is a live Arc<NativeString> owned by the caller for
        // this call; the borrows do not outlive it.
        let (sa, sb) = unsafe { (a.as_native_str_ref().unwrap(), b.as_native_str_ref().unwrap()) };
        return Some(*sa == *sb);
    }
    None
}

#[inline]
pub fn numeric_add(a: Value, b: Value) -> VMResult<Value> {
    core_arith::numeric_add(a, b).map_err(map_arith)
}

#[inline]
pub fn numeric_sub(a: Value, b: Value) -> VMResult<Value> {
    core_arith::numeric_sub(a, b).map_err(map_arith)
}

#[inline]
pub fn numeric_mul(a: Value, b: Value) -> VMResult<Value> {
    core_arith::numeric_mul(a, b).map_err(map_arith)
}

#[inline]
pub fn numeric_div(a: Value, b: Value) -> VMResult<Value> {
    core_arith::numeric_div(a, b).map_err(map_arith)
}

#[inline]
pub fn numeric_floordiv(a: Value, b: Value) -> VMResult<Value> {
    core_arith::numeric_floordiv(a, b).map_err(map_arith)
}

#[inline]
pub fn numeric_mod(a: Value, b: Value) -> VMResult<Value> {
    core_arith::numeric_mod(a, b).map_err(map_arith)
}

#[inline]
pub fn numeric_pow(a: Value, b: Value) -> VMResult<Value> {
    core_arith::numeric_pow(a, b).map_err(map_arith)
}

#[inline]
pub fn numeric_lt(a: Value, b: Value) -> VMResult<Value> {
    core_arith::numeric_lt(a, b).map_err(map_arith)
}

#[inline]
pub fn numeric_le(a: Value, b: Value) -> VMResult<Value> {
    core_arith::numeric_le(a, b).map_err(map_arith)
}

#[inline]
pub fn numeric_gt(a: Value, b: Value) -> VMResult<Value> {
    core_arith::numeric_gt(a, b).map_err(map_arith)
}

#[inline]
pub fn numeric_ge(a: Value, b: Value) -> VMResult<Value> {
    core_arith::numeric_ge(a, b).map_err(map_arith)
}

#[inline]
pub fn numeric_neg(a: Value) -> VMResult<Value> {
    core_arith::numeric_neg(a).map_err(map_arith)
}

// The generic bodies live in catnip_core; these tests exercise them through
// the real monomorphization on the pure Value (i64_div_floor/i64_mod_floor
// stay tested in catnip_core, which owns them).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numeric_add_smallint() {
        let a = Value::from_int(2);
        let b = Value::from_int(3);
        let r = numeric_add(a, b).unwrap();
        assert_eq!(r.as_int(), Some(5));
    }

    #[test]
    fn test_numeric_add_float() {
        let a = Value::from_float(1.5);
        let b = Value::from_float(2.5);
        let r = numeric_add(a, b).unwrap();
        assert_eq!(r.as_float(), Some(4.0));
    }

    #[test]
    fn test_numeric_add_mixed() {
        let a = Value::from_int(1);
        let b = Value::from_float(2.5);
        let r = numeric_add(a, b).unwrap();
        assert_eq!(r.as_float(), Some(3.5));
    }

    #[test]
    fn test_numeric_add_type_error() {
        let a = Value::from_bool(true);
        let b = Value::NIL;
        assert!(numeric_add(a, b).is_err());
    }

    #[test]
    fn test_numeric_div_zero() {
        let a = Value::from_int(1);
        let b = Value::from_int(0);
        match numeric_div(a, b) {
            Err(VMError::ZeroDivisionError(_)) => {}
            other => panic!("expected ZeroDivisionError, got {other:?}"),
        }
    }

    #[test]
    fn test_numeric_floordiv_python_semantics() {
        let a = Value::from_int(-7);
        let b = Value::from_int(2);
        let r = numeric_floordiv(a, b).unwrap();
        assert_eq!(r.as_int(), Some(-4)); // Python: -7 // 2 == -4
    }

    #[test]
    fn test_numeric_mod_python_semantics() {
        let a = Value::from_int(-7);
        let b = Value::from_int(3);
        let r = numeric_mod(a, b).unwrap();
        assert_eq!(r.as_int(), Some(2)); // Python: -7 % 3 == 2
    }

    #[test]
    fn test_numeric_pow_basic() {
        let a = Value::from_int(2);
        let b = Value::from_int(10);
        let r = numeric_pow(a, b).unwrap();
        assert_eq!(r.as_int(), Some(1024));
    }

    #[test]
    fn test_numeric_neg() {
        assert_eq!(numeric_neg(Value::from_int(5)).unwrap().as_int(), Some(-5));
        assert_eq!(numeric_neg(Value::from_float(1.5)).unwrap().as_float(), Some(-1.5));
    }

    #[test]
    fn test_numeric_comparisons() {
        let two = Value::from_int(2);
        let three = Value::from_int(3);
        assert_eq!(numeric_lt(two, three).unwrap().as_bool(), Some(true));
        assert_eq!(numeric_le(two, two).unwrap().as_bool(), Some(true));
        assert_eq!(numeric_gt(three, two).unwrap().as_bool(), Some(true));
        assert_eq!(numeric_ge(two, two).unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_eq_native_basic() {
        assert_eq!(eq_native(Value::from_int(1), Value::from_int(1)), Some(true));
        assert_eq!(eq_native(Value::from_int(1), Value::from_int(2)), Some(false));
        assert_eq!(eq_native(Value::NIL, Value::NIL), Some(true));
        assert_eq!(eq_native(Value::from_bool(true), Value::from_bool(false)), Some(false));
    }

    #[test]
    fn test_eq_native_float_nan() {
        let nan = Value::from_float(f64::NAN);
        // NaN != NaN per IEEE 754
        assert_eq!(eq_native(nan, nan), Some(false));
    }

    #[test]
    fn test_eq_native_cross_type() {
        // int vs float
        assert_eq!(eq_native(Value::from_int(1), Value::from_float(1.0)), Some(true));
        assert_eq!(eq_native(Value::from_int(1), Value::from_float(1.5)), Some(false));
    }
}
