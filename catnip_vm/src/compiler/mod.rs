// FILE: catnip_vm/src/compiler/mod.rs
//! Pure Rust compiler: IR -> bytecode (no PyO3).
//!
//! Extracts the "Pure IR" compilation path from catnip_rs/src/vm/unified_compiler.rs
//! into a standalone module that depends only on catnip_core and catnip_vm types.

pub mod code_object;
pub mod core;
pub mod error;
pub mod input;
pub mod pattern;

mod collections;
mod control_flow;
mod exceptions;
mod expr;
mod functions;
mod helpers;
mod structs;

pub use code_object::CodeObject;
pub use core::{CompilerCore, CompilerCoreExt};
pub use error::{CompileError, CompileResult};
pub use pattern::{VMPattern, VMPatternElement};

use crate::Value;
use catnip_core::ir::opcode::IROpCode;
use catnip_core::ir::pure::{BroadcastType, IR};
use catnip_core::vm::opcode::ParamCheck;
use catnip_core::vm::opcode::VMOpCode;
use indexmap::IndexMap;
use input::*;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// Output of a compilation: a main code object plus sub-functions.
pub struct CompileOutput {
    /// Main code object
    pub code: CodeObject,
    /// Sub-functions (lambdas, methods) compiled alongside main
    pub functions: Vec<CodeObject>,
}

/// Metadata for compiling a function body.
pub struct FunctionMeta {
    pub params: Vec<String>,
    pub name: String,
    pub defaults: Vec<Value>,
    pub vararg_idx: i32,
    pub parent_nesting_depth: u32,
}

/// Internal spec for function compilation.
struct FunctionCompileSpec<'a> {
    params: Vec<String>,
    body: &'a IR,
    name: &'a str,
    defaults: Vec<Value>,
    vararg_idx: i32,
    parent_nesting_depth: u32,
    /// Per-param prologue boundary check (aligned with `params`): a primitive
    /// `CheckType` code, a nominal type name (`CheckNominal`), or none. Empty
    /// means no checks.
    param_types: Vec<ParamCheck>,
}

/// Pure Rust IR -> bytecode compiler (no Python dependency).
/// Info about an active try/finally for inlining finally on break/continue/return.
#[derive(Clone)]
pub struct FinallyInfo {
    /// The finally body IR to inline
    pub body: IR,
    /// Whether this try also has an except handler to pop
    pub has_except: bool,
    /// Whether to emit ClearException before the finally (inside except handler bodies)
    pub needs_clear_exception: bool,
}

pub struct PureCompiler {
    core: CompilerCore,
    /// Sub-functions collected during compilation
    functions: Vec<CodeObject>,
    /// Stack of active finally bodies (for inlining on break/continue/return)
    finally_stack: Vec<FinallyInfo>,
}

impl Deref for PureCompiler {
    type Target = CompilerCore;
    fn deref(&self) -> &CompilerCore {
        &self.core
    }
}

impl DerefMut for PureCompiler {
    fn deref_mut(&mut self) -> &mut CompilerCore {
        &mut self.core
    }
}

impl PureCompiler {
    pub fn new() -> Self {
        Self {
            core: CompilerCore::new(),
            functions: Vec::new(),
            finally_stack: Vec::new(),
        }
    }

    /// Check that args has at least `n` elements.
    #[inline]
    fn require_args(args: &[IR], n: usize, ctx: &str) -> CompileResult<()> {
        if args.len() < n {
            Err(CompileError::SyntaxError(format!(
                "{ctx} requires {n} args, got {}",
                args.len()
            )))
        } else {
            Ok(())
        }
    }

    // ========== Public entry points ==========

    /// Compile an IR node to a CompileOutput.
    pub fn compile(&mut self, ir: &IR) -> CompileResult<CompileOutput> {
        self.core.reset();
        self.functions.clear();
        self.compile_node(ir)?;
        self.emit(VMOpCode::Halt, 0);
        let code = self.core.build_code_object()?;
        Ok(CompileOutput {
            code,
            functions: std::mem::take(&mut self.functions),
        })
    }

