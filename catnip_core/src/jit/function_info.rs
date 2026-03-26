// FILE: catnip_core/src/jit/function_info.rs
//! Pure Rust subset of CodeObject for JIT inlining/registry.
//!
//! Extracted from CodeObject at registration time to decouple from PyO3.

use crate::vm::opcode::Instruction;

/// JIT-visible constant (subset of Value: only int and float).
#[derive(Debug, Clone)]
pub enum JitConstant {
    Int(i64),
    Float(f64),
}

/// Pure Rust function info for JIT inlining/registry.
///
/// Contains only the fields needed by the inliner and registry,
/// without any PyO3 dependency (no `Py<PyAny>`, no `Value`).
pub struct JitFunctionInfo {
    pub instructions: Vec<Instruction>,
    pub constants: Vec<JitConstant>,
    pub names: Vec<String>,
    pub nargs: usize,
    pub complexity: usize,
    pub is_pure: bool,
}
