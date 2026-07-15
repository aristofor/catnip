// FILE: catnip_vm/src/compiler/code_object.rs
//! Pure Rust CodeObject for the standalone compiler.

use super::pattern::VMPattern;
use crate::Value;
use catnip_core::vm::opcode::{Instruction, ParamCheck};
use std::collections::HashMap;
use std::sync::Arc;

pub const NO_VARARG_IDX: i32 = -1;

/// Compiled bytecode for a function or module (pure Rust, no PyO3).
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
    /// Pre-classified type-union member specs, indexed by the `CheckUnion` arg.
    pub union_checks: Vec<Box<[ParamCheck]>>,
    /// Pre-classified composite specs (`list[T]`/`dict[K, V]`), indexed by the `CheckComposite` arg.
    pub composite_checks: Vec<ParamCheck>,
    /// Pre-classified generic-nominal specs (`Option[int]`), indexed by the `CheckGeneric` arg.
    pub generic_checks: Vec<ParamCheck>,
    /// Frozen IR body for ND process workers (raw bincode)
    pub encoded_ir: Option<Arc<Vec<u8>>>,
}

/// The pools own one reference per slot (producers insert freshly-built
/// `Value`s; consumers borrow or incref, never decrement). Cloning therefore
/// takes a reference per duplicated slot, and Drop is the single release
/// site -- the pair keeps every copy balanced by construction.
impl Clone for CodeObject {
    fn clone(&self) -> Self {
        for v in &self.constants {
            v.clone_refcount();
        }
        for v in &self.defaults {
            v.clone_refcount();
        }
        for p in &self.patterns {
            p.incref_values();
        }
        Self {
            instructions: self.instructions.clone(),
            constants: self.constants.clone(),
            names: self.names.clone(),
            nlocals: self.nlocals,
            varnames: self.varnames.clone(),
            slotmap: self.slotmap.clone(),
            nargs: self.nargs,
            defaults: self.defaults.clone(),
            name: self.name.clone(),
            freevars: self.freevars.clone(),
            vararg_idx: self.vararg_idx,
            is_pure: self.is_pure,
            complexity: self.complexity,
            line_table: self.line_table.clone(),
            patterns: self.patterns.clone(),
            union_checks: self.union_checks.clone(),
            composite_checks: self.composite_checks.clone(),
            generic_checks: self.generic_checks.clone(),
            encoded_ir: self.encoded_ir.clone(),
        }
    }
}

impl Drop for CodeObject {
    fn drop(&mut self) {
        for v in self.constants.drain(..) {
            v.decref();
        }
        for v in self.defaults.drain(..) {
            v.decref();
        }
        for p in self.patterns.drain(..) {
            p.decref_values();
        }
    }
}
