// FILE: catnip_vm/src/compiler/core.rs
//! Crate-side instantiation of the shared compiler state (pure Rust, no PyO3).
//!
//! The generic body lives in `catnip_core::vm::compiler_core` (Phase 5, step
//! E1); this module monomorphizes it for the pure `Value`/`VMPattern` and
//! keeps `build_code_object` (the `CodeObject` types differ per crate) plus
//! the `SyntaxError` -> `CompileError` mapping.

use super::code_object::CodeObject;
use super::error::{CompileError, CompileResult};
use super::pattern::VMPattern;
use crate::Value;
use catnip_core::vm::compiler_core::{self, CompilerPattern, CompilerValue};
use catnip_core::vm::peephole::PeepholeOptimizer;
use std::collections::HashMap;

pub use catnip_core::vm::compiler_core::LoopContext;

pub type CompilerCore = compiler_core::CompilerCore<Value, VMPattern>;

impl CompilerValue for Value {
    #[inline]
    fn release(self) {
        self.decref();
    }
}

impl CompilerPattern for VMPattern {
    #[inline]
    fn release_values(&self) {
        self.decref_values();
    }
}

impl From<compiler_core::SyntaxError> for CompileError {
    fn from(e: compiler_core::SyntaxError) -> Self {
        CompileError::SyntaxError(e.0.to_string())
    }
}

/// Crate-side methods on the monomorphized core (inherent impls are not
/// allowed on a type alias of a foreign generic).
pub trait CompilerCoreExt {
    fn build_code_object(&mut self) -> CompileResult<CodeObject>;
}

impl CompilerCoreExt for CompilerCore {
    fn build_code_object(&mut self) -> CompileResult<CodeObject> {
        let slotmap: HashMap<String, usize> = self
            .locals
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), i))
            .collect();

        let (optimized_instructions, optimized_line_table) =
            PeepholeOptimizer::optimize(self.instructions.clone(), self.line_table.clone());

        let complexity = optimized_instructions.len();

        // The refcounted pools MOVE into the CodeObject: exactly one owner for
        // each Value reference (CodeObject::drop releases them). A bit-copy
        // here would leave two release sites for one reference.
        Ok(CodeObject {
            instructions: optimized_instructions,
            constants: std::mem::take(&mut self.constants),
            names: self.names.clone(),
            nlocals: self.locals.len(),
            varnames: self.locals.clone(),
            slotmap,
            nargs: self.nargs,
            defaults: std::mem::take(&mut self.defaults),
            name: self.name.clone(),
            freevars: Vec::new(),
            vararg_idx: -1,
            is_pure: false,
            complexity,
            line_table: optimized_line_table,
            patterns: std::mem::take(&mut self.patterns),
            union_checks: self.union_checks.clone(),
            composite_checks: self.composite_checks.clone(),
            generic_checks: self.generic_checks.clone(),
            encoded_ir: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catnip_core::vm::VMOpCode;

    #[test]
    fn test_compiler_core_new() {
        let core = CompilerCore::new();
        assert!(core.instructions.is_empty());
        assert!(core.constants.is_empty());
        assert_eq!(core.name, "<module>");
    }

    #[test]
    fn test_emit_and_patch() {
        let mut core = CompilerCore::new();
        let idx = core.emit(VMOpCode::LoadConst, 0);
        assert_eq!(idx, 0);
        assert_eq!(core.instructions.len(), 1);
        core.patch(idx, 42);
        assert_eq!(core.instructions[0].arg, 42);
    }

    #[test]
    fn test_add_const_dedup() {
        let mut core = CompilerCore::new();
        let idx1 = core.add_const(Value::from_int(42));
        let idx2 = core.add_const(Value::from_int(42));
        assert_eq!(idx1, idx2);
        assert_eq!(core.constants.len(), 1);
    }

    #[test]
    fn test_add_name_dedup() {
        let mut core = CompilerCore::new();
        let idx1 = core.add_name("x");
        let idx2 = core.add_name("x");
        assert_eq!(idx1, idx2);
        assert_eq!(core.names.len(), 1);
    }

    #[test]
    fn test_add_local_dedup() {
        let mut core = CompilerCore::new();
        let idx1 = core.add_local("a");
        let idx2 = core.add_local("a");
        assert_eq!(idx1, idx2);
        assert_eq!(core.locals.len(), 1);
    }

    #[test]
    fn test_build_code_object() {
        let mut core = CompilerCore::new();
        core.emit(VMOpCode::LoadConst, 0);
        core.add_const(Value::from_int(42));
        core.emit(VMOpCode::Halt, 0);
        let code = core.build_code_object().unwrap();
        assert!(!code.instructions.is_empty());
        assert_eq!(code.constants.len(), 1);
    }
}
