// FILE: catnip_vm/src/ops/arith.rs
//! Pure numeric arithmetic and comparison operations.
//!
//! Shared between catnip_vm (PureHost) and catnip_rs (VM dispatch).
//! Handles SmallInt, BigInt, and Float. No Python dependency.

use crate::error::{VMError, VMResult};
use crate::value::Value;
use rug::Integer;
use rug::ops::{DivRounding, Pow, RemRounding};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Python-style integer floor division: result rounds toward negative infinity.
#[inline]
pub fn i64_div_floor(a: i64, b: i64) -> i64 {
    let d = a / b;
    let r = a % b;
    if (r != 0) && ((r ^ b) < 0) { d - 1 } else { d }
}

/// Python-style integer modulo: result has the sign of the divisor.
#[inline]
pub fn i64_mod_floor(a: i64, b: i64) -> i64 {
    let r = a % b;
    if (r != 0) && ((r ^ b) < 0) { r + b } else { r }
}

/// Convert a Value (SmallInt, BigInt, or Float) to f64.
#[inline]
pub fn to_f64(v: Value) -> Option<f64> {
    if let Some(f) = v.as_float() {
        Some(f)
    } else if let Some(i) = v.as_int() {
        Some(i as f64)
    } else if v.is_bigint() {
        let n = unsafe { v.as_bigint_ref().unwrap() };
        Some(n.to_f64())
    } else {
        None
    }
}

/// Promote a Value (SmallInt or BigInt) to owned Integer.
/// Clones when BigInt (needed for cases where we can't borrow).
#[inline]
pub fn to_bigint(v: Value) -> Option<Integer> {
    if let Some(i) = v.as_int() {
        Some(Integer::from(i))
    } else if v.is_bigint() {
        Some(unsafe { v.as_bigint_ref().unwrap().clone() })
    } else {
        None
    }
}

/// Apply a binary BigInt operation using references (zero-clone).
/// Handles all combinations: BigInt op BigInt, BigInt op SmallInt,
/// SmallInt op BigInt.
#[inline]
pub fn bigint_binop<F>(a: Value, b: Value, op: F) -> Option<Value>
where
    F: FnOnce(&Integer, &Integer) -> Integer,
{
    if a.is_bigint() && b.is_bigint() {
        let (ra, rb) = unsafe { (a.as_bigint_ref().unwrap(), b.as_bigint_ref().unwrap()) };
        return Some(Value::from_bigint_or_demote(op(ra, rb)));
    }
    if a.is_bigint() {
        if let Some(bi) = b.as_int() {
            let ra = unsafe { a.as_bigint_ref().unwrap() };
            let tmp = Integer::from(bi);
            return Some(Value::from_bigint_or_demote(op(ra, &tmp)));
        }
    }
    if b.is_bigint() {
        if let Some(ai) = a.as_int() {
            let rb = unsafe { b.as_bigint_ref().unwrap() };
            let tmp = Integer::from(ai);
            return Some(Value::from_bigint_or_demote(op(&tmp, rb)));
        }
    }
    None
}

/// Apply a BigInt comparison using references (zero-clone).
#[inline]
pub fn bigint_cmp<F>(a: Value, b: Value, cmp: F) -> Option<bool>
where
    F: FnOnce(&Integer, &Integer) -> bool,
{
    if a.is_bigint() && b.is_bigint() {
        let (ra, rb) = unsafe { (a.as_bigint_ref().unwrap(), b.as_bigint_ref().unwrap()) };
        return Some(cmp(ra, rb));
    }
    if a.is_bigint() {
        if let Some(bi) = b.as_int() {
            let ra = unsafe { a.as_bigint_ref().unwrap() };
            let tmp = Integer::from(bi);
            return Some(cmp(ra, &tmp));
        }
    }
    if b.is_bigint() {
        if let Some(ai) = a.as_int() {
            let rb = unsafe { b.as_bigint_ref().unwrap() };
            let tmp = Integer::from(ai);
            return Some(cmp(&tmp, rb));
        }
    }
    None
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
    if a.is_nil() || b.is_nil() {
        return Some(a.is_nil() && b.is_nil());
    }
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Some(ai == bi);
    }
    if let (Some(ab), Some(bb)) = (a.as_bool(), b.as_bool()) {
        return Some(ab == bb);
    }
    if a.is_bigint() || b.is_bigint() {
        return bigint_cmp(a, b, |x, y| x == y);
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Some(af == bf);
    }
    // NativeStr equality
    if a.is_native_str() && b.is_native_str() {
        let sa = unsafe { a.as_native_str_ref().unwrap() };
        let sb = unsafe { b.as_native_str_ref().unwrap() };
        return Some(*sa == *sb);
    }
    None
}

