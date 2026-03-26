// FILE: catnip_rs/src/pipeline/mod.rs
//! Standalone pipeline - Source → TreeSitter → IR → Bytecode → VM
//!
//! Full Rust pipeline: IR compiled directly to bytecode via UnifiedCompiler.

use crate::ir::IR;
use crate::parser::transform_pure;
use crate::vm::Value;
use crate::vm::host::{GlobalsProxy, NdMode};
use crate::vm::unified_compiler::UnifiedCompiler;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;
use tree_sitter::Parser;

pub mod executor;

// Re-exported from catnip_core (pure Rust, no PyO3)
pub use catnip_core::pipeline::semantic;

pub use executor::Executor;
pub use semantic::SemanticAnalyzer;

/// Detailed per-phase pipeline timings
#[derive(Debug, Default)]
pub struct PipelineTimings {
    /// Parse: source → tree-sitter AST
    pub parse_us: u64,
    /// Transform + semantic: AST → IR → optimized
    pub compile_us: u64,
    /// Compile bytecode + VM execution
    pub execute_us: u64,
    /// Total end-to-end
    pub total_us: u64,
}

/// Complete standalone pipeline with persistent context.
///
/// The executor (VM + VMHost) is reused across `execute()` calls:
/// variables, functions, and struct types persist between evaluations.
pub struct Pipeline {
    parser: Parser,
    /// Source file path for relative import resolution (META.file).
    source_path: Option<String>,
    /// Persistent executor (VM + host). Lazily initialized on first execute().
    executor: Option<Executor>,
    /// JIT enabled (default: true)
    jit_enabled: bool,
    /// JIT hot detection threshold
    jit_threshold: u32,
    /// TCO enabled (tail-call marking in semantic analyzer)
    tco_enabled: bool,
    /// Optimization enabled (false = skip all semantic optimization passes)
    optimize_enabled: bool,
    /// Prepared (parsed + analyzed) IR, ready for compile+execute.
    prepared_ir: Option<IR>,
}

impl Pipeline {
    /// Create a new standalone pipeline
    pub fn new() -> Result<Self, String> {
        let language = crate::get_tree_sitter_language();
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| format!("Failed to set language: {}", e))?;

