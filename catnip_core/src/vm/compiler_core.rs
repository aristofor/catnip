// FILE: catnip_core/src/vm/compiler_core.rs
//! Shared compiler state and helper methods, generic over each crate's
//! `Value` and pattern types (Phase 5, step E1 --
//! wip/PHASE5_TYPE_UNIFICATION.md).
//!
//! One body for both bytecode compilers: `catnip_rs` (Op/PyObject input,
//! used by `UnifiedCompiler` via Deref) and `catnip_vm` (IR input, used by
//! `PureCompiler` via Deref). Monomorphized per crate. Each crate keeps its
//! own `build_code_object` (the `CodeObject` types differ) and maps
//! [`SyntaxError`] into its own error type.

use super::opcode::{Instruction, ParamCheck, VMOpCode};
use crate::scalar::ScalarValue;
use std::collections::HashSet;

/// Crate-side Value hooks the shared compiler state needs: pool dedup
/// (`PartialEq`) and full-Value release (heap tags included -- broader than
/// the scalar-only helpers of `ScalarValue`).
pub trait CompilerValue: ScalarValue + PartialEq {
    /// Release one owned reference of a pooled Value.
    fn release(self);
}

/// Crate-side hooks for compiled match patterns pooled by the compiler.
pub trait CompilerPattern {
    /// Release the owned Values held by the pattern.
    fn release_values(&self);
}

