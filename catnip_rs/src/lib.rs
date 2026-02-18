// FILE: catnip_rs/src/lib.rs
//! Catnip Rust VM - High performance VM with NaN boxing and O(1) dispatch.
//!
//! This module provides:
//! - `RustVM`: The main VM class exposed to Python
//! - `PyVMContext`: Context for Python callbacks
//! - `Scope`: Shared O(1) scope for both VM and AST modes
//! - NaN-boxed value representation for compact, efficient execution

#![recursion_limit = "1024"]

pub mod cache;
pub mod cfg;
pub mod cli;
pub mod config;
pub mod constants;
pub mod core;
pub mod debug;
pub mod ir;
pub mod jit;
pub mod nd;
pub mod parser;
pub mod pipeline;
pub mod pragma;
pub mod repl;
pub mod semantic;
pub mod standalone;
pub mod tools;
pub mod transformer;
pub mod types;
pub mod vm;

/// Get the Tree-sitter Language for Catnip.
/// Delegated to catnip_tools which owns the grammar compilation.
pub use catnip_tools::get_language as get_tree_sitter_language;

use crate::core::{
    function, BoundCatnipMethod, CatnipMethod, Op, PatternLiteral, PatternOr, PatternStruct,
    PatternTuple, PatternVar, PatternWildcard, Ref, Registry, RustFunction, RustLambda, Scope,
    TailCall,
};
use crate::jit::HotLoopDetector;
use crate::nd::{NDFuture, NDRecur, NDScheduler, NDState};
use crate::parser::{TreeNode, TreeSitterParser};
use crate::semantic::{
    BlockFlatteningPass, BluntCodePass, CommonSubexpressionEliminationPass, ConstantFoldingPass,
    ConstantPropagationPass, CopyPropagationPass, DeadCodeEliminationPass,
    DeadStoreEliminationPass, FunctionInliningPass, OptimizationPassBase, Optimizer, Semantic,
    StrengthReductionPass, TailRecursionToLoopPass,
};
use crate::tools::{FormatConfig, Formatter};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use vm::py_interop::{
    append_constants_from_tuple, append_instructions_from_bytecode, convert_args,
};
use vm::{
    convert_code_object, CatnipStructProxy, CatnipStructType, CodeObject, Instruction, OpCode,
    PyCodeObject, PyCompiler, PyVMContext, RustClosureScope, RustVMFunction, SuperProxy,
    TraitRegistry, Value, VM,
};

/// Python-exposed VM.
#[pyclass(name = "VM")]
pub struct PyRustVM {
    vm: VM,
    context: Option<Py<PyAny>>,
}

#[pymethods]
impl PyRustVM {
    /// Create a new Rust VM.
    #[new]
    fn new() -> Self {
        Self {
            vm: VM::new(),
            context: None,
        }
    }

    /// Set the execution context.
    fn set_context(&mut self, py: Python<'_>, context: Py<PyAny>) {
        self.context = Some(context.clone_ref(py));
        self.vm.set_context(context);
    }

    /// Enable/disable execution tracing.
    fn set_trace(&mut self, enabled: bool) {
        self.vm.trace = enabled;
    }

    /// Enable/disable profiling.
    fn set_profile(&mut self, enabled: bool) {
        self.vm.profile = enabled;
    }

    /// Set source code and filename for error reporting.
    fn set_source(&mut self, source: Vec<u8>, filename: String) {
        self.vm.set_source(source, filename);
    }