// ---------------------------------------------------------------------------
// Numeric binary operations (int/float/bigint only, no collections)
// ---------------------------------------------------------------------------

#[inline]
pub fn numeric_add(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if let Some(sum) = ai.checked_add(bi) {
            if let Some(v) = Value::try_from_int(sum) {
                return Ok(v);
            }
        }
        return Ok(Value::from_bigint_or_demote(Integer::from(ai) + Integer::from(bi)));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x + y)) {
            return Ok(v);
        }
        if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
            return Ok(Value::from_float(af + bf));
        }
    }
    if let (Some(af), Some(bf)) = (a.as_float(), b.as_float()) {
        return Ok(Value::from_float(af + bf));
    }
    if let (Some(ai), Some(bf)) = (a.as_int(), b.as_float()) {
        return Ok(Value::from_float(ai as f64 + bf));
    }
    if let (Some(af), Some(bi)) = (a.as_float(), b.as_int()) {
        return Ok(Value::from_float(af + bi as f64));
    }
    Err(VMError::TypeError("unsupported operand types for +".into()))
}

#[inline]
pub fn numeric_sub(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if let Some(diff) = ai.checked_sub(bi) {
            if let Some(v) = Value::try_from_int(diff) {
                return Ok(v);
            }
        }
        return Ok(Value::from_bigint_or_demote(Integer::from(ai) - Integer::from(bi)));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x - y)) {
            return Ok(v);
        }
        if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
            return Ok(Value::from_float(af - bf));
        }
    }
    if let (Some(af), Some(bf)) = (a.as_float(), b.as_float()) {
        return Ok(Value::from_float(af - bf));
    }
    if let (Some(ai), Some(bf)) = (a.as_int(), b.as_float()) {
        return Ok(Value::from_float(ai as f64 - bf));
    }
    if let (Some(af), Some(bi)) = (a.as_float(), b.as_int()) {
        return Ok(Value::from_float(af - bi as f64));
    }
    Err(VMError::TypeError("unsupported operand types for -".into()))
}

#[inline]
pub fn numeric_mul(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if let Some(prod) = ai.checked_mul(bi) {
            if let Some(v) = Value::try_from_int(prod) {
                return Ok(v);
            }
        }
        return Ok(Value::from_bigint_or_demote(Integer::from(ai) * Integer::from(bi)));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x * y)) {
            return Ok(v);
        }
        if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
            return Ok(Value::from_float(af * bf));
        }
    }
    if let (Some(af), Some(bf)) = (a.as_float(), b.as_float()) {
        return Ok(Value::from_float(af * bf));
    }
    if let (Some(ai), Some(bf)) = (a.as_int(), b.as_float()) {
        return Ok(Value::from_float(ai as f64 * bf));
    }
    if let (Some(af), Some(bi)) = (a.as_float(), b.as_int()) {
        return Ok(Value::from_float(af * bi as f64));
    }
    Err(VMError::TypeError("unsupported operand types for *".into()))
}

#[inline]
pub fn numeric_div(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
        if bf == 0.0 {
            return Err(VMError::ZeroDivisionError("division by zero".into()));
        }
        return Ok(Value::from_float(af / bf));
    }
    Err(VMError::TypeError("unsupported operand types for /".into()))
}

#[inline]
pub fn numeric_floordiv(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if bi == 0 {
            return Err(VMError::ZeroDivisionError("integer division or modulo by zero".into()));
        }
        return Ok(Value::from_int(i64_div_floor(ai, bi)));
    }
    if a.is_bigint() || b.is_bigint() {
        if b.is_bigint() {
            if unsafe { b.as_bigint_ref().unwrap().cmp0() == std::cmp::Ordering::Equal } {
                return Err(VMError::ZeroDivisionError("integer division or modulo by zero".into()));
            }
        } else if b.as_int() == Some(0) {
            return Err(VMError::ZeroDivisionError("integer division or modulo by zero".into()));
        }
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x).div_floor(y)) {
            return Ok(v);
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        if bf == 0.0 {
            return Err(VMError::ZeroDivisionError("float floor division by zero".into()));
        }
        return Ok(Value::from_float((af / bf).floor()));
    }
    Err(VMError::TypeError("unsupported operand types for //".into()))
}

#[inline]
pub fn numeric_mod(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if bi == 0 {
            return Err(VMError::ZeroDivisionError("integer division or modulo by zero".into()));
        }
        return Ok(Value::from_int(i64_mod_floor(ai, bi)));
    }
    if a.is_bigint() || b.is_bigint() {
        if b.is_bigint() {
            if unsafe { b.as_bigint_ref().unwrap().cmp0() == std::cmp::Ordering::Equal } {
                return Err(VMError::ZeroDivisionError("integer division or modulo by zero".into()));
            }
        } else if b.as_int() == Some(0) {
            return Err(VMError::ZeroDivisionError("integer division or modulo by zero".into()));
        }
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x).rem_floor(y)) {
            return Ok(v);
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        if bf == 0.0 {
            return Err(VMError::ZeroDivisionError("float modulo by zero".into()));
        }
        // Python floored modulo: af - bf * floor(af / bf)
        return Ok(Value::from_float(af - bf * (af / bf).floor()));
    }
    Err(VMError::TypeError("unsupported operand types for %".into()))
}

