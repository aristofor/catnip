//! Shared scalar heap wrappers for the two NaN-boxed `Value` types.
//!
//! Phase 5 (wip/PHASE5_TYPE_UNIFICATION.md): the scalar logic common to
//! `catnip_rs::vm::Value` and `catnip_vm::Value` lives here. The heap
//! representations that differ by design (pyobj, structs, native containers)
//! stay in their crates.

use rug::Integer;
use std::fmt;
use std::sync::atomic::{AtomicIsize, Ordering};

/// Live BigInt allocations, shared by both `Value` types (they wrap the same
/// `GmpInt`). Read by the refcount-ledger probes (`_debug_live_counts`);
/// the ledger asserts a zero delta across a pipeline lifecycle.
pub static LIVE_BIGINT: AtomicIsize = AtomicIsize::new(0);

// ---------------------------------------------------------------------------
// GmpInt -- Sync wrapper for rug::Integer
// ---------------------------------------------------------------------------

/// Wrapper around `rug::Integer` that implements `Sync`.
///
/// `rug::Integer` is `Send` but not `Sync` (GMP limitation). All access is
/// single-threaded by construction: under the GIL on the PyO3 side, one VM
/// thread on the pure side. Sharing via `Arc` is therefore safe.
///
/// Construct via [`GmpInt::new`] (or `clone`) so the live ledger stays exact:
/// `Drop` decrements it, a literal `GmpInt(..)` would underflow the count.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct GmpInt(pub Integer);

impl GmpInt {
    #[inline]
    pub fn new(n: Integer) -> Self {
        LIVE_BIGINT.fetch_add(1, Ordering::Relaxed);
        GmpInt(n)
    }
}

impl Clone for GmpInt {
    fn clone(&self) -> Self {
        Self::new(self.0.clone())
    }
}

impl Drop for GmpInt {
    fn drop(&mut self) {
        LIVE_BIGINT.fetch_sub(1, Ordering::Relaxed);
    }
}

// SAFETY: access to GmpInt is serialized (GIL on the PyO3 side, single VM
// thread on the pure side). The underlying GMP memory is never accessed
// concurrently from multiple threads.
unsafe impl Sync for GmpInt {}

impl std::ops::Deref for GmpInt {
    type Target = Integer;
    #[inline]
    fn deref(&self) -> &Integer {
        &self.0
    }
}

impl std::ops::DerefMut for GmpInt {
    #[inline]
    fn deref_mut(&mut self) -> &mut Integer {
        &mut self.0
    }
}

impl fmt::Debug for GmpInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for GmpInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// ScalarValue -- shared scalar logic over the two NaN-boxed Value types
// ---------------------------------------------------------------------------

use crate::nanbox::{
    CANON_NAN, PAYLOAD_MASK, QNAN_BASE, SMALLINT_MAX, SMALLINT_MIN, SMALLINT_SIGN_BIT, SMALLINT_SIGN_EXT, TAG_BIGINT,
    TAG_BOOL, TAG_MASK, TAG_NIL, TAG_SMALLINT, TAG_SYMBOL, TAG_VMFUNC,
};
use std::sync::Arc;

/// Scalar contract shared by `catnip_rs::vm::Value` and `catnip_vm::Value`.
///
/// Both wrap the same NaN-box layout (`crate::nanbox`) for the scalar tags
/// (smallint/bool/nil/symbol/bigint/vmfunc) and the same `GmpInt` heap
/// wrapper, so every accessor and constructor over those tags has exactly one
/// body: the provided methods below. Only the raw bit round-trip is
/// per-crate. Heap tags that differ by design (pyobj vs native containers)
/// stay entirely in each crate's inherent impl.
pub trait ScalarValue: Copy {
    /// Raw NaN-box bits.
    fn bits(self) -> u64;
    /// Wrap raw NaN-box bits. Callers only pass bits built from the shared
    /// scalar tags; crate-specific tags never route through the trait.
    fn from_bits(bits: u64) -> Self;

    #[inline]
    fn scalar_is_float(self) -> bool {
        (self.bits() & 0x7FF8_0000_0000_0000) != QNAN_BASE
    }

    #[inline]
    fn scalar_is_int(self) -> bool {
        (self.bits() & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_SMALLINT)
    }

    #[inline]
    fn scalar_is_bool(self) -> bool {
        (self.bits() & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_BOOL)
    }

    #[inline]
    fn scalar_is_nil(self) -> bool {
        (self.bits() & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_NIL)
    }

    #[inline]
    fn scalar_is_symbol(self) -> bool {
        (self.bits() & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_SYMBOL)
    }

    #[inline]
    fn scalar_is_bigint(self) -> bool {
        (self.bits() & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_BIGINT)
    }

    #[inline]
    fn scalar_is_vmfunc(self) -> bool {
        (self.bits() & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_VMFUNC)
    }

