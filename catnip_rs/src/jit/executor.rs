// FILE: catnip_rs/src/jit/executor.rs
//! JIT executor bridge: unbox functions + CodeObject → JitFunctionInfo conversion.
//!
//! Core JITExecutor logic lives in `catnip_core::jit::executor`.

use crate::vm::frame::CodeObject;
use crate::vm::value::Value;
use catnip_core::jit::function_info::{JitConstant, JitFunctionInfo};
use std::sync::Arc;

/// Unbox a NaN-boxed integer value (called from JIT compiled code).
/// Returns the raw i64 value, or 0 if not an integer.
/// Special case: -1 (guard failure) is passed through as-is.
#[no_mangle]
pub extern "C" fn catnip_unbox_int(boxed: i64) -> i64 {
    if boxed == -1 {
        return -1;
    }
    let value = Value::from_raw(boxed as u64);
    value.as_int().unwrap_or(0)
}

/// Unbox a NaN-boxed float value (called from JIT compiled code).
/// Returns the raw f64 value, or 0.0 if not a float.
#[no_mangle]
pub extern "C" fn catnip_unbox_float(boxed: i64) -> f64 {
    let value = Value::from_raw(boxed as u64);
    value.as_float().unwrap_or(0.0)
}

/// Convert a CodeObject into a JitFunctionInfo (extracts int/float constants only).
pub fn code_object_to_jit_info(code: &CodeObject) -> JitFunctionInfo {
    let constants = code
        .constants
        .iter()
        .filter_map(|v| {
            if let Some(i) = v.as_int() {
                Some(JitConstant::Int(i))
            } else {
                v.as_float().map(JitConstant::Float)
            }
        })
        .collect();

    JitFunctionInfo {
        instructions: code.instructions.clone(),
        constants,
        names: code.names.clone(),
        nargs: code.nargs,
        complexity: code.complexity,
        is_pure: code.is_pure,
    }
}

/// Register a pure function on a JITExecutor from a CodeObject.
/// Convenience function bridging PyO3 types to pure Rust.
pub fn register_pure_function(
    executor: &mut catnip_core::jit::executor::JITExecutor,
    func_id: String,
    code: &CodeObject,
) {
    let info = code_object_to_jit_info(code);
    executor.register_pure_info(func_id, Arc::new(info));
}
