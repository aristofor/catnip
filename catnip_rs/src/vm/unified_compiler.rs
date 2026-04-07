// FILE: catnip_rs/src/vm/unified_compiler.rs
//! Unified bytecode compiler: converts both Op (PyObject) and IR inputs to CodeObject.
//!
//! Replaces the duplicated logic in `compiler.rs` (Op path) and `pure_compiler.rs` (IR path)
//! by using `CompilerNode` abstraction throughout. All `compile_*` methods are written once.

use super::compiler_core::{CompilerCore, LoopContext};
use super::compiler_input::{CompilerKwargs, CompilerNode, ir_to_name};
use super::frame::{CodeObject, PyCodeObject};
use super::opcode::VMOpCode;
use super::pattern::{VMPattern, VMPatternElement};
use super::value::Value;
use crate::core::Op;
use crate::core::pattern::*;
use crate::ir::pure::BroadcastType;
use crate::ir::{IR, IROpCode};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// Freeze the IR body of a lambda/function for ND process workers.
/// Only works for Pure IR nodes; returns None for PyObj nodes.
/// Uses raw bincode (no .catf header) -- for IPC transport, not disk persistence.
fn freeze_ir_body(body: &CompilerNode<'_>) -> Option<Arc<Vec<u8>>> {
    match body {
        CompilerNode::Pure(ir) => {
            let ir_vec = vec![(*ir).clone()];
            catnip_core::freeze::encode(&ir_vec).ok().map(Arc::new)
        }
        CompilerNode::PyObj(_) => None,
    }
}

// ========== 1. Struct definition, Deref/DerefMut, new() ==========

/// Unified bytecode compiler for both Op (PyObject) and IR input.
///
/// Delegates state and helpers to `CompilerCore` via Deref.
pub struct UnifiedCompiler {
    core: CompilerCore,
    /// Stack of active finally bodies as cloned Pure IR (for inlining on break/continue/return).
    finally_stack: Vec<UCFinallyInfo>,
}

struct UCFinallyInfo {
    body: UCFinallyBody,
    has_except: bool,
    needs_clear_exception: bool,
}

enum UCFinallyBody {
    Pure(catnip_core::ir::pure::IR),
    PyObj(pyo3::Py<pyo3::PyAny>),
}

impl Clone for UCFinallyInfo {
    fn clone(&self) -> Self {
        Self {
            body: self.body.clone(),
            has_except: self.has_except,
            needs_clear_exception: self.needs_clear_exception,
        }
    }
}

impl Clone for UCFinallyBody {
    fn clone(&self) -> Self {
        match self {
            UCFinallyBody::Pure(ir) => UCFinallyBody::Pure(ir.clone()),
            UCFinallyBody::PyObj(obj) => {
                // Safe: clone only happens during compilation which holds the GIL
                let py = unsafe { pyo3::Python::assume_attached() };
                UCFinallyBody::PyObj(obj.clone_ref(py))
            }
        }
    }
}

struct FunctionCompileSpec<'a, 'py> {
    params: Vec<String>,
    body: &'a CompilerNode<'py>,
    name: &'a str,
    defaults: Vec<Value>,
    vararg_idx: i32,
    parent_nesting_depth: u32,
}

pub struct FunctionCompileMeta<'a> {
    pub params: Vec<String>,
    pub name: &'a str,
    pub defaults: Vec<Value>,
    pub vararg_idx: i32,
    pub parent_nesting_depth: u32,
}

impl Deref for UnifiedCompiler {
    type Target = CompilerCore;
    fn deref(&self) -> &CompilerCore {
        &self.core
    }
}

impl DerefMut for UnifiedCompiler {
    fn deref_mut(&mut self) -> &mut CompilerCore {
        &mut self.core
    }
}

impl UnifiedCompiler {
    pub fn new() -> Self {
        Self {
            core: CompilerCore::new(),
            finally_stack: Vec::new(),
        }
    }

    // ========== 2. Public entry points ==========

    /// Compile a Python Op node to a CodeObject.
    pub fn compile_py(&mut self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<CodeObject> {
        let cn = CompilerNode::PyObj(node.clone());
        self.compile(py, &cn)
    }

    /// Compile an IR node to a CodeObject.
    ///
    /// Delegates to `catnip_vm::compiler::PureCompiler` when possible.
    /// Falls back to the inline UnifiedCompiler path for IR containing
    /// Python-dependent literals (Decimal, Imaginary).
    pub fn compile_pure(&mut self, py: Python<'_>, node: &IR) -> PyResult<CodeObject> {
        let mut pure_compiler = catnip_vm::compiler::PureCompiler::new();
        match pure_compiler.compile(node) {
            Ok(output) => Ok(crate::vm::py_interop::convert_pure_compile_output(py, &output)?),
            Err(catnip_vm::compiler::CompileError::UnsupportedLiteral(_)) => {
                // Fallback: IR contains Decimal/Imaginary that need Python
                let cn = CompilerNode::Pure(node);
                self.compile(py, &cn)
            }
            Err(catnip_vm::compiler::CompileError::NotImplemented(msg)) => {
                // Opcodes not yet lowered to bytecode
                Err(pyo3::exceptions::PyNotImplementedError::new_err(msg))
            }
            Err(e) => Err(pyo3::exceptions::PySyntaxError::new_err(e.to_string())),
        }
    }

    /// Compile a function body from Python Op input.
    pub fn compile_function_py(
        &mut self,
        py: Python<'_>,
        body: &Bound<'_, PyAny>,
        meta: FunctionCompileMeta<'_>,
    ) -> PyResult<CodeObject> {
        let cn = CompilerNode::PyObj(body.clone());
        self.compile_function(
            py,
            FunctionCompileSpec {
                params: meta.params,
                body: &cn,
                name: meta.name,
                defaults: meta.defaults,
                vararg_idx: meta.vararg_idx,
                parent_nesting_depth: meta.parent_nesting_depth,
            },
        )
    }

    /// Compile a function body from IR input.
    ///
    /// Uses the PureCompiler path via compile_pure internally.
    /// This entry point exists for direct function compilation requests.
    pub fn compile_function_pure(
        &mut self,
        py: Python<'_>,
        body: &IR,
        meta: FunctionCompileMeta<'_>,
    ) -> PyResult<CodeObject> {
        let cn = CompilerNode::Pure(body);
        self.compile_function(
            py,
            FunctionCompileSpec {
                params: meta.params,
                body: &cn,
                name: meta.name,
                defaults: meta.defaults,
                vararg_idx: meta.vararg_idx,
                parent_nesting_depth: meta.parent_nesting_depth,
            },
        )
    }

    // ========== 3. Internal compile / compile_function ==========

    fn compile<'py>(&mut self, py: Python<'py>, node: &CompilerNode<'py>) -> PyResult<CodeObject> {
        self.reset();
        self.compile_node(py, node)?;
        self.emit(VMOpCode::Halt, 0);
        self.build_code_object(py)
    }

