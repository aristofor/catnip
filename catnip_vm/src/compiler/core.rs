// FILE: catnip_vm/src/compiler/core.rs
//! Shared compiler state and helper methods (pure Rust, no PyO3).
//!
//! Ported from catnip_rs/src/vm/compiler_core.rs with all PyO3 dependencies removed.

use super::code_object::CodeObject;
use super::error::{CompileError, CompileResult};
use super::pattern::VMPattern;
use crate::Value;
use catnip_core::vm::opcode::{Instruction, VMOpCode};
use catnip_core::vm::peephole::PeepholeOptimizer;
use std::collections::{HashMap, HashSet};

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
pub struct CompilerCore {
    /// Emitted bytecode
    pub instructions: Vec<Instruction>,
    /// Constant pool (unified as Value)
    pub constants: Vec<Value>,
    /// Name table (for global/scope variables)
    pub names: Vec<String>,
    /// Local variable slots
    pub locals: Vec<String>,
    /// Number of function parameters
    pub nargs: usize,
    /// Default parameter values
    pub defaults: Vec<Value>,
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
    pub patterns: Vec<VMPattern>,
    /// Whether the current expression result is unused
    pub void_context: bool,
    /// Whether we're in an optimized loop (local-only variables)
    pub in_optimized_loop: bool,
    /// Variables modified inside an optimized loop (name, local slot)
    pub loop_modified_vars: Vec<(String, usize)>,
    /// Names read from outer scope (closure captures)
    pub outer_names: HashSet<String>,
}

impl CompilerCore {
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
            void_context: false,
            in_optimized_loop: false,
            loop_modified_vars: Vec::new(),
            outer_names: HashSet::new(),
        }
    }

    pub fn reset(&mut self) {
        self.instructions.clear();
        self.constants.clear();
        self.names.clear();
        self.locals.clear();
        self.nargs = 0;
        self.defaults.clear();
        self.name = "<module>".to_string();
        self.loop_stack.clear();
        self.in_function = false;
        self.nesting_depth = 0;
        self.current_start_byte = 0;
        self.line_table.clear();
        self.patterns.clear();
        self.void_context = false;
        self.in_optimized_loop = false;
        self.loop_modified_vars.clear();
        self.outer_names.clear();
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
        use catnip_core::vm::{
            FOR_RANGE_JUMP_MASK, FOR_RANGE_SLOT_I_SHIFT, FOR_RANGE_SLOT_STOP_SHIFT, FOR_RANGE_STEP_SIGN_SHIFT,
        };
        let step_bit = if step_positive { 0 } else { 1 };
        ((slot_i as u32) << FOR_RANGE_SLOT_I_SHIFT)
            | ((slot_stop as u32) << FOR_RANGE_SLOT_STOP_SHIFT)
            | (step_bit << FOR_RANGE_STEP_SIGN_SHIFT)
            | ((jump_offset as u32) & FOR_RANGE_JUMP_MASK)
    }

    pub fn encode_for_range_step(slot_i: usize, step: i64, jump_target: usize) -> u32 {
        use catnip_core::vm::{FOR_RANGE_SLOT_I_SHIFT, FOR_RANGE_STEP_JUMP_MASK, FOR_RANGE_STEP_SHIFT};
        ((slot_i as u32) << FOR_RANGE_SLOT_I_SHIFT)
            | (((step as i8 as u8) as u32) << FOR_RANGE_STEP_SHIFT)
            | ((jump_target as u32) & FOR_RANGE_STEP_JUMP_MASK)
    }

    // ========== Constant pool ==========

    /// Add a constant Value and return its index (dedup by equality).
    pub fn add_const(&mut self, value: Value) -> usize {
        for (i, existing) in self.constants.iter().enumerate() {
            if *existing == value {
                return i;
            }
        }
        let idx = self.constants.len();
        self.constants.push(value);
        idx
    }

    /// Add a constant i64 value (safe for any i64, uses BigInt if out of SmallInt range).
    pub fn add_const_i64(&mut self, value: i64) -> usize {
        self.add_const(Value::from_i64(value))
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

    pub fn compile_name_load(&mut self, name: &str) -> CompileResult<()> {
        if let Some(slot) = self.get_local_slot(name) {
            self.emit(VMOpCode::LoadLocal, slot as u32);
        } else {
            let idx = self.add_name(name);
            self.emit(VMOpCode::LoadScope, idx as u32);
            if self.in_function {
                self.outer_names.insert(name.to_string());
            }
        }
        Ok(())
    }

    pub fn compile_break(&mut self) -> CompileResult<()> {
        if self.loop_stack.is_empty() {
            return Err(CompileError::SyntaxError("'break' outside loop".to_string()));
        }
        let ctx = self.loop_stack.last_mut().unwrap();
        if ctx.is_for_loop {
            self.emit(VMOpCode::PopTop, 0);
        }
        if self.in_optimized_loop {
            self.emit_loop_sync();
        }
        let addr = self.emit(VMOpCode::Jump, 0);
        self.loop_stack.last_mut().unwrap().break_targets.push(addr);
        Ok(())
    }

    pub fn compile_continue(&mut self) -> CompileResult<()> {
        if self.loop_stack.is_empty() {
            return Err(CompileError::SyntaxError("'continue' outside loop".to_string()));
        }
        if self.in_optimized_loop {
            self.emit_loop_sync();
        }
        let ctx = self.loop_stack.last().unwrap();
        match ctx.continue_target {
            Some(target) => {
                self.emit(VMOpCode::Jump, target as u32);
            }
            None => {
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
    pub fn emit_store(&mut self, name: &str) {
        let existing_slot = self.locals.iter().position(|n| n == name);

        if let Some(slot) = existing_slot {
            if slot < self.nargs {
                self.emit(VMOpCode::StoreLocal, slot as u32);
            } else if self.nesting_depth == 0 {
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
            } else if self.outer_names.contains(name) {
                let name_idx = self.add_name(name);
                self.emit(VMOpCode::StoreScope, name_idx as u32);
            } else {
                self.emit(VMOpCode::StoreLocal, slot as u32);
            }
        } else {
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
            } else if self.outer_names.contains(name) {
                let name_idx = self.add_name(name);
                self.emit(VMOpCode::StoreScope, name_idx as u32);
            } else {
                self.emit(VMOpCode::StoreLocal, slot as u32);
            }
        }
    }

    // ========== Code object ==========

    pub fn build_code_object(&self) -> CompileResult<CodeObject> {
        let slotmap: HashMap<String, usize> = self
            .locals
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), i))
            .collect();

        let (optimized_instructions, optimized_line_table) =
            PeepholeOptimizer::optimize(self.instructions.clone(), self.line_table.clone());

        let complexity = optimized_instructions.len();

        Ok(CodeObject {
            instructions: optimized_instructions,
            constants: self.constants.clone(),
            names: self.names.clone(),
            nlocals: self.locals.len(),
            varnames: self.locals.clone(),
            slotmap,
            nargs: self.nargs,
            defaults: self.defaults.clone(),
            name: self.name.clone(),
            freevars: Vec::new(),
            vararg_idx: -1,
            is_pure: false,
            complexity,
            line_table: optimized_line_table,
            patterns: self.patterns.clone(),
            encoded_ir: None,
        })
    }
}

impl Default for CompilerCore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