    /// Compile a function body to a CompileOutput.
    pub fn compile_function(&mut self, ir: &IR, meta: FunctionMeta) -> CompileResult<CompileOutput> {
        self.core.reset();
        self.functions.clear();
        let code = self.compile_function_to_code(FunctionCompileSpec {
            params: meta.params,
            param_types: Vec::new(),
            body: ir,
            name: &meta.name,
            defaults: meta.defaults,
            vararg_idx: meta.vararg_idx,
            parent_nesting_depth: meta.parent_nesting_depth,
        })?;
        Ok(CompileOutput {
            code,
            functions: std::mem::take(&mut self.functions),
        })
    }

    // ========== Internal compile ==========

    fn compile_function_to_code(&mut self, spec: FunctionCompileSpec<'_>) -> CompileResult<CodeObject> {
        // Save pre-seeded outer_names (from compile_function_inner) before reset
        let pre_seeded = std::mem::take(&mut self.core.outer_names);
        self.core.reset();
        self.core.outer_names = pre_seeded;
        self.core.name = spec.name.to_string();
        self.core.nargs = spec.params.len();
        self.core.defaults = spec.defaults;
        self.core.in_function = true;
        self.core.nesting_depth = spec.parent_nesting_depth + 1;

        for (i, param) in spec.params.iter().enumerate() {
            let slot = self.core.add_local(param);
            // Enforce an annotated param at the prologue so the body reads it
            // already checked: a primitive is checked-and-coerced (CheckType),
            // a nominal type is checked for membership with subtyping
            // (CheckNominal, the type name riding the `names` table).
            if let Some(check) = spec.param_types.get(i) {
                if !matches!(check, ParamCheck::None) {
                    self.emit(VMOpCode::LoadLocal, slot as u32);
                    self.emit_check_opcode(check);
                    self.emit(VMOpCode::StoreLocal, slot as u32);
                }
            }
        }

        self.compile_node(spec.body)?;
        self.emit(VMOpCode::Return, 0);

        let mut code = self.core.build_code_object()?;
        code.vararg_idx = spec.vararg_idx;
        Ok(code)
    }

    /// Lower one classified boundary check to its `Check*` opcode on the top
    /// of stack. Shared by the param prologue (which wraps it in LoadLocal/
    /// StoreLocal) and the FT2-A `CheckReturn` lowering (which applies it to a
    /// call's result), so the ParamCheck -> opcode mapping lives once per
    /// compiler.
    fn emit_check_opcode(&mut self, check: &ParamCheck) {
        match check {
            ParamCheck::Primitive(code) => {
                self.emit(VMOpCode::CheckType, *code as u32);
            }
            ParamCheck::Nominal(tyname) => {
                let nidx = self.add_name(tyname) as u32;
                self.emit(VMOpCode::CheckNominal, nidx);
            }
            ParamCheck::Union(members) => {
                let uidx = self.core.add_union_check(members.clone()) as u32;
                self.emit(VMOpCode::CheckUnion, uidx);
            }
            check @ ParamCheck::Composite { .. } => {
                let cidx = self.core.add_composite_check(check.clone()) as u32;
                self.emit(VMOpCode::CheckComposite, cidx);
            }
            check @ ParamCheck::Generic { .. } => {
                let gidx = self.core.add_generic_check(check.clone()) as u32;
                self.emit(VMOpCode::CheckGeneric, gidx);
            }
            ParamCheck::Callable { arity } => {
                self.emit(VMOpCode::CheckCallable, *arity);
            }
            ParamCheck::None => {}
        }
    }

    fn compile_function_inner(&mut self, spec: FunctionCompileSpec<'_>) -> CompileResult<CodeObject> {
        let saved_core = std::mem::take(&mut self.core);
        let saved_finally = std::mem::take(&mut self.finally_stack);

        // No pre-seed of outer_names from assignments: a write-through to an
        // enclosing binding requires the name to be READ inside the function
        // (outer_names is fed by compile_name_load only). A write-only name
        // creates a local -- the documented rule ("une closure qui écrit sans
        // lire crée une locale"), settled 2026-07-04: the assignment-based
        // pre-seed made the semantics depend on whether the global was
        // defined before or after the lambda (order-dependent).

        let result = self.compile_function_to_code(spec);
        self.core = saved_core;
        self.finally_stack = saved_finally;
        result
    }
}

impl Default for PureCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