#[inline]
pub fn numeric_pow(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if bi >= 0 {
            if bi <= 64 {
                if let Some(result) = ai.checked_pow(bi as u32) {
                    if let Some(v) = Value::try_from_int(result) {
                        return Ok(v);
                    }
                }
            }
            let base = Integer::from(ai);
            if let Ok(exp) = u32::try_from(bi) {
                return Ok(Value::from_bigint_or_demote(base.pow(exp)));
            }
            return Ok(Value::from_float((ai as f64).powf(bi as f64)));
        }
        return Ok(Value::from_float((ai as f64).powf(bi as f64)));
    }
    if a.is_bigint() || b.is_bigint() {
        if let (Some(base), Some(bi)) = (to_bigint(a), b.as_int()) {
            if bi >= 0 {
                if let Ok(exp) = u32::try_from(bi) {
                    return Ok(Value::from_bigint_or_demote(base.pow(exp)));
                }
            }
            if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
                return Ok(Value::from_float(af.powf(bf)));
            }
        }
        if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
            return Ok(Value::from_float(af.powf(bf)));
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_float(af.powf(bf)));
    }
    Err(VMError::TypeError("unsupported operand types for **".into()))
}

// ---------------------------------------------------------------------------
// Unary operations
// ---------------------------------------------------------------------------

#[inline]
pub fn numeric_neg(a: Value) -> VMResult<Value> {
    if let Some(i) = a.as_int() {
        if let Some(v) = Value::try_from_int(-i) {
            return Ok(v);
        }
        return Ok(Value::from_bigint_or_demote(-Integer::from(i)));
    }
    if a.is_bigint() {
        let n = unsafe { a.as_bigint_ref().unwrap() };
        return Ok(Value::from_bigint_or_demote(Integer::from(-n)));
    }
    if let Some(f) = a.as_float() {
        return Ok(Value::from_float(-f));
    }
    Err(VMError::TypeError("bad operand type for unary -".into()))
}

// ---------------------------------------------------------------------------
// Numeric comparisons (int/float/bigint only)
// ---------------------------------------------------------------------------

#[inline]
pub fn numeric_lt(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai < bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(r) = bigint_cmp(a, b, |x, y| x < y) {
            return Ok(Value::from_bool(r));
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af < bf));
    }
    Err(VMError::TypeError("'<' not supported".into()))
}

#[inline]
pub fn numeric_le(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai <= bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(r) = bigint_cmp(a, b, |x, y| x <= y) {
            return Ok(Value::from_bool(r));
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af <= bf));
    }
    Err(VMError::TypeError("'<=' not supported".into()))
}

#[inline]
pub fn numeric_gt(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai > bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(r) = bigint_cmp(a, b, |x, y| x > y) {
            return Ok(Value::from_bool(r));
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af > bf));
    }
    Err(VMError::TypeError("'>' not supported".into()))
}

#[inline]
pub fn numeric_ge(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai >= bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(r) = bigint_cmp(a, b, |x, y| x >= y) {
            return Ok(Value::from_bool(r));
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af >= bf));
    }
    Err(VMError::TypeError("'>=' not supported".into()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_floor_div_mod_identity() {
        // i64_div_floor(a, b) * b + i64_mod_floor(a, b) == a
        let cases = [
            (7, 3),
            (-7, 3),
            (7, -3),
            (-7, -3),
            (0, 1),
            (1, 1),
            (-1, 1),
            (i64::MAX, 1),
            (i64::MIN + 1, -1),
            (10, 3),
            (-10, 3),
        ];
        for (a, b) in cases {
            let d = i64_div_floor(a, b);
            let m = i64_mod_floor(a, b);
            assert_eq!(d * b + m, a, "failed for a={a}, b={b}: d={d}, m={m}");
        }
    }

    #[test]
    fn test_floor_mod_sign() {
        // Result has sign of divisor (or is zero)
        assert_eq!(i64_mod_floor(7, 3), 1); // positive divisor -> positive
        assert_eq!(i64_mod_floor(-7, 3), 2); // positive divisor -> positive
        assert_eq!(i64_mod_floor(7, -3), -2); // negative divisor -> negative
        assert_eq!(i64_mod_floor(-7, -3), -1); // negative divisor -> negative
        assert_eq!(i64_mod_floor(6, 3), 0); // exact division -> zero
    }

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