/// Syntax failure from the shared helpers ('break'/'continue' outside a
/// loop); each crate maps it into its own error type.
#[derive(Debug, Clone, Copy)]
pub struct SyntaxError(pub &'static str);

/// Loop context for break/continue compilation.
pub struct LoopContext {
    /// Addresses of JUMP instructions to patch to loop end (for break)
    pub break_targets: Vec<usize>,
    /// Address to jump to for continue (None for for-range where it's patched later)
    pub continue_target: Option<usize>,
    /// Addresses of JUMP instructions to patch to increment (for continue in for-range)
    pub continue_patches: Vec<usize>,
    /// True if this is a for loop (has iterator on stack)
    pub is_for_loop: bool,
}

/// Shared compiler state for bytecode emission.
pub struct CompilerCore<V: CompilerValue, P: CompilerPattern> {
    /// Emitted bytecode
    pub instructions: Vec<Instruction>,
    /// Constant pool (unified as Value for both pipelines)
    pub constants: Vec<V>,
    /// Name table (for global/scope variables)
    pub names: Vec<String>,
    /// Local variable slots
    pub locals: Vec<String>,
    /// Number of function parameters
    pub nargs: usize,
    /// Default parameter values
    pub defaults: Vec<V>,
    /// Function/module name
    pub name: String,
    /// Loop nesting for break/continue
    pub loop_stack: Vec<LoopContext>,
    /// Whether we're compiling inside a function
    pub in_function: bool,
    /// Current function nesting depth (for JIT hot detection)
    pub nesting_depth: u32,
    /// Current source position (for line table)
    pub current_start_byte: u32,
    /// Line table: maps instruction index -> source byte offset
    pub line_table: Vec<u32>,
    /// Compiled match patterns
    pub patterns: Vec<P>,
    /// Pre-classified type-union member specs, indexed by the `CheckUnion` arg.
    pub union_checks: Vec<Box<[ParamCheck]>>,
    /// Pre-classified composite specs, indexed by the `CheckComposite` arg.
    pub composite_checks: Vec<ParamCheck>,
    /// Pre-classified generic-nominal specs, indexed by the `CheckGeneric` arg.
    pub generic_checks: Vec<ParamCheck>,
    /// Whether the current expression result is unused
    pub void_context: bool,
    /// Whether we're in an optimized loop (local-only variables)
    pub in_optimized_loop: bool,
    /// Variables modified inside an optimized loop (name, local slot)
    pub loop_modified_vars: Vec<(String, usize)>,
    /// Names read from outer scope (closure captures)
    pub outer_names: HashSet<String>,
    /// Nesting depth of active Finally handlers (for break/continue/return through finally)
    pub finally_depth: usize,
    /// Name being bound to the lambda currently compiled, if any (let-rec:
    /// MakeFunction injects a self-reference into the closure)
    pub pending_self_name: Option<String>,
    /// Set when the lambda currently compiled is decorated `@pure`: its
    /// CodeObject gets `is_pure = true` so the JIT records calls to it as
    /// CallPure (inlining candidate). Consumed (taken) by compile_lambda.
    pub pending_pure: bool,
    /// Named function definitions per open block (name, local slot),
    /// for letrec group patching (PatchClosure between siblings)
    pub block_fn_defs: Vec<Vec<(String, usize)>>,
}

impl<V: CompilerValue, P: CompilerPattern> CompilerCore<V, P> {
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            constants: Vec::new(),
            names: Vec::new(),
            locals: Vec::new(),
            nargs: 0,
            defaults: Vec::new(),
            name: "<module>".to_string(),
            loop_stack: Vec::new(),
            in_function: false,
            nesting_depth: 0,
            current_start_byte: 0,
            line_table: Vec::new(),
            patterns: Vec::new(),
            union_checks: Vec::new(),
            composite_checks: Vec::new(),
            generic_checks: Vec::new(),
            void_context: false,
            in_optimized_loop: false,
            loop_modified_vars: Vec::new(),
            outer_names: HashSet::new(),
            finally_depth: 0,
            pending_self_name: None,
            pending_pure: false,
            block_fn_defs: vec![Vec::new()],
        }
    }

    /// Release the refcounted `Value`s still sitting in the buffers.
    /// After a successful `build_code_object` the pools are empty (moved into
    /// the CodeObject, which owns them); anything left here is an aborted
    /// compile whose references would otherwise leak.
    fn drain_value_pools(&mut self) {
        for v in self.constants.drain(..) {
            v.release();
        }
        for v in self.defaults.drain(..) {
            v.release();
        }
        for p in self.patterns.drain(..) {
            p.release_values();
        }
    }

    pub fn reset(&mut self) {
        self.drain_value_pools();
        self.instructions.clear();
        self.names.clear();
        self.locals.clear();
        self.nargs = 0;
        self.name = "<module>".to_string();
        self.loop_stack.clear();
        self.in_function = false;
        self.nesting_depth = 0;
        self.current_start_byte = 0;
        self.line_table.clear();
        self.union_checks.clear();
        self.composite_checks.clear();
        self.generic_checks.clear();
        self.void_context = false;
        self.in_optimized_loop = false;
        self.loop_modified_vars.clear();
        self.outer_names.clear();
        self.block_fn_defs.clear();
        self.block_fn_defs.push(Vec::new());
    }

    // ========== Emit helpers ==========

    #[inline]
    pub fn emit(&mut self, op: VMOpCode, arg: u32) -> usize {
        let idx = self.instructions.len();
        self.instructions.push(Instruction::new(op, arg));
        self.line_table.push(self.current_start_byte);
        idx
    }

    #[inline]
    pub fn patch(&mut self, idx: usize, arg: u32) {
        self.instructions[idx].arg = arg;
    }

    #[inline]
    pub fn encode_for_range_args(slot_i: usize, slot_stop: usize, step_positive: bool, jump_offset: usize) -> u32 {
        use super::{
            FOR_RANGE_JUMP_MASK, FOR_RANGE_SLOT_I_SHIFT, FOR_RANGE_SLOT_STOP_SHIFT, FOR_RANGE_STEP_SIGN_SHIFT,
        };
        let step_bit = if step_positive { 0 } else { 1 };
        ((slot_i as u32) << FOR_RANGE_SLOT_I_SHIFT)
            | ((slot_stop as u32) << FOR_RANGE_SLOT_STOP_SHIFT)
            | (step_bit << FOR_RANGE_STEP_SIGN_SHIFT)
            | ((jump_offset as u32) & FOR_RANGE_JUMP_MASK)
    }

    pub fn encode_for_range_step(slot_i: usize, step: i64, jump_target: usize) -> u32 {
        use super::{FOR_RANGE_SLOT_I_SHIFT, FOR_RANGE_STEP_JUMP_MASK, FOR_RANGE_STEP_SHIFT};
        ((slot_i as u32) << FOR_RANGE_SLOT_I_SHIFT)
            | (((step as i8 as u8) as u32) << FOR_RANGE_STEP_SHIFT)
            | ((jump_target as u32) & FOR_RANGE_STEP_JUMP_MASK)
    }

    // ========== Constant pool ==========

    /// Add a constant Value and return its index (dedup by equality).
    /// Takes ownership: the pool keeps the candidate's reference, or releases
    /// it when an equal constant is already pooled (heap values -- strings,
    /// BigInt, Complex -- compare by value, so a deduped candidate is a
    /// distinct allocation to free).
    pub fn add_const(&mut self, value: V) -> usize {
        for (i, existing) in self.constants.iter().enumerate() {
            if *existing == value {
                value.release();
                return i;
            }
        }
        let idx = self.constants.len();
        self.constants.push(value);
        idx
    }

    /// Add a constant i64 value (safe for any i64, uses BigInt if out of SmallInt range).
    pub fn add_const_i64(&mut self, value: i64) -> usize {
        self.add_const(V::scalar_from_i64(value))
    }

    // ========== Name/local management ==========

    pub fn add_name(&mut self, name: &str) -> usize {
        if let Some(idx) = self.names.iter().position(|n| n == name) {
            return idx;
        }
        let idx = self.names.len();
        self.names.push(name.to_string());
        idx
    }

    /// Register a type-union member spec and return its `CheckUnion` arg index.
    pub fn add_union_check(&mut self, members: Box<[ParamCheck]>) -> usize {
        let idx = self.union_checks.len();
        self.union_checks.push(members);
        idx
    }

    /// Register a composite spec and return its `CheckComposite` arg index.
    pub fn add_composite_check(&mut self, check: ParamCheck) -> usize {
        let idx = self.composite_checks.len();
        self.composite_checks.push(check);
        idx
    }

    /// Register a generic-nominal spec and return its `CheckGeneric` arg index.
    pub fn add_generic_check(&mut self, check: ParamCheck) -> usize {
        let idx = self.generic_checks.len();
        self.generic_checks.push(check);
        idx
    }

    pub fn add_local(&mut self, name: &str) -> usize {
        if let Some(idx) = self.locals.iter().position(|n| n == name) {
            return idx;
        }
        let idx = self.locals.len();
        self.locals.push(name.to_string());
        idx
    }

    #[inline]
    pub fn get_local_slot(&self, name: &str) -> Option<usize> {
        self.locals.iter().position(|n| n == name)
    }

    // ========== Shared compile helpers ==========

    /// Emit a load for a variable name (infallible).
    pub fn compile_name_load(&mut self, name: &str) {
        if let Some(slot) = self.get_local_slot(name) {
            self.emit(VMOpCode::LoadLocal, slot as u32);
        } else {
            let idx = self.add_name(name);
            self.emit(VMOpCode::LoadScope, idx as u32);
            // Track names read from outer scope (closure captures)
            if self.in_function {
                self.outer_names.insert(name.to_string());
            }
        }
    }

    pub fn compile_break(&mut self) -> Result<(), SyntaxError> {
        if self.loop_stack.is_empty() {
            return Err(SyntaxError("'break' outside loop"));
        }
        let ctx = self.loop_stack.last_mut().unwrap();
        if ctx.is_for_loop {
            self.emit(VMOpCode::PopTop, 0); // Pop iterator
        }
        // Sync loop-modified vars before breaking out
        if self.in_optimized_loop {
            self.emit_loop_sync();
        }
        let addr = self.emit(VMOpCode::Jump, 0);
        self.loop_stack.last_mut().unwrap().break_targets.push(addr);
        Ok(())
    }

    pub fn compile_continue(&mut self) -> Result<(), SyntaxError> {
        if self.loop_stack.is_empty() {
            return Err(SyntaxError("'continue' outside loop"));
        }
        // Sync loop-modified vars before continuing
        if self.in_optimized_loop {
            self.emit_loop_sync();
        }
        let ctx = self.loop_stack.last().unwrap();
        match ctx.continue_target {
            Some(target) => {
                self.emit(VMOpCode::Jump, target as u32);
            }
            None => {
                // For-range: continue_target not yet known, emit placeholder and patch later
                let addr = self.emit(VMOpCode::Jump, 0);
                self.loop_stack.last_mut().unwrap().continue_patches.push(addr);
            }
        }
        Ok(())
    }

    pub fn emit_loop_sync(&mut self) {
        for (name, slot) in self.loop_modified_vars.clone() {
            self.emit(VMOpCode::LoadLocal, slot as u32);
            let name_idx = self.add_name(&name);
            self.emit(VMOpCode::StoreScope, name_idx as u32);
        }
    }

    // ========== Variable store ==========

    /// Emit a context-aware store for a variable name.
    ///
    /// Strategy:
    /// - Parameter slot (< nargs): always StoreLocal
    /// - Module level (nesting_depth == 0):
    ///   - Optimized loop: StoreLocal only, defer scope sync via loop_modified_vars
    ///   - Normal: DupTop + StoreLocal + StoreScope (JIT + LoadScope access)
    /// - Function scope (nesting_depth > 0):
    ///   - Captured outer names: StoreScope (closure semantics)
    ///   - Function-local: StoreLocal only
    pub fn emit_store(&mut self, name: &str) {
        let existing_slot = self.locals.iter().position(|n| n == name);

        if let Some(slot) = existing_slot {
            if slot < self.nargs {
                // Parameter - always StoreLocal
                self.emit(VMOpCode::StoreLocal, slot as u32);
            } else if self.nesting_depth == 0 {
                if self.in_optimized_loop {
                    self.emit(VMOpCode::StoreLocal, slot as u32);
                    if !self.loop_modified_vars.iter().any(|(n, _)| n == name) {
                        self.loop_modified_vars.push((name.to_string(), slot));
                    }
                } else {
                    // Module level: BOTH StoreLocal (JIT) + StoreScope (LoadScope)
                    self.emit(VMOpCode::DupTop, 0);
                    self.emit(VMOpCode::StoreLocal, slot as u32);
                    let name_idx = self.add_name(name);
                    self.emit(VMOpCode::StoreScope, name_idx as u32);
                }
            } else if self.outer_names.contains(name) {
                // Outer name that also owns a slot: keep both in sync, like the
                // module-level path above -- a scope-only store would leave the
                // slot stale for later LoadLocal reads.
                self.emit(VMOpCode::DupTop, 0);
                self.emit(VMOpCode::StoreLocal, slot as u32);
                let name_idx = self.add_name(name);
                self.emit(VMOpCode::StoreScope, name_idx as u32);
            } else {
                // Function-local
                self.emit(VMOpCode::StoreLocal, slot as u32);
            }
        } else if self.nesting_depth > 0 && self.outer_names.contains(name) {
            // Write-through to the enclosing scope from a closure: no local
            // slot. Registering one here would flip later reads of the name to
            // LoadLocal on a slot this store never fills (uninitialized read),
            // and the reads must stay LoadScope anyway -- resolution by name at
            // read time, so a mutation of the outer binding between this write
            // and a later read stays visible.
            let name_idx = self.add_name(name);
            self.emit(VMOpCode::StoreScope, name_idx as u32);
        } else {
            // New variable
            let slot = self.add_local(name);
            if self.nesting_depth == 0 {
                if self.in_optimized_loop {
                    self.emit(VMOpCode::StoreLocal, slot as u32);
                    if !self.loop_modified_vars.iter().any(|(n, _)| n == name) {
                        self.loop_modified_vars.push((name.to_string(), slot));
                    }
                } else {
                    self.emit(VMOpCode::DupTop, 0);
                    self.emit(VMOpCode::StoreLocal, slot as u32);
                    let name_idx = self.add_name(name);
                    self.emit(VMOpCode::StoreScope, name_idx as u32);
                }
            } else {
                self.emit(VMOpCode::StoreLocal, slot as u32);
            }
        }
    }
}

impl<V: CompilerValue, P: CompilerPattern> Default for CompilerCore<V, P> {
    fn default() -> Self {
        Self::new()
    }
}

/// A compiler dropped mid-compile (error path, no `reset`) still holds
/// references in its buffers; empty after a successful build (pools moved).
impl<V: CompilerValue, P: CompilerPattern> Drop for CompilerCore<V, P> {
    fn drop(&mut self) {
        self.drain_value_pools();
    }
}