    #[inline]
    fn scalar_as_int(self) -> Option<i64> {
        if self.scalar_is_int() {
            let payload = self.bits() & PAYLOAD_MASK;
            // Sign extend from the 47-bit payload
            let result = if payload & SMALLINT_SIGN_BIT != 0 {
                (payload | SMALLINT_SIGN_EXT) as i64
            } else {
                payload as i64
            };
            Some(result)
        } else {
            None
        }
    }

    #[inline]
    fn scalar_as_float(self) -> Option<f64> {
        if self.scalar_is_float() {
            Some(f64::from_bits(self.bits()))
        } else {
            None
        }
    }

    #[inline]
    fn scalar_as_bool(self) -> Option<bool> {
        if self.scalar_is_bool() {
            Some((self.bits() & 1) != 0)
        } else {
            None
        }
    }

    #[inline]
    fn scalar_as_symbol(self) -> Option<u32> {
        if self.scalar_is_symbol() {
            Some((self.bits() & PAYLOAD_MASK) as u32)
        } else {
            None
        }
    }

    #[inline]
    fn scalar_from_int(i: i64) -> Self {
        debug_assert!(
            (SMALLINT_MIN..=SMALLINT_MAX).contains(&i),
            "integer out of small int range"
        );
        let payload = (i as u64) & PAYLOAD_MASK;
        Self::from_bits(QNAN_BASE | TAG_SMALLINT | payload)
    }

    #[inline]
    fn scalar_try_from_int(i: i64) -> Option<Self> {
        if (SMALLINT_MIN..=SMALLINT_MAX).contains(&i) {
            let payload = (i as u64) & PAYLOAD_MASK;
            Some(Self::from_bits(QNAN_BASE | TAG_SMALLINT | payload))
        } else {
            None
        }
    }

    #[inline]
    fn scalar_from_float(f: f64) -> Self {
        let bits = f.to_bits();
        // Canonicalize any NaN payload: a non-canonical NaN would alias a tag
        if (bits & 0x7FF8_0000_0000_0000) == 0x7FF8_0000_0000_0000 {
            Self::from_bits(CANON_NAN)
        } else {
            Self::from_bits(bits)
        }
    }

    #[inline]
    fn scalar_from_bool(b: bool) -> Self {
        Self::from_bits(QNAN_BASE | TAG_BOOL | b as u64)
    }

    #[inline]
    fn scalar_from_symbol(idx: u32) -> Self {
        Self::from_bits(QNAN_BASE | TAG_SYMBOL | (idx as u64))
    }

    #[inline]
    fn scalar_from_bigint(n: Integer) -> Self {
        let arc = Arc::new(GmpInt::new(n));
        let ptr = Arc::into_raw(arc) as u64;
        debug_assert!(
            ptr & !PAYLOAD_MASK == 0,
            "BigInt Arc pointer exceeds 47-bit address space"
        );
        Self::from_bits(QNAN_BASE | TAG_BIGINT | (ptr & PAYLOAD_MASK))
    }

    #[inline]
    fn scalar_from_bigint_or_demote(n: Integer) -> Self {
        if let Some(i) = n.to_i64() {
            if let Some(v) = Self::scalar_try_from_int(i) {
                return v;
            }
        }
        Self::scalar_from_bigint(n)
    }
    /// Borrow the Integer behind a bigint payload without cloning.
    ///
    /// # Safety
    ///
    /// Caller must ensure the backing `Arc<GmpInt>` is still alive (not yet
    /// decremented to 0) for the duration of the borrow.
    #[inline]
    unsafe fn scalar_as_bigint_ref(&self) -> Option<&Integer> {
        if (*self).scalar_is_bigint() {
            let ptr = (self.bits() & PAYLOAD_MASK) as *const GmpInt;
            // SAFETY: the bigint tag was checked just above, so the payload is a
            // pointer produced by Arc::into_raw in scalar_from_bigint; the caller
            // guarantees the backing Arc is still alive (fn contract).
            unsafe { Some(&(*ptr).0) }
        } else {
            None
        }
    }

    // --- Complex: representations differ per crate (Arc<NativeComplex> on the
    // PyO3 side, ExtendedValue::Complex on the pure side) -- required. ---

    fn scalar_is_complex(self) -> bool;

    /// (real, imag) parts of a complex payload.
    ///
    /// # Safety
    ///
    /// Caller must ensure the operand's heap payload outlives the read.
    unsafe fn scalar_as_complex_parts(&self) -> Option<(f64, f64)>;

    fn scalar_from_complex(real: f64, imag: f64) -> Self;

    #[inline]
    fn scalar_from_i64(i: i64) -> Self {
        Self::scalar_try_from_int(i).unwrap_or_else(|| Self::scalar_from_bigint(Integer::from(i)))
    }

    /// Release the `Arc<GmpInt>` behind a bigint payload (no-op otherwise).
    #[inline]
    fn scalar_decref_bigint(self) {
        if self.scalar_is_bigint() {
            let ptr = (self.bits() & PAYLOAD_MASK) as *const GmpInt;
            // SAFETY: scalar_is_bigint() proves the payload is a live Arc<GmpInt>;
            // this decref balances the incref that created/cloned it.
            unsafe { Arc::decrement_strong_count(ptr) };
        }
    }
}