    /// Get the last error context (if any).
    ///
    /// Returns a dict with keys: error_type, message, start_byte, call_stack
    /// or None if no error context is available.
    fn get_last_error_context(&mut self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self.vm.take_last_error_context() {
            Some(ctx) => {
                let dict = PyDict::new(py);
                dict.set_item("error_type", &ctx.error_type)?;
                dict.set_item("message", &ctx.message)?;
                dict.set_item("start_byte", ctx.start_byte)?;
                let call_stack: Vec<Py<PyAny>> = ctx
                    .call_stack
                    .iter()
                    .map(|(name, sb)| {
                        let t = PyTuple::new(
                            py,
                            [
                                name.clone().into_pyobject(py).unwrap().into_any().unbind(),
                                (*sb).into_pyobject(py).unwrap().into_any().unbind(),
                            ],
                        )
                        .unwrap();
                        t.into_any().unbind()
                    })
                    .collect();
                dict.set_item("call_stack", PyList::new(py, call_stack)?)?;
                Ok(dict.into_any().unbind())
            }
            None => Ok(py.None()),
        }
    }

    /// Enable JIT compilation.
    fn enable_jit(&mut self) {
        self.vm.enable_jit();
    }

    /// Enable JIT compilation with custom threshold.
    fn enable_jit_with_threshold(&mut self, threshold: u32) {
        self.vm.enable_jit_with_threshold(threshold);
    }

    /// Disable JIT compilation.
    fn disable_jit(&mut self) {
        self.vm.disable_jit();
    }

    /// Check if JIT is enabled.
    fn is_jit_enabled(&self) -> bool {
        self.vm.jit_enabled
    }

    /// Get JIT statistics.
    fn get_jit_stats(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let detector_stats = self.vm.jit_detector.stats();
        let dict = PyDict::new(py);
        dict.set_item("total_loops_tracked", detector_stats.total_loops_tracked)?;
        dict.set_item("hot_loops", detector_stats.hot_loops)?;
        dict.set_item("tracing_loops", detector_stats.tracing_loops)?;

        // Get compiled count from executor if available
        let compiled_loops = if let Ok(jit) = self.vm.jit.lock() {
            if let Some(ref executor) = *jit {
                executor.stats().compiled_traces
            } else {
                detector_stats.compiled_loops
            }
        } else {
            detector_stats.compiled_loops
        };
        dict.set_item("compiled_loops", compiled_loops)?;

        // Cached traces on disk
        let cached_traces = if let Ok(jit) = self.vm.jit.lock() {
            if let Some(ref executor) = *jit {
                executor.trace_cache().len()
            } else {
                0
            }
        } else {
            0
        };
        dict.set_item("cached_traces", cached_traces)?;

        Ok(dict.into())
    }

    /// Get profile counts.
    fn get_profile_counts(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = PyDict::new(py);
        for (&op, &count) in &self.vm.profile_counts {
            if let Some(opcode) = OpCode::from_u8(op) {
                dict.set_item(format!("{:?}", opcode), count)?;
            }
        }
        Ok(dict.into())
    }

    /// Set the debug callback (called at breakpoints and step events).
    fn set_debug_callback(&mut self, callback: Option<Py<PyAny>>) {
        self.vm.debug_callback = callback;
    }

    /// Add a debug breakpoint at a source byte offset.
    fn add_debug_breakpoint(&mut self, start_byte: u32) {
        self.vm.debug_breakpoints.insert(start_byte);
    }

    /// Remove a debug breakpoint at a source byte offset.
    fn remove_debug_breakpoint(&mut self, start_byte: u32) {
        self.vm.debug_breakpoints.remove(&start_byte);
    }

    /// Clear all debug breakpoints.
    fn clear_debug_breakpoints(&mut self) {
        self.vm.debug_breakpoints.clear();
    }

