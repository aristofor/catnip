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

pub use code_object::CodeObject;
pub use core::CompilerCore;
pub use error::{CompileError, CompileResult};
pub use pattern::{VMPattern, VMPatternElement};

use crate::Value;
use catnip_core::ir::opcode::IROpCode;
use catnip_core::ir::pure::{BroadcastType, IR};
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
}

/// Pure Rust IR -> bytecode compiler (no Python dependency).
pub struct PureCompiler {
    core: CompilerCore,
    /// Sub-functions collected during compilation
    functions: Vec<CodeObject>,
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
        self.core.reset();
        self.core.name = spec.name.to_string();
        self.core.nargs = spec.params.len();
        self.core.defaults = spec.defaults;
        self.core.in_function = true;
        self.core.nesting_depth = spec.parent_nesting_depth + 1;

        for param in &spec.params {
            self.core.add_local(param);
        }

        self.compile_node(spec.body)?;
        self.emit(VMOpCode::Return, 0);

        let mut code = self.core.build_code_object()?;
        code.vararg_idx = spec.vararg_idx;
        Ok(code)
    }

    fn compile_function_inner(&mut self, spec: FunctionCompileSpec<'_>) -> CompileResult<CodeObject> {
        let saved_core = std::mem::take(&mut self.core);
        let result = self.compile_function_to_code(spec);
        self.core = saved_core;
        result
    }

    // ========== compile_node ==========

    fn compile_node(&mut self, ir: &IR) -> CompileResult<()> {
        match ir {
            // Literals
            IR::Int(n) => {
                let idx = self.core.add_const(Value::from_i64(*n));
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::Float(f) => {
                let idx = self.core.add_const(Value::from_float(*f));
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::Bool(b) => {
                let idx = self.core.add_const(Value::from_bool(*b));
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::None => {
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::String(s) => {
                let idx = self.core.add_const(Value::from_string(s.clone()));
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::Bytes(v) => {
                let idx = self.core.add_const(Value::from_bytes(v.clone()));
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::Decimal(s) => Err(CompileError::UnsupportedLiteral(format!(
                "Decimal literals not supported in standalone mode: {}",
                s
            ))),
            IR::Imaginary(s) => Err(CompileError::UnsupportedLiteral(format!(
                "Imaginary literals not supported in standalone mode: {}",
                s
            ))),

            // Variables
            IR::Ref(name, start_byte, _end_byte) => {
                if *start_byte >= 0 {
                    self.core.current_start_byte = *start_byte as u32;
                }
                self.core.compile_name_load(name)
            }
            IR::Identifier(name) => self.core.compile_name_load(name),

            // Sequences
            IR::Program(items) => self.compile_statement_list(items),
            IR::List(items) => {
                for item in items {
                    self.compile_node(item)?;
                }
                self.emit(VMOpCode::BuildList, items.len() as u32);
                Ok(())
            }
            IR::Tuple(items) => {
                for item in items {
                    self.compile_node(item)?;
                }
                self.emit(VMOpCode::BuildTuple, items.len() as u32);
                Ok(())
            }
            IR::Set(items) => {
                for item in items {
                    self.compile_node(item)?;
                }
                self.emit(VMOpCode::BuildSet, items.len() as u32);
                Ok(())
            }
            IR::Dict(pairs) => {
                for (key, value) in pairs {
                    self.compile_node(key)?;
                    self.compile_node(value)?;
                }
                self.emit(VMOpCode::BuildDict, pairs.len() as u32);
                Ok(())
            }

            // Function call
            IR::Call {
                func,
                args,
                kwargs,
                start_byte,
                tail,
                ..
            } => {
                if *start_byte > 0 {
                    self.core.current_start_byte = *start_byte as u32;
                }
                if *tail {
                    let mut all_args = vec![func.as_ref()];
                    all_args.extend(args.iter());
                    self.compile_call_from_args(&all_args, kwargs, true)
                } else {
                    self.compile_call_dispatch(func, args, kwargs)
                }
            }

            // Operations
            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                ..
            } => {
                if *start_byte > 0 {
                    self.core.current_start_byte = *start_byte as u32;
                }
                self.compile_op_dispatch(*opcode, args, kwargs, *tail)
            }

            // Broadcasting
            IR::Broadcast {
                target,
                operator,
                operand,
                broadcast_type,
            } => {
                if let Some(t) = target.as_deref() {
                    self.compile_node(t)?;
                }
                let nd_flag = match operator.as_ref() {
                    IR::Op { opcode, .. } if *opcode == IROpCode::NdRecursion => Some(4u32),
                    IR::Op { opcode, .. } if *opcode == IROpCode::NdMap => Some(8u32),
                    _ => None,
                };
                if let Some(nd_flag) = nd_flag {
                    if let IR::Op { args, .. } = operator.as_ref() {
                        if !args.is_empty() {
                            self.compile_node(&args[0])?;
                        }
                    }
                    let is_filter = matches!(broadcast_type, BroadcastType::If);
                    let mut flags = nd_flag;
                    if is_filter {
                        flags |= 1;
                    }
                    self.emit(VMOpCode::Broadcast, flags);
                } else {
                    self.compile_node(operator)?;
                    let has_operand = operand.is_some();
                    if let Some(o) = operand.as_deref() {
                        self.compile_node(o)?;
                    }
                    let is_filter = matches!(broadcast_type, BroadcastType::If);
                    let mut flags = 0u32;
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

            // Slice
            IR::Slice { start, stop, step } => {
                self.compile_node(start)?;
                self.compile_node(stop)?;
                self.compile_node(step)?;
                self.emit(VMOpCode::BuildSlice, 3);
                Ok(())
            }

            // Patterns only appear inside match cases
            IR::PatternLiteral(_)
            | IR::PatternVar(_)
            | IR::PatternWildcard
            | IR::PatternOr(_)
            | IR::PatternTuple(_)
            | IR::PatternStruct { .. } => {
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
        }
    }

    // ========== Statement list ==========

    fn compile_statement_list(&mut self, stmts: &[IR]) -> CompileResult<()> {
        if stmts.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }
        let len = stmts.len();
        for (i, stmt) in stmts.iter().enumerate() {
            let is_void = is_op_ir(stmt, IROpCode::SetItem) || is_op_ir(stmt, IROpCode::SetAttr);
            self.compile_node(stmt)?;
            if i < len - 1 {
                if !is_void {
                    self.emit(VMOpCode::PopTop, 0);
                }
            } else if is_void {
                // Void op as last stmt: push NIL
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
            }
        }
        Ok(())
    }

    // ========== Op dispatch ==========

    fn compile_op_dispatch(
        &mut self,
        opcode: IROpCode,
        args: &[IR],
        kwargs: &IndexMap<String, IR>,
        tail: bool,
    ) -> CompileResult<()> {
        match opcode {
            // Arithmetic
            IROpCode::Add => self.compile_binary(VMOpCode::Add, args),
            IROpCode::Sub => self.compile_binary(VMOpCode::Sub, args),
            IROpCode::Mul => self.compile_binary(VMOpCode::Mul, args),
            IROpCode::Div | IROpCode::TrueDiv => self.compile_binary(VMOpCode::Div, args),
            IROpCode::FloorDiv => self.compile_binary(VMOpCode::FloorDiv, args),
            IROpCode::Mod => self.compile_binary(VMOpCode::Mod, args),
            IROpCode::Pow => self.compile_binary(VMOpCode::Pow, args),
            IROpCode::Neg => self.compile_unary(VMOpCode::Neg, args),
            IROpCode::Pos => self.compile_unary(VMOpCode::Pos, args),

            // Comparison
            IROpCode::Lt => self.compile_binary(VMOpCode::Lt, args),
            IROpCode::Le => self.compile_binary(VMOpCode::Le, args),
            IROpCode::Gt => self.compile_binary(VMOpCode::Gt, args),
            IROpCode::Ge => self.compile_binary(VMOpCode::Ge, args),
            IROpCode::Eq => self.compile_binary(VMOpCode::Eq, args),
            IROpCode::Ne => self.compile_binary(VMOpCode::Ne, args),

            // Membership
            IROpCode::In => self.compile_binary(VMOpCode::In, args),
            IROpCode::NotIn => self.compile_binary(VMOpCode::NotIn, args),

            // Identity
            IROpCode::Is => self.compile_binary(VMOpCode::Is, args),
            IROpCode::IsNot => self.compile_binary(VMOpCode::IsNot, args),

            // Logical
            IROpCode::Not => self.compile_unary(VMOpCode::Not, args),
            IROpCode::And => self.compile_and(args),
            IROpCode::Or => self.compile_or(args),
            IROpCode::NullCoalesce => self.compile_null_coalesce(args),

            // Bitwise
            IROpCode::BAnd => self.compile_binary(VMOpCode::BAnd, args),
            IROpCode::BOr => self.compile_binary(VMOpCode::BOr, args),
            IROpCode::BXor => self.compile_binary(VMOpCode::BXor, args),
            IROpCode::BNot => self.compile_unary(VMOpCode::BNot, args),
            IROpCode::LShift => self.compile_binary(VMOpCode::LShift, args),
            IROpCode::RShift => self.compile_binary(VMOpCode::RShift, args),

            // Variables
            IROpCode::SetLocals => self.compile_set_locals(args, kwargs),
            IROpCode::GetAttr => self.compile_getattr(args),
            IROpCode::SetAttr => self.compile_setattr(args),
            IROpCode::GetItem => self.compile_getitem(args),
            IROpCode::SetItem => self.compile_setitem(args),
            IROpCode::Slice => self.compile_slice(args),

            // Control flow
            IROpCode::OpIf => self.compile_if(args),
            IROpCode::OpWhile => self.compile_while(args),
            IROpCode::OpFor => self.compile_for(args),
            IROpCode::OpBlock => self.compile_block(args),
            IROpCode::OpReturn => self.compile_return(args),
            IROpCode::OpBreak => self.core.compile_break(),
            IROpCode::OpContinue => self.core.compile_continue(),

            // Functions
            IROpCode::Call => self.compile_call_op(args, kwargs, tail),
            IROpCode::OpLambda => self.compile_lambda(args),
            IROpCode::FnDef => self.compile_fn_def(args),

            // Collections
            IROpCode::ListLiteral => self.compile_collection(VMOpCode::BuildList, args),
            IROpCode::TupleLiteral => self.compile_collection(VMOpCode::BuildTuple, args),
            IROpCode::SetLiteral => self.compile_collection(VMOpCode::BuildSet, args),
            IROpCode::DictLiteral => self.compile_dict_op(args),

            // String
            IROpCode::Fstring => self.compile_fstring(args),

            // Match
            IROpCode::OpMatch => self.compile_match(args),

            // Broadcasting
            IROpCode::Broadcast => self.compile_broadcast_op(args),

            // ND operations
            IROpCode::NdEmptyTopos => {
                self.emit(VMOpCode::NdEmptyTopos, 0);
                Ok(())
            }
            IROpCode::NdRecursion => self.compile_nd_recursion(args),
            IROpCode::NdMap => self.compile_nd_map(args),

            // Stack ops
            IROpCode::Push => {
                if !args.is_empty() {
                    self.compile_node(&args[0])
                } else {
                    Ok(())
                }
            }
            IROpCode::Pop => {
                self.emit(VMOpCode::PopTop, 0);
                Ok(())
            }
            IROpCode::Nop => {
                self.emit(VMOpCode::Nop, 0);
                Ok(())
            }

            IROpCode::Pragma => {
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }

            IROpCode::Breakpoint => {
                self.emit(VMOpCode::Breakpoint, 0);
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }

            IROpCode::TypeOf => {
                if !args.is_empty() {
                    self.compile_node(&args[0])?;
                }
                self.emit(VMOpCode::TypeOf, 0);
                Ok(())
            }

            IROpCode::Globals => {
                self.emit(VMOpCode::Globals, 0);
                Ok(())
            }

            IROpCode::Locals => {
                self.emit(VMOpCode::Locals, 0);
                Ok(())
            }

            IROpCode::OpStruct => self.compile_struct(args),
            IROpCode::TraitDef => self.compile_trait(args),

            _ => Err(CompileError::NotImplemented(format!(
                "PureCompiler: cannot compile IR opcode: {}",
                opcode
            ))),
        }
    }

    // ========== Binary/Unary operations ==========

    fn compile_binary(&mut self, vm_op: VMOpCode, args: &[IR]) -> CompileResult<()> {
        let (left, right) = if args.len() == 1 {
            match &args[0] {
                IR::List(items) | IR::Tuple(items) if items.len() >= 2 => (&items[0], &items[1]),
                _ => return Err(CompileError::ValueError("Invalid binary args".to_string())),
            }
        } else if args.len() >= 2 {
            (&args[0], &args[1])
        } else {
            return Err(CompileError::ValueError("Binary op requires 2 args".to_string()));
        };
        self.compile_node(left)?;
        self.compile_node(right)?;
        self.emit(vm_op, 0);
        Ok(())
    }

    fn compile_unary(&mut self, vm_op: VMOpCode, args: &[IR]) -> CompileResult<()> {
        if args.is_empty() {
            return Err(CompileError::ValueError("Unary op requires 1 arg".to_string()));
        }
        self.compile_node(&args[0])?;
        self.emit(vm_op, 0);
        Ok(())
    }

    // ========== Short-circuit logic ==========

    fn unwrap_binary_args<'a>(&self, args: &'a [IR]) -> CompileResult<(&'a IR, &'a IR)> {
        if args.len() == 1 {
            match &args[0] {
                IR::List(items) | IR::Tuple(items) if items.len() >= 2 => Ok((&items[0], &items[1])),
                _ => Err(CompileError::ValueError("requires 2 operands".to_string())),
            }
        } else if args.len() >= 2 {
            Ok((&args[0], &args[1]))
        } else {
            Err(CompileError::ValueError("requires 2 operands".to_string()))
        }
    }

    fn compile_and(&mut self, args: &[IR]) -> CompileResult<()> {
        let (left, right) = self.unwrap_binary_args(args)?;
        self.compile_node(left)?;
        self.emit(VMOpCode::ToBool, 0);
        let jump_idx = self.emit(VMOpCode::JumpIfFalseOrPop, 0);
        self.compile_node(right)?;
        self.emit(VMOpCode::ToBool, 0);
        let pos = self.instructions.len() as u32;
        self.patch(jump_idx, pos);
        Ok(())
    }

    fn compile_or(&mut self, args: &[IR]) -> CompileResult<()> {
        let (left, right) = self.unwrap_binary_args(args)?;
        self.compile_node(left)?;
        self.emit(VMOpCode::ToBool, 0);
        let jump_idx = self.emit(VMOpCode::JumpIfTrueOrPop, 0);
        self.compile_node(right)?;
        self.emit(VMOpCode::ToBool, 0);
        let pos = self.instructions.len() as u32;
        self.patch(jump_idx, pos);
        Ok(())
    }

    fn compile_null_coalesce(&mut self, args: &[IR]) -> CompileResult<()> {
        let (left, right) = self.unwrap_binary_args(args)?;
        self.compile_node(left)?;
        let jump_idx = self.emit(VMOpCode::JumpIfNotNoneOrPop, 0);
        self.compile_node(right)?;
        let pos = self.instructions.len() as u32;
        self.patch(jump_idx, pos);
        Ok(())
    }

    // ========== Variables ==========

    fn compile_set_locals(&mut self, args: &[IR], kwargs: &IndexMap<String, IR>) -> CompileResult<()> {
        let mut effective_args: Vec<&IR> = args.iter().collect();
        let mut explicit_unpack = false;
        if effective_args.len() >= 3 {
            if let Some(IR::Bool(b)) = effective_args.last() {
                explicit_unpack = *b;
                effective_args.pop();
            }
        }

        let names_pattern: Option<&IR>;
        let values: Vec<&IR>;

        if let Some(names_ir) = kwargs.get("names") {
            names_pattern = Some(names_ir);
            values = effective_args;
        } else if effective_args.len() >= 2 {
            if matches!(effective_args[0], IR::Tuple(_)) {
                names_pattern = Some(effective_args[0]);
                values = effective_args.into_iter().skip(1).collect();
            } else {
                names_pattern = None;
                values = Vec::new();
            }
        } else {
            names_pattern = None;
            values = Vec::new();
        }

        let is_void = self.void_context;
        self.void_context = false;

        // Complex patterns (star, nested) -> VM pattern matching path
        if let Some(pattern) = names_pattern {
            if has_complex_pattern_ir(pattern) && values.len() == 1 {
                let unwrapped = unwrap_single_tuple(pattern);

                let vm_pattern = self
                    .try_compile_assign_pattern_ir(unwrapped)?
                    .ok_or_else(|| CompileError::SyntaxError("Unsupported complex assignment pattern".to_string()))?;

                let pat_idx = self.patterns.len();
                self.patterns.push(vm_pattern);

                self.compile_node(values[0])?;
                self.emit(VMOpCode::DupTop, 0);
                self.emit(VMOpCode::MatchAssignPatternVM, pat_idx as u32);
                self.emit(VMOpCode::BindMatch, 0);

                let names_to_sync = extract_names_ir(unwrapped);
                for name in names_to_sync {
                    let Some(slot) = self.locals.iter().position(|n| n == &name) else {
                        continue;
                    };
                    let needs_scope_sync = if self.nesting_depth == 0 {
                        true
                    } else {
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

        let names: Vec<String> = if let Some(pattern) = names_pattern {
            extract_names_ir(pattern)
        } else {
            Vec::new()
        };

        if names.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        // Single name, single value: simple assignment
        if names.len() == 1 && values.len() == 1 && !explicit_unpack {
            self.compile_node(values[0])?;
            if !is_void {
                self.emit(VMOpCode::DupTop, 0);
            }
            self.emit_store(&names[0]);
            return Ok(());
        }

        // Multiple names OR explicit unpack, single value: unpacking
        if values.len() == 1 && (names.len() > 1 || explicit_unpack) {
            self.compile_node(values[0])?;
            self.emit(VMOpCode::UnpackSequence, names.len() as u32);
            for (i, name) in names.iter().enumerate() {
                let is_last = i == names.len() - 1;
                if is_last && !is_void {
                    self.emit(VMOpCode::DupTop, 0);
                }
                self.emit_store(name);
            }
            return Ok(());
        }

        // Multiple names, multiple values: parallel assignment
        for (i, name) in names.iter().enumerate() {
            if i < values.len() {
                self.compile_node(values[i])?;
            } else if !values.is_empty() {
                self.compile_node(values.last().unwrap())?;
            } else {
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
            }
            let is_last = i == names.len() - 1;
            if is_last && !is_void {
                self.emit(VMOpCode::DupTop, 0);
            }
            self.emit_store(name);
        }
        Ok(())
    }

    fn compile_getattr(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 2, "getattr")?;
        self.compile_node(&args[0])?;
        let attr = ir_to_name(&args[1]).ok_or_else(|| CompileError::TypeError("expected string".to_string()))?;
        let idx = self.add_name(&attr);
        self.emit(VMOpCode::GetAttr, idx as u32);
        Ok(())
    }

    fn compile_setattr(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 3, "setattr")?;
        self.compile_node(&args[0])?;
        self.compile_node(&args[2])?;
        let attr = ir_to_name(&args[1]).ok_or_else(|| CompileError::TypeError("expected string".to_string()))?;
        let idx = self.add_name(&attr);
        self.emit(VMOpCode::SetAttr, idx as u32);
        Ok(())
    }

    fn compile_getitem(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 2, "getitem")?;
        self.compile_node(&args[0])?;
        self.compile_node(&args[1])?;
        self.emit(VMOpCode::GetItem, 0);
        Ok(())
    }

    fn compile_setitem(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 3, "setitem")?;
        self.compile_node(&args[0])?;
        self.compile_node(&args[1])?;
        self.compile_node(&args[2])?;
        self.emit(VMOpCode::SetItem, 0);
        Ok(())
    }

    fn compile_slice(&mut self, args: &[IR]) -> CompileResult<()> {
        for arg in args {
            self.compile_node(arg)?;
        }
        self.emit(VMOpCode::BuildSlice, args.len() as u32);
        Ok(())
    }

    // ========== Control flow ==========

    fn compile_if(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 1, "if")?;
        let branches = match &args[0] {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => &args[0..1],
        };
        let else_branch = if args.len() > 1 { Some(&args[1]) } else { None };

        if branches.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        let mut end_jumps = Vec::new();

        for branch in branches {
            let items = match branch {
                IR::Tuple(items) | IR::List(items) => items,
                _ => continue,
            };
            if items.len() != 2 {
                continue;
            }
            let cond = &items[0];
            let then_body = &items[1];

            self.compile_node(cond)?;
            let jump_to_next = self.emit(VMOpCode::JumpIfFalse, 0);
            self.compile_body(then_body)?;
            // Skip end_jump if body already ends with unconditional jump (break/continue/return)
            let last_op = self.instructions.last().map(|i| i.op);
            if !matches!(last_op, Some(VMOpCode::Jump) | Some(VMOpCode::Return)) {
                end_jumps.push(self.emit(VMOpCode::Jump, 0));
            }
            let pos = self.instructions.len() as u32;
            self.patch(jump_to_next, pos);
        }

        if let Some(else_body) = else_branch {
            self.compile_body(else_body)?;
        } else {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        }

        let end_addr = self.instructions.len() as u32;
        for addr in end_jumps {
            self.patch(addr, end_addr);
        }
        Ok(())
    }

    fn compile_while(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 2, "while")?;
        let cond = &args[0];
        let body = &args[1];

        let loop_start = self.instructions.len();
        self.loop_stack.push(core::LoopContext {
            break_targets: Vec::new(),
            continue_target: Some(loop_start),
            continue_patches: Vec::new(),
            is_for_loop: false,
        });

        let can_optimize = self.nesting_depth == 0 && !self.body_has_calls(body);
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        self.compile_node(cond)?;
        let jump_to_end = self.emit(VMOpCode::JumpIfFalse, 0);
        self.compile_body_void(body)?;

        if can_optimize {
            self.core.emit_loop_sync();
        }

        self.emit(VMOpCode::Jump, loop_start as u32);
        let ctx = self.loop_stack.pop().unwrap();

        let loop_end = self.instructions.len() as u32;
        if can_optimize {
            self.core.emit_loop_sync();
        }

        let loadconst_pos = self.instructions.len() as u32;
        let idx = self.core.add_const(Value::NIL);
        self.emit(VMOpCode::LoadConst, idx as u32);

        self.patch(jump_to_end, loop_end);
        let break_target = if can_optimize { loadconst_pos } else { loop_end };
        for addr in ctx.break_targets {
            self.patch(addr, break_target);
        }

        self.in_optimized_loop = old_optimized;
        self.loop_modified_vars = old_modified;
        Ok(())
    }

    fn compile_for(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 3, "for")?;
        let var_pattern = &args[0];
        let iterable = &args[1];
        let body = &args[2];

        let var_name = ir_to_name(var_pattern);

        // Range optimization
        if let Some(ref vn) = var_name {
            if is_range_call_ir(iterable) {
                return self.compile_for_range(vn, iterable, body);
            }
        }

        // Save/restore for existing loop variable
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
        self.compile_node(iterable)?;
        self.emit(VMOpCode::GetIter, 0);

        let loop_start = self.instructions.len();
        self.loop_stack.push(core::LoopContext {
            break_targets: Vec::new(),
            continue_target: Some(loop_start),
            continue_patches: Vec::new(),
            is_for_loop: true,
        });

        let for_iter_idx = self.emit(VMOpCode::ForIter, 0);

        if let Some(ref name) = var_name {
            let slot = self.add_local(name);
            self.emit(VMOpCode::StoreLocal, slot as u32);
        } else {
            self.compile_unpack_pattern_ir(var_pattern, false)?;
        }

        let can_optimize = self.nesting_depth == 0 && !self.body_has_calls(body);
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        self.compile_body_void(body)?;

        if can_optimize {
            self.core.emit_loop_sync();
        }

        self.emit(VMOpCode::Jump, loop_start as u32);
        let ctx = self.loop_stack.pop().unwrap();

        let loop_end = self.instructions.len();
        if can_optimize {
            self.core.emit_loop_sync();
        }
        self.emit(VMOpCode::PopBlock, 0);

        if let Some((orig_slot, temp_slot)) = save_restore {
            self.emit(VMOpCode::LoadLocal, temp_slot as u32);
            self.emit(VMOpCode::StoreLocal, orig_slot as u32);
        }

        let idx = self.core.add_const(Value::NIL);
        self.emit(VMOpCode::LoadConst, idx as u32);

        self.patch(for_iter_idx, loop_end as u32);
        for addr in ctx.break_targets {
            self.patch(addr, loop_end as u32);
        }

        self.in_optimized_loop = old_optimized;
        self.loop_modified_vars = old_modified;
        Ok(())
    }

    fn compile_for_range(&mut self, var_name: &str, range_call: &IR, body: &IR) -> CompileResult<()> {
        let range_args =
            range_call_args_ir(range_call).ok_or_else(|| CompileError::ValueError("not a range call".to_string()))?;

        let (start, stop, step): (&IR, &IR, i64) = match range_args.len() {
            1 => (&IR::Int(0), &range_args[0], 1),
            2 => (&range_args[0], &range_args[1], 1),
            _ => {
                let step = if let IR::Int(n) = &range_args[2] {
                    *n
                } else {
                    try_extract_neg_literal_ir(&range_args[2]).unwrap_or(1)
                };
                (&range_args[0], &range_args[1], step)
            }
        };

        let step_is_positive = step > 0;

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

        let slot_i = self.add_local(var_name);
        let nlocals = self.locals.len();
        let slot_stop = self.add_local(&format!("_range_stop_{}", nlocals));

        self.compile_node(start)?;
        self.emit(VMOpCode::StoreLocal, slot_i as u32);
        self.compile_node(stop)?;
        self.emit(VMOpCode::StoreLocal, slot_stop as u32);

        let loop_start = self.instructions.len();
        self.loop_stack.push(core::LoopContext {
            break_targets: Vec::new(),
            continue_target: None,
            continue_patches: Vec::new(),
            is_for_loop: false,
        });

        let has_calls = self.body_has_calls(body);
        let can_optimize = self.nesting_depth == 0 && !has_calls;
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        let arg = CompilerCore::encode_for_range_args(slot_i, slot_stop, step_is_positive, 0);
        let for_range_idx = self.emit(VMOpCode::ForRangeInt, arg);

        self.compile_body_void(body)?;

        if can_optimize {
            self.core.emit_loop_sync();
        }

        let increment_addr = self.instructions.len();
        self.loop_stack.last_mut().unwrap().continue_target = Some(increment_addr);

        if (-128..=127).contains(&step) && loop_start <= 0xFFFF {
            let arg = CompilerCore::encode_for_range_step(slot_i, step, loop_start);
            self.emit(VMOpCode::ForRangeStep, arg);
        } else {
            self.emit(VMOpCode::LoadLocal, slot_i as u32);
            let step_idx = self.core.add_const(Value::from_i64(step));
            self.emit(VMOpCode::LoadConst, step_idx as u32);
            self.emit(VMOpCode::Add, 0);
            self.emit(VMOpCode::StoreLocal, slot_i as u32);
            self.emit(VMOpCode::Jump, loop_start as u32);
        }

        let ctx = self.loop_stack.pop().unwrap();

        for addr in &ctx.continue_patches {
            self.patch(*addr, increment_addr as u32);
        }

        let loop_end = self.instructions.len() as u32;
        self.emit(VMOpCode::PopBlock, 0);

        if let Some((orig_slot, temp_slot)) = save_restore {
            self.emit(VMOpCode::LoadLocal, temp_slot as u32);
            self.emit(VMOpCode::StoreLocal, orig_slot as u32);
        }

        if can_optimize {
            self.core.emit_loop_sync();
        }

        let idx = self.core.add_const(Value::NIL);
        self.emit(VMOpCode::LoadConst, idx as u32);

        // Same convention as catnip_rs UnifiedCompiler:
        // jump_offset = loop_end - for_range_idx
        // (loop_end was computed before PopBlock/sync/LoadConst emissions)
        let jump_offset = (loop_end as usize) - for_range_idx;
        let arg = CompilerCore::encode_for_range_args(slot_i, slot_stop, step_is_positive, jump_offset);
        self.patch(for_range_idx, arg);

        for addr in ctx.break_targets {
            self.patch(addr, loop_end);
        }

        self.in_optimized_loop = old_optimized;
        self.loop_modified_vars = old_modified;
        Ok(())
    }

    fn compile_block(&mut self, args: &[IR]) -> CompileResult<()> {
        if args.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        let slot_start = self.locals.len();
        let is_module_block = self.nesting_depth == 0;
        let push_arg = if is_module_block {
            slot_start as u32 | 0x8000_0000
        } else {
            slot_start as u32
        };
        self.emit(VMOpCode::PushBlock, push_arg);

        let len = args.len();
        for (i, item) in args.iter().enumerate() {
            let is_void = is_op_ir(item, IROpCode::SetItem) || is_op_ir(item, IROpCode::SetAttr);
            self.compile_node(item)?;
            if i < len - 1 {
                if !is_void {
                    self.emit(VMOpCode::PopTop, 0);
                }
            } else if is_void {
                // Void op as last stmt: push NIL
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
            }
        }

        let pop_arg = if is_module_block { 1u32 } else { 0u32 };
        self.emit(VMOpCode::PopBlock, pop_arg);
        Ok(())
    }

    fn compile_body(&mut self, body: &IR) -> CompileResult<()> {
        if let Some(contents) = as_block_contents_ir(body) {
            if contents.is_empty() {
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
                return Ok(());
            }
            let len = contents.len();
            for (i, item) in contents.iter().enumerate() {
                let is_last = i == len - 1;
                // SetItem/SetAttr are truly void (push nothing).
                // SetLocals is NOT void here: it emits DupTop when void_context=false.
                let is_void = is_op_ir(item, IROpCode::SetItem) || is_op_ir(item, IROpCode::SetAttr);
                self.compile_node(item)?;
                if !is_last {
                    if !is_void {
                        self.emit(VMOpCode::PopTop, 0);
                    }
                } else if is_void {
                    // void op as last stmt: push NIL so compile_body
                    // always leaves exactly 1 value on the stack
                    let idx = self.core.add_const(Value::NIL);
                    self.emit(VMOpCode::LoadConst, idx as u32);
                }
            }
            return Ok(());
        }
        // Single node
        let is_void = is_op_ir(body, IROpCode::SetItem) || is_op_ir(body, IROpCode::SetAttr);
        if is_void {
            self.compile_node(body)?;
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }
        self.compile_node(body)
    }

    fn compile_body_void(&mut self, body: &IR) -> CompileResult<()> {
        if let Some(contents) = as_block_contents_ir(body) {
            for stmt in contents {
                let is_set_locals = is_op_ir(stmt, IROpCode::SetLocals);
                let is_void_op = is_op_ir(stmt, IROpCode::SetItem) || is_op_ir(stmt, IROpCode::SetAttr);

                if is_set_locals {
                    self.void_context = true;
                    self.compile_node(stmt)?;
                    self.void_context = false;
                } else if is_void_op {
                    self.compile_node(stmt)?;
                } else {
                    self.compile_node(stmt)?;
                    self.emit(VMOpCode::PopTop, 0);
                }
            }
            return Ok(());
        }
        let is_set_locals = is_op_ir(body, IROpCode::SetLocals);
        let is_void_op = is_op_ir(body, IROpCode::SetItem) || is_op_ir(body, IROpCode::SetAttr);
        if is_set_locals {
            self.void_context = true;
            self.compile_node(body)?;
            self.void_context = false;
        } else if is_void_op {
            self.compile_node(body)?;
        } else {
            self.compile_node(body)?;
            self.emit(VMOpCode::PopTop, 0);
        }
        Ok(())
    }

    fn compile_return(&mut self, args: &[IR]) -> CompileResult<()> {
        if !args.is_empty() {
            self.compile_node(&args[0])?;
        } else {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        }
        self.emit(VMOpCode::Return, 0);
        Ok(())
    }

    // ========== Functions ==========

    fn compile_call_dispatch(&mut self, func: &IR, args: &[IR], kwargs: &IndexMap<String, IR>) -> CompileResult<()> {
        let is_empty_kwargs = kwargs.is_empty();

        // Detect method call
        let method_call_info = if is_empty_kwargs {
            as_getattr_parts_ir(func)
        } else {
            None
        };

        if let Some((obj, method_name)) = method_call_info {
            self.compile_node(obj)?;
            for arg in args {
                self.compile_node(arg)?;
            }
            let name_idx = self.add_name(&method_name);
            let encoding = ((name_idx as u32) << 16) | (args.len() as u32);
            self.emit(VMOpCode::CallMethod, encoding);
        } else {
            self.compile_node(func)?;
            for arg in args {
                self.compile_node(arg)?;
            }
            if !is_empty_kwargs {
                let mut kw_names = Vec::new();
                for (name, value) in kwargs {
                    kw_names.push(name.clone());
                    self.compile_node(value)?;
                }
                let kw_tuple_val = Value::from_tuple(kw_names.iter().map(|n| Value::from_string(n.clone())).collect());
                let kw_idx = self.core.add_const(kw_tuple_val);
                self.emit(VMOpCode::LoadConst, kw_idx as u32);
                let encoding = ((args.len() as u32) << 8) | (kwargs.len() as u32);
                self.emit(VMOpCode::CallKw, encoding);
            } else {
                self.emit(VMOpCode::Call, args.len() as u32);
            }
        }
        Ok(())
    }

    fn compile_call_op(&mut self, args: &[IR], kwargs: &IndexMap<String, IR>, is_tail: bool) -> CompileResult<()> {
        let func = &args[0];
        let call_args = &args[1..];
        let is_empty_kwargs = kwargs.is_empty();

        let method_call_info = if is_empty_kwargs && !is_tail {
            as_getattr_parts_ir(func)
        } else {
            None
        };

        if let Some((obj, method_name)) = method_call_info {
            self.compile_node(obj)?;
            for arg in call_args {
                self.compile_node(arg)?;
            }
            let name_idx = self.add_name(&method_name);
            let encoding = ((name_idx as u32) << 16) | (call_args.len() as u32);
            self.emit(VMOpCode::CallMethod, encoding);
        } else {
            self.compile_node(func)?;
            for arg in call_args {
                self.compile_node(arg)?;
            }
            if !is_empty_kwargs {
                let mut kw_names = Vec::new();
                for (name, value) in kwargs {
                    kw_names.push(name.clone());
                    self.compile_node(value)?;
                }
                let kw_tuple_val = Value::from_tuple(kw_names.iter().map(|n| Value::from_string(n.clone())).collect());
                let kw_idx = self.core.add_const(kw_tuple_val);
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

    fn compile_call_from_args(
        &mut self,
        args: &[&IR],
        kwargs: &IndexMap<String, IR>,
        is_tail: bool,
    ) -> CompileResult<()> {
        let func = args[0];
        let call_args = &args[1..];
        let is_empty_kwargs = kwargs.is_empty();

        let method_call_info = if is_empty_kwargs && !is_tail {
            as_getattr_parts_ir(func)
        } else {
            None
        };

        if let Some((obj, method_name)) = method_call_info {
            self.compile_node(obj)?;
            for arg in call_args {
                self.compile_node(arg)?;
            }
            let name_idx = self.add_name(&method_name);
            let encoding = ((name_idx as u32) << 16) | (call_args.len() as u32);
            self.emit(VMOpCode::CallMethod, encoding);
        } else {
            self.compile_node(func)?;
            for arg in call_args {
                self.compile_node(arg)?;
            }
            if !is_empty_kwargs {
                let mut kw_names = Vec::new();
                for (name, value) in kwargs {
                    kw_names.push(name.clone());
                    self.compile_node(value)?;
                }
                let kw_tuple_val = Value::from_tuple(kw_names.iter().map(|n| Value::from_string(n.clone())).collect());
                let kw_idx = self.core.add_const(kw_tuple_val);
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

    fn compile_lambda(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 2, "lambda")?;
        let raw_params = &args[0];
        let body = &args[1];

        let (param_names, defaults, vararg_idx) = self.extract_params(raw_params)?;

        let mut code = self.compile_function_inner(FunctionCompileSpec {
            params: param_names,
            body,
            name: "<lambda>",
            defaults,
            vararg_idx,
            parent_nesting_depth: self.nesting_depth,
        })?;

        code.encoded_ir = Self::freeze_ir_body(body);

        let func_idx = self.functions.len() as u32;
        self.functions.push(code);
        let val = Value::from_vmfunc(func_idx);
        let idx = self.core.add_const(val);
        self.emit(VMOpCode::LoadConst, idx as u32);
        self.emit(VMOpCode::MakeFunction, 0);
        Ok(())
    }

    fn compile_fn_def(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 3, "fn_def")?;
        let name = ir_to_name(&args[0]).unwrap_or_else(|| "<fn>".to_string());
        let raw_params = &args[1];
        let body = &args[2];

        let (param_names, defaults, vararg_idx) = self.extract_params(raw_params)?;

        let mut code = self.compile_function_inner(FunctionCompileSpec {
            params: param_names,
            body,
            name: &name,
            defaults,
            vararg_idx,
            parent_nesting_depth: self.nesting_depth,
        })?;

        code.encoded_ir = Self::freeze_ir_body(body);

        let func_idx = self.functions.len() as u32;
        self.functions.push(code);
        let val = Value::from_vmfunc(func_idx);
        let idx = self.core.add_const(val);
        self.emit(VMOpCode::LoadConst, idx as u32);
        self.emit(VMOpCode::MakeFunction, 0);

        self.core.emit_store(&name);
        Ok(())
    }

    // ========== Collections ==========

    fn compile_collection(&mut self, vm_op: VMOpCode, args: &[IR]) -> CompileResult<()> {
        for arg in args {
            self.compile_node(arg)?;
        }
        self.emit(vm_op, args.len() as u32);
        Ok(())
    }

    fn compile_dict_op(&mut self, args: &[IR]) -> CompileResult<()> {
        for arg in args {
            let items = match arg {
                IR::Tuple(items) | IR::List(items) => items,
                _ => return Err(CompileError::TypeError("dict entry must be pair".to_string())),
            };
            if items.len() < 2 {
                return Err(CompileError::TypeError("dict entry must have 2 elements".to_string()));
            }
            self.compile_node(&items[0])?;
            self.compile_node(&items[1])?;
        }
        self.emit(VMOpCode::BuildDict, args.len() as u32);
        Ok(())
    }

    // ========== Broadcast ==========

    fn compile_broadcast_op(&mut self, args: &[IR]) -> CompileResult<()> {
        if args.len() < 4 {
            return Err(CompileError::TypeError("Broadcast requires 4 arguments".to_string()));
        }
        self.compile_node(&args[0])?;
        self.compile_node(&args[1])?;

        let has_operand = !is_none_ir(&args[2]);
        if has_operand {
            self.compile_node(&args[2])?;
        }

        let is_filter = matches!(&args[3], IR::Bool(true));
        let mut flags = 0u32;
        if is_filter {
            flags |= 1;
        }
        if has_operand {
            flags |= 2;
        }
        self.emit(VMOpCode::Broadcast, flags);
        Ok(())
    }

    // ========== Match ==========

    fn compile_match(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 2, "match")?;
        let value_expr = &args[0];
        let cases_ir = &args[1];

        // Pre-allocate slots for pattern variables
        if let IR::Tuple(items) | IR::List(items) = cases_ir {
            for case in items {
                if let IR::Tuple(case_parts) = case {
                    if !case_parts.is_empty() {
                        self.collect_pattern_vars_ir(&case_parts[0]);
                    }
                }
            }
        }

        self.compile_node(value_expr)?;

        let cases = match cases_ir {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => return Err(CompileError::TypeError("match cases must be a sequence".to_string())),
        };

        let mut end_jumps = Vec::new();

        for case in cases {
            let case_parts = match case {
                IR::Tuple(items) | IR::List(items) => items,
                _ => continue,
            };
            if case_parts.len() < 3 {
                continue;
            }
            let pattern = &case_parts[0];
            let guard = &case_parts[1];
            let body = &case_parts[2];

            self.emit(VMOpCode::DupTop, 0);

            let vm_pattern = self
                .try_compile_pattern_ir(pattern)?
                .ok_or_else(|| CompileError::NotImplemented(format!("unsupported match pattern: {:?}", pattern)))?;
            let pat_idx = self.patterns.len();
            self.patterns.push(vm_pattern);
            self.emit(VMOpCode::MatchPatternVM, pat_idx as u32);

            self.emit(VMOpCode::DupTop, 0);
            let skip_jump = self.emit(VMOpCode::JumpIfNone, 0);

            let guard_fail = if !is_none_ir(guard) {
                self.emit(VMOpCode::DupTop, 0);
                self.emit(VMOpCode::PushBlock, 0);
                self.emit(VMOpCode::BindMatch, 0);
                self.compile_node(guard)?;
                self.emit(VMOpCode::PopBlock, 0);
                Some(self.emit(VMOpCode::JumpIfFalse, 0))
            } else {
                None
            };

            self.emit(VMOpCode::BindMatch, 0);
            self.emit(VMOpCode::PopTop, 0);
            self.compile_node(body)?;
            end_jumps.push(self.emit(VMOpCode::Jump, 0));

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
                let pos = self.instructions.len() as u32;
                self.patch(skip_jump, pos);
                self.emit(VMOpCode::PopTop, 0);
            }
        }

        // No match: pop value, raise error
        self.emit(VMOpCode::PopTop, 0);
        let msg_val = Value::from_string("No matching pattern".to_string());
        let msg_idx = self.core.add_const(msg_val);
        self.emit(VMOpCode::MatchFail, msg_idx as u32);

        let end_addr = self.instructions.len() as u32;
        for addr in end_jumps {
            self.patch(addr, end_addr);
        }
        Ok(())
    }

    // ========== ND operations ==========

    fn compile_nd_recursion(&mut self, args: &[IR]) -> CompileResult<()> {
        if args.len() < 2 {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        } else if is_none_ir(&args[1]) {
            self.compile_node(&args[0])?;
            self.emit(VMOpCode::NdRecursion, 1);
        } else {
            self.compile_node(&args[0])?;
            self.compile_node(&args[1])?;
            self.emit(VMOpCode::NdRecursion, 0);
        }
        Ok(())
    }

    fn compile_nd_map(&mut self, args: &[IR]) -> CompileResult<()> {
        if args.len() < 2 {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        } else if is_none_ir(&args[1]) {
            self.compile_node(&args[0])?;
            self.emit(VMOpCode::NdMap, 1);
        } else {
            self.compile_node(&args[0])?;
            self.compile_node(&args[1])?;
            self.emit(VMOpCode::NdMap, 0);
        }
        Ok(())
    }

    // ========== F-strings ==========

    fn compile_fstring(&mut self, args: &[IR]) -> CompileResult<()> {
        if args.is_empty() {
            let idx = self.core.add_const(Value::from_string(String::new()));
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        let mut n_parts: u32 = 0;

        for part in args {
            if let IR::String(text) = part {
                let idx = self.core.add_const(Value::from_string(text.clone()));
                self.emit(VMOpCode::LoadConst, idx as u32);
                n_parts += 1;
            } else if let IR::Tuple(items) = part {
                // Interpolation: (expr, conv, spec)
                let expr = &items[0];
                let conv = if let IR::Int(n) = &items[1] { *n as u32 } else { 0 };
                let has_spec = items.len() > 2 && !is_none_ir(&items[2]);

                self.compile_node(expr)?;

                if has_spec {
                    if let IR::String(spec_str) = &items[2] {
                        let idx = self.core.add_const(Value::from_string(spec_str.clone()));
                        self.emit(VMOpCode::LoadConst, idx as u32);
                    } else {
                        return Err(CompileError::SyntaxError(
                            "f-string format spec must be a string literal".to_string(),
                        ));
                    }
                }

                let flags = (conv << 1) | (has_spec as u32);
                self.emit(VMOpCode::FormatValue, flags);
                n_parts += 1;
            }
        }

        if n_parts == 0 {
            let idx = self.core.add_const(Value::from_string(String::new()));
            self.emit(VMOpCode::LoadConst, idx as u32);
        } else if n_parts > 1 {
            self.emit(VMOpCode::BuildString, n_parts);
        }
        Ok(())
    }

    // ========== Struct/Trait ==========

    fn compile_struct(&mut self, args: &[IR]) -> CompileResult<()> {
        let name = ir_to_name(&args[0]).unwrap_or_else(|| "<struct>".to_string());

        let fields_items = match &args[1] {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => &[],
        };
        let args_len = args.len();

        let mut implements_list: Vec<String> = Vec::new();
        let mut base_names: Vec<String> = Vec::new();
        let mut methods_index: Option<usize> = None;

        if args_len > 3 {
            if let IR::Tuple(impl_items) | IR::List(impl_items) = &args[2] {
                for imp in impl_items {
                    if let Some(s) = ir_to_name(imp) {
                        implements_list.push(s);
                    }
                }
            }
            if !is_none_ir(&args[3]) {
                if let IR::Tuple(base_items) | IR::List(base_items) = &args[3] {
                    for b in base_items {
                        if let Some(s) = ir_to_name(b) {
                            base_names.push(s);
                        }
                    }
                } else if let Some(s) = ir_to_name(&args[3]) {
                    base_names.push(s);
                }
            }
            if args_len > 4 {
                methods_index = Some(4);
            }
        } else if args_len > 2 {
            if let Some(s) = ir_to_name(&args[2]) {
                base_names.push(s);
                if args_len > 3 {
                    methods_index = Some(3);
                }
            } else if let IR::Tuple(impl_items) | IR::List(impl_items) = &args[2] {
                let mut is_impl_list = true;
                for imp in impl_items {
                    if let Some(s) = ir_to_name(imp) {
                        implements_list.push(s);
                    } else {
                        is_impl_list = false;
                        break;
                    }
                }
                if !is_impl_list {
                    implements_list.clear();
                    methods_index = Some(2);
                }
            } else {
                methods_index = Some(2);
            }
        }

        // Build fields info as NativeTuple: ((name, has_default), ...)
        let mut fields_info: Vec<Value> = Vec::new();
        let mut num_defaults: u32 = 0;

        for field in fields_items {
            let items = match field {
                IR::Tuple(items) | IR::List(items) => items,
                _ => continue,
            };
            if items.len() >= 2 {
                let fname = ir_to_name(&items[0]).unwrap_or_default();
                let has_default = matches!(&items[1], IR::Bool(true));
                if has_default && items.len() >= 3 {
                    self.compile_node(&items[2])?;
                    num_defaults += 1;
                }
                let entry = Value::from_tuple(vec![Value::from_string(fname), Value::from_bool(has_default)]);
                fields_info.push(entry);
            }
        }
        let fields_tuple = Value::from_tuple(fields_info);

        // Compile methods
        let methods_val = if let Some(idx) = methods_index {
            let methods_items = match &args[idx] {
                IR::Tuple(items) | IR::List(items) => items.as_slice(),
                _ => &[],
            };
            let mut compiled: Vec<Value> = Vec::new();
            for m in methods_items {
                let m_items = match m {
                    IR::Tuple(items) | IR::List(items) => items,
                    _ => continue,
                };
                let method_name = ir_to_name(&m_items[0]).unwrap_or_default();
                let is_static = if m_items.len() > 2 {
                    matches!(&m_items[2], IR::Bool(true))
                } else {
                    false
                };

                let lambda_node = &m_items[1];
                if is_none_ir(lambda_node) {
                    let entry = Value::from_tuple(vec![
                        Value::from_string(method_name),
                        Value::NIL,
                        Value::from_bool(is_static),
                    ]);
                    compiled.push(entry);
                    continue;
                }

                // Compile method body
                let lambda_items = match lambda_node {
                    IR::Op { args, .. } => args.as_slice(),
                    IR::Tuple(items) | IR::List(items) => items.as_slice(),
                    _ => &[],
                };
                if lambda_items.len() >= 2 {
                    let lambda_params = &lambda_items[0];
                    let lambda_body = &lambda_items[1];
                    let (param_names, defaults, vararg_idx) = self.extract_params(lambda_params)?;

                    let mut code = self.compile_function_inner(FunctionCompileSpec {
                        params: param_names,
                        body: lambda_body,
                        name: &method_name,
                        defaults,
                        vararg_idx,
                        parent_nesting_depth: self.nesting_depth,
                    })?;
                    code.encoded_ir = Self::freeze_ir_body(lambda_body);
                    let func_idx = self.functions.len() as u32;
                    self.functions.push(code);
                    let entry = Value::from_tuple(vec![
                        Value::from_string(method_name),
                        Value::from_vmfunc(func_idx),
                        Value::from_bool(is_static),
                    ]);
                    compiled.push(entry);
                }
            }
            Some(Value::from_list(compiled))
        } else {
            None
        };

        // Build struct info constant as NativeTuple
        let has_implements = !implements_list.is_empty();
        let has_bases = !base_names.is_empty();

        let struct_info = if has_implements || has_bases {
            let impl_tuple = Value::from_tuple(implements_list.iter().map(|s| Value::from_string(s.clone())).collect());
            let bases_val = if has_bases {
                Value::from_tuple(base_names.iter().map(|s| Value::from_string(s.clone())).collect())
            } else {
                Value::NIL
            };
            let mut items = vec![
                Value::from_string(name),
                fields_tuple,
                Value::from_i64(num_defaults as i64),
                impl_tuple,
                bases_val,
            ];
            if let Some(methods) = methods_val {
                items.push(methods);
            }
            Value::from_tuple(items)
        } else {
            match methods_val {
                Some(methods) => Value::from_tuple(vec![
                    Value::from_string(name),
                    fields_tuple,
                    Value::from_i64(num_defaults as i64),
                    methods,
                ]),
                None => Value::from_tuple(vec![
                    Value::from_string(name),
                    fields_tuple,
                    Value::from_i64(num_defaults as i64),
                ]),
            }
        };

        let idx = self.core.add_const(struct_info);
        self.emit(VMOpCode::MakeStruct, idx as u32);
        Ok(())
    }

    fn compile_trait(&mut self, args: &[IR]) -> CompileResult<()> {
        let name = ir_to_name(&args[0]).unwrap_or_default();

        let extends_items = match &args[1] {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => &[],
        };
        let fields_items = match &args[2] {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => &[],
        };

        let extends_tuple = Value::from_tuple(
            extends_items
                .iter()
                .filter_map(ir_to_name)
                .map(Value::from_string)
                .collect(),
        );

        let mut fields_info: Vec<Value> = Vec::new();
        let mut num_defaults: u32 = 0;
        for f in fields_items {
            let f_items = match f {
                IR::Tuple(items) | IR::List(items) => items,
                _ => continue,
            };
            if f_items.len() >= 2 {
                let fname = ir_to_name(&f_items[0]).unwrap_or_default();
                let has_default = !is_none_ir(&f_items[1]);
                if has_default {
                    self.compile_node(&f_items[1])?;
                    num_defaults += 1;
                }
                let entry = Value::from_tuple(vec![Value::from_string(fname), Value::from_bool(has_default)]);
                fields_info.push(entry);
            }
        }
        let fields_tuple = Value::from_tuple(fields_info);

        let methods_val = if args.len() > 3 {
            let methods_items = match &args[3] {
                IR::Tuple(items) | IR::List(items) => items.as_slice(),
                _ => &[],
            };
            let mut compiled: Vec<Value> = Vec::new();
            for m in methods_items {
                let m_items = match m {
                    IR::Tuple(items) | IR::List(items) => items,
                    _ => continue,
                };
                let method_name = ir_to_name(&m_items[0]).unwrap_or_default();
                let is_static = if m_items.len() > 2 {
                    matches!(&m_items[2], IR::Bool(true))
                } else {
                    false
                };

                let lambda_node = &m_items[1];
                if is_none_ir(lambda_node) {
                    let entry = Value::from_tuple(vec![
                        Value::from_string(method_name),
                        Value::NIL,
                        Value::from_bool(is_static),
                    ]);
                    compiled.push(entry);
                    continue;
                }

                let lambda_items = match lambda_node {
                    IR::Op { args, .. } => args.as_slice(),
                    IR::Tuple(items) | IR::List(items) => items.as_slice(),
                    _ => &[],
                };
                if lambda_items.len() >= 2 {
                    let lambda_params = &lambda_items[0];
                    let lambda_body = &lambda_items[1];
                    let (param_names, defaults, vararg_idx) = self.extract_params(lambda_params)?;

                    let mut code = self.compile_function_inner(FunctionCompileSpec {
                        params: param_names,
                        body: lambda_body,
                        name: &method_name,
                        defaults,
                        vararg_idx,
                        parent_nesting_depth: self.nesting_depth,
                    })?;
                    code.encoded_ir = Self::freeze_ir_body(lambda_body);
                    let func_idx = self.functions.len() as u32;
                    self.functions.push(code);
                    let entry = Value::from_tuple(vec![
                        Value::from_string(method_name),
                        Value::from_vmfunc(func_idx),
                        Value::from_bool(is_static),
                    ]);
                    compiled.push(entry);
                }
            }
            Some(Value::from_list(compiled))
        } else {
            None
        };

        let trait_info = if let Some(methods) = methods_val {
            Value::from_tuple(vec![
                Value::from_string(name),
                extends_tuple,
                fields_tuple,
                Value::from_i64(num_defaults as i64),
                methods,
            ])
        } else {
            Value::from_tuple(vec![
                Value::from_string(name),
                extends_tuple,
                fields_tuple,
                Value::from_i64(num_defaults as i64),
            ])
        };

        let idx = self.core.add_const(trait_info);
        self.emit(VMOpCode::MakeTrait, idx as u32);
        Ok(())
    }

    // ========== Helpers ==========

    fn body_has_calls(&self, node: &IR) -> bool {
        match node {
            IR::Op { opcode, args, .. } => {
                if *opcode == IROpCode::Call || *opcode == IROpCode::FnDef || *opcode == IROpCode::OpLambda {
                    return true;
                }
                args.iter().any(|a| self.body_has_calls(a))
            }
            IR::Call { .. } => true,
            IR::List(items) | IR::Tuple(items) | IR::Program(items) => items.iter().any(|i| self.body_has_calls(i)),
            _ => false,
        }
    }

    fn extract_params(&self, params: &IR) -> CompileResult<(Vec<String>, Vec<Value>, i32)> {
        let mut param_names = Vec::new();
        let mut defaults = Vec::new();
        let mut vararg_idx: i32 = -1;

        let children = match params {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => return Ok((param_names, defaults, vararg_idx)),
        };

        for item in children {
            let item_parts = match item {
                IR::Tuple(items) | IR::List(items) => Some(items.as_slice()),
                _ => None,
            };
            if let Some(parts) = item_parts {
                if parts.len() == 2 {
                    let name = ir_to_name(&parts[0]).unwrap_or_default();
                    if name == "*" {
                        vararg_idx = param_names.len() as i32;
                        param_names.push(ir_to_name(&parts[1]).unwrap_or_default());
                    } else {
                        param_names.push(name);
                        let val = self.ir_to_value(&parts[1]);
                        defaults.push(val);
                    }
                } else if let Some(name) = ir_to_name(item) {
                    param_names.push(name);
                }
            } else if let Some(name) = ir_to_name(item) {
                param_names.push(name);
            }
        }
        Ok((param_names, defaults, vararg_idx))
    }

    fn ir_to_value(&self, ir: &IR) -> Value {
        match ir {
            IR::Int(n) => Value::from_i64(*n),
            IR::Float(f) => Value::from_float(*f),
            IR::Bool(b) => Value::from_bool(*b),
            IR::None => Value::NIL,
            IR::String(s) => Value::from_string(s.clone()),
            _ => Value::NIL,
        }
    }

    fn try_compile_pattern_ir(&mut self, pattern: &IR) -> CompileResult<Option<VMPattern>> {
        match pattern {
            IR::PatternWildcard => Ok(Some(VMPattern::Wildcard)),
            IR::PatternVar(name) => {
                if name == "_" {
                    Ok(Some(VMPattern::Wildcard))
                } else {
                    let slot = self.add_local(name);
                    Ok(Some(VMPattern::Var(slot)))
                }
            }
            IR::PatternLiteral(value) => {
                let val = self.ir_to_value(value);
                Ok(Some(VMPattern::Literal(val)))
            }
            IR::PatternOr(patterns) => {
                let mut sub_patterns = Vec::new();
                for p in patterns {
                    match self.try_compile_pattern_ir(p)? {
                        Some(vp) => sub_patterns.push(vp),
                        None => return Ok(None),
                    }
                }
                Ok(Some(VMPattern::Or(sub_patterns)))
            }
            IR::PatternTuple(patterns) => {
                let mut elements = Vec::new();
                for p in patterns {
                    if let IR::Tuple(items) = p {
                        if items.len() == 2 {
                            if let (IR::String(star), IR::String(name)) = (&items[0], &items[1]) {
                                if star == "*" {
                                    let slot = if name.is_empty() || name == "_" {
                                        usize::MAX
                                    } else {
                                        self.add_local(name)
                                    };
                                    elements.push(VMPatternElement::Star(slot));
                                    continue;
                                }
                            }
                        }
                    }
                    match self.try_compile_pattern_ir(p)? {
                        Some(vp) => elements.push(VMPatternElement::Pattern(vp)),
                        None => return Ok(None),
                    }
                }
                Ok(Some(VMPattern::Tuple(elements)))
            }
            IR::PatternStruct { name, fields } => {
                let mut field_slots = Vec::new();
                for field_name in fields {
                    let slot = self.add_local(field_name);
                    field_slots.push((field_name.clone(), slot));
                }
                Ok(Some(VMPattern::Struct {
                    name: name.clone(),
                    field_slots,
                }))
            }
            _ => Ok(None),
        }
    }

    fn collect_pattern_vars_ir(&mut self, pattern: &IR) {
        match pattern {
            IR::PatternVar(name) => {
                if name != "_" && !self.locals.contains(name) {
                    self.add_local(name);
                }
            }
            IR::PatternStruct { fields, .. } => {
                for field in fields {
                    if field != "_" && !self.locals.contains(field) {
                        self.add_local(field);
                    }
                }
            }
            IR::PatternOr(pats) | IR::PatternTuple(pats) => {
                for p in pats {
                    self.collect_pattern_vars_ir(p);
                }
            }
            _ => {}
        }
    }

    fn try_compile_assign_pattern_ir(&mut self, pattern: &IR) -> CompileResult<Option<VMPattern>> {
        match pattern {
            IR::Tuple(items) => {
                let mut elements = Vec::new();
                for item in items {
                    if let IR::Tuple(pair) = item {
                        if pair.len() == 2 {
                            if let IR::String(s) = &pair[0] {
                                if s == "*" {
                                    let star_name = ir_to_name(&pair[1]).unwrap_or_default();
                                    let star_slot = self.add_local(&star_name);
                                    elements.push(VMPatternElement::Star(star_slot));
                                    continue;
                                }
                            }
                        }
                    }
                    match self.try_compile_assign_pattern_ir(item)? {
                        Some(sub) => elements.push(VMPatternElement::Pattern(sub)),
                        None => return Ok(None),
                    }
                }
                Ok(Some(VMPattern::Tuple(elements)))
            }
            IR::Ref(name, _, _) | IR::Identifier(name) | IR::String(name) => {
                let slot = self.add_local(name);
                Ok(Some(VMPattern::Var(slot)))
            }
            _ => Ok(None),
        }
    }

    fn compile_unpack_pattern_ir(&mut self, ir: &IR, keep_last: bool) -> CompileResult<()> {
        if let IR::Tuple(items) = ir {
            let mut star_idx: Option<usize> = None;
            for (i, item) in items.iter().enumerate() {
                if let IR::Tuple(pair) = item {
                    if pair.len() == 2 {
                        if let IR::String(s) = &pair[0] {
                            if s == "*" {
                                star_idx = Some(i);
                                break;
                            }
                        }
                    }
                }
            }

            if let Some(si) = star_idx {
                let before = si as u32;
                let after = (items.len() - si - 1) as u32;
                let arg = (before << 8) | after;
                self.emit(VMOpCode::UnpackEx, arg);
            } else {
                self.emit(VMOpCode::UnpackSequence, items.len() as u32);
            }

            for (idx, item) in items.iter().enumerate() {
                let is_last = idx == items.len() - 1;
                if is_last && keep_last {
                    self.emit(VMOpCode::DupTop, 0);
                }
                if let IR::Tuple(pair) = item {
                    if pair.len() == 2 {
                        if let IR::String(s) = &pair[0] {
                            if s == "*" {
                                let star_name = ir_to_name(&pair[1]).unwrap_or_default();
                                let slot = self.add_local(&star_name);
                                self.emit(VMOpCode::StoreLocal, slot as u32);
                                continue;
                            }
                        }
                    }
                }
                if let IR::Tuple(_) = item {
                    self.compile_unpack_pattern_ir(item, false)?;
                    continue;
                }
                if let Some(name) = ir_to_name(item) {
                    let slot = self.add_local(&name);
                    self.emit(VMOpCode::StoreLocal, slot as u32);
                }
            }
        }
        Ok(())
    }

    /// Freeze the IR body of a lambda/function for ND process workers.
    pub fn freeze_ir_body(body: &IR) -> Option<Arc<Vec<u8>>> {
        let ir_vec = vec![body.clone()];
        catnip_core::freeze::encode(&ir_vec).ok().map(Arc::new)
    }
}

impl Default for PureCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catnip_core::ir::pure::IR;

    #[test]
    fn test_compile_int_literal() {
        let mut compiler = PureCompiler::new();
        let ir = IR::Program(vec![IR::Int(42)]);
        let output = compiler.compile(&ir).unwrap();
        assert!(!output.code.instructions.is_empty());
        assert_eq!(output.code.constants.len(), 1);
    }

    #[test]
    fn test_compile_string_literal() {
        let mut compiler = PureCompiler::new();
        let ir = IR::Program(vec![IR::String("hello".to_string())]);
        let output = compiler.compile(&ir).unwrap();
        assert_eq!(output.code.constants.len(), 1);
        assert!(output.code.constants[0].is_native_str());
    }

    #[test]
    fn test_compile_binary_add() {
        let mut compiler = PureCompiler::new();
        let ir = IR::Program(vec![IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)])]);
        let output = compiler.compile(&ir).unwrap();
        // Should have: LoadConst(1), LoadConst(2), Add, Halt
        assert!(output.code.instructions.len() >= 3);
    }

    #[test]
    fn test_compile_decimal_unsupported() {
        let mut compiler = PureCompiler::new();
        let ir = IR::Program(vec![IR::Decimal("1.5".to_string())]);
        let result = compiler.compile(&ir);
        assert!(result.is_err());
        if let Err(CompileError::UnsupportedLiteral(_)) = result {
        } else {
            panic!("expected UnsupportedLiteral");
        }
    }

    #[test]
    fn test_compile_lambda() {
        let mut compiler = PureCompiler::new();
        let body = IR::op(IROpCode::Mul, vec![IR::Identifier("n".into()), IR::Int(2)]);
        let params = IR::List(vec![IR::Identifier("n".into())]);
        let lambda_ir = IR::op(IROpCode::OpLambda, vec![params, body]);
        let ir = IR::Program(vec![lambda_ir]);
        let output = compiler.compile(&ir).unwrap();
        assert_eq!(output.functions.len(), 1);
        assert_eq!(output.functions[0].name, "<lambda>");
    }

    #[test]
    fn test_compile_if() {
        let mut compiler = PureCompiler::new();
        let ir = IR::op(
            IROpCode::OpIf,
            vec![IR::Tuple(vec![IR::Tuple(vec![IR::Bool(true), IR::Int(1)])]), IR::Int(2)],
        );
        let program = IR::Program(vec![ir]);
        let output = compiler.compile(&program).unwrap();
        assert!(!output.code.instructions.is_empty());
    }

    /// Regression: for + if + SetItem caused a stack imbalance.
    /// compile_body must always leave exactly 1 value for compile_if symmetry.
    /// Without the fix, SetItem (void op) left nothing, and the subsequent
    /// PopTop in compile_body_void consumed the for-loop iterator.
    #[test]
    fn test_compile_for_if_setitem_stack_balance() {
        let mut compiler = PureCompiler::new();
        // for x in data { if (x != "b") { data[0] = x } }
        let setitem = IR::op(
            IROpCode::SetItem,
            vec![IR::Identifier("data".into()), IR::Int(0), IR::Identifier("x".into())],
        );
        let if_body = IR::op(IROpCode::OpBlock, vec![setitem]);
        let condition = IR::op(IROpCode::Ne, vec![IR::Identifier("x".into()), IR::String("b".into())]);
        let if_node = IR::op(
            IROpCode::OpIf,
            vec![IR::Tuple(vec![IR::Tuple(vec![condition, if_body])])],
        );
        let for_body = IR::op(IROpCode::OpBlock, vec![if_node]);
        let for_node = IR::op(
            IROpCode::OpFor,
            vec![IR::Identifier("x".into()), IR::Identifier("data".into()), for_body],
        );
        let program = IR::Program(vec![for_node]);
        let output = compiler.compile(&program).unwrap();
        // Verify compilation succeeds and produces instructions.
        // The real test is that this doesn't cause a segfault at runtime
        // (ForIter finding NoneType instead of iterator on the stack).
        assert!(!output.code.instructions.is_empty());

        // Verify stack balance: count pushes vs pops in the for-loop body
        // (between ForIter and Jump back). The body should be net-zero
        // on each iteration so the iterator stays on top.
        let instrs = &output.code.instructions;
        let for_iter_pos = instrs.iter().position(|i| i.op == VMOpCode::ForIter).unwrap();
        let jump_back_pos = instrs.iter().rposition(|i| i.op == VMOpCode::Jump).unwrap();

        let mut stack_delta: i32 = 0;
        for instr in &instrs[for_iter_pos + 1..jump_back_pos] {
            match instr.op {
                VMOpCode::LoadConst | VMOpCode::LoadLocal | VMOpCode::LoadGlobal => stack_delta += 1,
                VMOpCode::StoreLocal | VMOpCode::PopTop => stack_delta -= 1,
                VMOpCode::SetItem => stack_delta -= 3,
                VMOpCode::JumpIfFalse => stack_delta -= 1,
                _ => {}
            }
        }
        // ForIter pushes the next value (+1), StoreLocal pops it (-1),
        // then body should be net-zero. Overall delta = 0.
        assert_eq!(
            stack_delta, 0,
            "for-loop body has stack imbalance: delta = {stack_delta}"
        );
    }

    #[test]
    fn test_compile_set_locals() {
        let mut compiler = PureCompiler::new();
        let ir = IR::op(
            IROpCode::SetLocals,
            vec![IR::Tuple(vec![IR::Identifier("x".into())]), IR::Int(42)],
        );
        let program = IR::Program(vec![ir]);
        let output = compiler.compile(&program).unwrap();
        assert!(!output.code.instructions.is_empty());
    }
}
