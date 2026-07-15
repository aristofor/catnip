// FILE: catnip_core/src/arith.rs
//! Pure numeric arithmetic and comparison over any `ScalarValue`.
//!
//! One generic body for both NaN-boxed `Value` types (Phase 5, step B --
//! wip/PHASE5_TYPE_UNIFICATION.md). Handles SmallInt, BigInt, Float and
//! Complex; monomorphized per crate, no dynamic dispatch. Callers map
//! [`ArithError`] into their own `VMError`.

use crate::scalar::ScalarValue;
use rug::Integer;
use rug::ops::{DivRounding, Pow, RemRounding};

/// Arithmetic failure, mapped to each crate's `VMError` at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithError {
    Type(&'static str),
    ZeroDivision(&'static str),
}

pub type ArithResult<T> = Result<T, ArithError>;

// Error message constants -- the single source; catnip_vm::ops::errors
// re-exports them for existing call-sites.
// Arithmetic type errors
pub const ERR_UNSUPPORTED_ADD: &str = "unsupported operand types for +";
pub const ERR_UNSUPPORTED_SUB: &str = "unsupported operand types for -";
pub const ERR_UNSUPPORTED_MUL: &str = "unsupported operand types for *";
pub const ERR_UNSUPPORTED_DIV: &str = "unsupported operand types for /";
pub const ERR_UNSUPPORTED_FLOORDIV: &str = "unsupported operand types for //";
pub const ERR_UNSUPPORTED_MOD: &str = "unsupported operand types for %";
pub const ERR_UNSUPPORTED_POW: &str = "unsupported operand types for **";
// Bitwise type errors
pub const ERR_UNSUPPORTED_BITOR: &str = "unsupported operand types for |";
pub const ERR_UNSUPPORTED_BITXOR: &str = "unsupported operand types for ^";
pub const ERR_UNSUPPORTED_BITAND: &str = "unsupported operand types for &";
pub const ERR_UNSUPPORTED_LSHIFT: &str = "unsupported operand types for <<";
pub const ERR_UNSUPPORTED_RSHIFT: &str = "unsupported operand types for >>";
// Unary type errors
pub const ERR_BAD_UNARY_POS: &str = "bad operand type for unary +";
pub const ERR_BAD_UNARY_NEG: &str = "bad operand type for unary -";
pub const ERR_BAD_UNARY_NOT: &str = "bad operand type for unary ~";
// Comparison type errors
pub const ERR_CMP_LT: &str = "'<' not supported";
pub const ERR_CMP_LE: &str = "'<=' not supported";
pub const ERR_CMP_GT: &str = "'>' not supported";
pub const ERR_CMP_GE: &str = "'>=' not supported";
// Zero division errors
pub const ERR_INT_DIV_ZERO: &str = "integer division or modulo by zero";
pub const ERR_FLOAT_DIV_ZERO: &str = "division by zero";
pub const ERR_FLOAT_FLOORDIV_ZERO: &str = "float floor division by zero";
pub const ERR_FLOAT_MOD_ZERO: &str = "float modulo by zero";
// Runtime errors
pub const ERR_NO_ACTIVE_EXCEPTION: &str = "no active exception to re-raise";
pub const ERR_LEGACY_MATCH: &str = "legacy MatchPattern is no longer emitted";
pub const ERR_UNSUPPORTED_COMPARISON: &str = "unsupported comparison";
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
pub fn to_f64<V: ScalarValue>(v: V) -> Option<f64> {
    if let Some(f) = v.scalar_as_float() {
        Some(f)
    } else if let Some(i) = v.scalar_as_int() {
        Some(i as f64)
    } else if v.scalar_is_bigint() {
        // SAFETY: v.scalar_is_bigint() was checked just above, so the payload is a live
        // Arc<Integer> referenced by v (owned by the caller for this call); the
        // borrow does not outlive it, and the tag guard makes unwrap infallible.
        let n = unsafe { v.scalar_as_bigint_ref().unwrap() };
        Some(n.to_f64())
    } else {
        None
    }
}

/// Promote a Value to complex (real, imag) parts.
/// Complex → (real, imag), numeric → (value, 0.0), other → None.
#[inline]
pub fn to_complex<V: ScalarValue>(v: V) -> Option<(f64, f64)> {
    if v.scalar_is_complex() {
        // SAFETY: v.scalar_is_complex() was checked just above, so the payload is a live
        // Arc<ExtendedValue::Complex> referenced by v (owned by the caller for this
        // call); the read does not outlive it.
        return unsafe { v.scalar_as_complex_parts() };
    }
    to_f64(v).map(|f| (f, 0.0))
}

/// Complex exponentiation via polar form (De Moivre's theorem).
/// (ar+ai*j) ** (br+bi*j)
pub fn complex_pow<V: ScalarValue>(ar: f64, ai: f64, br: f64, bi: f64) -> ArithResult<V> {
    if br == 0.0 && bi == 0.0 {
        return Ok(V::scalar_from_complex(1.0, 0.0));
    }
    if ar == 0.0 && ai == 0.0 {
        // 0 ** positive_real is 0, everything else is undefined
        if bi == 0.0 && br > 0.0 {
            return Ok(V::scalar_from_complex(0.0, 0.0));
        }
        return Err(ArithError::ZeroDivision("0.0 to a negative or complex power"));
    }
    let r = (ar * ar + ai * ai).sqrt();
    let theta = ai.atan2(ar);
    // ln(a) = ln|a| + i*arg(a)
    let ln_r = r.ln();
    // b * ln(a) = (br + bi*j) * (ln_r + theta*j)
    //           = (br*ln_r - bi*theta) + (br*theta + bi*ln_r)*j
    let exp_r = br * ln_r - bi * theta;
    let exp_i = br * theta + bi * ln_r;
    // e^(exp_r + exp_i*j) = e^exp_r * (cos(exp_i) + sin(exp_i)*j)
    let mag = exp_r.exp();
    Ok(V::scalar_from_complex(mag * exp_i.cos(), mag * exp_i.sin()))
}

/// Promote a Value (SmallInt or BigInt) to owned Integer.
/// Clones when BigInt (needed for cases where we can't borrow).
#[inline]
pub fn to_bigint<V: ScalarValue>(v: V) -> Option<Integer> {
    if let Some(i) = v.scalar_as_int() {
        Some(Integer::from(i))
    } else if v.scalar_is_bigint() {
        // SAFETY: v.scalar_is_bigint() was checked just above, so the payload is a live
        // Arc<Integer> referenced by v (owned by the caller for this call); the
        // borrow used for the clone does not outlive it, so unwrap cannot fail.
        Some(unsafe { v.scalar_as_bigint_ref().unwrap().clone() })
    } else {
        None
    }
}

/// Apply a binary BigInt operation using references (zero-clone).
/// Handles all combinations: BigInt op BigInt, BigInt op SmallInt,
/// SmallInt op BigInt.
#[inline]
pub fn bigint_binop<V: ScalarValue, F>(a: V, b: V, op: F) -> Option<V>
where
    F: FnOnce(&Integer, &Integer) -> Integer,
{
    if a.scalar_is_bigint() && b.scalar_is_bigint() {
        // SAFETY: both a and b were checked as bigint just above, so each payload is
        // a live Arc<Integer> referenced by the operand (owned by the caller for this
        // call); the borrows do not outlive them, so both unwraps are infallible.
        let (ra, rb) = unsafe { (a.scalar_as_bigint_ref().unwrap(), b.scalar_as_bigint_ref().unwrap()) };
        return Some(V::scalar_from_bigint_or_demote(op(ra, rb)));
    }
    if a.scalar_is_bigint() {
        if let Some(bi) = b.scalar_as_int() {
            // SAFETY: a.scalar_is_bigint() was checked just above, so a's payload is a live
            // Arc<Integer> (owned by the caller for this call); the borrow does not
            // outlive it, so unwrap is infallible.
            let ra = unsafe { a.scalar_as_bigint_ref().unwrap() };
            let tmp = Integer::from(bi);
            return Some(V::scalar_from_bigint_or_demote(op(ra, &tmp)));
        }
    }
    if b.scalar_is_bigint() {
        if let Some(ai) = a.scalar_as_int() {
            // SAFETY: b.scalar_is_bigint() was checked just above, so b's payload is a live
            // Arc<Integer> (owned by the caller for this call); the borrow does not
            // outlive it, so unwrap is infallible.
            let rb = unsafe { b.scalar_as_bigint_ref().unwrap() };
            let tmp = Integer::from(ai);
            return Some(V::scalar_from_bigint_or_demote(op(&tmp, rb)));
        }
    }
    None
}

/// Apply a BigInt comparison using references (zero-clone).
#[inline]
pub fn bigint_cmp<V: ScalarValue, F>(a: V, b: V, cmp: F) -> Option<bool>
where
    F: FnOnce(&Integer, &Integer) -> bool,
{
    if a.scalar_is_bigint() && b.scalar_is_bigint() {
        // SAFETY: both a and b were checked as bigint just above, so each payload is
        // a live Arc<Integer> referenced by the operand (owned by the caller for this
        // call); the borrows do not outlive them, so both unwraps are infallible.
        let (ra, rb) = unsafe { (a.scalar_as_bigint_ref().unwrap(), b.scalar_as_bigint_ref().unwrap()) };
        return Some(cmp(ra, rb));
    }
    if a.scalar_is_bigint() {
        if let Some(bi) = b.scalar_as_int() {
            // SAFETY: a.scalar_is_bigint() was checked just above, so a's payload is a live
            // Arc<Integer> (owned by the caller for this call); the borrow does not
            // outlive it, so unwrap is infallible.
            let ra = unsafe { a.scalar_as_bigint_ref().unwrap() };
            let tmp = Integer::from(bi);
            return Some(cmp(ra, &tmp));
        }
    }
    if b.scalar_is_bigint() {
        if let Some(ai) = a.scalar_as_int() {
            // SAFETY: b.scalar_is_bigint() was checked just above, so b's payload is a live
            // Arc<Integer> (owned by the caller for this call); the borrow does not
            // outlive it, so unwrap is infallible.
            let rb = unsafe { b.scalar_as_bigint_ref().unwrap() };
            let tmp = Integer::from(ai);
            return Some(cmp(&tmp, rb));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Numeric binary operations (int/float/bigint only, no collections)
// ---------------------------------------------------------------------------

#[inline]
pub fn numeric_add<V: ScalarValue>(a: V, b: V) -> ArithResult<V> {
    if a.scalar_is_complex() || b.scalar_is_complex() {
        if let (Some((ar, ai)), Some((br, bi))) = (to_complex(a), to_complex(b)) {
            return Ok(V::scalar_from_complex(ar + br, ai + bi));
        }
    }
    if let (Some(ai), Some(bi)) = (a.scalar_as_int(), b.scalar_as_int()) {
        if let Some(sum) = ai.checked_add(bi) {
            if let Some(v) = V::scalar_try_from_int(sum) {
                return Ok(v);
            }
        }
        return Ok(V::scalar_from_bigint_or_demote(Integer::from(ai) + Integer::from(bi)));
    }
    if a.scalar_is_bigint() || b.scalar_is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x + y)) {
            return Ok(v);
        }
        if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
            return Ok(V::scalar_from_float(af + bf));
        }
    }
    if let (Some(af), Some(bf)) = (a.scalar_as_float(), b.scalar_as_float()) {
        return Ok(V::scalar_from_float(af + bf));
    }
    if let (Some(ai), Some(bf)) = (a.scalar_as_int(), b.scalar_as_float()) {
        return Ok(V::scalar_from_float(ai as f64 + bf));
    }
    if let (Some(af), Some(bi)) = (a.scalar_as_float(), b.scalar_as_int()) {
        return Ok(V::scalar_from_float(af + bi as f64));
    }
    Err(ArithError::Type(ERR_UNSUPPORTED_ADD))
}

