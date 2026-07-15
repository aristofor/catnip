// FILE: catnip_rs/src/vm/unified_compiler.rs
//! Unified bytecode compiler: converts both Op (PyObject) and IR inputs to CodeObject.
//!
//! Replaces the duplicated logic in `compiler.rs` (Op path) and `pure_compiler.rs` (IR path)
//! by using `CompilerNode` abstraction throughout. All `compile_*` methods are written once.

use super::compiler_core::{CompilerCore, CompilerCoreExt, LoopContext, syntax_err};
use super::compiler_input::{CompilerKwargs, CompilerNode, ir_to_name};
use super::frame::{CodeObject, PyCodeObject};
use super::opcode::VMOpCode;
use super::pattern::{VMPattern, VMPatternElement};
use super::value::Value;
use crate::core::Op;
use crate::core::pattern::*;
use crate::ir::pure::BroadcastType;
use crate::ir::{IR, IROpCode};
use catnip_core::vm::opcode::ParamCheck;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// Freeze the IR body of a lambda/function for ND process workers.
/// Only works for Pure IR nodes; returns None for PyObj nodes.
/// Uses raw bincode (no .catf header) -- for IPC transport, not disk persistence.
fn freeze_ir_body(body: &CompilerNode<'_>, params: &CompilerNode<'_>) -> Option<Arc<Vec<u8>>> {
    let CompilerNode::Pure(body_ir) = body else {
        return None;
    };
    // Element 0 is the body. Capture the params (with their type annotations) as
    // element 1 when they are pure IR, so an ND `process` worker can rebuild the
    // typed-param boundary checks (TH2-B 0b) instead of dropping them; matches
    // the PureCompiler freeze. PyObj params (rare) leave element 1 absent.
    let mut ir_vec = vec![(*body_ir).clone()];
    if let CompilerNode::Pure(params_ir) = params {
        ir_vec.push((*params_ir).clone());
    }
    catnip_core::freeze::encode(&ir_vec).ok().map(Arc::new)
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
                // SAFETY: clone() is only reached during compilation, which runs with the
                // GIL held, so a Python token is legitimately attached on this thread.
                let py = unsafe { pyo3::Python::assume_attached() };
                UCFinallyBody::PyObj(obj.clone_ref(py))
            }
        }
    }
}

pub(crate) struct FunctionCompileSpec<'a, 'py> {
    params: Vec<String>,
    body: &'a CompilerNode<'py>,
    name: &'a str,
    defaults: Vec<Value>,
    vararg_idx: i32,
    parent_nesting_depth: u32,
    /// Per-param prologue boundary check (aligned with `params`): a primitive
    /// `CheckType` code, a nominal type name (`CheckNominal`), or none. Empty
    /// means no checks.
    param_types: Vec<ParamCheck>,
}

