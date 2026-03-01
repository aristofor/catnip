// FILE: catnip_rs/src/vm/compiler.rs
//! Bytecode compiler: converts Op nodes (IR) to CodeObject bytecode.
//!
//! Port of catnip/vm/compiler.py to Rust for zero-overhead compilation.

use super::frame::{CodeObject, PyCodeObject};
use super::opcode::{Instruction, VMOpCode};
use super::pattern::{VMPattern, VMPatternElement};
use super::value::Value;
use crate::core::nodes::Ref;
use crate::core::pattern::*;
use crate::core::Op;
use crate::ir::IROpCode;
use crate::transformer::Lvalue;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use std::collections::{HashMap, HashSet};

/// Loop context for break/continue compilation.
struct LoopContext {
    /// Addresses of JUMP instructions to patch to loop end (for break)
    break_targets: Vec<usize>,
    /// Address to jump to for continue (known at emission time for while/for-iter)
    continue_target: usize,
    /// Addresses of JUMP instructions to patch to increment (for continue in for-range)
    continue_patches: Vec<usize>,
    /// True if this is a for loop (has iterator on stack)
    is_for_loop: bool,
}

/// Bytecode compiler state.
pub struct Compiler {
    /// Emitted bytecode
    instructions: Vec<Instruction>,
    /// Constant pool
    constants: Vec<Py<PyAny>>,
    /// Variable names for LOAD_NAME/STORE_NAME
    names: Vec<String>,
    /// Local variable names (slot allocation)
    locals: Vec<String>,
    /// Number of parameters
    nargs: usize,
    /// Default parameter values
    defaults: Vec<Py<PyAny>>,
    /// Function/module name
    name: String,
    /// Loop stack for break/continue
    loop_stack: Vec<LoopContext>,
    /// True when compiling function/lambda body (vs module level)
    in_function: bool,
    /// Function nesting depth (0=module, 1=top-level fn, 2+=nested/closures)
    nesting_depth: u32,
    /// Current source position (start_byte from the last Op node)
    current_start_byte: u32,
    /// Source position table: one entry per emitted instruction
    line_table: Vec<u32>,
    /// Pre-compiled VM-native patterns for match expressions
    patterns: Vec<VMPattern>,
    /// When true, SetLocals skips the DupTop that preserves expression value
    void_context: bool,
    /// When true, stores inside loop only use StoreLocal (deferred scope sync)
    in_optimized_loop: bool,
    /// Variables modified inside optimized loop: (name, local_slot)
    loop_modified_vars: Vec<(String, usize)>,
    /// Names loaded via LoadScope before first assignment (closure captures)
    outer_names: HashSet<String>,
}

impl Compiler {
    /// Create a new compiler instance.
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

    /// Reset compiler state for new compilation.
    fn reset(&mut self) {
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

    /// Compile an IR node to a CodeObject.
    pub fn compile(&mut self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<CodeObject> {
        self.reset();
        self.compile_node(py, node)?;
        self.emit(VMOpCode::Halt, 0);
        self.build_code_object(py)
    }

    /// Compile a function body with parameters.
    pub fn compile_function(
        &mut self,
        py: Python<'_>,
        params: Vec<String>,
        body: &Bound<'_, PyAny>,
        name: &str,
        defaults: Vec<Py<PyAny>>,
        vararg_idx: i32,
        parent_nesting_depth: u32,
    ) -> PyResult<CodeObject> {
        self.reset();
        self.name = name.to_string();
        self.nargs = params.len();
        self.defaults = defaults;
        self.in_function = true; // Use StoreLocal for function-level vars
        self.nesting_depth = parent_nesting_depth + 1;

        // Parameters become local slots
        for param in params {
            self.add_local(&param);
        }

        // Compile body
        self.compile_node(py, body)?;

        // Implicit return
        self.emit(VMOpCode::Return, 0);

        let mut code = self.build_code_object(py)?;
        code.vararg_idx = vararg_idx;
        Ok(code)
    }

    /// Build CodeObject from current compiler state.
    fn build_code_object(&self, py: Python<'_>) -> PyResult<CodeObject> {
        let slotmap: HashMap<String, usize> = self
            .locals
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), i))
            .collect();

        // Convert PyObject constants to Value
        let mut constants: Vec<Value> = Vec::with_capacity(self.constants.len());
        for obj in &self.constants {
            let value = Value::from_pyobject(py, obj.bind(py))?;
            constants.push(value);
        }

        // Clone defaults by re-binding
        let defaults: Vec<Py<PyAny>> = self.defaults.iter().map(|obj| obj.clone_ref(py)).collect();

        // Apply peephole optimization to bytecode (with line_table)
        let (optimized_instructions, optimized_line_table) = crate::vm::PeepholeOptimizer::optimize(
            self.instructions.clone(),
            self.line_table.clone(),
        );

        let complexity = optimized_instructions.len();