#[inline]
pub fn numeric_sub<V: ScalarValue>(a: V, b: V) -> ArithResult<V> {
    if a.scalar_is_complex() || b.scalar_is_complex() {
        if let (Some((ar, ai)), Some((br, bi))) = (to_complex(a), to_complex(b)) {
            return Ok(V::scalar_from_complex(ar - br, ai - bi));
        }
    }
    if let (Some(ai), Some(bi)) = (a.scalar_as_int(), b.scalar_as_int()) {
        if let Some(diff) = ai.checked_sub(bi) {
            if let Some(v) = V::scalar_try_from_int(diff) {
                return Ok(v);
            }
        }
        return Ok(V::scalar_from_bigint_or_demote(Integer::from(ai) - Integer::from(bi)));
    }
    if a.scalar_is_bigint() || b.scalar_is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x - y)) {
            return Ok(v);
        }
        if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
            return Ok(V::scalar_from_float(af - bf));
        }
    }
    if let (Some(af), Some(bf)) = (a.scalar_as_float(), b.scalar_as_float()) {
        return Ok(V::scalar_from_float(af - bf));
    }
    if let (Some(ai), Some(bf)) = (a.scalar_as_int(), b.scalar_as_float()) {
        return Ok(V::scalar_from_float(ai as f64 - bf));
    }
    if let (Some(af), Some(bi)) = (a.scalar_as_float(), b.scalar_as_int()) {
        return Ok(V::scalar_from_float(af - bi as f64));
    }
    Err(ArithError::Type(ERR_UNSUPPORTED_SUB))
}