pub struct FunctionCompileMeta<'a> {
    pub params: Vec<String>,
    /// Per-param prologue boundary check (aligned with `params`): primitive
    /// `CheckType` code, nominal type name, or none. Empty means no checks. Used
    /// by the ND `process` worker to rebuild boundary checks from frozen IR.
    pub param_types: Vec<ParamCheck>,
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
                param_types: meta.param_types,
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
                param_types: meta.param_types,
                body: &cn,
                name: meta.name,
                defaults: meta.defaults,
                vararg_idx: meta.vararg_idx,
                parent_nesting_depth: meta.parent_nesting_depth,
            },
        )
    }

    // ========== 3. Internal compile / compile_function ==========

    pub(crate) fn compile<'py>(&mut self, py: Python<'py>, node: &CompilerNode<'py>) -> PyResult<CodeObject> {
        self.reset();
        self.compile_node(py, node)?;
        self.emit(VMOpCode::Halt, 0);
        self.build_code_object(py)
    }

    pub(crate) fn compile_function<'py>(
        &mut self,
        py: Python<'py>,
        spec: FunctionCompileSpec<'_, 'py>,
    ) -> PyResult<CodeObject> {
        self.reset();
        self.name = spec.name.to_string();
        self.nargs = spec.params.len();
        self.defaults = spec.defaults;
        self.in_function = true;
        self.nesting_depth = spec.parent_nesting_depth + 1;

        for (i, param) in spec.params.iter().enumerate() {
            let slot = self.add_local(param);
            // Enforce an annotated param at the prologue so the body reads it
            // already checked: a primitive is checked-and-coerced (CheckType), a
            // nominal type is checked for membership with subtyping (CheckNominal,
            // the type name riding the `names` table).
            if let Some(check) = spec.param_types.get(i) {
                if !matches!(check, ParamCheck::None) {
                    self.emit(VMOpCode::LoadLocal, slot as u32);
                    self.emit_check_opcode(check);
                    self.emit(VMOpCode::StoreLocal, slot as u32);
                }
            }
        }

        self.compile_node(py, spec.body)?;
        self.emit(VMOpCode::Return, 0);

        let mut code = self.build_code_object(py)?;
        code.vararg_idx = spec.vararg_idx;
        Ok(code)
    }

    // ========== 4. compile_node (dispatches to compile_node_pure, compile_node_py) ==========

    pub(crate) fn compile_node<'py>(&mut self, py: Python<'py>, node: &CompilerNode<'py>) -> PyResult<()> {
        match node {
            CompilerNode::Pure(ir) => self.compile_node_pure(py, ir),
            CompilerNode::PyObj(obj) => self.compile_node_py(py, obj),
        }
    }

    // ========== 5. compile_node_pure, compile_node_py ==========

    pub(crate) fn compile_node_pure<'py>(&mut self, py: Python<'py>, node: &'py IR) -> PyResult<()> {
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
                self.core.compile_name_load(name);
                Ok(())
            }
            IR::Identifier(name) => {
                self.core.compile_name_load(name);
                Ok(())
            }

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

    pub(crate) fn compile_node_py<'py>(&mut self, py: Python<'py>, node: &Bound<'py, PyAny>) -> PyResult<()> {
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
            self.core.compile_name_load(&ident);
            return Ok(());
        }

        // Literal value
        let idx = self.add_const_pyobj(py, node);
        self.emit(VMOpCode::LoadConst, idx as u32);
        Ok(())
    }

    pub(crate) fn compile_statement_list_pure<'py>(&mut self, py: Python<'py>, stmts: &'py [IR]) -> PyResult<()> {
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

    pub(crate) fn compile_statement_list_py<'py>(
        &mut self,
        py: Python<'py>,
        stmts: &Bound<'py, PyList>,
    ) -> PyResult<()> {
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
    pub(crate) fn compile_op_py<'py>(&mut self, py: Python<'py>, _node: &Bound<'py, PyAny>, op: &Op) -> PyResult<()> {
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
    pub(crate) fn compile_call_node_py<'py>(&mut self, py: Python<'py>, node: &Bound<'py, PyAny>) -> PyResult<()> {
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
    pub(crate) fn compile_broadcast_py<'py>(&mut self, py: Python<'py>, node: &Bound<'py, PyAny>) -> PyResult<()> {
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

    /// Lower one classified boundary check to its `Check*` opcode on the top
    /// of stack. Shared by the param prologue (LoadLocal/StoreLocal wrap) and
    /// the FT2-A `CheckReturn` lowering, so the ParamCheck -> opcode mapping
    /// lives once per compiler.
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
                let uidx = self.add_union_check(members.clone()) as u32;
                self.emit(VMOpCode::CheckUnion, uidx);
            }
            check @ ParamCheck::Composite { .. } => {
                let cidx = self.add_composite_check(check.clone()) as u32;
                self.emit(VMOpCode::CheckComposite, cidx);
            }
            check @ ParamCheck::Generic { .. } => {
                let gidx = self.add_generic_check(check.clone()) as u32;
                self.emit(VMOpCode::CheckGeneric, gidx);
            }
            ParamCheck::Callable { arity } => {
                self.emit(VMOpCode::CheckCallable, *arity);
            }
            ParamCheck::None => {}
        }
    }

    pub(crate) fn compile_op_dispatch<'py>(
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
            // Typed arithmetic (TH4 canal A): analyzer-rewritten Add on proven types.
            IROpCode::AddInt => self.compile_binary(py, VMOpCode::AddInt, args),
            IROpCode::AddFloat => self.compile_binary(py, VMOpCode::AddFloat, args),
            IROpCode::SubInt => self.compile_binary(py, VMOpCode::SubInt, args),
            IROpCode::SubFloat => self.compile_binary(py, VMOpCode::SubFloat, args),
            IROpCode::MulInt => self.compile_binary(py, VMOpCode::MulInt, args),
            IROpCode::MulFloat => self.compile_binary(py, VMOpCode::MulFloat, args),
            IROpCode::DivFloat => self.compile_binary(py, VMOpCode::DivFloat, args),
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

            // FT2-A: enforce a declared-callback return on the caller side.
            // args[0] = the wrapped call, args[1] = the return annotation text;
            // lowered to the matching boundary opcode on the call's result
            // (same classification as a param prologue, no dedicated opcode).
            IROpCode::CheckReturn => {
                use catnip_core::vm::opcode::ParamCheck;
                let call = args
                    .first()
                    .ok_or_else(|| PyErr::new::<pyo3::exceptions::PySyntaxError, _>("CheckReturn without a call"))?;
                self.compile_node(py, call)?;
                let annotation = args
                    .get(1)
                    .ok_or_else(|| {
                        PyErr::new::<pyo3::exceptions::PySyntaxError, _>("CheckReturn without an annotation")
                    })?
                    .as_string()?;
                self.emit_check_opcode(&ParamCheck::from_annotation(&annotation));
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
            IROpCode::UnionDef => self.compile_union(py, args),

            IROpCode::OpTry => self.compile_try(py, args),
            IROpCode::OpRaise => self.compile_raise(py, args),

            _ => Err(pyo3::exceptions::PyNotImplementedError::new_err(format!(
                "UnifiedCompiler: cannot compile IR opcode: {}",
                opcode
            ))),
        }
    }
}

mod compile_defs_misc;
mod compile_expr;
mod compile_fn_types;

impl Default for UnifiedCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
