// FILE: catnip_core/src/nanbox.rs
//! NaN-boxing layout constants -- single source of truth.
//!
//! IEEE 754 double: [sign:1][exponent:11][mantissa:52]
//! Quiet NaN prefix: exponent all-1 + quiet bit = 13 bits fixed
//! Remaining 51 bits: [tag:TAG_BITS][payload:PAYLOAD_BITS]

// -- Geometry --
pub const PAYLOAD_BITS: u32 = 47;
pub const TAG_BITS: u32 = 4;
pub const TAG_SHIFT: u32 = PAYLOAD_BITS; // 47

// -- Masks (u64) --
pub const PAYLOAD_MASK: u64 = (1u64 << PAYLOAD_BITS) - 1;
pub const TAG_MASK: u64 = ((1u64 << TAG_BITS) - 1) << TAG_SHIFT;
pub const QNAN_BASE: u64 = 0x7FF8_0000_0000_0000; // IEEE 754 quiet NaN
pub const QNAN_TAG_MASK: u64 = QNAN_BASE | TAG_MASK;

// -- SmallInt (signed, PAYLOAD_BITS wide) --
pub const SMALLINT_SIGN_BIT: u64 = 1u64 << (PAYLOAD_BITS - 1);
pub const SMALLINT_SIGN_EXT: u64 = !PAYLOAD_MASK;
pub const SMALLINT_MAX: i64 = (1i64 << (PAYLOAD_BITS - 1)) - 1;
pub const SMALLINT_MIN: i64 = -(1i64 << (PAYLOAD_BITS - 1));

/// Canonical NaN -- signaling NaN that avoids collision with QNAN_BASE|SmallInt(0).
pub const CANON_NAN: u64 = 0x7FF0_0000_0000_0001;

// -- Tag values (pre-shifted) -- indices 0..7 shared between both VMs
pub const TAG_SMALLINT: u64 = 0;
pub const TAG_BOOL: u64 = 1 << TAG_SHIFT;
pub const TAG_NIL: u64 = 2 << TAG_SHIFT;
pub const TAG_SYMBOL: u64 = 3 << TAG_SHIFT;
// 4 = PYOBJ (catnip_rs only)
// 5 = STRUCT (catnip_rs only)
pub const TAG_BIGINT: u64 = 6 << TAG_SHIFT;
pub const TAG_VMFUNC: u64 = 7 << TAG_SHIFT;

// -- i64 aliases for JIT codegen (Cranelift iconst) --
pub const PAYLOAD_MASK_I64: i64 = PAYLOAD_MASK as i64;
pub const SMALLINT_SIGN_BIT_I64: i64 = SMALLINT_SIGN_BIT as i64;
pub const SMALLINT_SIGN_EXT_I64: i64 = SMALLINT_SIGN_EXT as i64;
pub const QNAN_BASE_I64: i64 = QNAN_BASE as i64;
pub const TAG_SMALLINT_I64: i64 = TAG_SMALLINT as i64;

// -- Value-boundary classification (canonical law) --
//
// Every NaN-box tag carries a payload of exactly one class. The class
// taxonomy and the scalar set are canonical here; each crate completes the
// Index/Pointer split for its own divergent tags (4-5 for catnip_rs, 8-15 for
// catnip_vm). Formal model + proofs: `proof/vm/CatnipValueClassProof.v` and
// `proof/vm/CatnipBoundaryProof.v`.

/// Payload class of a tag.
///
/// - `Scalar`  inline data, always boundary-safe (SmallInt, Bool, Nil, Symbol)
/// - `Index`   bounded table handle, safe iff the index is in bounds
/// - `Pointer` raw `Arc` pointer, NOT certifiable from bits alone
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagClass {
    Scalar,
    Index,
    Pointer,
}

/// Extract the 4-bit tag index from a quiet-NaN-tagged word.
#[inline]
pub const fn tag_index(bits: u64) -> u64 {
    (bits & TAG_MASK) >> TAG_SHIFT
}

/// True if `bits` is an ordinary IEEE-754 double (not a quiet-NaN-tagged
/// value). A float carries no pointer payload, so it is an inline scalar.
#[inline]
pub const fn is_float_bits(bits: u64) -> bool {
    (bits & 0x7FF8_0000_0000_0000) != QNAN_BASE
}

/// True for the scalar tags (SmallInt/Bool/Nil/Symbol), which are identical in
/// both Value implementations. The boundary lock depends only on this set.
#[inline]
pub const fn is_scalar_tag(tag: u64) -> bool {
    matches!(tag, 0..=3)
}

/// Scalar boundary constructor: admit a raw word from an untrusted source
/// (JIT codegen, plugin FFI) only when it is an inline scalar -- a float or a
/// scalar tag. A pointer/index tag is rejected (`None`), so no unvalidated
/// pointer is ever reconstructed and dereferenced.
///
/// Mirrors `CatnipBoundaryProof.from_raw_scalar`: every accepted word is
/// Scalar-class (`from_raw_scalar_class_scalar`), and any Pointer-tagged word
/// is rejected (`from_raw_scalar_rejects_pointer`).
#[inline]
pub const fn from_raw_scalar(bits: u64) -> Option<u64> {
    if is_float_bits(bits) || is_scalar_tag(tag_index(bits)) {
        Some(bits)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tagged(tag: u64, payload: u64) -> u64 {
        QNAN_BASE | tag | (payload & PAYLOAD_MASK)
    }

    // Mirrors CatnipValueClassProof: scalar tags are exactly {0,1,2,3}.
    #[test]
    fn scalar_set_is_zero_to_three() {
        assert!(is_scalar_tag(0) && is_scalar_tag(1) && is_scalar_tag(2) && is_scalar_tag(3));
        for tag in 4u64..16 {
            assert!(!is_scalar_tag(tag), "tag {tag} must not be scalar");
        }
    }

    // Mirrors CatnipBoundaryProof::ex_reject_forged_bigint.
    #[test]
    fn from_raw_scalar_rejects_pointer_tag() {
        assert_eq!(from_raw_scalar(tagged(TAG_BIGINT, 0x123456)), None);
    }

    // Mirrors CatnipBoundaryProof::ex_accept_smallint / ex_accept_float.
    #[test]
    fn from_raw_scalar_accepts_scalar_and_float() {
        let small = tagged(TAG_SMALLINT, 42);
        let symbol = tagged(TAG_SYMBOL, 7);
        let float_bits = 1.5f64.to_bits();
        assert_eq!(from_raw_scalar(small), Some(small));
        assert_eq!(from_raw_scalar(symbol), Some(symbol));
        assert_eq!(from_raw_scalar(float_bits), Some(float_bits));
    }
}