#[inline]
pub fn numeric_mul<V: ScalarValue>(a: V, b: V) -> ArithResult<V> {
    if a.scalar_is_complex() || b.scalar_is_complex() {
        if let (Some((ar, ai)), Some((br, bi))) = (to_complex(a), to_complex(b)) {
            return Ok(V::scalar_from_complex(ar * br - ai * bi, ar * bi + ai * br));
        }
    }
    if let (Some(ai), Some(bi)) = (a.scalar_as_int(), b.scalar_as_int()) {
        if let Some(prod) = ai.checked_mul(bi) {
            if let Some(v) = V::scalar_try_from_int(prod) {
                return Ok(v);
            }
        }
        return Ok(V::scalar_from_bigint_or_demote(Integer::from(ai) * Integer::from(bi)));
    }
    if a.scalar_is_bigint() || b.scalar_is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x * y)) {
            return Ok(v);
        }
        if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
            return Ok(V::scalar_from_float(af * bf));
        }
    }
    if let (Some(af), Some(bf)) = (a.scalar_as_float(), b.scalar_as_float()) {
        return Ok(V::scalar_from_float(af * bf));
    }
    if let (Some(ai), Some(bf)) = (a.scalar_as_int(), b.scalar_as_float()) {
        return Ok(V::scalar_from_float(ai as f64 * bf));
    }
    if let (Some(af), Some(bi)) = (a.scalar_as_float(), b.scalar_as_int()) {
        return Ok(V::scalar_from_float(af * bi as f64));
    }
    Err(ArithError::Type(ERR_UNSUPPORTED_MUL))
}

