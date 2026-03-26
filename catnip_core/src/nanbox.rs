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