    fn compile_function<'py>(&mut self, py: Python<'py>, spec: FunctionCompileSpec<'_, 'py>) -> PyResult<CodeObject> {
        self.reset();
        self.name = spec.name.to_string();
        self.nargs = spec.params.len();
        self.defaults = spec.defaults;
        self.in_function = true;
        self.nesting_depth = spec.parent_nesting_depth + 1;

        for param in spec.params {
            self.add_local(&param);
        }

        self.compile_node(py, spec.body)?;
        self.emit(VMOpCode::Return, 0);

        let mut code = self.build_code_object(py)?;
        code.vararg_idx = spec.vararg_idx;
        Ok(code)
    }

    // ========== 4. compile_node (dispatches to compile_node_pure, compile_node_py) ==========

    fn compile_node<'py>(&mut self, py: Python<'py>, node: &CompilerNode<'py>) -> PyResult<()> {
        match node {
            CompilerNode::Pure(ir) => self.compile_node_pure(py, ir),
            CompilerNode::PyObj(obj) => self.compile_node_py(py, obj),
        }
    }

    // ========== 5. compile_node_pure, compile_node_py ==========

    fn compile_node_pure<'py>(&mut self, py: Python<'py>, node: &'py IR) -> PyResult<()> {
        match node {
            // Literals - native Value
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

            // Literals that need Python
            IR::String(s) => {
                let py_str = s.as_str().into_pyobject(py)?.into_any();
                let idx = self.add_const_pyobj(py, &py_str);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::Bytes(v) => {
                let py_bytes = pyo3::types::PyBytes::new(py, v);
                let idx = self.add_const_pyobj(py, &py_bytes.into_any());
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::Decimal(s) => {
                let decimal_mod = py.import("decimal")?;
                let decimal_cls = decimal_mod.getattr("Decimal")?;
                let obj = decimal_cls.call1((s.as_str(),))?;
                let idx = self.add_const_pyobj(py, &obj);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::Imaginary(s) => {
                let imag: f64 = s.parse().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("invalid imaginary: {}", e))
                })?;
                let idx = self.core.add_const(Value::from_complex(0.0, imag));
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }

            // Variables
            IR::Ref(name, start_byte, _end_byte) => {
                if *start_byte >= 0 {
                    self.current_start_byte = *start_byte as u32;
                }
                self.core.compile_name_load(name)
            }
            IR::Identifier(name) => self.core.compile_name_load(name),

            // Sequences
            IR::Program(items) => self.compile_statement_list_pure(py, items),
            IR::List(items) => {
                for item in items {
                    self.compile_node_pure(py, item)?;
                }
                self.emit(VMOpCode::BuildList, items.len() as u32);
                Ok(())
            }
            IR::Tuple(items) => {
                for item in items {
                    self.compile_node_pure(py, item)?;
                }
                self.emit(VMOpCode::BuildTuple, items.len() as u32);
                Ok(())
            }
            IR::Set(items) => {
                for item in items {
                    self.compile_node_pure(py, item)?;
                }
                self.emit(VMOpCode::BuildSet, items.len() as u32);
                Ok(())
            }
            IR::Dict(pairs) => {
                for (key, value) in pairs {
                    self.compile_node_pure(py, key)?;
                    self.compile_node_pure(py, value)?;
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
                    self.current_start_byte = *start_byte as u32;
                }
                let func_cn = CompilerNode::Pure(func);
                let args_cn: Vec<CompilerNode<'py>> = args.iter().map(CompilerNode::Pure).collect();
                let kwargs_cn = CompilerKwargs::Pure(kwargs);
                if *tail {
                    // Tail call: use compile_call which emits TailCall opcode
                    let mut all_args = vec![func_cn];
                    all_args.extend(args_cn);
                    self.compile_call(py, &all_args, &kwargs_cn, true)
                } else {
                    self.compile_call_dispatch(py, &func_cn, &args_cn, &kwargs_cn)
                }
            }

            // Operations
            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte: _,
            } => {
                if *start_byte > 0 {
                    self.current_start_byte = *start_byte as u32;
                }
                let args_cn: Vec<CompilerNode<'py>> = args.iter().map(CompilerNode::Pure).collect();
                let kwargs_cn = CompilerKwargs::Pure(kwargs);
                self.compile_op_dispatch(py, *opcode, &args_cn, &kwargs_cn, *tail)
            }

            // Broadcasting
            IR::Broadcast {
                target,
                operator,
                operand,
                broadcast_type,
            } => {
                // Compile target
                if let Some(t) = target.as_deref() {
                    self.compile_node_pure(py, t)?;
                }
                // Check for ND operations
                let nd_flag = match operator.as_ref() {
                    IR::Op { opcode, .. } if *opcode == IROpCode::NdRecursion => Some(4u32),
                    IR::Op { opcode, .. } if *opcode == IROpCode::NdMap => Some(8u32),
                    _ => None,
                };
                if let Some(nd_flag) = nd_flag {
                    if let IR::Op { args, .. } = operator.as_ref() {
                        if !args.is_empty() {
                            self.compile_node_pure(py, &args[0])?;
                        }
                    }
                    let is_filter = matches!(broadcast_type, BroadcastType::If);
                    let mut flags = nd_flag;
                    if is_filter {
                        flags |= 1;
                    }
                    self.emit(VMOpCode::Broadcast, flags);
                } else {
                    self.compile_node_pure(py, operator)?;
                    let has_operand = operand.is_some();
                    if let Some(o) = operand.as_deref() {
                        self.compile_node_pure(py, o)?;
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
                self.compile_node_pure(py, start)?;
                self.compile_node_pure(py, stop)?;
                self.compile_node_pure(py, step)?;
                self.emit(VMOpCode::BuildSlice, 3);
                Ok(())
            }

            // Patterns only appear inside match cases
            IR::PatternLiteral(_)
            | IR::PatternVar(_)
            | IR::PatternWildcard
            | IR::PatternOr(_)
            | IR::PatternTuple(_)
            | IR::PatternStruct { .. }
            | IR::PatternEnum { .. } => {
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
        }
    }

    fn compile_node_py<'py>(&mut self, py: Python<'py>, node: &Bound<'py, PyAny>) -> PyResult<()> {
        // Handle list of statements
        if let Ok(list) = node.cast::<PyList>() {
            return self.compile_statement_list_py(py, list);
        }

        // Handle Broadcast nodes (before Op check)
        let type_name = node.get_type().name()?;
        if type_name == "Broadcast" {
            return self.compile_broadcast_py(py, node);
        }

        // Handle Call nodes (from pure_transforms)
        if type_name == "Call" {
            return self.compile_call_node_py(py, node);
        }

        // Handle Op nodes
        if let Ok(op) = node.extract::<PyRef<Op>>() {
            return self.compile_op_py(py, node, &op);
        }

        // Check for Ref (variable reference)
        if type_name == "Ref" {
            let sb: isize = node.getattr("start_byte")?.extract()?;
            if sb >= 0 {
                self.current_start_byte = sb as u32;
            }
            let ident: String = node.getattr("ident")?.extract()?;
            return self.core.compile_name_load(&ident);
        }

        // Literal value
        let idx = self.add_const_pyobj(py, node);
        self.emit(VMOpCode::LoadConst, idx as u32);
        Ok(())
    }

    fn compile_statement_list_pure<'py>(&mut self, py: Python<'py>, stmts: &'py [IR]) -> PyResult<()> {
        if stmts.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }
        let len = stmts.len();
        for (i, stmt) in stmts.iter().enumerate() {
            self.compile_node_pure(py, stmt)?;
            if i < len - 1 {
                self.emit(VMOpCode::PopTop, 0);
            }
        }
        Ok(())
    }

    fn compile_statement_list_py<'py>(&mut self, py: Python<'py>, stmts: &Bound<'py, PyList>) -> PyResult<()> {
        if stmts.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }
        let len = stmts.len();
        for (i, stmt) in stmts.iter().enumerate() {
            self.compile_node_py(py, &stmt)?;
            if i < len - 1 {
                self.emit(VMOpCode::PopTop, 0);
            }
        }
        Ok(())
    }

    /// Compile a Python Op node by extracting its opcode, args, kwargs and dispatching.
    fn compile_op_py<'py>(&mut self, py: Python<'py>, _node: &Bound<'py, PyAny>, op: &Op) -> PyResult<()> {
        if op.start_byte >= 0 {
            self.current_start_byte = op.start_byte as u32;
        }
        let args_bound = op.args.bind(py);
        let kwargs_bound: &Bound<'py, PyDict> = op.kwargs.bind(py).cast()?;
        let ident = IROpCode::from_u8(op.ident as u8)
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err(format!("Invalid IR opcode: {}", op.ident)))?;

        let args_len = args_bound.len()?;
        let args_cn: Vec<CompilerNode<'py>> = (0..args_len)
            .map(|i| Ok(CompilerNode::PyObj(args_bound.get_item(i)?)))
            .collect::<PyResult<Vec<_>>>()?;
        let kwargs_cn = CompilerKwargs::Py(kwargs_bound.clone());

        self.compile_op_dispatch(py, ident, &args_cn, &kwargs_cn, op.tail)
    }

    /// Handle a Python Call node (from pure_transforms).
    fn compile_call_node_py<'py>(&mut self, py: Python<'py>, node: &Bound<'py, PyAny>) -> PyResult<()> {
        if let Ok(sb) = node.getattr("start_byte") {
            if let Ok(sb_val) = sb.extract::<isize>() {
                if sb_val >= 0 {
                    self.current_start_byte = sb_val as u32;
                }
            }
        }
        let func = node.getattr("func")?;
        let args = node.getattr("args")?;
        let kwargs = node.getattr("kwargs")?;
        let kwargs_dict = kwargs.cast::<PyDict>()?;
        let args_len = args.len()?;

        let func_cn = CompilerNode::PyObj(func);
        let args_cn: Vec<CompilerNode<'py>> = (0..args_len)
            .map(|i| Ok(CompilerNode::PyObj(args.get_item(i)?)))
            .collect::<PyResult<Vec<_>>>()?;
        let kwargs_cn = CompilerKwargs::Py(kwargs_dict.clone());

        self.compile_call_dispatch(py, &func_cn, &args_cn, &kwargs_cn)
    }

    /// Handle a Python Broadcast node.
    fn compile_broadcast_py<'py>(&mut self, py: Python<'py>, node: &Bound<'py, PyAny>) -> PyResult<()> {
        let target = node.getattr("target")?;
        self.compile_node_py(py, &target)?;

        let operator = node.getattr("operator")?;
        let op_type = operator.get_type();
        let is_nd_op = if op_type.name()? == "Op" {
            let op_ident: i32 = operator.getattr("ident")?.extract()?;
            if op_ident == IROpCode::NdRecursion as i32 {
                Some(4u32)
            } else if op_ident == IROpCode::NdMap as i32 {
                Some(8u32)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(nd_flag) = is_nd_op {
            let op_args = operator.getattr("args")?;
            let op_args_tuple = op_args.cast::<PyTuple>()?;
            let lambda_node = op_args_tuple.get_item(0)?;
            self.compile_node_py(py, &lambda_node)?;

            let is_filter: bool = node.getattr("is_filter")?.extract()?;
            let mut flags: u32 = nd_flag;
            if is_filter {
                flags |= 1;
            }
            self.emit(VMOpCode::Broadcast, flags);
        } else {
            self.compile_node_py(py, &operator)?;

            let operand = node.getattr("operand")?;
            let has_operand = !operand.is_none();
            if has_operand {
                self.compile_node_py(py, &operand)?;
            }

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

    // ========== 6. compile_op_dispatch ==========

    fn compile_op_dispatch<'py>(
        &mut self,
        py: Python<'py>,
        opcode: IROpCode,
        args: &[CompilerNode<'py>],
        kwargs: &CompilerKwargs<'py>,
        tail: bool,
    ) -> PyResult<()> {
        match opcode {
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

            // Membership
            IROpCode::In => self.compile_binary(py, VMOpCode::In, args),
            IROpCode::NotIn => self.compile_binary(py, VMOpCode::NotIn, args),

            // Identity
            IROpCode::Is => self.compile_binary(py, VMOpCode::Is, args),
            IROpCode::IsNot => self.compile_binary(py, VMOpCode::IsNot, args),

            // Logical
            IROpCode::Not => self.compile_unary(py, VMOpCode::Not, args),
            IROpCode::And => self.compile_and(py, args),
            IROpCode::Or => self.compile_or(py, args),
            IROpCode::NullCoalesce => self.compile_null_coalesce(py, args),

            // Bitwise
            IROpCode::BAnd => self.compile_binary(py, VMOpCode::BAnd, args),
            IROpCode::BOr => self.compile_binary(py, VMOpCode::BOr, args),
            IROpCode::BXor => self.compile_binary(py, VMOpCode::BXor, args),
            IROpCode::BNot => self.compile_unary(py, VMOpCode::BNot, args),
            IROpCode::LShift => self.compile_binary(py, VMOpCode::LShift, args),
            IROpCode::RShift => self.compile_binary(py, VMOpCode::RShift, args),

            // Variables
            IROpCode::SetLocals => self.compile_set_locals(py, args, kwargs),
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
            IROpCode::OpBreak => self.compile_break_with_finally(py),
            IROpCode::OpContinue => self.compile_continue_with_finally(py),

            // Functions
            IROpCode::Call => self.compile_call(py, args, kwargs, tail),
            IROpCode::OpLambda => self.compile_lambda(py, args),
            IROpCode::FnDef => self.compile_fn_def(py, args, kwargs),

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
                if !args.is_empty() {
                    self.compile_node(py, &args[0])
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
                // Pragma is a directive, not an expression, but must leave a value
                // on the stack when used as a statement in a Program sequence
                // (compile_statement_list_pure emits PopTop between statements).
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }

            IROpCode::Breakpoint => {
                self.emit(VMOpCode::Breakpoint, 0);
                // Must leave a value on the stack (same reason as Pragma)
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }

            IROpCode::TypeOf => {
                if !args.is_empty() {
                    self.compile_node(py, &args[0])?;
                }
                self.emit(VMOpCode::TypeOf, 0);
                Ok(())
            }

            IROpCode::ExcInfo => {
                // Push (exc_type, exc_value, None) from active exception
                self.emit(VMOpCode::LoadException, 1);
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

            IROpCode::OpStruct => self.compile_struct(py, args),
            IROpCode::TraitDef => self.compile_trait(py, args),
            IROpCode::EnumDef => self.compile_enum(py, args),

            IROpCode::OpTry => self.compile_try(py, args),
            IROpCode::OpRaise => self.compile_raise(py, args),

            _ => Err(pyo3::exceptions::PyNotImplementedError::new_err(format!(
                "UnifiedCompiler: cannot compile IR opcode: {}",
                opcode
            ))),
        }
    }

    // ========== 7. Binary/Unary operations ==========

    fn compile_binary<'py>(&mut self, py: Python<'py>, vm_op: VMOpCode, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let (left, right) = if args.len() == 1 {
            let inner = &args[0];
            if inner.is_list_or_tuple() {
                (inner.child(py, 0)?, inner.child(py, 1)?)
            } else {
                return Err(pyo3::exceptions::PyValueError::new_err("Invalid binary args"));
            }
        } else if args.len() >= 2 {
            (args[0].clone(), args[1].clone())
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err("Binary op requires 2 args"));
        };
        self.compile_node(py, &left)?;
        self.compile_node(py, &right)?;
        self.emit(vm_op, 0);
        Ok(())
    }

    fn compile_unary<'py>(&mut self, py: Python<'py>, vm_op: VMOpCode, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if args.is_empty() {
            return Err(pyo3::exceptions::PyValueError::new_err("Unary op requires 1 arg"));
        }
        self.compile_node(py, &args[0])?;
        self.emit(vm_op, 0);
        Ok(())
    }

    // ========== 8. Short-circuit logic ==========

    fn compile_and<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let (left, right) = if args.len() == 1 && args[0].is_list_or_tuple() {
            (args[0].child(py, 0)?, args[0].child(py, 1)?)
        } else if args.len() >= 2 {
            (args[0].clone(), args[1].clone())
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err("And requires 2 operands"));
        };
        self.compile_node(py, &left)?;
        self.emit(VMOpCode::ToBool, 0);
        let jump_idx = self.emit(VMOpCode::JumpIfFalseOrPop, 0);
        self.compile_node(py, &right)?;
        self.emit(VMOpCode::ToBool, 0);
        let pos = self.instructions.len() as u32;
        self.patch(jump_idx, pos);
        Ok(())
    }

    fn compile_or<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let (left, right) = if args.len() == 1 && args[0].is_list_or_tuple() {
            (args[0].child(py, 0)?, args[0].child(py, 1)?)
        } else if args.len() >= 2 {
            (args[0].clone(), args[1].clone())
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err("Or requires 2 operands"));
        };
        self.compile_node(py, &left)?;
        self.emit(VMOpCode::ToBool, 0);
        let jump_idx = self.emit(VMOpCode::JumpIfTrueOrPop, 0);
        self.compile_node(py, &right)?;
        self.emit(VMOpCode::ToBool, 0);
        let pos = self.instructions.len() as u32;
        self.patch(jump_idx, pos);
        Ok(())
    }

    fn compile_null_coalesce<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let (left, right) = if args.len() == 1 && args[0].is_list_or_tuple() {
            (args[0].child(py, 0)?, args[0].child(py, 1)?)
        } else if args.len() >= 2 {
            (args[0].clone(), args[1].clone())
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "NullCoalesce requires 2 operands",
            ));
        };
        self.compile_node(py, &left)?;
        let jump_idx = self.emit(VMOpCode::JumpIfNotNoneOrPop, 0);
        self.compile_node(py, &right)?;
        let pos = self.instructions.len() as u32;
        self.patch(jump_idx, pos);
        Ok(())
    }

    // ========== 9. Variables (set_locals, getattr, etc.) ==========

    fn compile_set_locals<'py>(
        &mut self,
        py: Python<'py>,
        args: &[CompilerNode<'py>],
        kwargs: &CompilerKwargs<'py>,
    ) -> PyResult<()> {
        // Check if last arg is a boolean explicit_unpack flag
        let mut effective_args: Vec<CompilerNode<'py>> = args.to_vec();
        let mut explicit_unpack = false;
        if effective_args.len() >= 3 {
            if let Some(last) = effective_args.last() {
                if last.is_bool() {
                    explicit_unpack = last.as_bool().unwrap_or(false);
                    effective_args.pop();
                }
            }
        }

        // Detect format: kwargs['names'] or args[0] is tuple of names
        let names_pattern: Option<CompilerNode<'py>>;
        let values: Vec<CompilerNode<'py>>;

        if let Some(names_obj) = kwargs.get(py, "names")? {
            names_pattern = Some(names_obj);
            values = effective_args;
        } else if effective_args.len() >= 2 {
            if effective_args[0].is_tuple() {
                names_pattern = Some(effective_args[0].clone());
                values = effective_args.into_iter().skip(1).collect();
            } else {
                names_pattern = None;
                values = Vec::new();
            }
        } else {
            names_pattern = None;
            values = Vec::new();
        }

        // Capture void_context, then disable for sub-expressions
        let is_void = self.void_context;
        self.void_context = false;

        // Check for complex patterns (star, nested) -> VM pattern matching path
        if let Some(ref pattern) = names_pattern {
            if pattern.has_complex_pattern(py) && values.len() == 1 {
                let unwrapped = pattern.unwrap_single_tuple(py)?;

                let vm_pattern = self.try_compile_assign_pattern(py, &unwrapped)?.ok_or_else(|| {
                    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                        "Unsupported complex assignment pattern in VM compiler",
                    )
                })?;

                let pat_idx = self.patterns.len();
                self.patterns.push(vm_pattern);

                self.compile_node(py, &values[0])?;
                self.emit(VMOpCode::DupTop, 0);
                self.emit(VMOpCode::MatchAssignPatternVM, pat_idx as u32);
                self.emit(VMOpCode::BindMatch, 0);

                // Sync bound names to scope where needed
                let names_to_sync = unwrapped.extract_names(py);
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

        // Extract flat names
        let names: Vec<String> = if let Some(ref pattern) = names_pattern {
            pattern.extract_names(py)
        } else {
            Vec::new()
        };

        if names.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        // Single name, single value: simple assignment (unless explicit_unpack)
        if names.len() == 1 && values.len() == 1 && !explicit_unpack {
            self.compile_node(py, &values[0])?;
            if !is_void {
                self.emit(VMOpCode::DupTop, 0);
            }
            self.emit_store(&names[0]);
            return Ok(());
        }

        // Multiple names OR explicit unpack, single value: unpacking
        if values.len() == 1 && (names.len() > 1 || explicit_unpack) {
            self.compile_node(py, &values[0])?;
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
                self.compile_node(py, &values[i])?;
            } else if !values.is_empty() {
                self.compile_node(py, values.last().unwrap())?;
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

    fn compile_getattr<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        self.compile_node(py, &args[0])?;
        let attr = args[1].as_string()?;
        let idx = self.add_name(&attr);
        self.emit(VMOpCode::GetAttr, idx as u32);
        Ok(())
    }

    fn compile_setattr<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        self.compile_node(py, &args[0])?;
        self.compile_node(py, &args[2])?;
        let attr = args[1].as_string()?;
        let idx = self.add_name(&attr);
        self.emit(VMOpCode::SetAttr, idx as u32);
        Ok(())
    }

    fn compile_getitem<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        self.compile_node(py, &args[0])?;
        self.compile_node(py, &args[1])?;
        self.emit(VMOpCode::GetItem, 0);
        Ok(())
    }

    fn compile_setitem<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        self.compile_node(py, &args[0])?;
        self.compile_node(py, &args[1])?;
        self.compile_node(py, &args[2])?;
        self.emit(VMOpCode::SetItem, 0);
        Ok(())
    }

    fn compile_slice<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        for arg in args {
            self.compile_node(py, arg)?;
        }
        self.emit(VMOpCode::BuildSlice, args.len() as u32);
        Ok(())
    }

    // ========== 10. Control flow (if, while, for, block, body, return) ==========

    fn compile_if<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let branches_node = &args[0];
        let else_branch = if args.len() > 1 { Some(&args[1]) } else { None };

        let branches = branches_node.children(py)?;
        if branches.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        let mut end_jumps = Vec::new();

        for branch in &branches {
            let len = branch.children_len(py)?;
            if len != 2 {
                continue;
            }
            let cond = branch.child(py, 0)?;
            let then_body = branch.child(py, 1)?;

            self.compile_node(py, &cond)?;
            let jump_to_next = self.emit(VMOpCode::JumpIfFalse, 0);
            self.compile_body(py, &then_body)?;
            end_jumps.push(self.emit(VMOpCode::Jump, 0));
            let pos = self.instructions.len() as u32;
            self.patch(jump_to_next, pos);
        }

        if let Some(else_body) = else_branch {
            self.compile_body(py, else_body)?;
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

    fn compile_while<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let cond = &args[0];
        let body = &args[1];

        let loop_start = self.instructions.len();
        self.loop_stack.push(LoopContext {
            break_targets: Vec::new(),
            continue_target: Some(loop_start),
            continue_patches: Vec::new(),
            is_for_loop: false,
        });

        let can_optimize = self.nesting_depth == 0 && !self.body_has_calls(py, body);
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        self.compile_node(py, cond)?;
        let jump_to_end = self.emit(VMOpCode::JumpIfFalse, 0);
        self.compile_body_void(py, body)?;

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

    fn compile_for<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let var_pattern = &args[0];
        let iterable = &args[1];
        let body = &args[2];

        let var_name = var_pattern.as_name(py);

        // Range optimization
        if let Some(var_name) = var_name.as_ref().filter(|_| iterable.is_range_call(py)) {
            return self.compile_for_range(py, var_name, iterable, body);
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
        self.compile_node(py, iterable)?;
        self.emit(VMOpCode::GetIter, 0);

        let loop_start = self.instructions.len();
        self.loop_stack.push(LoopContext {
            break_targets: Vec::new(),
            continue_target: Some(loop_start),
            continue_patches: Vec::new(),
            is_for_loop: true,
        });

        let for_iter_idx = self.emit(VMOpCode::ForIter, 0);

        // Store loop variable
        if let Some(ref name) = var_name {
            let slot = self.add_local(name);
            self.emit(VMOpCode::StoreLocal, slot as u32);
        } else {
            // Pattern unpacking
            self.compile_unpack_pattern(py, var_pattern, false)?;
        }

        let can_optimize = self.nesting_depth == 0 && !self.body_has_calls(py, body);
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        self.compile_body_void(py, body)?;

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
        // Always use loop_end so break hits PopBlock + save_restore
        for addr in ctx.break_targets {
            self.patch(addr, loop_end as u32);
        }

        self.in_optimized_loop = old_optimized;
        self.loop_modified_vars = old_modified;
        Ok(())
    }

    fn compile_for_range<'py>(
        &mut self,
        py: Python<'py>,
        var_name: &str,
        range_call: &CompilerNode<'py>,
        body: &CompilerNode<'py>,
    ) -> PyResult<()> {
        let range_args = range_call.range_call_args(py)?;

        let (start, stop, step): (CompilerNode<'py>, CompilerNode<'py>, i64) = match range_args.len() {
            1 => {
                let zero = CompilerNode::Pure(&IR::Int(0));
                (zero, range_args[0].clone(), 1)
            }
            2 => (range_args[0].clone(), range_args[1].clone(), 1),
            _ => {
                let step = range_args[2]
                    .as_int()
                    .or_else(|_| {
                        range_args[2]
                            .try_extract_neg_literal(py)
                            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("non-literal step"))
                    })
                    .unwrap_or(1);
                (range_args[0].clone(), range_args[1].clone(), step)
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

        self.compile_node(py, &start)?;
        self.emit(VMOpCode::StoreLocal, slot_i as u32);
        self.compile_node(py, &stop)?;
        self.emit(VMOpCode::StoreLocal, slot_stop as u32);

        let loop_start = self.instructions.len();
        self.loop_stack.push(LoopContext {
            break_targets: Vec::new(),
            continue_target: None,
            continue_patches: Vec::new(),
            is_for_loop: false,
        });

        let can_optimize = self.nesting_depth == 0 && !self.body_has_calls(py, body);
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        let arg = CompilerCore::encode_for_range_args(slot_i, slot_stop, step_is_positive, 0);
        let for_range_idx = self.emit(VMOpCode::ForRangeInt, arg);

        self.compile_body_void(py, body)?;

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

        let jump_offset = (loop_end as usize) - for_range_idx;
        let arg = CompilerCore::encode_for_range_args(slot_i, slot_stop, step_is_positive, jump_offset);
        self.patch(for_range_idx, arg);

        // Always use loop_end so break hits PopBlock + save_restore
        for addr in ctx.break_targets {
            self.patch(addr, loop_end);
        }

        self.in_optimized_loop = old_optimized;
        self.loop_modified_vars = old_modified;
        Ok(())
    }

    fn compile_block<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
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
            self.compile_node(py, item)?;
            if i < len - 1 {
                self.emit(VMOpCode::PopTop, 0);
            }
        }

        let pop_arg = if is_module_block { 1u32 } else { 0u32 };
        self.emit(VMOpCode::PopBlock, pop_arg);
        Ok(())
    }

    /// Compile body without PushBlock/PopBlock (for control structures).
    /// If body is an OpBlock, compile its contents inline.
    fn compile_body<'py>(&mut self, py: Python<'py>, body: &CompilerNode<'py>) -> PyResult<()> {
        if let Some(contents) = body.as_block_contents(py) {
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
                let is_void = item.is_void_op(py);
                self.compile_node(py, item)?;
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
        // Single node: check if void
        if body.is_void_op(py) {
            self.compile_node(py, body)?;
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }
        self.compile_node(py, body)
    }

    /// Compile body in void context: statements don't leave values on the stack.
    fn compile_body_void<'py>(&mut self, py: Python<'py>, body: &CompilerNode<'py>) -> PyResult<()> {
        if let Some(contents) = body.as_block_contents(py) {
            for stmt in &contents {
                let is_set_locals = stmt.is_set_locals(py);
                let is_void_op = stmt.is_void_op(py);

                if is_set_locals {
                    self.void_context = true;
                    self.compile_node(py, stmt)?;
                    self.void_context = false;
                } else if is_void_op {
                    self.compile_node(py, stmt)?;
                } else {
                    self.compile_node(py, stmt)?;
                    self.emit(VMOpCode::PopTop, 0);
                }
            }
            return Ok(());
        }
        // Not a block: compile single node
        let is_set_locals = body.is_set_locals(py);
        let is_void_op = body.is_void_op(py);
        if is_set_locals {
            self.void_context = true;
            self.compile_node(py, body)?;
            self.void_context = false;
        } else if is_void_op {
            self.compile_node(py, body)?;
        } else {
            self.compile_node(py, body)?;
            self.emit(VMOpCode::PopTop, 0);
        }
        Ok(())
    }

    fn compile_return<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if !args.is_empty() {
            self.compile_node(py, &args[0])?;
        } else {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        }
        if self.core.finally_depth > 0 {
            let n = self.finally_stack.len();
            self.emit_finally_unwind(py)?;
            self.core.finally_depth += n;
        }
        self.emit(VMOpCode::Return, 0);
        Ok(())
    }

    // ========== 11. Functions (call, lambda, fn_def) ==========

    /// Shared call dispatch: detects method calls, emits Call/CallKw/CallMethod/TailCall.
    fn compile_call_dispatch<'py>(
        &mut self,
        py: Python<'py>,
        func: &CompilerNode<'py>,
        args: &[CompilerNode<'py>],
        kwargs: &CompilerKwargs<'py>,
    ) -> PyResult<()> {
        let is_empty_kwargs = kwargs.is_empty()?;

        // Detect method call: func is GetAttr
        let method_call_info = if is_empty_kwargs {
            func.as_getattr_parts(py)
        } else {
            None
        };

        if let Some((obj, method_name)) = method_call_info {
            self.compile_node(py, &obj)?;
            for arg in args {
                self.compile_node(py, arg)?;
            }
            let name_idx = self.add_name(&method_name);
            let encoding = ((name_idx as u32) << 16) | (args.len() as u32);
            self.emit(VMOpCode::CallMethod, encoding);
        } else {
            self.compile_node(py, func)?;
            for arg in args {
                self.compile_node(py, arg)?;
            }
            if !is_empty_kwargs {
                let kw_pairs = kwargs.iter(py)?;
                let mut kw_names = Vec::new();
                for (name, value) in &kw_pairs {
                    kw_names.push(name.clone());
                    self.compile_node(py, value)?;
                }
                let kw_tuple = PyTuple::new(py, &kw_names)?;
                let kw_idx = self.add_const_pyobj(py, &kw_tuple.into_any());
                self.emit(VMOpCode::LoadConst, kw_idx as u32);
                let encoding = ((args.len() as u32) << 8) | (kw_pairs.len() as u32);
                self.emit(VMOpCode::CallKw, encoding);
            } else {
                self.emit(VMOpCode::Call, args.len() as u32);
            }
        }
        Ok(())
    }

    fn compile_call<'py>(
        &mut self,
        py: Python<'py>,
        args: &[CompilerNode<'py>],
        kwargs: &CompilerKwargs<'py>,
        is_tail: bool,
    ) -> PyResult<()> {
        let func = &args[0];
        let call_args = &args[1..];
        let is_empty_kwargs = kwargs.is_empty()?;

        // Detect method call
        let method_call_info = if is_empty_kwargs && !is_tail {
            func.as_getattr_parts(py)
        } else {
            None
        };

        if let Some((obj, method_name)) = method_call_info {
            self.compile_node(py, &obj)?;
            for arg in call_args {
                self.compile_node(py, arg)?;
            }
            let name_idx = self.add_name(&method_name);
            let encoding = ((name_idx as u32) << 16) | (call_args.len() as u32);
            self.emit(VMOpCode::CallMethod, encoding);
        } else {
            self.compile_node(py, func)?;
            for arg in call_args {
                self.compile_node(py, arg)?;
            }
            if !is_empty_kwargs {
                let kw_pairs = kwargs.iter(py)?;
                let mut kw_names = Vec::new();
                for (name, value) in &kw_pairs {
                    kw_names.push(name.clone());
                    self.compile_node(py, value)?;
                }
                let kw_tuple = PyTuple::new(py, &kw_names)?;
                let kw_idx = self.add_const_pyobj(py, &kw_tuple.into_any());
                self.emit(VMOpCode::LoadConst, kw_idx as u32);
                let encoding = ((call_args.len() as u32) << 8) | (kw_pairs.len() as u32);
                self.emit(VMOpCode::CallKw, encoding);
            } else if is_tail {
                self.emit(VMOpCode::TailCall, call_args.len() as u32);
            } else {
                self.emit(VMOpCode::Call, call_args.len() as u32);
            }
        }
        Ok(())
    }

    fn compile_lambda<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let raw_params = &args[0];
        let body = &args[1];

        let (param_names, defaults, vararg_idx) = self.extract_params(py, raw_params)?;

        let mut func_compiler = UnifiedCompiler::new();
        let mut code = func_compiler.compile_function(
            py,
            FunctionCompileSpec {
                params: param_names,
                body,
                name: "<lambda>",
                defaults,
                vararg_idx,
                parent_nesting_depth: self.nesting_depth,
            },
        )?;

        // Freeze IR source for ND process workers
        code.encoded_ir = freeze_ir_body(body);

        let py_code = Py::new(py, PyCodeObject::new(code))?;
        let val = Value::from_pyobject(py, py_code.bind(py))?;
        let idx = self.core.add_const(val);
        self.emit(VMOpCode::LoadConst, idx as u32);
        self.emit(VMOpCode::MakeFunction, 0);
        Ok(())
    }

    fn compile_fn_def<'py>(
        &mut self,
        py: Python<'py>,
        args: &[CompilerNode<'py>],
        _kwargs: &CompilerKwargs<'py>,
    ) -> PyResult<()> {
        let name = args[0].as_name(py).unwrap_or_else(|| "<fn>".to_string());
        let raw_params = &args[1];
        let body = &args[2];

        let (param_names, defaults, vararg_idx) = self.extract_params(py, raw_params)?;

        let mut func_compiler = UnifiedCompiler::new();
        let mut code = func_compiler.compile_function(
            py,
            FunctionCompileSpec {
                params: param_names,
                body,
                name: &name,
                defaults,
                vararg_idx,
                parent_nesting_depth: self.nesting_depth,
            },
        )?;

        // Freeze IR source for ND process workers
        code.encoded_ir = freeze_ir_body(body);

        let py_code = Py::new(py, PyCodeObject::new(code))?;
        let val = Value::from_pyobject(py, py_code.bind(py))?;
        let idx = self.core.add_const(val);
        self.emit(VMOpCode::LoadConst, idx as u32);
        self.emit(VMOpCode::MakeFunction, 0);

        self.core.emit_store(&name);
        Ok(())
    }

    // ========== 12. Collections ==========

    fn compile_list<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        for arg in args {
            self.compile_node(py, arg)?;
        }
        self.emit(VMOpCode::BuildList, args.len() as u32);
        Ok(())
    }

    fn compile_tuple<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        for arg in args {
            self.compile_node(py, arg)?;
        }
        self.emit(VMOpCode::BuildTuple, args.len() as u32);
        Ok(())
    }

    fn compile_set<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        for arg in args {
            self.compile_node(py, arg)?;
        }
        self.emit(VMOpCode::BuildSet, args.len() as u32);
        Ok(())
    }

    fn compile_dict<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        for arg in args {
            // Each arg is a 2-child node (key, value)
            let key = arg.child(py, 0)?;
            let value = arg.child(py, 1)?;
            self.compile_node(py, &key)?;
            self.compile_node(py, &value)?;
        }
        self.emit(VMOpCode::BuildDict, args.len() as u32);
        Ok(())
    }

    // ========== 13. Broadcast ==========

    fn compile_broadcast_op<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if args.len() < 4 {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "Broadcast requires 4 arguments: target, operator, operand, is_filter",
            ));
        }
        let target_expr = &args[0];
        let operator_expr = &args[1];
        let operand_expr = &args[2];
        let is_filter = args[3].as_bool().unwrap_or(false);

        self.compile_node(py, target_expr)?;
        self.compile_node(py, operator_expr)?;

        let has_operand = !operand_expr.is_none_value();
        if has_operand {
            self.compile_node(py, operand_expr)?;
        }

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

    // ========== 14. Match ==========

    fn compile_match<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let value_expr = &args[0];
        let cases_node = &args[1];

        // Pre-allocate slots for pattern variables
        self.collect_pattern_vars(py, cases_node)?;

        // Compile value to match
        self.compile_node(py, value_expr)?;

        let cases = cases_node.children(py)?;
        let mut end_jumps = Vec::new();

        for case in &cases {
            let case_len = case.children_len(py)?;
            if case_len < 3 {
                continue;
            }
            let pattern = case.child(py, 0)?;
            let guard = case.child(py, 1)?;
            let body = case.child(py, 2)?;

            self.emit(VMOpCode::DupTop, 0);

            let vm_pattern = self
                .try_compile_pattern(py, &pattern)?
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("unsupported match pattern"))?;
            let pat_idx = self.patterns.len();
            self.patterns.push(vm_pattern);
            self.emit(VMOpCode::MatchPatternVM, pat_idx as u32);

            self.emit(VMOpCode::DupTop, 0);
            let skip_jump = self.emit(VMOpCode::JumpIfNone, 0);

            let guard_fail = if !guard.is_none_value() {
                self.emit(VMOpCode::DupTop, 0);
                self.emit(VMOpCode::PushBlock, 0);
                self.emit(VMOpCode::BindMatch, 0);
                self.compile_node(py, &guard)?;
                self.emit(VMOpCode::PopBlock, 0);
                Some(self.emit(VMOpCode::JumpIfFalse, 0))
            } else {
                None
            };

            self.emit(VMOpCode::BindMatch, 0);
            self.emit(VMOpCode::PopTop, 0);
            self.compile_node(py, &body)?;
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
        let msg = "No matching pattern";
        let msg_py = msg.into_pyobject(py)?.into_any();
        let msg_idx = self.add_const_pyobj(py, &msg_py);
        self.emit(VMOpCode::MatchFail, msg_idx as u32);

        let end_addr = self.instructions.len() as u32;
        for addr in end_jumps {
            self.patch(addr, end_addr);
        }
        Ok(())
    }

    // ========== 15. Struct/Trait ==========

    fn compile_struct<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let name = args[0].as_name(py).unwrap_or_else(|| "<struct>".to_string());

        let fields_cn = args[1].children(py)?;
        let args_len = args.len();

        let mut implements_list: Vec<String> = Vec::new();
        let mut base_names: Vec<String> = Vec::new();
        let mut methods_index: Option<usize> = None;

        if args_len > 3 {
            // args[2] = implements, args[3] = bases
            let impl_items = args[2].children(py)?;
            for imp in &impl_items {
                if let Some(s) = imp.as_name(py) {
                    implements_list.push(s);
                }
            }
            // bases
            if !args[3].is_none_value() {
                let base_items = args[3].children(py).unwrap_or_default();
                if !base_items.is_empty() {
                    for b in &base_items {
                        if let Some(s) = b.as_name(py) {
                            base_names.push(s);
                        }
                    }
                } else if let Some(s) = args[3].as_name(py) {
                    base_names.push(s);
                }
            }
            if args_len > 4 {
                methods_index = Some(4);
            }
        } else if args_len > 2 {
            if let Some(s) = args[2].as_name(py) {
                base_names.push(s);
                if args_len > 3 {
                    methods_index = Some(3);
                }
            } else {
                let impl_items = args[2].children(py).unwrap_or_default();
                if !impl_items.is_empty() {
                    let mut is_impl_list = true;
                    for imp in &impl_items {
                        if let Some(s) = imp.as_name(py) {
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
        }

        // Compile field defaults and build fields_info
        let mut fields_info: Vec<Py<PyAny>> = Vec::new();
        let mut num_defaults: u32 = 0;

        for field in &fields_cn {
            let field_len = field.children_len(py)?;
            if field_len >= 2 {
                let fname = field.child(py, 0)?.as_name(py).unwrap_or_default();
                let has_default = field.child(py, 1)?.as_bool().unwrap_or(false);
                if has_default && field_len >= 3 {
                    let default_expr = field.child(py, 2)?;
                    self.compile_node(py, &default_expr)?;
                    num_defaults += 1;
                }
                let entry = PyTuple::new(
                    py,
                    &[
                        fname.into_pyobject(py)?.into_any().unbind(),
                        has_default.into_pyobject(py)?.to_owned().into_any().unbind(),
                    ],
                )?;
                fields_info.push(entry.into_any().unbind());
            }
        }
        let fields_tuple = PyTuple::new(py, &fields_info)?;

        // Compile methods
        let methods_list = if let Some(idx) = methods_index {
            let methods_cn = args[idx].children(py)?;
            let mut compiled: Vec<Py<PyAny>> = Vec::new();
            for m in &methods_cn {
                let method_name = m.child(py, 0)?.as_name(py).unwrap_or_default();
                let is_static = if m.children_len(py)? > 2 {
                    m.child(py, 2)?.as_bool().unwrap_or(false)
                } else {
                    false
                };
                let is_static_py = is_static.into_pyobject(py)?.to_owned().into_any().unbind();

                // Abstract method check
                let lambda_node = m.child(py, 1)?;
                if lambda_node.is_none_value() {
                    let pair = PyTuple::new(
                        py,
                        &[
                            method_name.into_pyobject(py)?.into_any().unbind(),
                            py.None(),
                            is_static_py,
                        ],
                    )?;
                    compiled.push(pair.into_any().unbind());
                    continue;
                }

                // Compile method body (lambda Op) - extract params and body from the lambda
                let lambda_params = lambda_node.child(py, 0)?;
                let lambda_body = lambda_node.child(py, 1)?;
                let (param_names, defaults, vararg_idx) = self.extract_params(py, &lambda_params)?;

                let mut func_compiler = UnifiedCompiler::new();
                let mut code = func_compiler.compile_function(
                    py,
                    FunctionCompileSpec {
                        params: param_names,
                        body: &lambda_body,
                        name: &method_name,
                        defaults,
                        vararg_idx,
                        parent_nesting_depth: self.nesting_depth,
                    },
                )?;
                code.encoded_ir = freeze_ir_body(&lambda_body);
                let py_code = Py::new(py, PyCodeObject::new(code))?;
                let pair = PyTuple::new(
                    py,
                    &[
                        method_name.into_pyobject(py)?.into_any().unbind(),
                        py_code.into_any(),
                        is_static_py,
                    ],
                )?;
                compiled.push(pair.into_any().unbind());
            }
            Some(PyList::new(py, &compiled)?)
        } else {
            None
        };

        // Build struct info constant
        let num_defaults_py = num_defaults.into_pyobject(py)?.into_any().as_any().clone();
        let name_py = name.as_str().into_pyobject(py)?.into_any().as_any().clone();

        let has_implements = !implements_list.is_empty();
        let has_bases = !base_names.is_empty();

        let struct_info = if has_implements || has_bases {
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
            let mut items: Vec<Py<PyAny>> = vec![
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
            match methods_list {
                Some(methods) => PyTuple::new(
                    py,
                    &[name_py, fields_tuple.into_any(), num_defaults_py, methods.into_any()],
                )?,
                None => PyTuple::new(py, &[name_py, fields_tuple.into_any(), num_defaults_py])?,
            }
        };

        let idx = self.add_const_pyobj(py, &struct_info.into_any());
        self.emit(VMOpCode::MakeStruct, idx as u32);
        Ok(())
    }

    fn compile_trait<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let name = args[0].as_name(py).unwrap_or_default();

        // args[1] = extends list, args[2] = fields, args[3] = methods (optional)
        let extends_cn = args[1].children(py)?;
        let fields_cn = args[2].children(py)?;

        let mut extends: Vec<Py<PyAny>> = Vec::new();
        for e in &extends_cn {
            if let Some(s) = e.as_name(py) {
                extends.push(s.into_pyobject(py)?.into_any().unbind());
            }
        }
        let extends_tuple = PyTuple::new(py, &extends)?;

        let mut fields_info: Vec<Py<PyAny>> = Vec::new();
        let mut num_defaults: u32 = 0;
        for f in &fields_cn {
            let f_len = f.children_len(py)?;
            if f_len >= 2 {
                let fname = f.child(py, 0)?.as_name(py).unwrap_or_default();
                let default_node = f.child(py, 1)?;
                let has_default = !default_node.is_none_value();
                if has_default {
                    self.compile_node(py, &default_node)?;
                    num_defaults += 1;
                }
                let entry = PyTuple::new(
                    py,
                    &[
                        fname.into_pyobject(py)?.into_any().unbind(),
                        has_default.into_pyobject(py)?.to_owned().into_any().unbind(),
                    ],
                )?;
                fields_info.push(entry.into_any().unbind());
            }
        }
        let fields_tuple = PyTuple::new(py, &fields_info)?;

        // Methods
        let methods_list = if args.len() > 3 {
            let methods_cn = args[3].children(py)?;
            let mut compiled: Vec<Py<PyAny>> = Vec::new();
            for m in &methods_cn {
                let method_name = m.child(py, 0)?.as_name(py).unwrap_or_default();
                let is_static = if m.children_len(py)? > 2 {
                    m.child(py, 2)?.as_bool().unwrap_or(false)
                } else {
                    false
                };
                let is_static_py = is_static.into_pyobject(py)?.to_owned().into_any().unbind();

                let lambda_node = m.child(py, 1)?;
                if lambda_node.is_none_value() {
                    let pair = PyTuple::new(
                        py,
                        &[
                            method_name.into_pyobject(py)?.into_any().unbind(),
                            py.None(),
                            is_static_py,
                        ],
                    )?;
                    compiled.push(pair.into_any().unbind());
                    continue;
                }

                let lambda_params = lambda_node.child(py, 0)?;
                let lambda_body = lambda_node.child(py, 1)?;
                let (param_names, defaults, vararg_idx) = self.extract_params(py, &lambda_params)?;

                let mut func_compiler = UnifiedCompiler::new();
                let mut code = func_compiler.compile_function(
                    py,
                    FunctionCompileSpec {
                        params: param_names,
                        body: &lambda_body,
                        name: &method_name,
                        defaults,
                        vararg_idx,
                        parent_nesting_depth: self.nesting_depth,
                    },
                )?;
                code.encoded_ir = freeze_ir_body(&lambda_body);
                let py_code = Py::new(py, PyCodeObject::new(code))?;
                let pair = PyTuple::new(
                    py,
                    &[
                        method_name.into_pyobject(py)?.into_any().unbind(),
                        py_code.into_any(),
                        is_static_py,
                    ],
                )?;
                compiled.push(pair.into_any().unbind());
            }
            Some(PyList::new(py, &compiled)?)
        } else {
            None
        };

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

        let idx = self.add_const_pyobj(py, &trait_info.into_any());
        self.emit(VMOpCode::MakeTrait, idx as u32);
        Ok(())
    }

    // ========== 15b. Enum definition ==========

    fn compile_enum<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let name = args[0].as_name(py).unwrap_or_default();

        // args[1] = tuple of variant name strings
        let variant_nodes = args[1].children(py)?;
        let mut variant_names: Vec<Bound<'py, PyAny>> = Vec::new();
        for v in &variant_nodes {
            let vname = v.as_name(py).unwrap_or_default();
            variant_names.push(vname.into_pyobject(py)?.into_any());
        }
        let variants_tuple = PyTuple::new(py, &variant_names)?;

        let enum_info = PyTuple::new(
            py,
            &[
                name.as_str().into_pyobject(py)?.into_any().as_any().clone(),
                variants_tuple.into_any(),
            ],
        )?;

        let idx = self.add_const_pyobj(py, &enum_info.into_any());
        self.emit(VMOpCode::MakeEnum, idx as u32);
        Ok(())
    }

    // ========== 16. ND operations ==========

    fn compile_nd_recursion<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if args.len() == 1 {
            // Declaration form: ~~(lambda) → wraps lambda in NDVmDecl
            self.compile_node(py, &args[0])?;
            self.emit(VMOpCode::NdRecursion, 1);
        } else if args.len() >= 2 && args[1].is_none_value() {
            // Declaration form with explicit None seed
            self.compile_node(py, &args[0])?;
            self.emit(VMOpCode::NdRecursion, 1);
        } else if args.len() >= 2 {
            // Combinator form: ~~(seed, lambda)
            self.compile_node(py, &args[0])?;
            self.compile_node(py, &args[1])?;
            self.emit(VMOpCode::NdRecursion, 0);
        } else {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        }
        Ok(())
    }

    fn compile_nd_map<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if args.len() == 1 {
            // Lift form: ~>(func) → return func as-is
            self.compile_node(py, &args[0])?;
            self.emit(VMOpCode::NdMap, 1);
        } else if args.len() >= 2 && args[1].is_none_value() {
            // Lift form with explicit None
            self.compile_node(py, &args[0])?;
            self.emit(VMOpCode::NdMap, 1);
        } else if args.len() >= 2 {
            // Applicative form: ~>(data, func)
            self.compile_node(py, &args[0])?;
            self.compile_node(py, &args[1])?;
            self.emit(VMOpCode::NdMap, 0);
        } else {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        }
        Ok(())
    }

    // ========== 17. F-strings ==========

    fn compile_fstring<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if args.is_empty() {
            let py_str = "".into_pyobject(py)?.into_any();
            let idx = self.add_const_pyobj(py, &py_str);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        let mut n_parts: u32 = 0;

        for part in args {
            if let Ok(text) = part.as_string() {
                // Text part → LoadConst
                let py_str = text.into_pyobject(py)?.into_any();
                let idx = self.add_const_pyobj(py, &py_str);
                self.emit(VMOpCode::LoadConst, idx as u32);
                n_parts += 1;
            } else if part.is_tuple() {
                // Interpolation: Tuple([expr, Int(conv), spec])
                let expr = part.child(py, 0)?;
                let conv = part.child(py, 1)?.as_int().unwrap_or(0) as u32;
                let spec_node = part.child(py, 2)?;
                let has_spec = !spec_node.is_none_value();

                self.compile_node(py, &expr)?;

                if has_spec {
                    let spec_str = spec_node.as_string()?;
                    let py_str = spec_str.into_pyobject(py)?.into_any();
                    let idx = self.add_const_pyobj(py, &py_str);
                    self.emit(VMOpCode::LoadConst, idx as u32);
                }

                // flags = (conv << 1) | has_spec
                let flags = (conv << 1) | (has_spec as u32);
                self.emit(VMOpCode::FormatValue, flags);
                n_parts += 1;
            }
        }

        if n_parts == 0 {
            let py_str = "".into_pyobject(py)?.into_any();
            let idx = self.add_const_pyobj(py, &py_str);
            self.emit(VMOpCode::LoadConst, idx as u32);
        } else if n_parts > 1 {
            self.emit(VMOpCode::BuildString, n_parts);
        }
        // n_parts == 1: result already on stack
        Ok(())
    }

    // ========== 18. Exception handling ==========

    /// Emit inline finally cleanup for break/continue/return paths.
    fn emit_finally_unwind<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
        let bodies: Vec<UCFinallyInfo> = self.finally_stack.iter().rev().cloned().collect();
        for info in &bodies {
            if info.needs_clear_exception {
                self.emit(VMOpCode::ClearException, 0);
            }
            if info.has_except {
                self.emit(VMOpCode::PopHandler, 0);
            }
            self.emit(VMOpCode::PopHandler, 0);
            self.core.finally_depth -= 1;
            match &info.body {
                UCFinallyBody::Pure(ir) => {
                    let cn = CompilerNode::Pure(ir);
                    self.compile_node(py, &cn)?;
                }
                UCFinallyBody::PyObj(obj) => {
                    let cn = CompilerNode::PyObj(obj.bind(py).clone());
                    self.compile_node(py, &cn)?;
                }
            }
            self.emit(VMOpCode::PopTop, 0);
        }
        Ok(())
    }

    fn compile_break_with_finally<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
        if self.core.finally_depth == 0 {
            return self
                .core
                .compile_break()
                .map_err(|e| pyo3::exceptions::PySyntaxError::new_err(e.to_string()));
        }
        let n = self.finally_stack.len();
        self.emit_finally_unwind(py)?;
        let result = self.core.compile_break();
        self.core.finally_depth += n;
        result.map_err(|e| pyo3::exceptions::PySyntaxError::new_err(e.to_string()))
    }

    fn compile_continue_with_finally<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
        if self.core.finally_depth == 0 {
            return self
                .core
                .compile_continue()
                .map_err(|e| pyo3::exceptions::PySyntaxError::new_err(e.to_string()));
        }
        let n = self.finally_stack.len();
        self.emit_finally_unwind(py)?;
        let result = self.core.compile_continue();
        self.core.finally_depth += n;
        result.map_err(|e| pyo3::exceptions::PySyntaxError::new_err(e.to_string()))
    }

    fn compile_try<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let body = &args[0];
        let handlers_node = &args[1];
        let finally_node = &args[2];
        let has_finally = !finally_node.is_none_value();

        let handlers = if handlers_node.is_list_or_tuple() {
            handlers_node.children(py)?
        } else {
            Vec::new()
        };
        let has_except = !handlers.is_empty();

        let mut finally_setup_addr = None;
        let mut except_setup_addr = None;

        // Install handlers (Finally first, Except on top)
        if has_finally {
            finally_setup_addr = Some(self.emit(VMOpCode::SetupFinally, 0));
            self.core.finally_depth += 1;
            let body = match finally_node {
                CompilerNode::Pure(ir) => UCFinallyBody::Pure((*ir).clone()),
                CompilerNode::PyObj(obj) => UCFinallyBody::PyObj(obj.clone().unbind()),
            };
            self.finally_stack.push(UCFinallyInfo {
                body,
                has_except,
                needs_clear_exception: false,
            });
        }
        if has_except {
            except_setup_addr = Some(self.emit(VMOpCode::SetupExcept, 0));
        }

        // Try body
        self.compile_node(py, body)?;

        // Happy path: pop handlers
        if has_except {
            self.emit(VMOpCode::PopHandler, 0);
        }
        // Save the finally body before popping (needed for handler bodies below)
        let saved_finally_body = if has_finally {
            self.finally_stack.last().map(|info| info.body.clone())
        } else {
            None
        };
        if has_finally {
            self.emit(VMOpCode::PopHandler, 0);
            self.core.finally_depth -= 1;
            self.finally_stack.pop();
        }

        // Inline finally on happy path
        if has_finally {
            self.compile_node(py, finally_node)?;
            self.emit(VMOpCode::PopTop, 0);
        }
        let end_jump = self.emit(VMOpCode::Jump, 0);
        let mut handler_end_jumps: Vec<usize> = Vec::new();

        // Except dispatch
        if has_except {
            let except_addr = self.instructions.len();
            if let Some(addr) = except_setup_addr {
                self.core.patch(addr, except_addr as u32);
            }

            // Restore finally context for handler bodies so break/continue inline ClearException + finally
            if let Some(ref body) = saved_finally_body {
                self.core.finally_depth += 1;
                self.finally_stack.push(UCFinallyInfo {
                    body: body.clone(),
                    has_except: false,
                    needs_clear_exception: true,
                });
            }

            for handler_node in &handlers {
                let handler_len = handler_node.children_len(py)?;
                if handler_len < 3 {
                    continue;
                }
                let types_node = handler_node.child(py, 0)?;
                let binding_node = handler_node.child(py, 1)?;
                let handler_body = handler_node.child(py, 2)?;

                let type_list = if types_node.is_list_or_tuple() {
                    types_node.children(py)?
                } else {
                    Vec::new()
                };
                let is_wildcard = type_list.is_empty();

                if !is_wildcard {
                    // Typed handler: check each exception type
                    let mut type_match_jumps = Vec::new();
                    for type_ir in &type_list {
                        let type_name = type_ir.as_string()?;
                        let py_str = type_name.into_pyobject(py)?.into_any();
                        let const_idx = self.add_const_pyobj(py, &py_str);
                        self.emit(VMOpCode::CheckExcMatch, const_idx as u32);
                        type_match_jumps.push(self.emit(VMOpCode::JumpIfTrue, 0));
                    }
                    let skip_jump = self.emit(VMOpCode::Jump, 0);

                    // Handler body start
                    let handler_start = self.instructions.len() as u32;
                    for addr in type_match_jumps {
                        self.core.patch(addr, handler_start);
                    }

                    // Bind exception message if binding present
                    if !binding_node.is_none_value() {
                        if let Ok(name) = binding_node.as_string() {
                            self.emit(VMOpCode::LoadException, 0);
                            let slot = self.add_local(&name);
                            self.emit(VMOpCode::StoreLocal, slot as u32);
                        }
                    }

                    self.compile_node(py, &handler_body)?;

                    // Pop exception stack + inline finally
                    self.emit(VMOpCode::ClearException, 0);
                    if has_finally {
                        self.emit(VMOpCode::PopHandler, 0);
                        self.compile_node(py, finally_node)?;
                        self.emit(VMOpCode::PopTop, 0);
                    }
                    handler_end_jumps.push(self.emit(VMOpCode::Jump, 0));

                    let next = self.instructions.len() as u32;
                    self.core.patch(skip_jump, next);
                } else {
                    // Wildcard handler
                    if !binding_node.is_none_value() {
                        if let Ok(name) = binding_node.as_string() {
                            self.emit(VMOpCode::LoadException, 0);
                            let slot = self.add_local(&name);
                            self.emit(VMOpCode::StoreLocal, slot as u32);
                        }
                    }

                    self.compile_node(py, &handler_body)?;

                    // Pop exception stack + inline finally
                    self.emit(VMOpCode::ClearException, 0);
                    if has_finally {
                        self.emit(VMOpCode::PopHandler, 0);
                        self.compile_node(py, finally_node)?;
                        self.emit(VMOpCode::PopTop, 0);
                    }
                    handler_end_jumps.push(self.emit(VMOpCode::Jump, 0));
                    break; // Wildcard is always last
                }
            }

            // Restore finally context
            if saved_finally_body.is_some() {
                self.core.finally_depth -= 1;
                self.finally_stack.pop();
            }

            // No handler matched: bare re-raise (goes through handler stack for finally)
            self.emit(VMOpCode::Raise, 1);
        }

        // Finally landing pad (reached by VM when it pops a Finally handler)
        if has_finally {
            let finally_landing = self.instructions.len() as u32;
            if let Some(addr) = finally_setup_addr {
                self.core.patch(addr, finally_landing);
            }
            self.compile_node(py, finally_node)?;
            self.emit(VMOpCode::PopTop, 0);
            self.emit(VMOpCode::ResumeUnwind, 0);
        }

        // End label: all paths (happy, handler, finally-only) converge here
        let end_addr = self.instructions.len() as u32;
        self.core.patch(end_jump, end_addr);
        for addr in handler_end_jumps {
            self.core.patch(addr, end_addr);
        }
        Ok(())
    }

    fn compile_raise<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if args.is_empty() {
            // Bare raise
            self.emit(VMOpCode::Raise, 1);
        } else {
            // raise expr
            self.compile_node(py, &args[0])?;
            self.emit(VMOpCode::Raise, 0);
        }
        Ok(())
    }

    // ========== 19. Helper methods ==========

    /// Add a Python object constant (fallback to NIL on conversion error).
    fn add_const_pyobj(&mut self, py: Python<'_>, obj: &Bound<'_, PyAny>) -> usize {
        self.core.add_const_py(py, obj).unwrap_or_else(|e| {
            #[cfg(debug_assertions)]
            eprintln!("[compiler] failed to convert PyObject to Value: {e}");
            let _ = e;
            self.core.add_const(Value::NIL)
        })
    }

    /// Check if body contains function calls (recursive scan).
    fn body_has_calls<'py>(&self, py: Python<'py>, node: &CompilerNode<'py>) -> bool {
        match node {
            CompilerNode::Pure(ir) => self.body_has_calls_ir(ir),
            CompilerNode::PyObj(obj) => self.body_has_calls_py(py, obj),
        }
    }

    fn body_has_calls_ir(&self, node: &IR) -> bool {
        match node {
            IR::Op { opcode, args, .. } => {
                if *opcode == IROpCode::Call || *opcode == IROpCode::FnDef || *opcode == IROpCode::OpLambda {
                    return true;
                }
                args.iter().any(|a| self.body_has_calls_ir(a))
            }
            IR::Call { .. } => true,
            IR::List(items) | IR::Tuple(items) | IR::Program(items) => items.iter().any(|i| self.body_has_calls_ir(i)),
            _ => false,
        }
    }

    fn body_has_calls_py(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> bool {
        if let Ok(list) = node.cast::<PyList>() {
            for item in list.iter() {
                if self.body_has_calls_py(py, &item) {
                    return true;
                }
            }
            return false;
        }
        if let Ok(tuple) = node.cast::<PyTuple>() {
            for item in tuple.iter() {
                if self.body_has_calls_py(py, &item) {
                    return true;
                }
            }
            return false;
        }
        if let Ok(op) = node.extract::<PyRef<Op>>() {
            let ident = op.ident;
            if ident == IROpCode::Call as i32 || ident == IROpCode::FnDef as i32 || ident == IROpCode::OpLambda as i32 {
                return true;
            }
            let args = op.args.bind(py);
            if let Ok(len) = args.len() {
                for i in 0..len {
                    if let Ok(arg) = args.get_item(i) {
                        if self.body_has_calls_py(py, &arg) {
                            return true;
                        }
                    }
                }
            }
            return false;
        }
        if let Ok(type_name) = node.get_type().name() {
            if type_name == "Call" {
                return true;
            }
        }
        false
    }

    /// Extract params, defaults, vararg_idx from a CompilerNode params node.
    fn extract_params<'py>(
        &self,
        py: Python<'py>,
        params: &CompilerNode<'py>,
    ) -> PyResult<(Vec<String>, Vec<Value>, i32)> {
        let mut param_names = Vec::new();
        let mut defaults = Vec::new();
        let mut vararg_idx: i32 = -1;

        let children = params.children(py)?;
        for item in &children {
            let item_len = item.children_len(py).unwrap_or(0);
            // 2-element tuple: (name, default) or ("*", vararg_name)
            if item_len == 2 {
                let first = item.child(py, 0)?;
                let second = item.child(py, 1)?;
                let name = first.as_name(py).unwrap_or_default();
                if name == "*" {
                    vararg_idx = param_names.len() as i32;
                    param_names.push(second.as_name(py).unwrap_or_default());
                } else {
                    param_names.push(name);
                    // 2-tuple always means default is present (None is a valid default)
                    let val = self.ir_to_value(py, &second)?;
                    defaults.push(val);
                }
            } else {
                // Simple param name
                if let Some(name) = item.as_name(py) {
                    param_names.push(name);
                }
            }
        }
        Ok((param_names, defaults, vararg_idx))
    }

    /// Convert a literal CompilerNode to a Value.
    fn ir_to_value<'py>(&self, py: Python<'py>, node: &CompilerNode<'py>) -> PyResult<Value> {
        match node {
            CompilerNode::Pure(ir) => match ir {
                IR::Int(n) => Ok(Value::from_i64(*n)),
                IR::Float(f) => Ok(Value::from_float(*f)),
                IR::Bool(b) => Ok(Value::from_bool(*b)),
                IR::None => Ok(Value::NIL),
                IR::String(s) => {
                    let py_str = s.as_str().into_pyobject(py)?.into_any();
                    Value::from_pyobject(py, &py_str)
                }
                _ => Ok(Value::NIL),
            },
            CompilerNode::PyObj(obj) => Value::from_pyobject(py, obj).or(Ok(Value::NIL)),
        }
    }

    /// Try to compile a pattern into a VMPattern (native VM path).
    /// Returns None if the pattern can't be compiled natively (fallback to legacy).
    fn try_compile_pattern<'py>(
        &mut self,
        py: Python<'py>,
        pattern: &CompilerNode<'py>,
    ) -> PyResult<Option<VMPattern>> {
        match pattern {
            CompilerNode::Pure(ir) => self.try_compile_pattern_ir(py, ir),
            CompilerNode::PyObj(obj) => self.try_compile_pattern_py(py, obj),
        }
    }

    fn try_compile_pattern_ir(&mut self, py: Python<'_>, pattern: &IR) -> PyResult<Option<VMPattern>> {
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
                let val = self.ir_to_value(py, &CompilerNode::Pure(value))?;
                Ok(Some(VMPattern::Literal(val)))
            }
            IR::PatternOr(patterns) => {
                let mut sub_patterns = Vec::new();
                for p in patterns {
                    match self.try_compile_pattern_ir(py, p)? {
                        Some(vp) => sub_patterns.push(vp),
                        None => return Ok(None),
                    }
                }
                Ok(Some(VMPattern::Or(sub_patterns)))
            }
            IR::PatternTuple(patterns) => {
                let mut elements = Vec::new();
                for p in patterns {
                    // Star pattern: encoded as Tuple(["*", name])
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
                    match self.try_compile_pattern_ir(py, p)? {
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
            IR::PatternEnum {
                enum_name,
                variant_name,
            } => Ok(Some(VMPattern::Enum {
                enum_name: enum_name.clone(),
                variant_name: variant_name.clone(),
            })),
            _ => Ok(None),
        }
    }

    fn try_compile_pattern_py(&mut self, py: Python<'_>, pattern: &Bound<'_, PyAny>) -> PyResult<Option<VMPattern>> {
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
                    match self.try_compile_pattern_py(py, &sub)? {
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
                    match self.try_compile_pattern_py(py, &sub)? {
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

    /// Pre-allocate local slots for all pattern variables in match cases.
    fn collect_pattern_vars<'py>(&mut self, py: Python<'py>, cases: &CompilerNode<'py>) -> PyResult<()> {
        match cases {
            CompilerNode::Pure(ir) => {
                let items = match ir {
                    IR::Tuple(items) | IR::List(items) => items.as_slice(),
                    _ => return Ok(()),
                };
                for case in items {
                    if let IR::Tuple(case_parts) = case {
                        if !case_parts.is_empty() {
                            self.collect_pattern_vars_ir(&case_parts[0]);
                        }
                    }
                }
            }
            CompilerNode::PyObj(obj) => {
                let len = obj.len()?;
                for i in 0..len {
                    let case = obj.get_item(i)?;
                    let pattern = case.get_item(0)?;
                    self.collect_vars_from_pattern_py(py, &pattern)?;
                }
            }
        }
        Ok(())
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

    fn collect_vars_from_pattern_py(&mut self, _py: Python<'_>, pattern: &Bound<'_, PyAny>) -> PyResult<()> {
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
                self.collect_vars_from_pattern_py(_py, &p)?;
            }
        }
        Ok(())
    }

    /// Try to compile an assignment pattern for set_locals complex patterns.
    fn try_compile_assign_pattern<'py>(
        &mut self,
        py: Python<'py>,
        pattern: &CompilerNode<'py>,
    ) -> PyResult<Option<VMPattern>> {
        match pattern {
            CompilerNode::Pure(ir) => self.try_compile_assign_pattern_ir(ir),
            CompilerNode::PyObj(obj) => self.try_compile_assign_pattern_py(py, obj),
        }
    }

    fn try_compile_assign_pattern_ir(&mut self, pattern: &IR) -> PyResult<Option<VMPattern>> {
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

    fn try_compile_assign_pattern_py(
        &mut self,
        py: Python<'_>,
        pattern: &Bound<'_, PyAny>,
    ) -> PyResult<Option<VMPattern>> {
        if let Ok(tuple) = pattern.cast::<PyTuple>() {
            let mut elements = Vec::new();
            for item in tuple.iter() {
                if let Ok(star_tuple) = item.cast::<PyTuple>() {
                    if star_tuple.len() == 2 {
                        let first: String = star_tuple.get_item(0)?.extract().unwrap_or_default();
                        if first == "*" {
                            let star_name = self.extract_single_name_py(py, &star_tuple.get_item(1)?)?;
                            let star_slot = self.add_local(&star_name);
                            elements.push(VMPatternElement::Star(star_slot));
                            continue;
                        }
                    }
                }
                match self.try_compile_assign_pattern_py(py, &item)? {
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
                            let star_name = self.extract_single_name_py(py, &star_tuple.get_item(1)?)?;
                            let star_slot = self.add_local(&star_name);
                            elements.push(VMPatternElement::Star(star_slot));
                            continue;
                        }
                    }
                }
                match self.try_compile_assign_pattern_py(py, &item)? {
                    Some(sub) => elements.push(VMPatternElement::Pattern(sub)),
                    None => return Ok(None),
                }
            }
            return Ok(Some(VMPattern::Tuple(elements)));
        }

        // Ref, Lvalue, or plain string
        if let Ok(name) = self.extract_single_name_py(py, pattern) {
            let slot = self.add_local(&name);
            return Ok(Some(VMPattern::Var(slot)));
        }

        Ok(None)
    }

    /// Compile unpack pattern for for-loop tuple variable patterns.
    fn compile_unpack_pattern<'py>(
        &mut self,
        py: Python<'py>,
        pattern: &CompilerNode<'py>,
        keep_last: bool,
    ) -> PyResult<()> {
        match pattern {
            CompilerNode::Pure(ir) => self.compile_unpack_pattern_ir(ir, keep_last),
            CompilerNode::PyObj(obj) => self.compile_unpack_pattern_py(py, obj, keep_last),
        }
    }

    fn compile_unpack_pattern_ir(&mut self, ir: &IR, keep_last: bool) -> PyResult<()> {
        if let IR::Tuple(items) = ir {
            // Check for star pattern
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
                // Star pattern: store the rest list
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
                // Nested tuple pattern: recursive unpack
                if let IR::Tuple(_) = item {
                    self.compile_unpack_pattern_ir(item, false)?;
                    continue;
                }
                // Simple name
                if let Some(name) = ir_to_name(item) {
                    let slot = self.add_local(&name);
                    self.emit(VMOpCode::StoreLocal, slot as u32);
                }
            }
        }
        Ok(())
    }

    fn compile_unpack_pattern_py(
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
            let before = star_idx as u32;
            let after = (len as i32 - star_idx - 1) as u32;
            let arg = (before << 8) | after;
            self.emit(VMOpCode::UnpackEx, arg);
        } else {
            self.emit(VMOpCode::UnpackSequence, len as u32);
        }

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
                    let name = self.extract_single_name_py(py, &item.get_item(1)?)?;
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
                self.compile_unpack_pattern_py(py, &item, false)?;
            } else {
                let name = self.extract_single_name_py(py, &item)?;
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

    /// Extract a single variable name from a Python pattern node.
    fn extract_single_name_py(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<String> {
        use crate::types::catnip;

        if let Ok(s) = node.extract::<String>() {
            return Ok(s);
        }
        let node_type = node.get_type();
        let type_name = node_type.name()?;

        if type_name == catnip::LVALUE {
            return node.getattr("value")?.extract();
        }
        if type_name == catnip::REF {
            return node.getattr("ident")?.extract();
        }
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
}

impl Default for UnifiedCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{IR, IROpCode};
    use crate::vm::frame::PyCodeObject;

    #[test]
    fn test_freeze_ir_body_pure() {
        let body = IR::op(IROpCode::Mul, vec![IR::Identifier("n".into()), IR::Int(2)]);
        let node = CompilerNode::Pure(&body);
        let frozen = freeze_ir_body(&node);
        assert!(frozen.is_some(), "freeze_ir_body should return Some for Pure IR");

        // Verify the frozen bytes can be decoded back (raw bincode, no header)
        let bytes = frozen.unwrap();
        let decoded: Vec<IR> = catnip_core::freeze::decode(&bytes).unwrap();
        assert_eq!(decoded.len(), 1);
    }

    #[test]
    fn test_compile_lambda_has_encoded_ir() {
        Python::attach(|py| {
            // Build a lambda IR: (n) => { n * 2 }
            let body = IR::op(IROpCode::Mul, vec![IR::Identifier("n".into()), IR::Int(2)]);
            let params = IR::List(vec![IR::Identifier("n".into())]);
            let lambda_ir = IR::op(IROpCode::OpLambda, vec![params, body]);

            // Compile via the full pipeline
            let program = IR::Program(vec![lambda_ir]);
            let mut compiler = UnifiedCompiler::new();
            let code = compiler.compile_pure(py, &program).unwrap();

            // The top-level code should have a constant that is a PyCodeObject
            // with encoded_ir set
            let mut found_encoded_ir = false;
            for c in &code.constants {
                if c.is_pyobj() {
                    let obj = c.as_pyobject(py).unwrap();
                    let bound = obj.bind(py);
                    if let Ok(py_code) = bound.cast::<PyCodeObject>() {
                        if py_code.borrow().inner.encoded_ir.is_some() {
                            found_encoded_ir = true;
                        }
                    }
                }
            }
            assert!(
                found_encoded_ir,
                "compiled lambda should have encoded_ir in its CodeObject"
            );
        });
    }
}