#[inline]
pub fn numeric_div<V: ScalarValue>(a: V, b: V) -> ArithResult<V> {
    if a.scalar_is_complex() || b.scalar_is_complex() {
        if let (Some((ar, ai)), Some((br, bi))) = (to_complex(a), to_complex(b)) {
            let denom = br * br + bi * bi;
            if denom == 0.0 {
                return Err(ArithError::ZeroDivision(ERR_FLOAT_DIV_ZERO));
            }
            return Ok(V::scalar_from_complex(
                (ar * br + ai * bi) / denom,
                (ai * br - ar * bi) / denom,
            ));
        }
    }
    if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
        if bf == 0.0 {
            return Err(ArithError::ZeroDivision(ERR_FLOAT_DIV_ZERO));
        }
        return Ok(V::scalar_from_float(af / bf));
    }
    Err(ArithError::Type(ERR_UNSUPPORTED_DIV))
}

#[inline]
pub fn numeric_floordiv<V: ScalarValue>(a: V, b: V) -> ArithResult<V> {
    if let (Some(ai), Some(bi)) = (a.scalar_as_int(), b.scalar_as_int()) {
        if bi == 0 {
            return Err(ArithError::ZeroDivision(ERR_INT_DIV_ZERO));
        }
        return Ok(V::scalar_from_int(i64_div_floor(ai, bi)));
    }
    if a.scalar_is_bigint() || b.scalar_is_bigint() {
        if b.scalar_is_bigint() {
            // SAFETY: b.scalar_is_bigint() was checked just above, so b's payload is a live
            // Arc<Integer> (owned by the caller for this call); the borrow does not
            // outlive it, so unwrap is infallible.
            if unsafe { b.scalar_as_bigint_ref().unwrap().cmp0() == std::cmp::Ordering::Equal } {
                return Err(ArithError::ZeroDivision(ERR_INT_DIV_ZERO));
            }
        } else if b.scalar_as_int() == Some(0) {
            return Err(ArithError::ZeroDivision(ERR_INT_DIV_ZERO));
        }
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x).div_floor(y)) {
            return Ok(v);
        }
    }
    let af = a.scalar_as_float().or_else(|| a.scalar_as_int().map(|i| i as f64));
    let bf = b.scalar_as_float().or_else(|| b.scalar_as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        if bf == 0.0 {
            return Err(ArithError::ZeroDivision(ERR_FLOAT_FLOORDIV_ZERO));
        }
        return Ok(V::scalar_from_float((af / bf).floor()));
    }
    Err(ArithError::Type(ERR_UNSUPPORTED_FLOORDIV))
}

