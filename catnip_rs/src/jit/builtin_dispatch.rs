// FILE: catnip_rs/src/jit/builtin_dispatch.rs
//! Dispatch table for JIT builtin callbacks.
//!
//! Provides extern "C" functions callable from Cranelift-generated code
//! for builtins that can't be trivially compiled to native instructions
//! but are still pure (no side effects, deterministic).

/// Builtin ID: float()
pub const BUILTIN_FLOAT: u8 = 0;

/// Map builtin name to dispatch ID.
pub fn builtin_name_to_id(name: &str) -> Option<u8> {
    match name {
        "float" => Some(BUILTIN_FLOAT),
        _ => None,
    }
}

// NaN-boxing constants (mirror value.rs)
const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
const SMALLINT_SIGN_BIT: u64 = 0x0000_8000_0000_0000;
const SMALLINT_SIGN_EXT: u64 = 0xFFFF_0000_0000_0000;

/// Unbox a NaN-boxed int to i64.
#[inline]
fn unbox_int(boxed: i64) -> i64 {
    let bits = boxed as u64;
    let payload = bits & PAYLOAD_MASK;
    if payload & SMALLINT_SIGN_BIT != 0 {
        (payload | SMALLINT_SIGN_EXT) as i64
    } else {
        payload as i64
    }
}

/// Box a float into NaN-boxed format (raw f64 bits).
#[inline]
fn box_float(f: f64) -> i64 {
    f.to_bits() as i64
}

/// Extern C callback for JIT-compiled builtin calls.
///
/// Called from Cranelift-generated code with NaN-boxed arguments.
/// Returns NaN-boxed result, or -1 sentinel on error.
///
/// GIL is already held (called from VM::run context).
#[no_mangle]
pub extern "C" fn catnip_call_builtin(
    builtin_id: i64,
    arg0: i64,
    _arg1: i64,
    _num_args: i64,
) -> i64 {
    match builtin_id as u8 {
        BUILTIN_FLOAT => {
            // float(int) -> float
            let int_val = unbox_int(arg0);
            box_float(int_val as f64)
        }
        _ => -1, // Unknown builtin -> sentinel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QNAN_BASE: u64 = 0x7FF8_0000_0000_0000;
    const TAG_SMALLINT: u64 = 0x0000_0000_0000_0000;

    fn box_int(i: i64) -> i64 {
        let payload = (i as u64) & PAYLOAD_MASK;
        (QNAN_BASE | TAG_SMALLINT | payload) as i64
    }

    #[test]
    fn test_builtin_name_to_id() {
        assert_eq!(builtin_name_to_id("float"), Some(BUILTIN_FLOAT));
        assert_eq!(builtin_name_to_id("unknown"), None);
        assert_eq!(builtin_name_to_id("abs"), None); // native, not callback
    }

    #[test]
    fn test_float_callback() {
        let boxed_42 = box_int(42);
        let result = catnip_call_builtin(BUILTIN_FLOAT as i64, boxed_42, 0, 1);
        let expected = box_float(42.0);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_float_callback_negative() {
        let boxed_neg = box_int(-7);
        let result = catnip_call_builtin(BUILTIN_FLOAT as i64, boxed_neg, 0, 1);
        let expected = box_float(-7.0);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_unknown_builtin_sentinel() {
        let result = catnip_call_builtin(255, 0, 0, 0);
        assert_eq!(result, -1);
    }
}
