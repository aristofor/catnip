// FILE: catnip_rs/src/vm/compiler_core.rs
//! Crate-side instantiation of the shared compiler state.
//!
//! The generic body lives in `catnip_core::vm::compiler_core` (Phase 5, step
//! E1); this module monomorphizes it for the PyO3 `Value`/`VMPattern` and
//! keeps the PyO3-specific pieces: `add_const_py`, `build_code_object` (the
//! `CodeObject` here carries `bytecode_hash`), and the `SyntaxError` ->
//! `PySyntaxError` mapping.

use super::frame::CodeObject;
use super::pattern::VMPattern;
use super::value::Value;
use catnip_core::vm::compiler_core::{self, CompilerPattern, CompilerValue};
use pyo3::prelude::*;
use std::collections::HashMap;

pub(crate) use catnip_core::vm::compiler_core::LoopContext;

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

/// Map a shared-helper syntax failure into a Python exception.
pub(crate) fn syntax_err(e: compiler_core::SyntaxError) -> PyErr {
    pyo3::exceptions::PySyntaxError::new_err(e.0)
}

/// PyO3-side methods on the monomorphized core (inherent impls are not
/// allowed on a type alias of a foreign generic).
pub(crate) trait CompilerCoreExt {
    /// Add a constant from a Python object.
    fn add_const_py(&mut self, py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<usize>;
    fn build_code_object(&mut self, py: Python<'_>) -> PyResult<CodeObject>;
}

impl CompilerCoreExt for CompilerCore {
    fn add_const_py(&mut self, py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<usize> {
        let value = Value::from_pyobject(py, obj)?;
        Ok(self.add_const(value))
    }

    fn build_code_object(&mut self, _py: Python<'_>) -> PyResult<CodeObject> {
        let slotmap: HashMap<String, usize> = self
            .locals
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), i))
            .collect();

        let (optimized_instructions, optimized_line_table) =
            crate::vm::PeepholeOptimizer::optimize(self.instructions.clone(), self.line_table.clone());

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
            bytecode_hash: std::sync::OnceLock::new(),
            encoded_ir: None,
        })
    }
}