#[inline]
pub fn numeric_mod<V: ScalarValue>(a: V, b: V) -> ArithResult<V> {
    if let (Some(ai), Some(bi)) = (a.scalar_as_int(), b.scalar_as_int()) {
        if bi == 0 {
            return Err(ArithError::ZeroDivision(ERR_INT_DIV_ZERO));
        }
        return Ok(V::scalar_from_int(i64_mod_floor(ai, bi)));
    }
    if a.scalar_is_bigint() || b.scalar_is_bigint() {
        if b.scalar_is_bigint() {
            // SAFETY: b.scalar_is_bigint() was checked just above, so b's payload is a live
            // Arc<Integer> (owned by the caller for this call); the borrow does not
            // outlive it, so unwrap is infallible.
            if unsafe { b.scalar_as_bigint_ref().unwrap().cmp0() == std::cmp::Ordering::Equal } {
                return Err(ArithError::ZeroDivision(ERR_INT_DIV_ZERO));
            }
        } else if b.scalar_as_int() == Some(0) {
            return Err(ArithError::ZeroDivision(ERR_INT_DIV_ZERO));
        }
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x).rem_floor(y)) {
            return Ok(v);
        }
    }
    let af = a.scalar_as_float().or_else(|| a.scalar_as_int().map(|i| i as f64));
    let bf = b.scalar_as_float().or_else(|| b.scalar_as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        if bf == 0.0 {
            return Err(ArithError::ZeroDivision(ERR_FLOAT_MOD_ZERO));
        }
        // Python floored modulo: af - bf * floor(af / bf)
        return Ok(V::scalar_from_float(af - bf * (af / bf).floor()));
    }
    Err(ArithError::Type(ERR_UNSUPPORTED_MOD))
}

#[inline]
pub fn numeric_pow<V: ScalarValue>(a: V, b: V) -> ArithResult<V> {
    if a.scalar_is_complex() || b.scalar_is_complex() {
        if let (Some((ar, ai)), Some((br, bi))) = (to_complex(a), to_complex(b)) {
            return complex_pow(ar, ai, br, bi);
        }
    }
    if let (Some(ai), Some(bi)) = (a.scalar_as_int(), b.scalar_as_int()) {
        if bi >= 0 {
            if bi <= 64 {
                if let Some(result) = ai.checked_pow(bi as u32) {
                    if let Some(v) = V::scalar_try_from_int(result) {
                        return Ok(v);
                    }
                }
            }
            let base = Integer::from(ai);
            if let Ok(exp) = u32::try_from(bi) {
                return Ok(V::scalar_from_bigint_or_demote(base.pow(exp)));
            }
            return Ok(V::scalar_from_float((ai as f64).powf(bi as f64)));
        }
        return Ok(V::scalar_from_float((ai as f64).powf(bi as f64)));
    }
    if a.scalar_is_bigint() || b.scalar_is_bigint() {
        if let (Some(base), Some(bi)) = (to_bigint(a), b.scalar_as_int()) {
            if bi >= 0 {
                if let Ok(exp) = u32::try_from(bi) {
                    return Ok(V::scalar_from_bigint_or_demote(base.pow(exp)));
                }
            }
            if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
                return Ok(V::scalar_from_float(af.powf(bf)));
            }
        }
        if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
            return Ok(V::scalar_from_float(af.powf(bf)));
        }
    }
    let af = a.scalar_as_float().or_else(|| a.scalar_as_int().map(|i| i as f64));
    let bf = b.scalar_as_float().or_else(|| b.scalar_as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(V::scalar_from_float(af.powf(bf)));
    }
    Err(ArithError::Type(ERR_UNSUPPORTED_POW))
}

// ---------------------------------------------------------------------------
// Unary operations
// ---------------------------------------------------------------------------

