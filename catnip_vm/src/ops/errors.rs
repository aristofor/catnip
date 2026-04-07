// FILE: catnip_vm/src/ops/errors.rs
//! Shared error message constants for VM operations.
//!
//! Centralized here to avoid string duplication across catnip_vm and catnip_rs.

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