        Ok(Self {
            parser,
            source_path: None,
            executor: None,
            jit_enabled: false,
            jit_threshold: crate::constants::JIT_THRESHOLD_DEFAULT,
            tco_enabled: true,
            optimize_enabled: true,
            prepared_ir: None,
        })
    }

    /// Set source file path for relative import resolution.
    pub fn set_source_path(&mut self, path: &str) {
        self.source_path = Some(path.to_string());
    }

    pub fn set_jit_enabled(&mut self, enabled: bool) {
        self.jit_enabled = enabled;
        if let Some(ref mut executor) = self.executor {
            if enabled {
                executor.enable_jit_with_threshold(self.jit_threshold);
            } else {
                executor.disable_jit();
            }
        }
    }

    pub fn set_jit_threshold(&mut self, threshold: u32) {
        self.jit_threshold = threshold;
        if self.jit_enabled {
            if let Some(ref mut executor) = self.executor {
                executor.enable_jit_with_threshold(threshold);
            }
        }
    }

    pub fn set_tco_enabled(&mut self, enabled: bool) {
        self.tco_enabled = enabled;
    }

    pub fn set_optimize_enabled(&mut self, enabled: bool) {
        self.optimize_enabled = enabled;
    }

    /// Create a SemanticAnalyzer with current pipeline settings applied.
    fn create_analyzer(&self) -> SemanticAnalyzer {
        let mut analyzer = if self.optimize_enabled {
            SemanticAnalyzer::with_optimizer()
        } else {
            SemanticAnalyzer::new()
        };
        analyzer.set_tco_enabled(self.tco_enabled);
        analyzer
    }

    /// Set ND broadcast mode ("sequential", "thread", or "process").
    pub fn set_nd_mode(&mut self, mode: &str) {
        let nd_mode = match mode {
            "thread" | "parallel" => NdMode::Thread,
            "process" => NdMode::Process,
            _ => NdMode::Sequential,
        };
        if let Some(ref mut executor) = self.executor {
            executor.set_nd_mode(nd_mode);
        }
    }

    /// Set ND memoization on/off.
    pub fn set_nd_memoize(&mut self, memoize: bool) {
        if let Some(ref mut executor) = self.executor {
            executor.set_nd_memoize(memoize);
        }
    }

    /// Get or create the persistent executor, reinstalling thread-local tables.
    pub fn ensure_executor(&mut self) -> &mut Executor {
        if self.executor.is_none() {
            let mut exec = Executor::new();
            if self.jit_enabled {
                exec.enable_jit_with_threshold(self.jit_threshold);
            }
            self.executor = Some(exec);
        }
        let executor = self.executor.as_mut().unwrap();
        // Reinstall thread-locals (may have been overwritten by other code)
        executor.install_tables();
        executor
    }

    /// Reset the persistent context. Next execute() starts fresh.
    pub fn reset(&mut self) {
        self.executor = None;
        self.prepared_ir = None;
    }

    /// Parse + transform (+ optional semantic) and return the IR tree.
    /// Used by `-p 1/2` and MCP `parse_catnip` for inspection.
    pub fn parse_to_ir(&mut self, source: &str, semantic: bool) -> Result<IR, String> {
        let tree = self.parser.parse(source, None).ok_or("Failed to parse source")?;
        let root = tree.root_node();

        if let Some(error_msg) = catnip_tools::errors::find_errors(root, source) {
            return Err(error_msg);
        }

        let ir = transform_pure(root, source)?;
        if semantic {
            let mut analyzer = self.create_analyzer();
            analyzer.analyze(&ir)
        } else {
            Ok(ir)
        }
    }

    /// Parse + transform + semantic analysis, storing the optimized IR
    /// for later execution via `execute_prepared()`.
    pub fn prepare(&mut self, source: &str) -> Result<(), String> {
        let tree = self.parser.parse(source, None).ok_or("Failed to parse source")?;
        let root = tree.root_node();

        if let Some(error_msg) = catnip_tools::errors::find_errors(root, source) {
            return Err(error_msg);
        }

        let ir = transform_pure(root, source)?;
        let mut semantic = self.create_analyzer();
        let optimized = semantic.analyze(&ir)?;
        self.prepared_ir = Some(optimized);
        Ok(())
    }

    /// Compile + execute from previously prepared IR (set by `prepare()`).
    pub fn execute_prepared(&mut self) -> Result<Value, String> {
        if self.prepared_ir.is_none() {
            return Err("No prepared IR. Call prepare() first.".to_string());
        }

        self.ensure_executor();
        Python::attach(|py| {
            let ir = self.prepared_ir.as_ref().unwrap();
            let executor = self.executor.as_mut().unwrap();
            if let Some(ref path) = self.source_path {
                executor.set_source_path(py, path)?;
            }
            let mut compiler = UnifiedCompiler::new();
            let code = compiler
                .compile_pure(py, ir)
                .map_err(|e| format!("Compilation error: {}", e))?;

            let last_result = executor.execute(Arc::new(code))?;

            if last_result.is_pyobj() {
                let py_obj = last_result.to_pyobject(py);
                Ok(Value::from_pyobject(py, py_obj.bind(py)).unwrap_or(last_result))
            } else {
                Ok(last_result)
            }
        })
    }

    /// Execute source code and return the result
    pub fn execute(&mut self, source: &str) -> Result<Value, String> {
        // 1. Parse: Source → Tree-sitter AST
        let tree = self.parser.parse(source, None).ok_or("Failed to parse source")?;

        let root = tree.root_node();

        // Check for syntax errors
        if let Some(error_msg) = catnip_tools::errors::find_errors(root, source) {
            return Err(error_msg);
        }

        // 2. Transform: AST → IR
        let ir = transform_pure(root, source)?;

        // 3. Semantic: IR → IR (validation + optimisation)
        let mut semantic = self.create_analyzer();
        let _optimized = semantic.analyze(&ir)?;

        // 4. Compile & Execute: IR → Bytecode → VM (direct, no Op Python)
        self.ensure_executor();
        let filename = self.source_path.as_deref().unwrap_or("<input>");
        self.executor.as_mut().unwrap().set_source(source.as_bytes(), filename);
        Python::attach(|py| {
            let executor = self.executor.as_mut().unwrap();
            if let Some(ref path) = self.source_path {
                executor.set_source_path(py, path)?;
            }
            let mut compiler = UnifiedCompiler::new();
            let code = compiler
                .compile_pure(py, &_optimized)
                .map_err(|e| format!("Compilation error: {}", e))?;

            let last_result = executor.execute(Arc::new(code))?;

            // Convert PyObject result to native Value if possible
            if last_result.is_pyobj() {
                let py_obj = last_result.to_pyobject(py);
                Ok(Value::from_pyobject(py, py_obj.bind(py)).unwrap_or(last_result))
            } else {
                Ok(last_result)
            }
        })
    }

    /// Simplified version without debug output
    pub fn execute_quiet(&mut self, source: &str) -> Result<Value, String> {
        let tree = self.parser.parse(source, None).ok_or("Failed to parse source")?;

        let root = tree.root_node();

        // Check for syntax errors (same as execute)
        if let Some(error_msg) = catnip_tools::errors::find_errors(root, source) {
            return Err(error_msg);
        }

        let ir = transform_pure(root, source)?;

        let mut semantic = self.create_analyzer();
        let optimized = semantic.analyze(&ir)?;

        // Extract first statement from Program (transform_pure returns Program of statements)
        let to_compile = match &optimized {
            IR::Program(items) if items.len() == 1 => &items[0],
            IR::Program(items) if items.is_empty() => {
                return Ok(Value::NIL); // Empty input
            }
            IR::Program(items) => {
                // Multiple statements: wrap in block
                &IR::op(crate::ir::IROpCode::OpBlock, items.clone())
            }
            other => other, // Single node (shouldn't happen from transform_pure)
        };

        // Compile + VM
        self.ensure_executor();
        let filename = self.source_path.as_deref().unwrap_or("<input>");
        self.executor.as_mut().unwrap().set_source(source.as_bytes(), filename);
        Python::attach(|py| {
            let executor = self.executor.as_mut().unwrap();
            if let Some(ref path) = self.source_path {
                executor.set_source_path(py, path)?;
            }

            let mut compiler = UnifiedCompiler::new();
            let code = compiler
                .compile_pure(py, to_compile)
                .map_err(|e| format!("Compilation error: {}", e))?;

            executor.execute(Arc::new(code))
        })
    }

    /// Execute with detailed per-phase timings
    pub fn execute_timed(&mut self, source: &str) -> Result<(Value, PipelineTimings), String> {
        let total_start = Instant::now();
        let mut timings = PipelineTimings::default();

        // Phase 1: Parse (tree-sitter)
        let parse_start = Instant::now();
        let tree = self.parser.parse(source, None).ok_or("Failed to parse source")?;

        let root = tree.root_node();
        if let Some(error_msg) = catnip_tools::errors::find_errors(root, source) {
            return Err(error_msg);
        }
        timings.parse_us = parse_start.elapsed().as_micros() as u64;

        // Phase 2: Transform + Semantic (AST → IR → optimized)
        let compile_start = Instant::now();
        let ir = transform_pure(root, source)?;
        let mut semantic = self.create_analyzer();
        let optimized = semantic.analyze(&ir)?;
        timings.compile_us = compile_start.elapsed().as_micros() as u64;

        // Phase 3: Bytecode compilation + VM execution
        let exec_start = Instant::now();
        self.ensure_executor();
        let filename = self.source_path.as_deref().unwrap_or("<input>");
        self.executor.as_mut().unwrap().set_source(source.as_bytes(), filename);
        let result = Python::attach(|py| {
            let executor = self.executor.as_mut().unwrap();
            if let Some(ref path) = self.source_path {
                executor.set_source_path(py, path)?;
            }
            let mut compiler = UnifiedCompiler::new();
            let code = compiler
                .compile_pure(py, &optimized)
                .map_err(|e| format!("Compilation error: {}", e))?;

            let last_result = executor.execute(Arc::new(code))?;

            let final_result = if last_result.is_pyobj() {
                let py_obj = last_result.to_pyobject(py);
                Value::from_pyobject(py, py_obj.bind(py)).unwrap_or(last_result)
            } else {
                last_result
            };
            Ok::<_, String>(final_result)
        })?;
        timings.execute_us = exec_start.elapsed().as_micros() as u64;

        timings.total_us = total_start.elapsed().as_micros() as u64;
        Ok((result, timings))
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new().expect("Failed to create standalone pipeline")
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        // executor's Drop clears thread-local pointers
        self.executor = None;
    }
}