#[inline]
pub fn numeric_neg<V: ScalarValue>(a: V) -> ArithResult<V> {
    if a.scalar_is_complex() {
        // SAFETY: a.scalar_is_complex() was checked just above, so a's payload is a live
        // Arc<ExtendedValue::Complex> (owned by the caller for this call); the read
        // does not outlive it, so unwrap is infallible.
        let (r, i) = unsafe { a.scalar_as_complex_parts().unwrap() };
        return Ok(V::scalar_from_complex(-r, -i));
    }
    if let Some(i) = a.scalar_as_int() {
        if let Some(v) = V::scalar_try_from_int(-i) {
            return Ok(v);
        }
        return Ok(V::scalar_from_bigint_or_demote(-Integer::from(i)));
    }
    if a.scalar_is_bigint() {
        // SAFETY: a.scalar_is_bigint() was checked just above, so a's payload is a live
        // Arc<Integer> (owned by the caller for this call); the borrow does not
        // outlive it, so unwrap is infallible.
        let n = unsafe { a.scalar_as_bigint_ref().unwrap() };
        return Ok(V::scalar_from_bigint_or_demote(Integer::from(-n)));
    }
    if let Some(f) = a.scalar_as_float() {
        return Ok(V::scalar_from_float(-f));
    }
    Err(ArithError::Type(ERR_BAD_UNARY_NEG))
}

// ---------------------------------------------------------------------------
// Numeric comparisons (int/float/bigint only)
// ---------------------------------------------------------------------------

#[inline]
pub fn numeric_lt<V: ScalarValue>(a: V, b: V) -> ArithResult<V> {
    if let (Some(ai), Some(bi)) = (a.scalar_as_int(), b.scalar_as_int()) {
        return Ok(V::scalar_from_bool(ai < bi));
    }
    if a.scalar_is_bigint() || b.scalar_is_bigint() {
        if let Some(r) = bigint_cmp(a, b, |x, y| x < y) {
            return Ok(V::scalar_from_bool(r));
        }
    }
    let af = a.scalar_as_float().or_else(|| a.scalar_as_int().map(|i| i as f64));
    let bf = b.scalar_as_float().or_else(|| b.scalar_as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(V::scalar_from_bool(af < bf));
    }
    Err(ArithError::Type(ERR_CMP_LT))
}

#[inline]
pub fn numeric_le<V: ScalarValue>(a: V, b: V) -> ArithResult<V> {
    if let (Some(ai), Some(bi)) = (a.scalar_as_int(), b.scalar_as_int()) {
        return Ok(V::scalar_from_bool(ai <= bi));
    }
    if a.scalar_is_bigint() || b.scalar_is_bigint() {
        if let Some(r) = bigint_cmp(a, b, |x, y| x <= y) {
            return Ok(V::scalar_from_bool(r));
        }
    }
    let af = a.scalar_as_float().or_else(|| a.scalar_as_int().map(|i| i as f64));
    let bf = b.scalar_as_float().or_else(|| b.scalar_as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(V::scalar_from_bool(af <= bf));
    }
    Err(ArithError::Type(ERR_CMP_LE))
}

#[inline]
pub fn numeric_gt<V: ScalarValue>(a: V, b: V) -> ArithResult<V> {
    if let (Some(ai), Some(bi)) = (a.scalar_as_int(), b.scalar_as_int()) {
        return Ok(V::scalar_from_bool(ai > bi));
    }
    if a.scalar_is_bigint() || b.scalar_is_bigint() {
        if let Some(r) = bigint_cmp(a, b, |x, y| x > y) {
            return Ok(V::scalar_from_bool(r));
        }
    }
    let af = a.scalar_as_float().or_else(|| a.scalar_as_int().map(|i| i as f64));
    let bf = b.scalar_as_float().or_else(|| b.scalar_as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(V::scalar_from_bool(af > bf));
    }
    Err(ArithError::Type(ERR_CMP_GT))
}

#[inline]
pub fn numeric_ge<V: ScalarValue>(a: V, b: V) -> ArithResult<V> {
    if let (Some(ai), Some(bi)) = (a.scalar_as_int(), b.scalar_as_int()) {
        return Ok(V::scalar_from_bool(ai >= bi));
    }
    if a.scalar_is_bigint() || b.scalar_is_bigint() {
        if let Some(r) = bigint_cmp(a, b, |x, y| x >= y) {
            return Ok(V::scalar_from_bool(r));
        }
    }
    let af = a.scalar_as_float().or_else(|| a.scalar_as_int().map(|i| i as f64));
    let bf = b.scalar_as_float().or_else(|| b.scalar_as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(V::scalar_from_bool(af >= bf));
    }
    Err(ArithError::Type(ERR_CMP_GE))
}

