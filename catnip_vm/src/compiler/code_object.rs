// FILE: catnip_vm/src/compiler/code_object.rs
//! Pure Rust CodeObject for the standalone compiler.

use super::pattern::VMPattern;
use crate::Value;
use catnip_core::vm::opcode::Instruction;
use std::collections::HashMap;
use std::sync::Arc;

pub const NO_VARARG_IDX: i32 = -1;

/// Compiled bytecode for a function or module (pure Rust, no PyO3).
#[derive(Clone)]
pub struct CodeObject {
    /// Bytecode instructions
    pub instructions: Vec<Instruction>,
    /// Constant pool (NaN-boxed values)
    pub constants: Vec<Value>,
    /// Variable names for LOAD_NAME/STORE_NAME
    pub names: Vec<String>,
    /// Number of local variable slots
    pub nlocals: usize,
    /// Names of local variables (for debugging)
    pub varnames: Vec<String>,
    /// Map from variable name to slot index
    pub slotmap: HashMap<String, usize>,
    /// Number of parameters (not including *args)
    pub nargs: usize,
    /// Default parameter values
    pub defaults: Vec<Value>,
    /// Function name
    pub name: String,
    /// Free variables (closure captures)
    pub freevars: Vec<String>,
    /// Index of *args parameter (-1 if none)
    pub vararg_idx: i32,
    /// Function marked pure (no side effects)
    pub is_pure: bool,
    /// Complexity estimate (number of instructions) for inline decision
    pub complexity: usize,
    /// Line table: maps instruction index -> source byte offset
    pub line_table: Vec<u32>,
    /// Compiled match patterns
    pub patterns: Vec<VMPattern>,
    /// Frozen IR body for ND process workers (raw bincode)
    pub encoded_ir: Option<Arc<Vec<u8>>>,
}