/// PyO3 wrapper exposing Pipeline to Python.
/// `unsendable` because VMHost contains Arc<RefCell<_>> (not Sync).
#[pyclass(name = "Pipeline", unsendable)]
pub struct PyPipeline {
    inner: Pipeline,
}

#[pymethods]
impl PyPipeline {
    #[new]
    fn new() -> PyResult<Self> {
        Ok(Self {
            inner: Pipeline::new().map_err(PyErr::new::<pyo3::exceptions::PyValueError, _>)?,
        })
    }

    /// Set source file path for relative import resolution.
    fn set_source_path(&mut self, path: &str) {
        self.inner.set_source_path(path);
    }

    /// Reset the persistent context. Next execute() starts fresh.
    fn reset(&mut self) {
        self.inner.reset();
    }

    /// Parse + transform + semantic analysis without execution.
    /// Raises on syntax/semantic errors (same as standard Catnip.parse).
    fn check(&mut self, source: &str) -> PyResult<()> {
        let tree = self
            .inner
            .parser
            .parse(source, None)
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Failed to parse"))?;
        let root = tree.root_node();
        if let Some(error_msg) = catnip_tools::errors::find_errors(root, source) {
            return Err(PyErr::new::<pyo3::exceptions::PySyntaxError, _>(error_msg));
        }
        let ir = transform_pure(root, source).map_err(PyErr::new::<pyo3::exceptions::PySyntaxError, _>)?;
        let mut semantic = self.inner.create_analyzer();
        semantic
            .analyze(&ir)
            .map_err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>)?;
        Ok(())
    }

    /// Return prepared IR as Python Op nodes (for AST mode execution).
    fn prepared_ir_to_op(&self, py: Python) -> PyResult<Py<PyAny>> {
        let ir = self
            .inner
            .prepared_ir
            .as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("No prepared IR"))?;
        crate::ir::ir_pure_to_python(py, ir.clone())
    }

    /// Return prepared IR as PyIRNode list (no re-parse).
    fn get_prepared_ir_nodes(&self) -> PyResult<Vec<crate::ir::PyIRNode>> {
        let ir = self
            .inner
            .prepared_ir
            .as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("No prepared IR"))?;
        Ok(crate::ir::PyIRNode::unwrap_program(ir.clone()))
    }

    /// Parse + transform (+ optional semantic) and return the IR tree.
    #[pyo3(signature = (source, semantic=true))]
    fn parse_to_ir(&mut self, source: &str, semantic: bool) -> PyResult<Vec<crate::ir::PyIRNode>> {
        let ir = self
            .inner
            .parse_to_ir(source, semantic)
            .map_err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>)?;
        Ok(crate::ir::PyIRNode::unwrap_program(ir))
    }

    /// Parse + transform + semantic analysis, storing the optimized IR.
    fn prepare(&mut self, source: &str) -> PyResult<()> {
        self.inner
            .prepare(source)
            .map_err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>)
    }

    /// Compile + execute from previously prepared IR.
    fn execute_prepared(&mut self, py: Python) -> PyResult<Py<PyAny>> {
        let value = self
            .inner
            .execute_prepared()
            .map_err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>)?;
        Ok(value.to_pyobject(py))
    }

    /// Execute source code and return the result as a PyObject.
    fn execute(&mut self, py: Python, source: &str) -> PyResult<Py<PyAny>> {
        let value = self
            .inner
            .execute(source)
            .map_err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>)?;
        Ok(value.to_pyobject(py))
    }

    /// Get the last error context from the VM (start_byte, error_type, message).
    /// Returns a dict or None.
    fn get_last_error_context(&mut self, py: Python) -> PyResult<Py<PyAny>> {
        if let Some(ref mut executor) = self.inner.executor {
            if let Some(ctx) = executor.take_last_error_context() {
                let dict = PyDict::new(py);
                dict.set_item("error_type", &ctx.error_type)?;
                dict.set_item("message", &ctx.message)?;
                dict.set_item("start_byte", ctx.start_byte)?;
                // Expose call stack as list of (func_name, start_byte) tuples
                let cs: Vec<(String, u32)> = ctx.call_stack;
                dict.set_item("call_stack", cs)?;
                return Ok(dict.into_any().unbind());
            }
        }
        Ok(py.None())
    }

    /// Enable or disable tail-call optimization.
    fn set_tco_enabled(&mut self, enabled: bool) {
        self.inner.set_tco_enabled(enabled);
    }

    /// Enable or disable optimization passes (optimize=0 disables).
    fn set_optimize_enabled(&mut self, enabled: bool) {
        self.inner.set_optimize_enabled(enabled);
    }

    /// Enable or disable JIT compilation.
    fn set_jit_enabled(&mut self, enabled: bool) {
        self.inner.set_jit_enabled(enabled);
    }

    /// Set JIT hot detection threshold.
    fn set_jit_threshold(&mut self, threshold: u32) {
        self.inner.set_jit_threshold(threshold);
    }

    /// Set ND broadcast mode ("sequential" or "thread").
    fn set_nd_mode(&mut self, mode: &str) {
        self.inner.set_nd_mode(mode);
    }

    /// Set ND memoization on/off.
    fn set_nd_memoize(&mut self, memoize: bool) {
        self.inner.set_nd_memoize(memoize);
    }

    /// Version without debug output.
    fn execute_quiet(&mut self, py: Python, source: &str) -> PyResult<Py<PyAny>> {
        let value = self
            .inner
            .execute_quiet(source)
            .map_err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>)?;
        Ok(value.to_pyobject(py))
    }

    /// Get a global variable by name. Returns None if not found.
    fn get_global(&mut self, py: Python, name: &str) -> PyResult<Option<Py<PyAny>>> {
        if let Some(executor) = &self.inner.executor {
            if let Some(globals) = executor.globals() {
                let g = globals.borrow();
                return Ok(g.get(name).map(|v| v.to_pyobject(py)));
            }
        }
        Ok(None)
    }

    /// Get a GlobalsProxy dict-like object over the internal globals.
    fn globals(&mut self, py: Python) -> PyResult<GlobalsProxy> {
        // Ensure executor + host are initialized
        let executor = self.inner.ensure_executor();
        executor
            .ensure_host(py)
            .map_err(pyo3::exceptions::PyRuntimeError::new_err)?;
        let globals_arc = executor
            .globals()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Host not initialized"))?;
        Ok(GlobalsProxy::new(Rc::clone(globals_arc)))
    }

    /// Set the Python context (enables pass_context and registry access).
    fn set_context(&mut self, py: Python, context: &Bound<'_, pyo3::types::PyAny>) -> PyResult<()> {
        let executor = self.inner.ensure_executor();
        executor
            .set_context(py, context.clone().unbind())
            .map_err(pyo3::exceptions::PyRuntimeError::new_err)
    }

    /// Inject all entries from a Python dict into the pipeline's Rust globals.
    fn inject_globals(&mut self, py: Python, dict: &Bound<'_, PyDict>) -> PyResult<()> {
        let executor = self.inner.ensure_executor();
        executor
            .inject_from_pydict(py, dict)
            .map_err(pyo3::exceptions::PyRuntimeError::new_err)
    }

    /// Export the pipeline's Rust globals into a Python dict.
    fn export_globals(&mut self, py: Python, dict: &Bound<'_, PyDict>) -> PyResult<()> {
        if let Some(ref executor) = self.inner.executor {
            executor
                .export_to_pydict(py, dict)
                .map_err(pyo3::exceptions::PyRuntimeError::new_err)?;
        }
        Ok(())
    }

    /// Set a global variable.
    fn set_global(&mut self, py: Python, name: &str, value: Bound<'_, pyo3::types::PyAny>) -> PyResult<()> {
        let executor = self.inner.ensure_executor();
        executor
            .ensure_host(py)
            .map_err(pyo3::exceptions::PyRuntimeError::new_err)?;
        let globals_arc = executor
            .globals()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Host not initialized"))?;
        let val = Value::from_pyobject(py, &value).map_err(pyo3::exceptions::PyValueError::new_err)?;
        globals_arc.borrow_mut().insert(name.to_string(), val);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_creation() {
        let pipeline = Pipeline::new();
        assert!(pipeline.is_ok());
    }

    #[test]
    fn test_parse_number() {
        let mut pipeline = Pipeline::new().unwrap();
        let tree = pipeline.parser.parse("42", None).unwrap();
        let root = tree.root_node();
        let ir = transform_pure(root, "42").unwrap();

        // L'IR devrait être un Program contenant un Int(42)
        match ir {
            IR::Program(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0], IR::Int(42));
            }
            _ => panic!("Expected Program, got {:?}", ir),
        }
    }

    #[test]
    fn test_parse_addition() {
        let mut pipeline = Pipeline::new().unwrap();
        let tree = pipeline.parser.parse("2 + 3", None).unwrap();
        let root = tree.root_node();
        let ir = transform_pure(root, "2 + 3").unwrap();

        match ir {
            IR::Program(items) => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    IR::Op { opcode, .. } => {
                        assert_eq!(*opcode, crate::ir::IROpCode::Add);
                    }
                    _ => panic!("Expected Op node"),
                }
            }
            _ => panic!("Expected Program"),
        }
    }

    #[test]
    fn test_end_to_end_addition() {
        let mut pipeline = Pipeline::new().unwrap();
        let result = pipeline.execute_quiet("2 + 3").unwrap();
        assert!(result.is_int());
        assert_eq!(result.as_int(), Some(5));
    }

    #[test]
    fn test_end_to_end_number() {
        let mut pipeline = Pipeline::new().unwrap();
        let result = pipeline.execute_quiet("42").unwrap();
        assert!(result.is_int());
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_end_to_end_complex() {
        let mut pipeline = Pipeline::new().unwrap();
        // (10 - 3) * 2
        let result = pipeline.execute_quiet("(10 - 3) * 2").unwrap();
        assert!(result.is_int());
        assert_eq!(result.as_int(), Some(14));
    }

    #[test]
    fn test_end_to_end_for_loop() {
        let mut pipeline = Pipeline::new().unwrap();

        // for i in list(1, 2, 3) { i }
        let result = pipeline.execute_quiet("for i in list(1, 2, 3) { i }");

        // For loop returns NIL
        assert!(result.is_ok(), "Execution failed: {:?}", result.err());
        assert!(result.unwrap().is_nil());
    }

    #[test]
    fn test_end_to_end_lambda() {
        let mut pipeline = Pipeline::new().unwrap();

        // Lambda avec assignation puis appel
        let result = pipeline.execute("double = (x) => { x * 2 }; double(5)");

        assert!(result.is_ok(), "Execution failed: {:?}", result.err());
        let val = result.unwrap();

        assert!(val.is_int(), "Expected int, got {:?}", val);
        assert_eq!(val.as_int(), Some(10));
    }

    #[test]
    fn test_end_to_end_closure() {
        let mut pipeline = Pipeline::new().unwrap();

        // Closure capturing variable
        let result = pipeline.execute_quiet("x = 5; add_x = (y) => { x + y }; add_x(3)");

        assert!(result.is_ok(), "Execution failed: {:?}", result.err());
        let val = result.unwrap();
        assert!(val.is_int(), "Expected int, got {:?}", val);
        assert_eq!(val.as_int(), Some(8));
    }

    #[test]
    fn test_persistence_variables() {
        let mut pipeline = Pipeline::new().unwrap();

        // Call 1: define variable
        pipeline.execute("x = 42").unwrap();

        // Call 2: read variable from previous call
        let result = pipeline.execute("x + 8").unwrap();
        assert_eq!(result.as_int(), Some(50));
    }

    #[test]
    fn test_persistence_functions() {
        let mut pipeline = Pipeline::new().unwrap();

        // Call 1: define function
        pipeline.execute("double = (n) => { n * 2 }").unwrap();

        // Call 2: call function from previous call
        let result = pipeline.execute("double(21)").unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_persistence_mutation() {
        let mut pipeline = Pipeline::new().unwrap();

        // Call 1: define
        pipeline.execute("counter = 0").unwrap();

        // Call 2: mutate
        pipeline.execute("counter = counter + 1").unwrap();

        // Call 3: read mutated value
        let result = pipeline.execute("counter").unwrap();
        assert_eq!(result.as_int(), Some(1));
    }

    #[test]
    fn test_persistence_structs() {
        let mut pipeline = Pipeline::new().unwrap();

        // Call 1: define struct
        pipeline.execute("struct Point { x; y }").unwrap();

        // Call 2: instantiate struct from previous call
        let result = pipeline.execute("p = Point(3, 4); p.x + p.y").unwrap();
        assert_eq!(result.as_int(), Some(7));
    }

    #[test]
    fn test_reset() {
        let mut pipeline = Pipeline::new().unwrap();

        // Define variable
        pipeline.execute("x = 42").unwrap();

        // Reset clears state
        pipeline.reset();

        // Variable should no longer exist
        let result = pipeline.execute("x");
        assert!(result.is_err(), "Expected error after reset, got {:?}", result);
    }
}