/// Scalar-equality core: nil/int/bool/complex/bigint and mixed float. The
/// caller runs its own leading bit-identity fast path (per-crate tag
/// exclusions) and its heap extensions (NativeStr on the pure side, PyObject
/// deferral on the PyO3 side); None means "not decidable on scalars".
#[inline]
pub fn eq_scalar<V: ScalarValue>(a: V, b: V) -> Option<bool> {
    if a.scalar_is_nil() || b.scalar_is_nil() {
        return Some(a.scalar_is_nil() && b.scalar_is_nil());
    }
    if let (Some(ai), Some(bi)) = (a.scalar_as_int(), b.scalar_as_int()) {
        return Some(ai == bi);
    }
    if let (Some(ab), Some(bb)) = (a.scalar_as_bool(), b.scalar_as_bool()) {
        return Some(ab == bb);
    }
    // Complex equality before BigInt (to_complex promotes BigInt via to_f64)
    if a.scalar_is_complex() || b.scalar_is_complex() {
        if let (Some((ar, ai)), Some((br, bi))) = (to_complex(a), to_complex(b)) {
            return Some(ar == br && ai == bi);
        }
    }
    if a.scalar_is_bigint() || b.scalar_is_bigint() {
        return bigint_cmp(a, b, |x, y| x == y);
    }
    let af = a.scalar_as_float().or_else(|| a.scalar_as_int().map(|i| i as f64));
    let bf = b.scalar_as_float().or_else(|| b.scalar_as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Some(af == bf);
    }
    None
}

// ---------------------------------------------------------------------------
// Boundary coercion (CheckType) -- scalar arms shared by both VMs
// ---------------------------------------------------------------------------

/// Verdict of the scalar boundary coercion. `Mismatch` leaves the TypeError
/// message to the caller (it names the value's primitive type, a per-crate
/// concern); `Unhandled` covers the non-scalar codes (str, composites).
pub enum ScalarCoerce<V> {
    Ok(V),
    /// float(huge_int) raises rather than yielding inf -- surfaced as a
    /// boundary TypeError so `except TypeError` catches it uniformly.
    HugeInt,
    Mismatch,
    Unhandled,
}

/// TH2-B numeric-tower coercion over the scalar type codes: a value already of
/// the declared type passes through; a widening (`int`/`bool` -> `float`,
/// `bool` -> `int`) is coerced; anything else is a mismatch. The bigint ->
/// float widening releases the operand's Arc (replaced by a fresh inline
/// float).
pub fn coerce_scalar<V: ScalarValue>(val: V, code: u8) -> ScalarCoerce<V> {
    use crate::vm::opcode::type_code;
    match code {
        type_code::INT => {
            if val.scalar_is_int() || val.scalar_is_bigint() {
                ScalarCoerce::Ok(val)
            } else if let Some(b) = val.scalar_as_bool() {
                ScalarCoerce::Ok(V::scalar_from_i64(b as i64))
            } else {
                ScalarCoerce::Mismatch
            }
        }
        type_code::FLOAT => {
            if val.scalar_is_float() {
                ScalarCoerce::Ok(val)
            } else if let Some(b) = val.scalar_as_bool() {
                ScalarCoerce::Ok(V::scalar_from_float(if b { 1.0 } else { 0.0 }))
            } else if let Some(i) = val.scalar_as_int() {
                ScalarCoerce::Ok(V::scalar_from_float(i as f64))
            } else if val.scalar_is_bigint() {
                // SAFETY: scalar_is_bigint() guards the Arc<Integer> payload.
                let f = unsafe { val.scalar_as_bigint_ref() }.unwrap().to_f64();
                if f.is_finite() {
                    // `val` is replaced by a fresh inline float; release its Arc.
                    val.scalar_decref_bigint();
                    ScalarCoerce::Ok(V::scalar_from_float(f))
                } else {
                    // The caller releases `val`.
                    ScalarCoerce::HugeInt
                }
            } else {
                ScalarCoerce::Mismatch
            }
        }
        type_code::BOOL => {
            if val.scalar_is_bool() {
                ScalarCoerce::Ok(val)
            } else {
                ScalarCoerce::Mismatch
            }
        }
        type_code::NONE => {
            if val.scalar_is_nil() {
                ScalarCoerce::Ok(val)
            } else {
                ScalarCoerce::Mismatch
            }
        }
        _ => ScalarCoerce::Unhandled,
    }
}

// ---------------------------------------------------------------------------
// Tests (Value-dependent coverage lives in catnip_vm::ops::arith, on the real
// monomorphization)
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
}