    /// Execute a code object.
    ///
    /// Args:
    ///     code: A Catnip CodeObject (from catnip.vm.frame)
    ///     args: Positional arguments tuple
    ///     kwargs: Keyword arguments dict (optional)
    ///     closure_scope: Optional closure scope for captured variables
    ///
    /// Returns:
    ///     The execution result
    #[pyo3(signature = (code, args=None, kwargs=None, closure_scope=None))]
    #[allow(unused_variables)]
    fn execute(
        &mut self,
        py: Python<'_>,
        code: &Bound<'_, PyAny>,
        args: Option<&Bound<'_, PyTuple>>,
        kwargs: Option<&Bound<'_, PyDict>>,
        closure_scope: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        // Convert Python CodeObject to Rust CodeObject
        let rust_code = convert_code_object(py, code)?;

        // Convert args
        let rust_args = if let Some(a) = args {
            convert_args(py, a)?
        } else {
            Vec::new()
        };

        // Execute with optional closure scope
        let result = self
            .vm
            .execute_with_closure(py, rust_code, &rust_args, closure_scope)
            .map_err(PyErr::from)?;

        // Sync globals back to Python context
        if let Some(ref ctx) = self.context {
            let ctx_bound = ctx.bind(py);
            // Access context.globals (a dict)
            if let Ok(py_globals) = ctx_bound.getattr("globals") {
                for (name, value) in self.vm.get_globals() {
                    let py_value = value.to_pyobject(py);
                    py_globals.set_item(name, py_value)?;
                }
            }
        }

        // Convert result back to Python
        Ok(result.to_pyobject(py))
    }

    /// Compile IR to bytecode and execute in one step.
    ///
    /// This is the fast path for Catnip execution - compilation and execution
    /// happen entirely in Rust without returning to Python.
    ///
    /// Args:
    ///     ir_node: An Op node or list of Op nodes from semantic analysis
    ///
    /// Returns:
    ///     The execution result
    fn compile_and_run(
        &mut self,
        py: Python<'_>,
        ir_node: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        // Compile IR to bytecode
        let mut compiler = vm::Compiler::new();
        let code = compiler.compile(py, ir_node)?;

        // Execute
        let result = self.vm.execute(py, code, &[]).map_err(PyErr::from)?;

        // Convert result back to Python
        Ok(result.to_pyobject(py))
    }

    /// Execute simple bytecode for testing.
    ///
    /// Args:
    ///     bytecode: List of (opcode, arg) tuples
    ///     constants: List of constant values
    ///
    /// Returns:
    ///     The execution result
    fn execute_simple(
        &mut self,
        py: Python<'_>,
        bytecode: &Bound<'_, PyTuple>,
        constants: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        let mut code = CodeObject::new("simple");

        append_instructions_from_bytecode(bytecode, &mut code)?;
        append_constants_from_tuple(py, constants, &mut code)?;

        // Execute
        let result = self.vm.execute(py, code, &[]).map_err(PyErr::from)?;

        Ok(result.to_pyobject(py))
    }

    /// Benchmark: run simple arithmetic loop.
    fn benchmark_add(&mut self, py: Python<'_>, iterations: i64) -> PyResult<Py<PyAny>> {
        const NLOCALS: usize = 2;
        const LOCAL_I: u32 = 0;
        const LOCAL_SUM: u32 = 1;

        const ZERO: i64 = 0;
        const ONE: i64 = 1;

        const CONST_I: u32 = 0;
        const CONST_SUM: u32 = 1;
        const CONST_LIMIT: u32 = 2;
        const CONST_INC: u32 = 3;

        const LOOP_START_IP: u32 = 4;
        const LOOP_END_IP: u32 = 17;

        let mut code = CodeObject::new("bench_add");
        code.nlocals = NLOCALS; // i, sum

        // i = 0
        code.constants.push(Value::from_int(ZERO));
        // sum = 0
        code.constants.push(Value::from_int(ZERO));
        // limit
        code.constants.push(Value::from_int(iterations));
        // increment
        code.constants.push(Value::from_int(ONE));

        code.instructions = vec![
            // 0: i = 0
            Instruction::new(OpCode::LoadConst, CONST_I),
            // 1:
            Instruction::new(OpCode::StoreLocal, LOCAL_I),
            // 2: sum = 0
            Instruction::new(OpCode::LoadConst, CONST_SUM),
            // 3:
            Instruction::new(OpCode::StoreLocal, LOCAL_SUM),
            // 4: loop start - if i >= iterations: jump to end
            Instruction::new(OpCode::LoadLocal, LOCAL_I),
            // 5:
            Instruction::new(OpCode::LoadConst, CONST_LIMIT),
            // 6:
            Instruction::simple(OpCode::Lt),
            // 7: Jump to 17 (LoadLocal for return) if condition is false
            Instruction::new(OpCode::JumpIfFalse, LOOP_END_IP),
            // 8: sum = sum + i
            Instruction::new(OpCode::LoadLocal, LOCAL_SUM),
            // 9:
            Instruction::new(OpCode::LoadLocal, LOCAL_I),
            // 10:
            Instruction::simple(OpCode::Add),
            // 11:
            Instruction::new(OpCode::StoreLocal, LOCAL_SUM),
            // 12: i = i + 1
            Instruction::new(OpCode::LoadLocal, LOCAL_I),
            // 13:
            Instruction::new(OpCode::LoadConst, CONST_INC),
            // 14:
            Instruction::simple(OpCode::Add),
            // 15:
            Instruction::new(OpCode::StoreLocal, LOCAL_I),
            // 16: jump to loop start
            Instruction::new(OpCode::Jump, LOOP_START_IP),
            // 17: end - return sum
            Instruction::new(OpCode::LoadLocal, LOCAL_SUM),
            // 18:
            Instruction::simple(OpCode::Halt),
        ];

        let result = self.vm.execute(py, code, &[]).map_err(PyErr::from)?;

        Ok(result.to_pyobject(py))
    }
}