        Ok(CodeObject {
            instructions: optimized_instructions,
            constants,
            names: self.names.clone(),
            nlocals: self.locals.len(),
            varnames: self.locals.clone(),
            slotmap,
            nargs: self.nargs,
            defaults,
            name: self.name.clone(),
            freevars: Vec::new(),
            vararg_idx: -1,
            is_pure: false,
            complexity,
            line_table: optimized_line_table,
            patterns: self.patterns.clone(),
        })
    }

    // ========== Emit helpers ==========

    /// Emit a bytecode instruction, return its index.
    #[inline]
    fn emit(&mut self, op: VMOpCode, arg: u32) -> usize {
        let idx = self.instructions.len();
        self.instructions.push(Instruction::new(op, arg));
        self.line_table.push(self.current_start_byte);
        idx
    }

    /// Patch the argument of an instruction (for jumps).
    #[inline]
    fn patch(&mut self, idx: usize, arg: u32) {
        self.instructions[idx].arg = arg;
    }

    /// Encode ForRangeInt opcode arguments.
    /// Packs slot_i, slot_stop, step_sign, and jump_offset into a single u32.
    /// Layout: (slot_i << 24) | (slot_stop << 16) | (step_sign << 15) | jump_offset
    #[inline]
    fn encode_for_range_args(
        slot_i: usize,
        slot_stop: usize,
        step_positive: bool,
        jump_offset: usize,
    ) -> u32 {
        const SLOT_I_SHIFT: u32 = 24;
        const SLOT_STOP_SHIFT: u32 = 16;
        const STEP_SIGN_SHIFT: u32 = 15;

        let step_bit = if step_positive { 0 } else { 1 };
        ((slot_i as u32) << SLOT_I_SHIFT)
            | ((slot_stop as u32) << SLOT_STOP_SHIFT)
            | (step_bit << STEP_SIGN_SHIFT)
            | ((jump_offset as u32) & 0x7FFF)
    }

    /// Encode ForRangeStep arg: slot_i (8 bits) | step as u8 (8 bits) | jump_target (16 bits)
    fn encode_for_range_step(slot_i: usize, step: i64, jump_target: usize) -> u32 {
        ((slot_i as u32) << 24)
            | (((step as i8 as u8) as u32) << 16)
            | ((jump_target as u32) & 0xFFFF)
    }

    /// Try to extract a negated literal: Op(NEG, [int]) -> -int
    fn try_extract_neg_literal(py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<i64> {
        let op: PyRef<Op> = node.extract()?;
        if op.ident != IROpCode::Neg as i32 {
            return Err(pyo3::exceptions::PyValueError::new_err("not a NEG op"));
        }
        let args = op.args.bind(py);
        if args.len()? != 1 {
            return Err(pyo3::exceptions::PyValueError::new_err("NEG expects 1 arg"));
        }
        let val: i64 = args.get_item(0)?.extract()?;
        Ok(-val)
    }

    /// Add a constant and return its index.
    fn add_const(&mut self, py: Python<'_>, value: &Bound<'_, PyAny>) -> usize {
        // Check for existing constant (identity-aware)
        for (i, existing) in self.constants.iter().enumerate() {
            if existing.bind(py).is(value) {
                return i;
            }
        }
        let idx = self.constants.len();
        self.constants.push(value.clone().unbind());
        idx
    }

    /// Add an i64 as constant.
    fn add_const_i64(&mut self, py: Python<'_>, value: i64) -> usize {
        // Use Python's int() builtin
        let builtins = py.import("builtins").unwrap();
        let int_fn = builtins.getattr("int").unwrap();
        let obj = int_fn.call1((value,)).unwrap();
        let idx = self.constants.len();
        self.constants.push(obj.unbind());
        idx
    }

    /// Wrapper that accepts py.None() directly
    fn add_const_value(&mut self, _py: Python<'_>, value: Py<PyAny>) -> usize {
        let idx = self.constants.len();
        self.constants.push(value);
        idx
    }

    /// Add a name and return its index.
    fn add_name(&mut self, name: &str) -> usize {
        if let Some(idx) = self.names.iter().position(|n| n == name) {
            return idx;
        }
        let idx = self.names.len();
        self.names.push(name.to_string());
        idx
    }

    /// Add a local variable and return its slot index.
    fn add_local(&mut self, name: &str) -> usize {
        if let Some(idx) = self.locals.iter().position(|n| n == name) {
            return idx;
        }
        let idx = self.locals.len();
        self.locals.push(name.to_string());
        idx
    }

    /// Get slot index for a local variable.
    #[inline]
    fn get_local_slot(&self, name: &str) -> Option<usize> {
        self.locals.iter().position(|n| n == name)
    }

    // ========== Node compilation ==========

    /// Compile any node type.
    fn compile_node(&mut self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<()> {
        // Handle list of statements
        if let Ok(list) = node.cast::<PyList>() {
            return self.compile_statement_list(py, list);
        }

        // Handle Broadcast nodes (before Op check)
        let type_name = node.get_type().name()?;
        if type_name == "Broadcast" {
            return self.compile_broadcast(py, node);
        }

        // Handle Call nodes (from pure_transforms)
        if type_name == "Call" {
            return self.compile_call_node(py, node);
        }

        // Handle Op nodes
        if let Ok(op) = node.extract::<PyRef<Op>>() {
            return self.compile_op(py, node, &op);
        }

        // Check for Ref (variable reference)
        if type_name == "Ref" {
            let sb: isize = node.getattr("start_byte")?.extract()?;
            if sb >= 0 {
                self.current_start_byte = sb as u32;
            }
            let ident: String = node.getattr("ident")?.extract()?;
            return self.compile_name_load(&ident);
        }

        // Literal value
        self.compile_literal(py, node)
    }

    /// Compile a list of statements.
    fn compile_statement_list(
        &mut self,
        py: Python<'_>,
        stmts: &Bound<'_, PyList>,
    ) -> PyResult<()> {
        if stmts.is_empty() {
            let idx = self.add_const_value(py, py.None());
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        let len = stmts.len();
        for (i, stmt) in stmts.iter().enumerate() {
            self.compile_node(py, &stmt)?;
            // Pop intermediate results, keep last
            if i < len - 1 {
                self.emit(VMOpCode::PopTop, 0);
            }
        }
        Ok(())
    }

    /// Compile an Op node.
    fn compile_op(&mut self, py: Python<'_>, _node: &Bound<'_, PyAny>, op: &Op) -> PyResult<()> {
        // Track source position for line_table
        if op.start_byte >= 0 {
            self.current_start_byte = op.start_byte as u32;
        }

        let args = op.args.bind(py);
        let kwargs = op.kwargs.bind(py);

        let ident = IROpCode::from_u8(op.ident as u8).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid IR opcode: {}", op.ident))
        })?;

        match ident {
            // Arithmetic
            IROpCode::Add => self.compile_binary(py, VMOpCode::Add, args),
            IROpCode::Sub => self.compile_binary(py, VMOpCode::Sub, args),
            IROpCode::Mul => self.compile_binary(py, VMOpCode::Mul, args),
            IROpCode::Div | IROpCode::TrueDiv => self.compile_binary(py, VMOpCode::Div, args),
            IROpCode::FloorDiv => self.compile_binary(py, VMOpCode::FloorDiv, args),
            IROpCode::Mod => self.compile_binary(py, VMOpCode::Mod, args),
            IROpCode::Pow => self.compile_binary(py, VMOpCode::Pow, args),
            IROpCode::Neg => self.compile_unary(py, VMOpCode::Neg, args),
            IROpCode::Pos => self.compile_unary(py, VMOpCode::Pos, args),

            // Comparison
            IROpCode::Lt => self.compile_binary(py, VMOpCode::Lt, args),
            IROpCode::Le => self.compile_binary(py, VMOpCode::Le, args),
            IROpCode::Gt => self.compile_binary(py, VMOpCode::Gt, args),
            IROpCode::Ge => self.compile_binary(py, VMOpCode::Ge, args),
            IROpCode::Eq => self.compile_binary(py, VMOpCode::Eq, args),
            IROpCode::Ne => self.compile_binary(py, VMOpCode::Ne, args),

            // Logical
            IROpCode::Not => self.compile_unary(py, VMOpCode::Not, args),
            IROpCode::And => self.compile_and(py, args),
            IROpCode::Or => self.compile_or(py, args),

            // Bitwise
            IROpCode::BAnd => self.compile_binary(py, VMOpCode::BAnd, args),
            IROpCode::BOr => self.compile_binary(py, VMOpCode::BOr, args),
            IROpCode::BXor => self.compile_binary(py, VMOpCode::BXor, args),
            IROpCode::BNot => self.compile_unary(py, VMOpCode::BNot, args),
            IROpCode::LShift => self.compile_binary(py, VMOpCode::LShift, args),
            IROpCode::RShift => self.compile_binary(py, VMOpCode::RShift, args),

            // Variables
            IROpCode::SetLocals => self.compile_set_locals(py, args, kwargs.cast()?),
            IROpCode::GetAttr => self.compile_getattr(py, args),
            IROpCode::SetAttr => self.compile_setattr(py, args),
            IROpCode::GetItem => self.compile_getitem(py, args),
            IROpCode::SetItem => self.compile_setitem(py, args),
            IROpCode::Slice => self.compile_slice(py, args),

            // Control flow
            IROpCode::OpIf => self.compile_if(py, args),
            IROpCode::OpWhile => self.compile_while(py, args),
            IROpCode::OpFor => self.compile_for(py, args),
            IROpCode::OpBlock => self.compile_block(py, args),
            IROpCode::OpReturn => self.compile_return(py, args),
            IROpCode::OpBreak => self.compile_break(),
            IROpCode::OpContinue => self.compile_continue(),

            // Functions
            IROpCode::Call => self.compile_call(py, args, kwargs.cast()?, op.tail),
            IROpCode::OpLambda => self.compile_lambda(py, args),
            IROpCode::FnDef => self.compile_fn_def(py, args, kwargs.cast()?),

            // Collections
            IROpCode::ListLiteral => self.compile_list(py, args),
            IROpCode::TupleLiteral => self.compile_tuple(py, args),
            IROpCode::SetLiteral => self.compile_set(py, args),
            IROpCode::DictLiteral => self.compile_dict(py, args),

            // String
            IROpCode::Fstring => self.compile_fstring(py, args),

            // Match
            IROpCode::OpMatch => self.compile_match(py, args),

            // Broadcasting
            IROpCode::Broadcast => self.compile_broadcast_op(py, args),

            // ND operations
            IROpCode::NdEmptyTopos => {
                self.emit(VMOpCode::NdEmptyTopos, 0);
                Ok(())
            }
            IROpCode::NdRecursion => self.compile_nd_recursion(py, args),
            IROpCode::NdMap => self.compile_nd_map(py, args),

            // Stack ops
            IROpCode::Push => {
                let item = args.get_item(0)?;
                self.compile_node(py, &item)
            }
            IROpCode::Pop => {
                self.emit(VMOpCode::PopTop, 0);
                Ok(())
            }
            IROpCode::Nop => {
                self.emit(VMOpCode::Nop, 0);
                Ok(())
            }

            // Pragma - already processed by semantic analyzer, compile to nop
            IROpCode::Pragma => {
                self.emit(VMOpCode::Nop, 0);
                Ok(())
            }

            IROpCode::Breakpoint => {
                self.emit(VMOpCode::Breakpoint, 0);
                Ok(())
            }

            IROpCode::OpStruct => self.compile_struct(py, args),
            IROpCode::TraitDef => self.compile_trait(py, args),

            _ => Err(pyo3::exceptions::PyNotImplementedError::new_err(format!(
                "Cannot compile IR opcode: {}",
                ident
            ))),
        }
    }

    /// Compile a literal value.
    fn compile_literal(&mut self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let idx = self.add_const(py, value);
        self.emit(VMOpCode::LoadConst, idx as u32);
        Ok(())
    }

    /// Compile a name load (variable reference).
    fn compile_name_load(&mut self, name: &str) -> PyResult<()> {
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
        Ok(())
    }

    // ========== Binary/Unary operations ==========

    fn compile_binary(
        &mut self,
        py: Python<'_>,
        vm_op: VMOpCode,
        args: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        // Handle two IR formats: (left, right) or ([left, right],)
        let (left, right) = if args.len()? == 1 {
            let inner = args.get_item(0)?;
            if inner.is_instance_of::<PyList>() || inner.is_instance_of::<PyTuple>() {
                (inner.get_item(0)?, inner.get_item(1)?)
            } else {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "Invalid binary args",
                ));
            }
        } else {
            (args.get_item(0)?, args.get_item(1)?)
        };

        self.compile_node(py, &left)?;
        self.compile_node(py, &right)?;
        self.emit(vm_op, 0);
        Ok(())
    }

    fn compile_unary(
        &mut self,
        py: Python<'_>,
        vm_op: VMOpCode,
        args: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let operand = args.get_item(0)?;
        self.compile_node(py, &operand)?;
        self.emit(vm_op, 0);
        Ok(())
    }

    fn compile_and(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let operands = args.get_item(0)?;
        self.compile_node(py, &operands.get_item(0)?)?;
        let jump_idx = self.emit(VMOpCode::JumpIfFalseOrPop, 0);
        self.compile_node(py, &operands.get_item(1)?)?;
        self.patch(jump_idx, self.instructions.len() as u32);
        Ok(())
    }

    fn compile_or(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let operands = args.get_item(0)?;
        self.compile_node(py, &operands.get_item(0)?)?;
        let jump_idx = self.emit(VMOpCode::JumpIfTrueOrPop, 0);
        self.compile_node(py, &operands.get_item(1)?)?;
        self.patch(jump_idx, self.instructions.len() as u32);
        Ok(())
    }

    // ========== Variables ==========

    fn compile_set_locals(
        &mut self,
        py: Python<'_>,
        args: &Bound<'_, PyAny>,
        kwargs: &Bound<'_, PyDict>,
    ) -> PyResult<()> {
        // Check if last arg is a boolean explicit_unpack flag
        let args_len = args.len()?;
        let mut args_iter: Vec<Bound<'_, PyAny>> = args.try_iter()?.map(|r| r.unwrap()).collect();
        let mut explicit_unpack = false;
        if args_len >= 3 {
            if let Some(last) = args_iter.last() {
                if last.is_instance_of::<pyo3::types::PyBool>() {
                    explicit_unpack = last.extract::<bool>().unwrap_or(false);
                    args_iter.pop(); // Remove the boolean flag
                }
            }
        }

        // Detect format: kwargs['names'] or args[0] is tuple of names
        let names_pattern: Option<Bound<'_, PyAny>>;
        let values: Vec<Bound<'_, PyAny>>;

        if let Some(names_obj) = kwargs.get_item("names")? {
            names_pattern = Some(names_obj);
            values = args_iter;
        } else if args_iter.len() >= 2 {
            let first = &args_iter[0];
            if first.is_instance_of::<PyTuple>() {
                names_pattern = Some(first.clone());
                values = args_iter.into_iter().skip(1).collect();
            } else {
                names_pattern = None;
                values = Vec::new();
            }
        } else {
            names_pattern = None;
            values = Vec::new();
        }

        // Capture void_context for this SetLocals, then disable it for sub-expressions.
        // Sub-expressions (rhs values) must always produce values on the stack.
        let is_void = self.void_context;
        self.void_context = false;

        // Check for complex patterns (star, nested).
        // Use VM-native pattern matching + BindMatch so assignment is atomic:
        // either all bindings are committed, or none.
        if let Some(ref pattern) = names_pattern {
            if self.has_complex_pattern(pattern) && values.len() == 1 {
                // Unwrap pattern: ((Ref, ('*', 'name'), Ref),) -> (Ref, ('*', 'name'), Ref)
                let unwrapped = if let Ok(tuple) = pattern.cast::<PyTuple>() {
                    if tuple.len() == 1 {
                        tuple.get_item(0)?
                    } else {
                        pattern.clone()
                    }
                } else {
                    pattern.clone()
                };

                let vm_pattern = self
                    .try_compile_assign_pattern(py, &unwrapped)?
                    .ok_or_else(|| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                            "Unsupported complex assignment pattern in VM compiler",
                        )
                    })?;

                let pat_idx = self.patterns.len();
                self.patterns.push(vm_pattern);

                // Keep one copy on stack as assignment expression value.
                self.compile_node(py, &values[0])?;
                self.emit(VMOpCode::DupTop, 0);
                self.emit(VMOpCode::MatchAssignPatternVM, pat_idx as u32);
                self.emit(VMOpCode::BindMatch, 0);

                // Sync bound names to scope where needed.
                let names_to_sync = self.extract_names(py, &unwrapped)?;
                for name in names_to_sync {
                    let Some(slot) = self.locals.iter().position(|n| n == &name) else {
                        continue;
                    };
                    let needs_scope_sync = if self.nesting_depth == 0 {
                        // Module-level assignment updates scope/globals.
                        true
                    } else {
                        // In function scope, only propagate captured outer names.
                        self.outer_names.contains(&name)
                    };
                    if needs_scope_sync {
                        self.emit(VMOpCode::LoadLocal, slot as u32);
                        let name_idx = self.add_name(&name);
                        self.emit(VMOpCode::StoreScope, name_idx as u32);
                    }
                }

                if is_void {
                    self.emit(VMOpCode::PopTop, 0);
                }
                return Ok(());
            }
        }

        // Simple case: extract flat names
        let names: Vec<String> = if let Some(ref pattern) = names_pattern {
            self.extract_names(py, pattern)?
        } else {
            Vec::new()
        };

        if names.is_empty() {
            let idx = self.add_const_value(py, py.None());
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        // Helper closure for emitting store instruction
        // Strategy: Module level uses BOTH StoreLocal (JIT) + StoreScope (LoadScope access)
        // Functions use StoreScope for non-parameters to preserve closure semantics
        // In optimized loops (nesting_depth==0): StoreLocal only, scope sync deferred
        let emit_store = |s: &mut Self, name: &str| {
            // Check if variable is already a local (parameter or previously defined)
            let existing_slot = s.locals.iter().position(|n| n == name);

            if let Some(slot) = existing_slot {
                // Variable already in locals
                if slot < s.nargs {
                    // This is a parameter - always use StoreLocal (owned by function)
                    s.emit(VMOpCode::StoreLocal, slot as u32);
                } else if s.nesting_depth == 0 {
                    if s.in_optimized_loop {
                        // Loop-local: StoreLocal only, defer scope sync
                        s.emit(VMOpCode::StoreLocal, slot as u32);
                        if !s.loop_modified_vars.iter().any(|(n, _)| n == name) {
                            s.loop_modified_vars.push((name.to_string(), slot));
                        }
                    } else {
                        // Module level - use BOTH (StoreLocal for JIT, StoreScope for LoadScope)
                        s.emit(VMOpCode::DupTop, 0);
                        s.emit(VMOpCode::StoreLocal, slot as u32);
                        let name_idx = s.add_name(name);
                        s.emit(VMOpCode::StoreScope, name_idx as u32);
                    }
                } else if s.outer_names.contains(name) {
                    // Closure capture - use StoreScope (propagates to parent chain)
                    let name_idx = s.add_name(name);
                    s.emit(VMOpCode::StoreScope, name_idx as u32);
                } else {
                    // Function-local variable - StoreLocal only (no globals pollution)
                    s.emit(VMOpCode::StoreLocal, slot as u32);
                }
            } else {
                // New variable - add to locals
                let slot = s.add_local(name);
                if s.nesting_depth == 0 {
                    if s.in_optimized_loop {
                        // Loop-local: StoreLocal only, defer scope sync
                        s.emit(VMOpCode::StoreLocal, slot as u32);
                        if !s.loop_modified_vars.iter().any(|(n, _)| n == name) {
                            s.loop_modified_vars.push((name.to_string(), slot));
                        }
                    } else {
                        // Module level - use BOTH
                        s.emit(VMOpCode::DupTop, 0);
                        s.emit(VMOpCode::StoreLocal, slot as u32);
                        let name_idx = s.add_name(name);
                        s.emit(VMOpCode::StoreScope, name_idx as u32);
                    }
                } else if s.outer_names.contains(name) {
                    // New variable but name was loaded from outer scope - closure capture
                    let name_idx = s.add_name(name);
                    s.emit(VMOpCode::StoreScope, name_idx as u32);
                } else {
                    // New function-local variable - StoreLocal only
                    s.emit(VMOpCode::StoreLocal, slot as u32);
                }
            }
        };

        // Single name, single value: simple assignment (unless explicit_unpack)
        if names.len() == 1 && values.len() == 1 && !explicit_unpack {
            self.compile_node(py, &values[0])?;
            if !is_void {
                self.emit(VMOpCode::DupTop, 0);
            }
            emit_store(self, &names[0]);
            return Ok(());
        }

        // Multiple names OR explicit unpack, single value: unpacking
        if values.len() == 1 && (names.len() > 1 || explicit_unpack) {
            self.compile_node(py, &values[0])?;
            self.emit(VMOpCode::UnpackSequence, names.len() as u32);

            // Store each name
            for (i, name) in names.iter().enumerate() {
                let is_last = i == names.len() - 1;
                if is_last && !is_void {
                    self.emit(VMOpCode::DupTop, 0);
                }
                emit_store(self, name);
            }
            return Ok(());
        }

        // Multiple names, multiple values: parallel assignment
        for (i, name) in names.iter().enumerate() {
            let value = if i < values.len() {
                &values[i]
            } else if !values.is_empty() {
                values.last().unwrap()
            } else {
                let idx = self.add_const_value(py, py.None());
                self.emit(VMOpCode::LoadConst, idx as u32);
                emit_store(self, name);
                continue;
            };

            self.compile_node(py, value)?;

            let is_last = i == names.len() - 1;
            if is_last && !is_void {
                self.emit(VMOpCode::DupTop, 0);
            }

            emit_store(self, name);
        }
        Ok(())
    }

    fn try_compile_assign_pattern(
        &mut self,
        _py: Python<'_>,
        pattern: &Bound<'_, PyAny>,
    ) -> PyResult<Option<VMPattern>> {
        if let Ok(tuple) = pattern.cast::<PyTuple>() {
            let mut elements = Vec::new();
            for item in tuple.iter() {
                if let Ok(star_tuple) = item.cast::<PyTuple>() {
                    if star_tuple.len() == 2 {
                        let first: String = star_tuple.get_item(0)?.extract().unwrap_or_default();
                        if first == "*" {
                            let star_name =
                                self.extract_single_name(_py, &star_tuple.get_item(1)?)?;
                            let star_slot = self.add_local(&star_name);
                            elements.push(VMPatternElement::Star(star_slot));
                            continue;
                        }
                    }
                }

                match self.try_compile_assign_pattern(_py, &item)? {
                    Some(sub) => elements.push(VMPatternElement::Pattern(sub)),
                    None => return Ok(None),
                }
            }
            return Ok(Some(VMPattern::Tuple(elements)));
        }

        if let Ok(list) = pattern.cast::<PyList>() {
            let mut elements = Vec::new();
            for item in list.iter() {
                if let Ok(star_tuple) = item.cast::<PyTuple>() {
                    if star_tuple.len() == 2 {
                        let first: String = star_tuple.get_item(0)?.extract().unwrap_or_default();
                        if first == "*" {
                            let star_name =
                                self.extract_single_name(_py, &star_tuple.get_item(1)?)?;
                            let star_slot = self.add_local(&star_name);
                            elements.push(VMPatternElement::Star(star_slot));
                            continue;
                        }
                    }
                }

                match self.try_compile_assign_pattern(_py, &item)? {
                    Some(sub) => elements.push(VMPatternElement::Pattern(sub)),
                    None => return Ok(None),
                }
            }
            return Ok(Some(VMPattern::Tuple(elements)));
        }

        if pattern.is_instance_of::<Ref>() || pattern.is_instance_of::<Lvalue>() {
            let name = self.extract_single_name(_py, pattern)?;
            let slot = self.add_local(&name);
            return Ok(Some(VMPattern::Var(slot)));
        }

        if let Ok(name) = pattern.extract::<String>() {
            let slot = self.add_local(&name);
            return Ok(Some(VMPattern::Var(slot)));
        }

        Ok(None)
    }

    fn compile_getattr(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let obj = args.get_item(0)?;
        let attr: String = args.get_item(1)?.extract()?;
        self.compile_node(py, &obj)?;
        let idx = self.add_name(&attr);
        self.emit(VMOpCode::GetAttr, idx as u32);
        Ok(())
    }

    fn compile_setattr(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let obj = args.get_item(0)?;
        let attr: String = args.get_item(1)?.extract()?;
        let value = args.get_item(2)?;
        self.compile_node(py, &obj)?;
        self.compile_node(py, &value)?;
        let idx = self.add_name(&attr);
        self.emit(VMOpCode::SetAttr, idx as u32);
        Ok(())
    }

    /// Check if pattern contains star (*rest) or nested patterns.
    /// Returns true only for star patterns ('*', 'name') or truly nested unpacking.
    fn has_complex_pattern(&self, pattern: &Bound<'_, PyAny>) -> bool {
        // Handle wrapped pattern: ((Ref, Ref, ...),) -> unwrap to (Ref, Ref, ...)
        let actual_pattern = if let Ok(tuple) = pattern.cast::<PyTuple>() {
            if tuple.len() == 1 {
                if let Ok(inner) = tuple.get_item(0) {
                    if inner.cast::<PyTuple>().is_ok() {
                        inner
                    } else {
                        return false;
                    }
                } else {
                    return false;
                }
            } else {
                pattern.clone()
            }
        } else {
            return false;
        };

        if let Ok(tuple) = actual_pattern.cast::<PyTuple>() {
            for item in tuple.iter() {
                if let Ok(inner_tuple) = item.cast::<PyTuple>() {
                    // Star pattern: ('*', 'name')
                    if inner_tuple.len() == 2 {
                        if let Ok(first) =
                            inner_tuple.get_item(0).and_then(|f| f.extract::<String>())
                        {
                            if first == "*" {
                                return true;
                            }
                        }
                    }
                    // Nested pattern: a tuple that is NOT a star pattern and contains Refs or tuples
                    // e.g., ((a, b), c) where (a, b) needs nested unpacking
                    // e.g., (a, (b, (c, d))) with deeply nested structure
                    if inner_tuple.len() > 0 {
                        let mut is_nested_pattern = true;
                        for nested_item in inner_tuple.iter() {
                            if let Ok(nested_type) = nested_item.get_type().name() {
                                let nested_type_str: &str = nested_type.to_str().unwrap_or("");
                                // Nested pattern can contain Refs or more tuples
                                if nested_type_str != "Ref" && nested_type_str != "tuple" {
                                    is_nested_pattern = false;
                                    break;
                                }
                            } else {
                                is_nested_pattern = false;
                                break;
                            }
                        }
                        if is_nested_pattern {
                            return true;
                        }
                    }
                } else if item.cast::<PyList>().is_ok() {
                    return true;
                }
            }
        }
        false
    }

    /// Extract variable names from patterns (Lvalue, Ref, or plain strings).
    ///
    /// Supports:
    /// - Plain strings: "x"
    /// - Lvalue nodes: Lvalue(value="x")
    /// - Ref nodes: Ref(ident="x")
    /// - Tuples of any of the above: (Lvalue("x"), Lvalue("y"))
    fn extract_names(&self, py: Python<'_>, pattern: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
        let mut names = Vec::new();
        self.extract_names_recursive(py, pattern, &mut names)?;
        Ok(names)
    }

    fn extract_names_recursive(
        &self,
        py: Python<'_>,
        pattern: &Bound<'_, PyAny>,
        names: &mut Vec<String>,
    ) -> PyResult<()> {
        // Try as tuple first
        if let Ok(tuple) = pattern.cast::<PyTuple>() {
            for item in tuple.iter() {
                // Recurse into nested tuples (the complex pattern check already
                // redirects star/nested patterns to compile_unpack_pattern)
                if item.cast::<PyTuple>().is_ok() || item.cast::<PyList>().is_ok() {
                    self.extract_names_recursive(py, &item, names)?;
                } else {
                    let name = self.extract_single_name(py, &item)?;
                    names.push(name);
                }
            }
            return Ok(());
        }

        // Try as list
        if let Ok(list) = pattern.cast::<PyList>() {
            for item in list.iter() {
                if item.cast::<PyTuple>().is_ok() || item.cast::<PyList>().is_ok() {
                    self.extract_names_recursive(py, &item, names)?;
                } else {
                    let name = self.extract_single_name(py, &item)?;
                    names.push(name);
                }
            }
            return Ok(());
        }

        // Single pattern
        let name = self.extract_single_name(py, pattern)?;
        names.push(name);
        Ok(())
    }

    /// Extract a single variable name from a pattern node.
    fn extract_single_name(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<String> {
        use crate::types::catnip;

        // Try as plain string
        if let Ok(s) = node.extract::<String>() {
            return Ok(s);
        }

        // Check node type
        let node_type = node.get_type();
        let type_name = node_type.name()?;

        // Lvalue node: extract 'value' attribute
        if type_name == catnip::LVALUE {
            return node.getattr("value")?.extract();
        }

        // Ref node: extract 'ident' attribute
        if type_name == catnip::REF {
            return node.getattr("ident")?.extract();
        }

        // Identifier node: extract 'name' attribute (if exists)
        if type_name == "Identifier" {
            if let Ok(name) = node.getattr("name").and_then(|n| n.extract()) {
                return Ok(name);
            }
        }

        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
            "Cannot extract variable name from type: {}",
            type_name
        )))
    }

    fn compile_getitem(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        self.compile_node(py, &args.get_item(0)?)?;
        self.compile_node(py, &args.get_item(1)?)?;
        self.emit(VMOpCode::GetItem, 0);
        Ok(())
    }

    fn compile_setitem(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        self.compile_node(py, &args.get_item(0)?)?;
        self.compile_node(py, &args.get_item(1)?)?;
        self.compile_node(py, &args.get_item(2)?)?;
        self.emit(VMOpCode::SetItem, 0);
        Ok(())
    }

    fn compile_slice(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let len = args.len()?;
        for i in 0..len {
            self.compile_node(py, &args.get_item(i)?)?;
        }
        self.emit(VMOpCode::BuildSlice, len as u32);
        Ok(())
    }

    // ========== Control flow ==========

    fn compile_if(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let branches = args.get_item(0)?;
        let else_branch = if args.len()? > 1 {
            Some(args.get_item(1)?)
        } else {
            None
        };

        if branches.is_none() || !branches.is_instance_of::<PyTuple>() {
            let idx = self.add_const_value(py, py.None());
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        let mut end_jumps = Vec::new();

        for branch in branches.try_iter()? {
            let branch = branch?;
            if !branch.is_instance_of::<PyTuple>() || branch.len()? != 2 {
                continue;
            }

            let cond = branch.get_item(0)?;
            let then_body = branch.get_item(1)?;

            // Compile condition
            self.compile_node(py, &cond)?;

            // Jump if false to next branch
            let jump_to_next = self.emit(VMOpCode::JumpIfFalse, 0);

            // Then body (inline block contents)
            self.compile_body(py, &then_body)?;

            // Jump to end
            end_jumps.push(self.emit(VMOpCode::Jump, 0));

            // Patch jump to next branch
            self.patch(jump_to_next, self.instructions.len() as u32);
        }

        // Else branch (inline block contents)
        if let Some(else_body) = else_branch {
            self.compile_body(py, &else_body)?;
        } else {
            let idx = self.add_const_value(py, py.None());
            self.emit(VMOpCode::LoadConst, idx as u32);
        }

        // Patch all end jumps
        let end_addr = self.instructions.len() as u32;
        for addr in end_jumps {
            self.patch(addr, end_addr);
        }

        Ok(())
    }

    fn compile_while(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let cond = args.get_item(0)?;
        let body = args.get_item(1)?;

        let loop_start = self.instructions.len();

        // Set up loop context
        self.loop_stack.push(LoopContext {
            break_targets: Vec::new(),
            continue_target: loop_start,
            continue_patches: Vec::new(),
            is_for_loop: false,
        });

        // Check if loop body is simple enough for loop-local optimization
        let can_optimize = self.nesting_depth == 0 && !self.body_has_calls(py, &body);
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        // Compile condition
        self.compile_node(py, &cond)?;
        let jump_to_end = self.emit(VMOpCode::JumpIfFalse, 0);

        // Body in void context (no value left on stack)
        self.compile_body_void(py, &body)?;

        // Sync modified variables before jumping back to condition
        if can_optimize {
            self.emit_loop_sync();
        }

        // Jump back
        self.emit(VMOpCode::Jump, loop_start as u32);

        // Pop loop context
        let ctx = self.loop_stack.pop().unwrap();

        // Sync code at loop exit (JumpIfFalse uses absolute addressing, lands exactly here)
        let loop_end = self.instructions.len() as u32;
        if can_optimize {
            self.emit_loop_sync();
        }

        // While returns None
        let loadconst_pos = self.instructions.len() as u32;
        let idx = self.add_const_value(py, py.None());
        self.emit(VMOpCode::LoadConst, idx as u32);

        // JumpIfFalse lands on sync code (or LoadConst if no sync)
        self.patch(jump_to_end, loop_end);

        // Break targets: jump past sync to LoadConst None (break already synced inline)
        let break_target = if can_optimize {
            loadconst_pos
        } else {
            loop_end
        };
        for addr in ctx.break_targets {
            self.patch(addr, break_target);
        }

        // Restore loop optimization state
        self.in_optimized_loop = old_optimized;
        self.loop_modified_vars = old_modified;

        Ok(())
    }

    fn compile_for(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let var_pattern = args.get_item(0)?;
        let iterable = args.get_item(1)?;
        let body = args.get_item(2)?;

        // Check if loop variable is simple or pattern (unpacking)
        let is_simple_var = !var_pattern.is_instance_of::<PyTuple>();
        let var_name: Option<String> = if is_simple_var {
            if let Ok(name) = var_pattern.extract::<String>() {
                Some(name)
            } else {
                Some(self.extract_single_name(py, &var_pattern)?)
            }
        } else {
            None
        };

        // Range optimization: compile_for_range handles its own PushBlock/PopBlock
        if is_simple_var && self.is_range_call(py, &iterable)? {
            return self.compile_for_range(py, var_name.as_ref().unwrap(), &iterable, &body);
        }

        // If the loop variable already exists, save its value to a temp slot
        // so we can restore it after the loop (loop var must not leak)
        let save_restore = if let Some(ref name) = var_name {
            if let Some(existing) = self.get_local_slot(name) {
                let temp = self.add_local(&format!("_for_save_{}", existing));
                self.emit(VMOpCode::LoadLocal, existing as u32);
                self.emit(VMOpCode::StoreLocal, temp as u32);
                Some((existing, temp))
            } else {
                None
            }
        } else {
            None
        };

        let slot_start = self.locals.len();
        self.emit(VMOpCode::PushBlock, slot_start as u32);

        // Compile iterable and get iterator
        self.compile_node(py, &iterable)?;
        self.emit(VMOpCode::GetIter, 0);

        let loop_start = self.instructions.len();

        // Set up loop context
        self.loop_stack.push(LoopContext {
            break_targets: Vec::new(),
            continue_target: loop_start,
            continue_patches: Vec::new(),
            is_for_loop: true,
        });

        // FOR_ITER
        let for_iter_idx = self.emit(VMOpCode::ForIter, 0);

        // Store loop variable(s)
        if let Some(ref name) = var_name {
            let slot = self.add_local(name);
            self.emit(VMOpCode::StoreLocal, slot as u32);
        } else {
            // Pattern unpacking - use compile_unpack_pattern
            self.compile_unpack_pattern(py, &var_pattern, false)?;
        }

        // Check if loop body is simple enough for loop-local optimization
        let can_optimize = self.nesting_depth == 0 && !self.body_has_calls(py, &body);
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        // Body in void context (no value left on stack)
        self.compile_body_void(py, &body)?;

        // Sync modified variables before jumping back to ForIter
        if can_optimize {
            self.emit_loop_sync();
        }

        // Jump back
        self.emit(VMOpCode::Jump, loop_start as u32);

        // Pop loop context
        let ctx = self.loop_stack.pop().unwrap();

        // Calculate loop_end - sync code must be at this address (reachable via ForIter exit)
        let loop_end = self.instructions.len();

        // Scope sync at loop exit (ForIter and break targets land here)
        if can_optimize {
            self.emit_loop_sync();
        }

        self.emit(VMOpCode::PopBlock, 0);

        // Restore saved loop variable value
        if let Some((orig_slot, temp_slot)) = save_restore {
            self.emit(VMOpCode::LoadLocal, temp_slot as u32);
            self.emit(VMOpCode::StoreLocal, orig_slot as u32);
        }

        // For returns None
        let loadconst_pos = self.instructions.len() as u32;
        let idx = self.add_const_value(py, py.None());
        self.emit(VMOpCode::LoadConst, idx as u32);

        // Patch FOR_ITER to jump to sync/PopBlock
        self.patch(for_iter_idx, loop_end as u32);

        // Break targets: jump past sync to LoadConst None (break already synced inline)
        let break_target = if can_optimize {
            loadconst_pos
        } else {
            loop_end as u32
        };
        for addr in ctx.break_targets {
            self.patch(addr, break_target);
        }

        // Restore loop optimization state
        self.in_optimized_loop = old_optimized;
        self.loop_modified_vars = old_modified;

        Ok(())
    }

    /// Check if node is a call to range().
    fn is_range_call(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Ok(op) = node.extract::<PyRef<Op>>() {
            if op.ident != IROpCode::Call as i32 {
                return Ok(false);
            }
            let args = op.args.bind(py);
            if args.len()? < 2 {
                return Ok(false);
            }
            let func_ref = args.get_item(0)?;
            if func_ref.get_type().name()? == "Ref" {
                let ident: String = func_ref.getattr("ident")?.extract()?;
                return Ok(ident == "range");
            }
        }
        Ok(false)
    }

    /// Compile optimized for loop over range().
    fn compile_for_range(
        &mut self,
        py: Python<'_>,
        var_name: &str,
        range_call: &Bound<'_, PyAny>,
        body: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let op: PyRef<Op> = range_call.extract()?;
        let args = op.args.bind(py);

        // Parse range arguments (skip func ref at index 0)
        let range_args: Vec<Bound<'_, PyAny>> = (1..args.len()?)
            .map(|i| args.get_item(i).unwrap())
            .collect();

        let (start, stop, step): (Bound<'_, PyAny>, Bound<'_, PyAny>, i64) = match range_args.len()
        {
            1 => (0i64.into_pyobject(py)?.into_any(), range_args[0].clone(), 1),
            2 => (range_args[0].clone(), range_args[1].clone(), 1),
            _ => {
                let step: i64 = range_args[2]
                    .extract()
                    .or_else(|_| Self::try_extract_neg_literal(py, &range_args[2]))
                    .unwrap_or(1);
                (range_args[0].clone(), range_args[1].clone(), step)
            }
        };

        let step_is_positive = step > 0;

        // If the loop variable already exists, save its value to a temp slot
        let save_restore = if let Some(existing) = self.get_local_slot(var_name) {
            let temp = self.add_local(&format!("_for_save_{}", existing));
            self.emit(VMOpCode::LoadLocal, existing as u32);
            self.emit(VMOpCode::StoreLocal, temp as u32);
            Some((existing, temp))
        } else {
            None
        };

        let slot_start = self.locals.len();
        self.emit(VMOpCode::PushBlock, slot_start as u32);

        // Allocate slots
        let slot_i = self.add_local(var_name);
        let slot_stop = self.add_local(&format!("_range_stop_{}", self.locals.len()));

        // Initialize: i = start
        self.compile_node(py, &start)?;
        self.emit(VMOpCode::StoreLocal, slot_i as u32);

        // Store stop
        self.compile_node(py, &stop)?;
        self.emit(VMOpCode::StoreLocal, slot_stop as u32);

        let loop_start = self.instructions.len();

        // Set up loop context
        self.loop_stack.push(LoopContext {
            break_targets: Vec::new(),
            continue_target: 0, // patched after body
            continue_patches: Vec::new(),
            is_for_loop: false,
        });

        // Check if loop body is simple enough for loop-local optimization
        let can_optimize = self.nesting_depth == 0 && !self.body_has_calls(py, body);
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        // Optimized loop condition using ForRangeInt (1 opcode instead of 4)
        // Replaces: LoadLocal i + LoadLocal stop + Ge/Le + JumpIfTrue
        let arg = Self::encode_for_range_args(slot_i, slot_stop, step_is_positive, 0);
        let for_range_idx = self.emit(VMOpCode::ForRangeInt, arg);

        // Body in void context (no value left on stack)
        self.compile_body_void(py, body)?;

        // Sync modified variables before increment and jump back
        if can_optimize {
            self.emit_loop_sync();
        }

        // Increment i += step and jump back
        // continue_target must point here so `continue` increments before looping
        let increment_addr = self.instructions.len();
        self.loop_stack.last_mut().unwrap().continue_target = increment_addr;

        if step >= -128 && step <= 127 && loop_start <= 0xFFFF {
            let arg = Self::encode_for_range_step(slot_i, step, loop_start);
            self.emit(VMOpCode::ForRangeStep, arg);
        } else {
            // Fallback: step hors i8 ou jump_target > u16
            self.emit(VMOpCode::LoadLocal, slot_i as u32);
            let step_idx = self.add_const_i64(py, step);
            self.emit(VMOpCode::LoadConst, step_idx as u32);
            self.emit(VMOpCode::Add, 0);
            self.emit(VMOpCode::StoreLocal, slot_i as u32);
            self.emit(VMOpCode::Jump, loop_start as u32);
        }

        // Pop loop context and patch continue/break targets
        let ctx = self.loop_stack.pop().unwrap();

        // Patch continue targets to increment address
        for addr in &ctx.continue_patches {
            self.patch(*addr, increment_addr as u32);
        }

        // PopBlock first: ForRangeInt's jump_offset targets this position,
        // but due to IP pre-increment in the VM dispatch loop (frame.ip += 1
        // before handler), the VM actually lands at pop_block_pos + 1.
        // This means PopBlock is skipped on normal exit (preserving locals).
        let loop_end = self.instructions.len() as u32;
        self.emit(VMOpCode::PopBlock, 0);

        // Restore saved loop variable value
        if let Some((orig_slot, temp_slot)) = save_restore {
            self.emit(VMOpCode::LoadLocal, temp_slot as u32);
            self.emit(VMOpCode::StoreLocal, orig_slot as u32);
        }

        // Scope sync after PopBlock: VM lands here (pop_block_pos + 1)
        if can_optimize {
            self.emit_loop_sync();
        }

        // For returns None
        let loadconst_pos = self.instructions.len() as u32;
        let idx = self.add_const_value(py, py.None());
        self.emit(VMOpCode::LoadConst, idx as u32);

        // Patch ForRangeInt: jump_offset = loop_end - for_range_idx
        // VM lands at for_range_idx + 1 + jump_offset = loop_end + 1 = sync start
        let jump_offset = (loop_end as usize) - for_range_idx;
        let arg = Self::encode_for_range_args(slot_i, slot_stop, step_is_positive, jump_offset);
        self.patch(for_range_idx, arg);

        // Break targets: jump past sync to LoadConst None (break already synced inline)
        let break_target = if can_optimize {
            loadconst_pos
        } else {
            loop_end
        };
        for addr in ctx.break_targets {
            self.patch(addr, break_target);
        }

        // Restore loop optimization state
        self.in_optimized_loop = old_optimized;
        self.loop_modified_vars = old_modified;

        Ok(())
    }

    fn compile_block(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        if args.len()? == 0 {
            let idx = self.add_const_value(py, py.None());
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        let slot_start = self.locals.len();
        let is_module_block = self.nesting_depth == 0;
        // High bit flags module-level block (triggers globals snapshot)
        let push_arg = if is_module_block {
            slot_start as u32 | 0x8000_0000
        } else {
            slot_start as u32
        };
        self.emit(VMOpCode::PushBlock, push_arg);

        let len = args.len()?;
        for i in 0..len {
            self.compile_node(py, &args.get_item(i)?)?;
            if i < len - 1 {
                self.emit(VMOpCode::PopTop, 0);
            }
        }

        // arg=1 at module level: PopBlock cleans block-local names from globals
        let pop_arg = if is_module_block { 1u32 } else { 0u32 };
        self.emit(VMOpCode::PopBlock, pop_arg);

        Ok(())
    }

    /// Compile body without PUSH_BLOCK/POP_BLOCK (for control structures).
    /// If body is a block Op, compile its contents inline.
    fn compile_body(&mut self, py: Python<'_>, body: &Bound<'_, PyAny>) -> PyResult<()> {
        // Check if body is a block Op
        if let Ok(op) = body.extract::<PyRef<Op>>() {
            if op.ident == IROpCode::OpBlock as i32 {
                let args = op.args.bind(py);
                let len = args.len()?;
                if len == 0 {
                    let idx = self.add_const_value(py, py.None());
                    self.emit(VMOpCode::LoadConst, idx as u32);
                    return Ok(());
                }
                for i in 0..len {
                    self.compile_node(py, &args.get_item(i)?)?;
                    if i < len - 1 {
                        self.emit(VMOpCode::PopTop, 0);
                    }
                }
                return Ok(());
            }
        }
        // Not a block, compile normally
        self.compile_node(py, body)
    }

    /// Compile body in void context: statements don't leave values on the stack.
    /// SetLocals nodes skip the DupTop that preserves expression value.
    /// Other nodes get a PopTop after them.
    ///
    /// void_context is set only around each top-level statement, NOT for
    /// sub-expressions (e.g. a Block inside a SetLocals rhs must still
    /// produce a value).
    fn compile_body_void(&mut self, py: Python<'_>, body: &Bound<'_, PyAny>) -> PyResult<()> {
        // Unwrap block Op to get inner statements
        if let Ok(op) = body.extract::<PyRef<Op>>() {
            if op.ident == IROpCode::OpBlock as i32 {
                let args = op.args.bind(py);
                let len = args.len()?;
                for i in 0..len {
                    let stmt = args.get_item(i)?;
                    let is_set_locals = self.is_set_locals_node(py, &stmt);
                    if is_set_locals {
                        // Set void_context only for this SetLocals compilation.
                        // compile_set_locals will check it for the outer DupTop.
                        // Sub-expressions (rhs) are compiled with void_context=false
                        // because compile_set_locals calls compile_node on the rhs
                        // BEFORE checking void_context for DupTop.
                        self.void_context = true;
                        self.compile_node(py, &stmt)?;
                        self.void_context = false;
                    } else {
                        self.compile_node(py, &stmt)?;
                        self.emit(VMOpCode::PopTop, 0);
                    }
                }
                return Ok(());
            }
        }
        // Not a block: compile single node and pop
        let is_set_locals = self.is_set_locals_node(py, body);
        if is_set_locals {
            self.void_context = true;
            self.compile_node(py, body)?;
            self.void_context = false;
        } else {
            self.compile_node(py, body)?;
            self.emit(VMOpCode::PopTop, 0);
        }
        Ok(())
    }

    /// Check if a node is a SetLocals operation.
    #[inline]
    fn is_set_locals_node(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> bool {
        if let Ok(op) = node.extract::<PyRef<Op>>() {
            op.ident == IROpCode::SetLocals as i32
        } else {
            false
        }
    }

    /// Check if body contains any Call/CallKw nodes (recursive IR scan).
    /// Used to determine if loop-local store optimization is safe.
    fn body_has_calls(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> bool {
        // Check list of statements
        if let Ok(list) = node.cast::<PyList>() {
            for item in list.iter() {
                if self.body_has_calls(py, &item) {
                    return true;
                }
            }
            return false;
        }

        // Check tuples (binary op args are wrapped in tuples)
        if let Ok(tuple) = node.cast::<PyTuple>() {
            for item in tuple.iter() {
                if self.body_has_calls(py, &item) {
                    return true;
                }
            }
            return false;
        }

        // Check Op nodes
        if let Ok(op) = node.extract::<PyRef<Op>>() {
            let ident = op.ident;
            if ident == IROpCode::Call as i32
                || ident == IROpCode::FnDef as i32
                || ident == IROpCode::OpLambda as i32
            {
                return true;
            }
            // Recurse into args
            let args = op.args.bind(py);
            if let Ok(len) = args.len() {
                for i in 0..len {
                    if let Ok(arg) = args.get_item(i) {
                        if self.body_has_calls(py, &arg) {
                            return true;
                        }
                    }
                }
            }
            return false;
        }

        // Check Call nodes (from pure_transforms)
        if let Ok(type_name) = node.get_type().name() {
            if type_name == "Call" {
                return true;
            }
        }

        false
    }

    /// Emit scope sync instructions for all loop-modified vars.
    /// Used at loop exit (normal, break) to sync StoreLocal → StoreScope.
    fn emit_loop_sync(&mut self) {
        for (name, slot) in self.loop_modified_vars.clone() {
            self.emit(VMOpCode::LoadLocal, slot as u32);
            let name_idx = self.add_name(&name);
            self.emit(VMOpCode::StoreScope, name_idx as u32);
        }
    }

    fn compile_return(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        if args.len()? > 0 {
            self.compile_node(py, &args.get_item(0)?)?;
        } else {
            let idx = self.add_const_value(py, py.None());
            self.emit(VMOpCode::LoadConst, idx as u32);
        }
        self.emit(VMOpCode::Return, 0);
        Ok(())
    }

    fn compile_break(&mut self) -> PyResult<()> {
        if self.loop_stack.is_empty() {
            return Err(pyo3::exceptions::PySyntaxError::new_err(
                "'break' outside loop",
            ));
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

    fn compile_continue(&mut self) -> PyResult<()> {
        if self.loop_stack.is_empty() {
            return Err(pyo3::exceptions::PySyntaxError::new_err(
                "'continue' outside loop",
            ));
        }
        // Sync loop-modified vars before continuing
        if self.in_optimized_loop {
            self.emit_loop_sync();
        }
        let ctx = self.loop_stack.last().unwrap();
        let target = ctx.continue_target;
        let addr = self.emit(VMOpCode::Jump, target as u32);
        // For for-range loops, continue_target is 0 (not yet known) and needs patching
        if target == 0 {
            self.loop_stack
                .last_mut()
                .unwrap()
                .continue_patches
                .push(addr);
        }
        Ok(())
    }

    // ========== Functions ==========

    /// Compile a Call node (from pure_transforms IRPure::Call).
    fn compile_call_node(&mut self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<()> {
        // Capture source position from Call node
        if let Ok(sb) = node.getattr("start_byte") {
            if let Ok(sb_val) = sb.extract::<isize>() {
                if sb_val >= 0 {
                    self.current_start_byte = sb_val as u32;
                }
            }
        }

        // Extract func, args, kwargs from Call node
        let func = node.getattr("func")?;
        let args = node.getattr("args")?;
        let kwargs = node.getattr("kwargs")?;
        let kwargs_dict = kwargs.cast::<PyDict>()?;
        let args_len = args.len()?;

        // Detect method call pattern: func is GetAttr Op (no kwargs)
        let method_call_info = if kwargs_dict.is_empty() {
            if let Ok(func_op) = func.extract::<PyRef<Op>>() {
                if func_op.ident == crate::ir::opcode::IROpCode::GetAttr as i32 {
                    let func_args = func_op.args.bind(py);
                    let obj = func_args.get_item(0)?;
                    let method_name: String = func_args.get_item(1)?.extract()?;
                    Some((obj.unbind(), method_name))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some((obj, method_name)) = method_call_info {
            // Fused path: push obj, push args, emit CallMethod
            self.compile_node(py, obj.bind(py))?;
            for i in 0..args_len {
                let arg = args.get_item(i)?;
                self.compile_node(py, &arg)?;
            }
            let name_idx = self.add_name(&method_name);
            let encoding = ((name_idx as u32) << 16) | (args_len as u32);
            self.emit(VMOpCode::CallMethod, encoding);
        } else {
            // Normal path
            self.compile_node(py, &func)?;
            for i in 0..args_len {
                let arg = args.get_item(i)?;
                self.compile_node(py, &arg)?;
            }

            if !kwargs_dict.is_empty() {
                let mut kw_names = Vec::new();
                for (name, value) in kwargs_dict.iter() {
                    kw_names.push(name.extract::<String>()?);
                    self.compile_node(py, &value)?;
                }
                let kw_tuple = PyTuple::new(py, &kw_names)?;
                let kw_idx = self.add_const(py, kw_tuple.as_any());
                self.emit(VMOpCode::LoadConst, kw_idx as u32);

                let encoding = ((args_len as u32) << 8) | (kwargs_dict.len() as u32);
                self.emit(VMOpCode::CallKw, encoding);
            } else {
                self.emit(VMOpCode::Call, args_len as u32);
            }
        }

        Ok(())
    }

    fn compile_call(
        &mut self,
        py: Python<'_>,
        args: &Bound<'_, PyAny>,
        kwargs: &Bound<'_, PyDict>,
        is_tail: bool,
    ) -> PyResult<()> {
        let func = args.get_item(0)?;
        let call_args: Vec<Bound<'_, PyAny>> = (1..args.len()?)
            .map(|i| args.get_item(i).unwrap())
            .collect();

        // Detect method call pattern: Call(GetAttr(obj, name), args...)
        // Emit fused CallMethod to avoid BoundCatnipMethod allocation
        let method_call_info = if kwargs.is_empty() && !is_tail {
            if let Ok(func_op) = func.extract::<PyRef<Op>>() {
                if func_op.ident == crate::ir::opcode::IROpCode::GetAttr as i32 {
                    let func_args = func_op.args.bind(py);
                    let obj = func_args.get_item(0)?;
                    let method_name: String = func_args.get_item(1)?.extract()?;
                    Some((obj.unbind(), method_name))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some((obj, method_name)) = method_call_info {
            // Fused path: push obj, push args, emit CallMethod
            self.compile_node(py, obj.bind(py))?;
            for arg in &call_args {
                self.compile_node(py, arg)?;
            }
            let name_idx = self.add_name(&method_name);
            let encoding = ((name_idx as u32) << 16) | (call_args.len() as u32);
            self.emit(VMOpCode::CallMethod, encoding);
        } else {
            // Normal path
            self.compile_node(py, &func)?;
            for arg in &call_args {
                self.compile_node(py, arg)?;
            }

            if !kwargs.is_empty() {
                let mut kw_names = Vec::new();
                for (name, value) in kwargs.iter() {
                    kw_names.push(name.extract::<String>()?);
                    self.compile_node(py, &value)?;
                }
                let kw_tuple = PyTuple::new(py, &kw_names)?;
                let kw_idx = self.add_const(py, kw_tuple.as_any());
                self.emit(VMOpCode::LoadConst, kw_idx as u32);

                let encoding = ((call_args.len() as u32) << 8) | (kwargs.len() as u32);
                self.emit(VMOpCode::CallKw, encoding);
            } else if is_tail {
                self.emit(VMOpCode::TailCall, call_args.len() as u32);
            } else {
                self.emit(VMOpCode::Call, call_args.len() as u32);
            }
        }

        Ok(())
    }

    fn compile_lambda(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let raw_params = args.get_item(0)?;
        let body = args.get_item(1)?;

        let mut param_names = Vec::new();
        let mut defaults = Vec::new();
        let mut vararg_idx: i32 = -1;

        for p in raw_params.try_iter()? {
            let p = p?;
            if p.is_instance_of::<PyTuple>() && p.len()? == 2 {
                let name: String = p.get_item(0)?.extract()?;
                let default = p.get_item(1)?;
                if name == "*" {
                    vararg_idx = param_names.len() as i32;
                    param_names.push(default.extract::<String>()?);
                } else {
                    param_names.push(name);
                    defaults.push(default.unbind());
                }
            } else {
                param_names.push(p.extract()?);
            }
        }

        // Create new compiler for function body
        let mut func_compiler = Compiler::new();
        let code = func_compiler.compile_function(
            py,
            param_names,
            &body,
            "<lambda>",
            defaults,
            vararg_idx,
            self.nesting_depth, // Pass current depth (will be incremented inside)
        )?;

        // Create PyCodeObject and store as constant
        let py_code = Py::new(py, PyCodeObject::new(code))?;
        let idx = self.add_const_value(py, py_code.into_any());
        self.emit(VMOpCode::LoadConst, idx as u32);
        self.emit(VMOpCode::MakeFunction, 0);

        Ok(())
    }

    fn compile_fn_def(
        &mut self,
        py: Python<'_>,
        args: &Bound<'_, PyAny>,
        _kwargs: &Bound<'_, PyDict>,
    ) -> PyResult<()> {
        let name: String = args.get_item(0)?.extract()?;
        let params = args.get_item(1)?;
        let body = args.get_item(2)?;

        let mut param_names = Vec::new();
        let mut vararg_idx: i32 = -1;

        for p in params.try_iter()? {
            let p = p?;
            if p.is_instance_of::<PyTuple>() && p.len()? == 2 {
                let pname: String = p.get_item(0)?.extract()?;
                if pname == "*" {
                    vararg_idx = param_names.len() as i32;
                    param_names.push(p.get_item(1)?.extract::<String>()?);
                } else {
                    param_names.push(pname);
                }
            } else {
                param_names.push(p.extract()?);
            }
        }

        // Create new compiler for function body
        let mut func_compiler = Compiler::new();
        let code = func_compiler.compile_function(
            py,
            param_names,
            &body,
            &name,
            Vec::new(),
            vararg_idx,
            self.nesting_depth, // Pass current depth (will be incremented inside)
        )?;

        // Create PyCodeObject and store as constant
        let py_code = Py::new(py, PyCodeObject::new(code))?;
        let idx = self.add_const_value(py, py_code.into_any());
        self.emit(VMOpCode::LoadConst, idx as u32);
        self.emit(VMOpCode::MakeFunction, 0);

        // Store in local
        let slot = self.add_local(&name);
        self.emit(VMOpCode::StoreLocal, slot as u32);

        Ok(())
    }

    // ========== Collections ==========

    fn compile_list(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let len = args.len()?;
        for i in 0..len {
            self.compile_node(py, &args.get_item(i)?)?;
        }
        self.emit(VMOpCode::BuildList, len as u32);
        Ok(())
    }

    fn compile_tuple(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let len = args.len()?;
        for i in 0..len {
            self.compile_node(py, &args.get_item(i)?)?;
        }
        self.emit(VMOpCode::BuildTuple, len as u32);
        Ok(())
    }

    fn compile_set(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let len = args.len()?;
        for i in 0..len {
            self.compile_node(py, &args.get_item(i)?)?;
        }
        self.emit(VMOpCode::BuildSet, len as u32);
        Ok(())
    }

    fn compile_dict(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let len = args.len()?;
        for i in 0..len {
            let pair = args.get_item(i)?;

            // Handle both formats: Python tuples (from pure_transforms) or Op nodes
            let (key, value) = if pair.is_instance_of::<PyTuple>() {
                // Direct tuple (key, value)
                (pair.get_item(0)?, pair.get_item(1)?)
            } else {
                // Op node with .args attribute
                let pair_args = pair.getattr("args")?;
                (pair_args.get_item(0)?, pair_args.get_item(1)?)
            };

            self.compile_node(py, &key)?;
            self.compile_node(py, &value)?;
        }
        self.emit(VMOpCode::BuildDict, len as u32);
        Ok(())
    }

    // ========== Broadcast ==========

    fn compile_broadcast(&mut self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<()> {
        // Compile target
        let target = node.getattr("target")?;
        self.compile_node(py, &target)?;

        // Get operator and check if it's an ND operation
        let operator = node.getattr("operator")?;
        let op_type = operator.get_type();
        let is_nd_op = if op_type.name()? == "Op" {
            // Check if it's ND_RECURSION or ND_MAP
            let op_ident: i32 = operator.getattr("ident")?.extract()?;
            let opcode_module = py.import("catnip.semantic.opcode")?;
            let opcode_class = opcode_module.getattr("OpCode")?;
            let nd_recursion: i32 = opcode_class.getattr("ND_RECURSION")?.extract()?;
            let nd_map: i32 = opcode_class.getattr("ND_MAP")?.extract()?;

            if op_ident == nd_recursion {
                Some(4) // Flag for ND_RECURSION (bit 2)
            } else if op_ident == nd_map {
                Some(8) // Flag for ND_MAP (bit 3)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(nd_flag) = is_nd_op {
            // ND operation: extract lambda/func from Op and compile it
            let op_args = operator.getattr("args")?;
            let op_args_tuple = op_args.cast::<pyo3::types::PyTuple>()?;
            let lambda_node = op_args_tuple.get_item(0)?;
            self.compile_node(py, &lambda_node)?;

            // Encode flags: bit 0 = is_filter, bit 1 = has_operand (always false for ND), bits 2-3 = ND type
            let is_filter: bool = node.getattr("is_filter")?.extract()?;
            let mut flags: u32 = nd_flag;
            if is_filter {
                flags |= 1;
            }

            self.emit(VMOpCode::Broadcast, flags);
        } else {
            // Regular broadcast: compile operator normally
            self.compile_node(py, &operator)?;

            // Compile operand if present
            let operand = node.getattr("operand")?;
            let has_operand = !operand.is_none();
            if has_operand {
                self.compile_node(py, &operand)?;
            }

            // Encode flags: bit 0 = is_filter, bit 1 = has_operand
            let is_filter: bool = node.getattr("is_filter")?.extract()?;
            let mut flags: u32 = 0;
            if is_filter {
                flags |= 1;
            }
            if has_operand {
                flags |= 2;
            }

            self.emit(VMOpCode::Broadcast, flags);
        }

        Ok(())
    }

    fn compile_broadcast_op(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        // Args from pure_transforms: [target, operator_str, operand, is_filter]
        if args.len()? < 4 {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "Broadcast requires 4 arguments: target, operator, operand, is_filter",
            ));
        }

        let target_expr = args.get_item(0)?;
        let operator_expr = args.get_item(1)?;
        let operand_expr = args.get_item(2)?;
        let is_filter_expr = args.get_item(3)?;

        // Extract is_filter boolean
        let is_filter: bool = is_filter_expr.extract()?;

        // Compile target
        self.compile_node(py, &target_expr)?;

        // Compile operator (should be a string)
        self.compile_node(py, &operator_expr)?;

        // Check if operand is None
        let has_operand = !operand_expr.is_none();
        if has_operand {
            // Compile operand
            self.compile_node(py, &operand_expr)?;
        }

        // Build flags: bit 0 = is_filter, bit 1 = has_operand
        const FLAG_FILTER: u32 = 1;
        const FLAG_OPERAND: u32 = 2;
        let mut flags = 0u32;
        if is_filter {
            flags |= FLAG_FILTER;
        }
        if has_operand {
            flags |= FLAG_OPERAND;
        }

        // Emit Broadcast opcode
        self.emit(VMOpCode::Broadcast, flags);

        Ok(())
    }

    // ========== Struct ==========

    fn compile_struct(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let name: String = args.get_item(0)?.extract()?;
        let fields_ir = args.get_item(1)?;
        let args_len = args.len()?;

        // IR structure: (name, fields, implements, bases, methods)
        // args[0] = name
        // args[1] = fields
        // args[2] = implements (list, may be empty) - optional
        // args[3] = bases (list or None) - optional
        // args[4] = methods (list) - optional

        let mut implements_list: Vec<String> = Vec::new();
        let mut base_names: Vec<String> = Vec::new();
        let mut methods_index: Option<usize> = None;

        // Check if we have implements, bases, and/or methods
        if args_len > 3 {
            // args[2] is implements, args[3] is bases
            let implements_ir = args.get_item(2)?;
            for imp in implements_ir.try_iter()? {
                let imp = imp?;
                implements_list.push(imp.extract()?);
            }
            let arg3 = args.get_item(3)?;
            if !arg3.is_none() {
                // bases is a list of strings
                if arg3.is_instance_of::<PyList>() {
                    for b in arg3.try_iter()? {
                        let b = b?;
                        base_names.push(b.extract()?);
                    }
                } else if let Ok(base) = arg3.extract::<String>() {
                    // Legacy single base string
                    base_names.push(base);
                }
            }
            if args_len > 4 {
                methods_index = Some(4);
            }
        } else if args_len > 2 {
            // Could be new format with empty implements, or legacy
            let arg2 = args.get_item(2)?;
            if let Ok(base) = arg2.extract::<String>() {
                base_names.push(base);
                if args_len > 3 {
                    methods_index = Some(3);
                }
            } else if arg2.is_instance_of::<PyList>() {
                // implements list (possibly empty) or bases list
                // Disambiguate: if we only have 3 args total and it's a list, it's implements
                for imp in arg2.try_iter()? {
                    let imp = imp?;
                    implements_list.push(imp.extract()?);
                }
            } else {
                methods_index = Some(2);
            }
        }

        // Fields are tuples (name, has_default, default_expr)
        // Build fields_info: tuple of (field_name, has_default) for the constant
        // Compile default expressions onto the stack
        let mut fields_info: Vec<Py<PyAny>> = Vec::new();
        let mut num_defaults: u32 = 0;
        for f in fields_ir.try_iter()? {
            let f = f?;
            let pair = f.cast::<PyTuple>()?;
            let fname: String = pair.get_item(0)?.extract()?;
            let has_default: bool = pair.get_item(1)?.extract()?;
            if has_default {
                let default = pair.get_item(2)?;
                // Compile default expression -- pushes value onto stack
                self.compile_node(py, &default)?;
                num_defaults += 1;
            }
            let entry = PyTuple::new(
                py,
                &[
                    fname.into_pyobject(py)?.into_any().unbind(),
                    has_default
                        .into_pyobject(py)?
                        .to_owned()
                        .into_any()
                        .unbind(),
                ],
            )?;
            fields_info.push(entry.into_any().unbind());
        }
        let fields_tuple = PyTuple::new(py, &fields_info)?;

        // Methods: args[2] if present
        // Each method in IR is (name, lambda_or_none, is_static)
        // Compiled to (name, code_or_none, is_static)
        let methods_list = if let Some(idx) = methods_index {
            let methods = args.get_item(idx)?;
            let mut compiled_methods: Vec<Py<PyAny>> = Vec::new();
            for m in methods.try_iter()? {
                let method = m?;
                let method_tuple = method.cast::<PyTuple>()?;
                let method_name: String = method_tuple.get_item(0)?.extract()?;
                let lambda_op = method_tuple.get_item(1)?;
                let is_static: bool = if method_tuple.len() > 2 {
                    method_tuple.get_item(2)?.extract().unwrap_or(false)
                } else {
                    false
                };
                let is_static_py = is_static.into_pyobject(py)?.to_owned().into_any().unbind();

                // Abstract method: (name, None, is_static) - skip compilation
                if lambda_op.is_none() {
                    let pair = PyTuple::new(
                        py,
                        &[
                            method_name.into_pyobject(py)?.into_any().unbind(),
                            py.None(),
                            is_static_py,
                        ],
                    )?;
                    compiled_methods.push(pair.into_any().unbind());
                    continue;
                }

                // lambda_op is an Op(OpLambda) - extract .args = [params, body]
                let lambda_args = lambda_op.getattr("args")?;
                let raw_params = lambda_args.get_item(0)?;
                let body = lambda_args.get_item(1)?;

                let mut param_names = Vec::new();
                let mut defaults = Vec::new();
                let mut vararg_idx: i32 = -1;

                for p in raw_params.try_iter()? {
                    let p = p?;
                    if p.is_instance_of::<PyTuple>() && p.len()? == 2 {
                        let pname: String = p.get_item(0)?.extract()?;
                        let default = p.get_item(1)?;
                        if pname == "*" {
                            vararg_idx = param_names.len() as i32;
                            param_names.push(default.extract::<String>()?);
                        } else {
                            param_names.push(pname);
                            defaults.push(default.unbind());
                        }
                    } else {
                        param_names.push(p.extract()?);
                    }
                }

                let mut func_compiler = Compiler::new();
                let code = func_compiler.compile_function(
                    py,
                    param_names,
                    &body,
                    &method_name,
                    defaults,
                    vararg_idx,
                    self.nesting_depth,
                )?;
                let py_code = Py::new(py, PyCodeObject::new(code))?;

                let pair = PyTuple::new(
                    py,
                    &[
                        method_name.into_pyobject(py)?.into_any().unbind(),
                        py_code.into_any(),
                        is_static_py,
                    ],
                )?;
                compiled_methods.push(pair.into_any().unbind());
            }
            Some(PyList::new(py, &compiled_methods)?)
        } else {
            None
        };

        // Store constant: (name, fields_info, num_defaults, implements, bases, [methods])
        // Unified format: always include implements tuple and bases tuple when either is present.
        let num_defaults_py = num_defaults.into_pyobject(py)?.into_any().as_any().clone();
        let name_for_const = name;
        let name_py = name_for_const
            .as_str()
            .into_pyobject(py)?
            .into_any()
            .as_any()
            .clone();

        let has_implements = !implements_list.is_empty();
        let has_bases = !base_names.is_empty();

        let struct_info = if has_implements || has_bases {
            // New format: (name, fields, num_defaults, implements, bases_tuple, [methods])
            let impl_py = PyTuple::new(
                py,
                implements_list
                    .iter()
                    .map(|s| s.as_str().into_pyobject(py).unwrap().into_any().unbind())
                    .collect::<Vec<_>>()
                    .as_slice(),
            )?;
            let bases_py = if has_bases {
                PyTuple::new(
                    py,
                    base_names
                        .iter()
                        .map(|s| s.as_str().into_pyobject(py).unwrap().into_any().unbind())
                        .collect::<Vec<_>>()
                        .as_slice(),
                )?
                .into_any()
                .unbind()
            } else {
                py.None()
            };
            let mut items: Vec<Py<pyo3::PyAny>> = vec![
                name_py.unbind(),
                fields_tuple.into_any().unbind(),
                num_defaults_py.unbind(),
                impl_py.into_any().unbind(),
                bases_py,
            ];
            if let Some(methods) = methods_list {
                items.push(methods.into_any().unbind());
            }
            PyTuple::new(py, items.as_slice())?
        } else {
            // Minimal format (no implements, no bases)
            match methods_list {
                Some(methods) => PyTuple::new(
                    py,
                    &[
                        name_py,
                        fields_tuple.into_any(),
                        num_defaults_py,
                        methods.into_any(),
                    ],
                )?,
                None => PyTuple::new(py, &[name_py, fields_tuple.into_any(), num_defaults_py])?,
            }
        };

        let idx = self.add_const(py, &struct_info.into_any());
        self.emit(VMOpCode::MakeStruct, idx as u32);

        Ok(())
    }

    // ========== Trait Definition ==========

    fn compile_trait(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        // IR: args = [name, extends_list, fields_tuple, methods_list?]
        let name: String = args.get_item(0)?.extract()?;
        let extends_ir = args.get_item(1)?;
        let fields_ir = args.get_item(2)?;
        let args_len = args.len()?;

        // Extends: list of trait names
        let mut extends: Vec<Py<PyAny>> = Vec::new();
        for e in extends_ir.try_iter()? {
            let e = e?;
            extends.push(e.unbind());
        }
        let extends_tuple = PyTuple::new(py, &extends)?;

        // Fields: tuple of (name, default_or_None)
        // Compile default expressions onto the stack
        let mut fields_info: Vec<Py<PyAny>> = Vec::new();
        let mut num_defaults: u32 = 0;
        for f in fields_ir.try_iter()? {
            let f = f?;
            let pair = f.cast::<PyTuple>()?;
            let fname: String = pair.get_item(0)?.extract()?;
            let default = pair.get_item(1)?;
            let has_default = !default.is_none();
            if has_default {
                self.compile_node(py, &default)?;
                num_defaults += 1;
            }
            let entry = PyTuple::new(
                py,
                &[
                    fname.into_pyobject(py)?.into_any().unbind(),
                    has_default
                        .into_pyobject(py)?
                        .to_owned()
                        .into_any()
                        .unbind(),
                ],
            )?;
            fields_info.push(entry.into_any().unbind());
        }
        let fields_tuple = PyTuple::new(py, &fields_info)?;

        // Methods: compile to CodeObjects (same as compile_struct)
        // Each method in IR is (name, lambda_or_none, is_static)
        let methods_list = if args_len > 3 {
            let methods = args.get_item(3)?;
            let mut compiled_methods: Vec<Py<PyAny>> = Vec::new();
            for m in methods.try_iter()? {
                let method = m?;
                let method_tuple = method.cast::<PyTuple>()?;
                let method_name: String = method_tuple.get_item(0)?.extract()?;
                let lambda_op = method_tuple.get_item(1)?;
                let is_static: bool = if method_tuple.len() > 2 {
                    method_tuple.get_item(2)?.extract().unwrap_or(false)
                } else {
                    false
                };
                let is_static_py = is_static.into_pyobject(py)?.to_owned().into_any().unbind();

                // Abstract method: (name, None, is_static) - skip compilation
                if lambda_op.is_none() {
                    let pair = PyTuple::new(
                        py,
                        &[
                            method_name.into_pyobject(py)?.into_any().unbind(),
                            py.None(),
                            is_static_py,
                        ],
                    )?;
                    compiled_methods.push(pair.into_any().unbind());
                    continue;
                }

                let lambda_args = lambda_op.getattr("args")?;
                let raw_params = lambda_args.get_item(0)?;
                let body = lambda_args.get_item(1)?;

                let mut param_names = Vec::new();
                let mut defaults = Vec::new();
                let mut vararg_idx: i32 = -1;

                for p in raw_params.try_iter()? {
                    let p = p?;
                    if p.is_instance_of::<PyTuple>() && p.len()? == 2 {
                        let pname: String = p.get_item(0)?.extract()?;
                        let default = p.get_item(1)?;
                        if pname == "*" {
                            vararg_idx = param_names.len() as i32;
                            param_names.push(default.extract::<String>()?);
                        } else {
                            param_names.push(pname);
                            defaults.push(default.unbind());
                        }
                    } else {
                        param_names.push(p.extract()?);
                    }
                }

                let mut func_compiler = Compiler::new();
                let code = func_compiler.compile_function(
                    py,
                    param_names,
                    &body,
                    &method_name,
                    defaults,
                    vararg_idx,
                    self.nesting_depth,
                )?;
                let py_code = Py::new(py, PyCodeObject::new(code))?;

                let pair = PyTuple::new(
                    py,
                    &[
                        method_name.into_pyobject(py)?.into_any().unbind(),
                        py_code.into_any(),
                        is_static_py,
                    ],
                )?;
                compiled_methods.push(pair.into_any().unbind());
            }
            Some(PyList::new(py, &compiled_methods)?)
        } else {
            None
        };

        // Store (name, extends, fields_info, num_defaults, [methods]) as constant
        let num_defaults_py = num_defaults.into_pyobject(py)?.into_any().as_any().clone();
        let trait_info = if let Some(methods) = methods_list {
            PyTuple::new(
                py,
                &[
                    name.as_str().into_pyobject(py)?.into_any().as_any().clone(),
                    extends_tuple.into_any(),
                    fields_tuple.into_any(),
                    num_defaults_py,
                    methods.into_any(),
                ],
            )?
        } else {
            PyTuple::new(
                py,
                &[
                    name.as_str().into_pyobject(py)?.into_any().as_any().clone(),
                    extends_tuple.into_any(),
                    fields_tuple.into_any(),
                    num_defaults_py,
                ],
            )?
        };

        let idx = self.add_const(py, &trait_info.into_any());
        self.emit(VMOpCode::MakeTrait, idx as u32);

        Ok(())
    }

    // ========== ND Operations ==========

    fn compile_nd_recursion(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        // Args: [arg0, arg1]
        // Declaration form: ~~lambda → args=(lambda, None)
        // Combinator form: ~~(seed, lambda) → args=(seed, lambda)
        let arg0 = args.get_item(0)?;
        let arg1 = args.get_item(1)?;

        if arg1.is_none() {
            // Declaration form: ~~ lambda (arg1 is None)
            self.compile_node(py, &arg0)?;
            self.emit(VMOpCode::NdRecursion, 1);
        } else {
            // Combinator form: ~~(seed, lambda)
            self.compile_node(py, &arg0)?;
            self.compile_node(py, &arg1)?;
            self.emit(VMOpCode::NdRecursion, 0);
        }
        Ok(())
    }

    fn compile_nd_map(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        // Args: [arg0, arg1]
        // Lift form: ~>f → args=(func, None)
        // Applicative form: ~>(data, f) → args=(data, func)
        let arg0 = args.get_item(0)?;
        let arg1 = args.get_item(1)?;

        if arg1.is_none() {
            // Lift form: ~> f (arg1 is None)
            self.compile_node(py, &arg0)?;
            self.emit(VMOpCode::NdMap, 1);
        } else {
            // Applicative form: ~>(data, f)
            self.compile_node(py, &arg0)?;
            self.compile_node(py, &arg1)?;
            self.emit(VMOpCode::NdMap, 0);
        }
        Ok(())
    }

    // ========== F-strings ==========

    fn compile_fstring(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let len = args.len()?;
        if len == 0 {
            let empty_str = "".into_pyobject(py)?.into_any();
            let idx = self.add_const(py, &empty_str);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        for i in 0..len {
            let part = args.get_item(i)?;
            let part_type: String = part.get_item(0)?.extract()?;
            let part_value = part.get_item(1)?;

            if part_type == "text" {
                let idx = self.add_const(py, &part_value);
                self.emit(VMOpCode::LoadConst, idx as u32);
            } else if part_type == "expr" {
                // part_value is (expr_code, format_spec_or_none)
                let expr_code = part_value.get_item(0)?;
                let format_spec = part_value.get_item(1)?;

                let builtins = py.import("builtins")?;

                if format_spec.is_none() {
                    // No format spec: use str(expr)
                    let str_builtin = builtins.getattr("str")?;
                    let str_idx = self.add_const(py, &str_builtin);
                    self.emit(VMOpCode::LoadConst, str_idx as u32);
                    self.compile_node(py, &expr_code)?;
                    self.emit(VMOpCode::Call, 1);
                } else {
                    // Has format spec: use format(expr, spec)
                    let format_builtin = builtins.getattr("format")?;
                    let format_idx = self.add_const(py, &format_builtin);
                    self.emit(VMOpCode::LoadConst, format_idx as u32);
                    self.compile_node(py, &expr_code)?;
                    let spec_idx = self.add_const(py, &format_spec);
                    self.emit(VMOpCode::LoadConst, spec_idx as u32);
                    self.emit(VMOpCode::Call, 2);
                }
            }
        }

        // Concatenate all parts
        if len > 1 {
            for _ in 0..(len - 1) {
                self.emit(VMOpCode::Add, 0);
            }
        }
        Ok(())
    }

    // ========== Pattern matching ==========

    fn compile_match(&mut self, py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
        let value_expr = args.get_item(0)?;
        let cases = args.get_item(1)?;

        // Pre-allocate slots for pattern variables
        self.collect_pattern_vars(py, &cases)?;

        // Compile value to match
        self.compile_node(py, &value_expr)?;

        let mut end_jumps = Vec::new();
        let cases_len = cases.len()?;

        for i in 0..cases_len {
            let case = cases.get_item(i)?;
            let pattern = case.get_item(0)?;
            let guard = case.get_item(1)?;
            let body = case.get_item(2)?;

            // Duplicate value for this case
            self.emit(VMOpCode::DupTop, 0);

            // Try native VM pattern; fallback to legacy constant-based path
            let use_native = self.try_compile_pattern(py, &pattern)?;
            if let Some(vm_pattern) = use_native {
                let pat_idx = self.patterns.len();
                self.patterns.push(vm_pattern);
                self.emit(VMOpCode::MatchPatternVM, pat_idx as u32);
            } else {
                let pattern_idx = self.add_const(py, &pattern);
                self.emit(VMOpCode::MatchPattern, pattern_idx as u32);
            }

            // Duplicate bindings before test
            self.emit(VMOpCode::DupTop, 0);

            // Jump to next case if no match
            let skip_jump = self.emit(VMOpCode::JumpIfNone, 0);

            let guard_fail = if !guard.is_none() {
                // Guard present
                self.emit(VMOpCode::DupTop, 0);
                // Guard should not leak temporary bindings into existing locals.
                // Snapshot from slot 0 to restore all potentially shadowed slots.
                self.emit(VMOpCode::PushBlock, 0);
                self.emit(VMOpCode::BindMatch, 0);
                self.compile_node(py, &guard)?;
                self.emit(VMOpCode::PopBlock, 0);
                Some(self.emit(VMOpCode::JumpIfFalse, 0))
            } else {
                None
            };

            // Bind pattern variables for body
            self.emit(VMOpCode::BindMatch, 0);

            // Pop match value before body so break/continue don't leave it on stack
            self.emit(VMOpCode::PopTop, 0);

            // Compile body
            self.compile_node(py, &body)?;

            // Jump to end
            end_jumps.push(self.emit(VMOpCode::Jump, 0));

            // Patch skip_jump
            let next_case = self.instructions.len();

            if let Some(guard_fail_addr) = guard_fail {
                self.patch(guard_fail_addr, next_case as u32);
                self.emit(VMOpCode::PopTop, 0);
                let guard_cleanup_done = self.emit(VMOpCode::Jump, 0);

                let skip_cleanup = self.instructions.len();
                self.patch(skip_jump, skip_cleanup as u32);
                self.emit(VMOpCode::PopTop, 0);

                let next_case_start = self.instructions.len();
                self.patch(guard_cleanup_done, next_case_start as u32);
            } else {
                self.patch(skip_jump, self.instructions.len() as u32);
                self.emit(VMOpCode::PopTop, 0);
            }
        }

        // No match: pop value, raise error
        self.emit(VMOpCode::PopTop, 0);
        let msg = "No matching pattern".to_string();
        let msg_idx = self.add_const_value(py, msg.into_pyobject(py)?.into_any().unbind());
        self.emit(VMOpCode::MatchFail, msg_idx as u32);

        // Patch end jumps
        let end_addr = self.instructions.len() as u32;
        for addr in end_jumps {
            self.patch(addr, end_addr);
        }

        Ok(())
    }

    fn collect_pattern_vars(&mut self, py: Python<'_>, cases: &Bound<'_, PyAny>) -> PyResult<()> {
        let cases_len = cases.len()?;
        for i in 0..cases_len {
            let case = cases.get_item(i)?;
            let pattern = case.get_item(0)?;
            self.collect_vars_from_pattern(py, &pattern)?;
        }
        Ok(())
    }

    fn collect_vars_from_pattern(
        &mut self,
        py: Python<'_>,
        pattern: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let type_name = pattern.get_type().name()?;

        if type_name == "PatternVar" {
            let name: String = pattern.getattr("name")?.extract()?;
            if name != "_" && !self.locals.contains(&name) {
                self.add_local(&name);
            }
        } else if type_name == "PatternStruct" {
            let fields = pattern.getattr("fields")?;
            for field_result in fields.try_iter()? {
                let name: String = field_result?.extract()?;
                if name != "_" && !self.locals.contains(&name) {
                    self.add_local(&name);
                }
            }
        } else if type_name == "PatternOr" || type_name == "PatternTuple" {
            let patterns = pattern.getattr("patterns")?;
            let len = patterns.len()?;
            for i in 0..len {
                let p = patterns.get_item(i)?;
                // Check for star pattern tuple
                if p.is_instance_of::<PyTuple>() && p.len()? == 2 {
                    let first: String = p.get_item(0)?.extract().unwrap_or_default();
                    if first == "*" {
                        let name: String = p.get_item(1)?.extract().unwrap_or_default();
                        if !name.is_empty() && name != "_" && !self.locals.contains(&name) {
                            self.add_local(&name);
                        }
                        continue;
                    }
                }
                self.collect_vars_from_pattern(py, &p)?;
            }
        }
        Ok(())
    }

    /// Try to compile a pattern into a VMPattern (native VM path).
    /// Returns None if the pattern can't be compiled natively (fallback to legacy).
    fn try_compile_pattern(
        &mut self,
        py: Python<'_>,
        pattern: &Bound<'_, PyAny>,
    ) -> PyResult<Option<VMPattern>> {
        let tag = match get_pattern_tag(pattern) {
            Some(t) => t,
            None => return Ok(None),
        };

        match tag {
            TAG_WILDCARD => Ok(Some(VMPattern::Wildcard)),
            TAG_VAR => {
                let pat = pattern.cast::<PatternVar>().unwrap();
                let name = pat.borrow().name.clone();
                if name == "_" {
                    Ok(Some(VMPattern::Wildcard))
                } else {
                    let slot = self.add_local(&name);
                    Ok(Some(VMPattern::Var(slot)))
                }
            }
            TAG_LITERAL => {
                let pat = pattern.cast::<PatternLiteral>().unwrap();
                let value_obj = pat.borrow().value.clone_ref(py);
                let value_bound = value_obj.bind(py);

                // Try to convert to Value; if it's an IR/Op node (e.g. -42), bail
                if value_bound.cast::<Op>().is_ok() {
                    return Ok(None);
                }
                match Value::from_pyobject(py, value_bound) {
                    Ok(val) => Ok(Some(VMPattern::Literal(val))),
                    Err(_) => Ok(None),
                }
            }
            TAG_OR => {
                let pat = pattern.cast::<PatternOr>().unwrap();
                let patterns_obj = pat.borrow().patterns.clone_ref(py);
                let mut sub_patterns = Vec::new();
                for sub_result in patterns_obj.bind(py).try_iter()? {
                    let sub = sub_result?;
                    match self.try_compile_pattern(py, &sub)? {
                        Some(p) => sub_patterns.push(p),
                        None => return Ok(None),
                    }
                }
                Ok(Some(VMPattern::Or(sub_patterns)))
            }
            TAG_TUPLE => {
                let pat = pattern.cast::<PatternTuple>().unwrap();
                let patterns_obj = pat.borrow().patterns.clone_ref(py);
                let mut elements = Vec::new();
                for sub_result in patterns_obj.bind(py).try_iter()? {
                    let sub = sub_result?;
                    // Check for star pattern tuple ("*", name)
                    if sub.is_instance_of::<PyTuple>() && sub.len()? == 2 {
                        let first: String = sub.get_item(0)?.extract().unwrap_or_default();
                        if first == "*" {
                            let name: String = sub.get_item(1)?.extract().unwrap_or_default();
                            let slot = if name.is_empty() || name == "_" {
                                usize::MAX
                            } else {
                                self.add_local(&name)
                            };
                            elements.push(VMPatternElement::Star(slot));
                            continue;
                        }
                    }
                    match self.try_compile_pattern(py, &sub)? {
                        Some(p) => elements.push(VMPatternElement::Pattern(p)),
                        None => return Ok(None),
                    }
                }
                Ok(Some(VMPattern::Tuple(elements)))
            }
            TAG_STRUCT => {
                let pat = pattern.cast::<PatternStruct>().unwrap();
                let struct_name = pat.borrow().name.clone();
                let fields_obj = pat.borrow().fields.clone_ref(py);
                let mut field_slots = Vec::new();
                for field_result in fields_obj.bind(py).try_iter()? {
                    let field_name: String = field_result?.extract()?;
                    let slot = self.add_local(&field_name);
                    field_slots.push((field_name, slot));
                }
                Ok(Some(VMPattern::Struct {
                    name: struct_name,
                    field_slots,
                }))
            }
            _ => Ok(None),
        }
    }

    // ========== Unpacking ==========

    fn compile_unpack_pattern(
        &mut self,
        py: Python<'_>,
        pattern: &Bound<'_, PyAny>,
        keep_last: bool,
    ) -> PyResult<()> {
        let len = pattern.len()?;

        // Find star pattern index
        let mut star_idx: i32 = -1;
        for i in 0..len {
            let item = pattern.get_item(i)?;
            if item.is_instance_of::<PyTuple>() && item.len()? == 2 {
                let first: String = item.get_item(0)?.extract().unwrap_or_default();
                if first == "*" {
                    star_idx = i as i32;
                    break;
                }
            }
        }

        if star_idx >= 0 {
            // Unpacking with *rest: encode (before << 8) | after
            let before = star_idx as u32;
            let after = (len as i32 - star_idx - 1) as u32;
            let arg = (before << 8) | after;
            self.emit(VMOpCode::UnpackEx, arg);
        } else {
            self.emit(VMOpCode::UnpackSequence, len as u32);
        }

        // Store each unpacked value
        let in_block = !self.loop_stack.is_empty();
        for idx in 0..len {
            let item = pattern.get_item(idx)?;
            let is_last = idx == len - 1;

            if is_last && keep_last {
                self.emit(VMOpCode::DupTop, 0);
            }

            if item.is_instance_of::<PyTuple>() && item.len()? == 2 {
                let first: String = item.get_item(0)?.extract().unwrap_or_default();
                if first == "*" {
                    // Star pattern
                    let name = self.extract_single_name(py, &item.get_item(1)?)?;
                    let slot = self.add_local(&name);
                    if in_block {
                        self.emit(VMOpCode::StoreLocal, slot as u32);
                    } else {
                        let name_idx = self.add_name(&name);
                        self.emit(VMOpCode::StoreScope, name_idx as u32);
                    }
                    continue;
                }
            }

            if item.is_instance_of::<PyList>() || item.is_instance_of::<PyTuple>() {
                // Nested pattern
                self.compile_unpack_pattern(py, &item, false)?;
            } else {
                // Simple variable
                let name = self.extract_single_name(py, &item)?;
                let slot = self.add_local(&name);
                if in_block {
                    self.emit(VMOpCode::StoreLocal, slot as u32);
                } else {
                    let name_idx = self.add_name(&name);
                    self.emit(VMOpCode::StoreScope, name_idx as u32);
                }
            }
        }

        Ok(())
    }
}

/// PyO3 wrapper for the Compiler.
#[pyclass(name = "Compiler", module = "catnip._rs")]
pub struct PyCompiler {
    inner: Compiler,
}

#[pymethods]
impl PyCompiler {
    #[new]
    fn new() -> Self {
        Self {
            inner: Compiler::new(),
        }
    }

    /// Compile IR to bytecode and return PyCodeObject.
    ///
    /// Args:
    ///     node: Op node or list of Op nodes
    ///     name: Optional name for the code object (defaults to "<module>")
    #[pyo3(signature = (node, name=None))]
    fn compile(
        &mut self,
        py: Python<'_>,
        node: &Bound<'_, PyAny>,
        name: Option<&str>,
    ) -> PyResult<PyCodeObject> {
        let mut code = self.inner.compile(py, node)?;
        if let Some(n) = name {
            code.name = n.to_string();
        }
        Ok(PyCodeObject::new(code))
    }

    /// Compile a function body with parameters.
    ///
    /// Args:
    ///     params: List of parameter names
    ///     body: Function body Op node
    ///     name: Function name
    ///     defaults: List of default values (may be empty)
    #[pyo3(signature = (params, body, name, defaults=None))]
    fn compile_function(
        &mut self,
        py: Python<'_>,
        params: Vec<String>,
        body: &Bound<'_, PyAny>,
        name: &str,
        defaults: Option<Vec<Py<PyAny>>>,
    ) -> PyResult<PyCodeObject> {
        let defaults_vec = defaults.unwrap_or_default();
        let code = self
            .inner
            .compile_function(py, params, body, name, defaults_vec, -1, 0)?;
        Ok(PyCodeObject::new(code))
    }
}