/// Python module definition.
#[pymodule]
fn _rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyRustVM>()?;
    m.add_class::<PyVMContext>()?;
    m.add_class::<Scope>()?;
    m.add_class::<Op>()?;
    m.add_class::<Registry>()?;
    m.add_class::<PyCompiler>()?;
    m.add_class::<PyCodeObject>()?;
    m.add_class::<TreeSitterParser>()?;
    m.add_class::<TreeNode>()?;
    m.add_class::<OptimizationPassBase>()?;
    m.add_class::<Optimizer>()?;
    m.add_class::<BluntCodePass>()?;
    m.add_class::<BlockFlatteningPass>()?;
    m.add_class::<ConstantPropagationPass>()?;
    m.add_class::<ConstantFoldingPass>()?;
    m.add_class::<CopyPropagationPass>()?;
    m.add_class::<FunctionInliningPass>()?;
    m.add_class::<DeadStoreEliminationPass>()?;
    m.add_class::<DeadCodeEliminationPass>()?;
    m.add_class::<StrengthReductionPass>()?;
    m.add_class::<CommonSubexpressionEliminationPass>()?;
    m.add_class::<TailRecursionToLoopPass>()?;
    m.add_class::<Semantic>()?;
    m.add_class::<HotLoopDetector>()?;
    m.add_class::<RustFunction>()?;
    m.add_class::<RustLambda>()?;
    m.add_class::<RustVMFunction>()?;
    m.add_class::<RustClosureScope>()?;

    // Pattern classes
    m.add_class::<PatternLiteral>()?;
    m.add_class::<PatternVar>()?;
    m.add_class::<PatternWildcard>()?;
    m.add_class::<PatternOr>()?;
    m.add_class::<PatternTuple>()?;
    m.add_class::<PatternStruct>()?;

    // Struct types and method descriptors
    m.add_class::<CatnipStructProxy>()?;
    m.add_class::<CatnipStructType>()?;
    m.add_class::<SuperProxy>()?;
    m.add_class::<CatnipMethod>()?;
    m.add_class::<BoundCatnipMethod>()?;

    // Trait registry
    m.add_class::<TraitRegistry>()?;

    // Node classes
    m.add_class::<Ref>()?;
    m.add_class::<TailCall>()?;

    // ND module classes
    m.add_class::<NDState>()?;
    m.add_class::<NDFuture>()?;
    m.add_class::<NDRecur>()?;
    m.add_class::<NDScheduler>()?;

    // Register function module functions
    function::register_module(m)?;

    // Register REPL functions
    m.add_function(wrap_pyfunction!(repl::should_continue_multiline, m)?)?;
    m.add_function(wrap_pyfunction!(repl::preprocess_multiline, m)?)?;
    m.add_function(wrap_pyfunction!(repl::parse_repl_command, m)?)?;

    // Standalone pipeline
    m.add_class::<standalone::PyStandalonePipeline>()?;

    // Register pragma classes
    pragma::register_module(m)?;

    // Register transformer classes
    transformer::register_module(m)?;

    // Register JSON serialization
    ir::json::register_module(m)?;

    // Register pipeline functions
    pipeline::init_module(m)?;

    // Register CFG module
    cfg::register_module(m.py(), m)?;

    // Register formatting tools (delegated to catnip_tools plugin)
    m.add_class::<FormatConfig>()?;
    m.add_class::<Formatter>()?;
    m.add_function(wrap_pyfunction!(tools::shims::format_code, m)?)?;

    // Register linting tools (delegated to catnip_tools plugin)
    m.add_class::<tools::Severity>()?;
    m.add_class::<tools::Diagnostic>()?;
    m.add_class::<tools::LintConfig>()?;
    m.add_function(wrap_pyfunction!(tools::shims::lint_code, m)?)?;

    // Register debugger tools (delegated to catnip_tools plugin)
    m.add_class::<tools::PyDebugCommandKind>()?;
    m.add_class::<tools::PyParsedDebugCommand>()?;
    m.add_class::<tools::PySourceMap>()?;
    m.add_function(wrap_pyfunction!(
        tools::debugger_shims::parse_debug_command,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        tools::debugger_shims::format_debug_help,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        tools::debugger_shims::format_debug_header,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        tools::debugger_shims::format_debug_pause,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        tools::debugger_shims::format_debug_vars,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        tools::debugger_shims::format_debug_backtrace,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        tools::debugger_shims::format_debug_unknown_command,
        m
    )?)?;

    // Register config module
    config::register_module(m)?;

    // Register cache module
    cache::register_module(m)?;

    // Register debug module
    debug::register_module(m)?;

    // Export opcode constants for compatibility
    let opcodes = PyDict::new(m.py());
    opcodes.set_item("LOAD_CONST", OpCode::LoadConst as u8)?;
    opcodes.set_item("LOAD_LOCAL", OpCode::LoadLocal as u8)?;
    opcodes.set_item("STORE_LOCAL", OpCode::StoreLocal as u8)?;
    opcodes.set_item("LOAD_NAME", OpCode::LoadScope as u8)?;
    opcodes.set_item("STORE_NAME", OpCode::StoreScope as u8)?;
    opcodes.set_item("LOAD_GLOBAL", OpCode::LoadGlobal as u8)?;
    opcodes.set_item("POP_TOP", OpCode::PopTop as u8)?;
    opcodes.set_item("DUP_TOP", OpCode::DupTop as u8)?;
    opcodes.set_item("ROT_TWO", OpCode::RotTwo as u8)?;
    opcodes.set_item("ADD", OpCode::Add as u8)?;
    opcodes.set_item("SUB", OpCode::Sub as u8)?;
    opcodes.set_item("MUL", OpCode::Mul as u8)?;
    opcodes.set_item("DIV", OpCode::Div as u8)?;
    opcodes.set_item("FLOORDIV", OpCode::FloorDiv as u8)?;
    opcodes.set_item("MOD", OpCode::Mod as u8)?;
    opcodes.set_item("POW", OpCode::Pow as u8)?;
    opcodes.set_item("NEG", OpCode::Neg as u8)?;
    opcodes.set_item("POS", OpCode::Pos as u8)?;
    opcodes.set_item("BOR", OpCode::BOr as u8)?;
    opcodes.set_item("BXOR", OpCode::BXor as u8)?;
    opcodes.set_item("BAND", OpCode::BAnd as u8)?;
    opcodes.set_item("BNOT", OpCode::BNot as u8)?;
    opcodes.set_item("LSHIFT", OpCode::LShift as u8)?;
    opcodes.set_item("RSHIFT", OpCode::RShift as u8)?;
    opcodes.set_item("LT", OpCode::Lt as u8)?;
    opcodes.set_item("LE", OpCode::Le as u8)?;
    opcodes.set_item("GT", OpCode::Gt as u8)?;
    opcodes.set_item("GE", OpCode::Ge as u8)?;
    opcodes.set_item("EQ", OpCode::Eq as u8)?;
    opcodes.set_item("NE", OpCode::Ne as u8)?;
    opcodes.set_item("NOT", OpCode::Not as u8)?;
    opcodes.set_item("JUMP", OpCode::Jump as u8)?;
    opcodes.set_item("JUMP_IF_FALSE", OpCode::JumpIfFalse as u8)?;
    opcodes.set_item("JUMP_IF_TRUE", OpCode::JumpIfTrue as u8)?;
    opcodes.set_item("JUMP_IF_FALSE_OR_POP", OpCode::JumpIfFalseOrPop as u8)?;
    opcodes.set_item("JUMP_IF_TRUE_OR_POP", OpCode::JumpIfTrueOrPop as u8)?;
    opcodes.set_item("GET_ITER", OpCode::GetIter as u8)?;
    opcodes.set_item("FOR_ITER", OpCode::ForIter as u8)?;
    opcodes.set_item("CALL", OpCode::Call as u8)?;
    opcodes.set_item("CALL_KW", OpCode::CallKw as u8)?;
    opcodes.set_item("TAILCALL", OpCode::TailCall as u8)?;
    opcodes.set_item("RETURN", OpCode::Return as u8)?;
    opcodes.set_item("MAKE_FUNCTION", OpCode::MakeFunction as u8)?;
    opcodes.set_item("BUILD_LIST", OpCode::BuildList as u8)?;
    opcodes.set_item("BUILD_TUPLE", OpCode::BuildTuple as u8)?;
    opcodes.set_item("BUILD_SET", OpCode::BuildSet as u8)?;
    opcodes.set_item("BUILD_DICT", OpCode::BuildDict as u8)?;
    opcodes.set_item("BUILD_SLICE", OpCode::BuildSlice as u8)?;
    opcodes.set_item("GETATTR", OpCode::GetAttr as u8)?;
    opcodes.set_item("SETATTR", OpCode::SetAttr as u8)?;
    opcodes.set_item("GETITEM", OpCode::GetItem as u8)?;
    opcodes.set_item("SETITEM", OpCode::SetItem as u8)?;
    opcodes.set_item("PUSH_BLOCK", OpCode::PushBlock as u8)?;
    opcodes.set_item("POP_BLOCK", OpCode::PopBlock as u8)?;
    opcodes.set_item("BREAK", OpCode::Break as u8)?;
    opcodes.set_item("CONTINUE", OpCode::Continue as u8)?;
    opcodes.set_item("BROADCAST", OpCode::Broadcast as u8)?;
    opcodes.set_item("MATCH_PATTERN", OpCode::MatchPattern as u8)?;
    opcodes.set_item("BIND_MATCH", OpCode::BindMatch as u8)?;
    opcodes.set_item("JUMP_IF_NONE", OpCode::JumpIfNone as u8)?;
    opcodes.set_item("UNPACK_SEQUENCE", OpCode::UnpackSequence as u8)?;
    opcodes.set_item("UNPACK_EX", OpCode::UnpackEx as u8)?;
    opcodes.set_item("NOP", OpCode::Nop as u8)?;
    opcodes.set_item("HALT", OpCode::Halt as u8)?;
    m.add("OPCODES", opcodes)?;
    m.add(
        "JIT_PURE_BUILTINS",
        PyTuple::new(m.py(), crate::constants::JIT_PURE_BUILTINS.iter().copied())?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_initialization() {
        Python::initialize();
        Python::attach(|_py| {
            let vm = PyRustVM::new();
            assert!(vm.context.is_none());
        });
    }
}
