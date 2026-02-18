// FILE: catnip_rs/src/standalone/mod.rs
//! Standalone pipeline - Source → TreeSitter → IRPure → OpPure → Bytecode → VM
//!
//! Full Rust pipeline with no Python dependencies for standalone execution.

use crate::ir::IRPure;
use crate::parser::transform_pure;
use crate::vm::Value;
use pyo3::prelude::*;
use std::time::Instant;
use tree_sitter::Parser;

pub mod convert;
pub mod executor;
pub mod semantic;

pub use executor::StandaloneExecutor;
pub use semantic::SemanticAnalyzer;

/// Detailed per-phase pipeline timings
#[derive(Debug, Default)]
pub struct PipelineTimings {
    /// Parse: source → tree-sitter AST
    pub parse_us: u64,
    /// Transform + semantic: AST → IRPure → optimized
    pub compile_us: u64,
    /// Compile bytecode + VM execution
    pub execute_us: u64,
    /// Total end-to-end
    pub total_us: u64,
}

/// Complete standalone pipeline.
///
/// Each call to `execute()` creates a fresh executor: the VM and Context
/// are ephemeral. Variables from one statement are visible to the next
/// *within the same call*, but nothing persists across calls.
///
/// For a persistent context between evaluations, use the REPL.
pub struct StandalonePipeline {
    parser: Parser,
}

impl StandalonePipeline {
    /// Create a new standalone pipeline
    pub fn new() -> Result<Self, String> {
        let language = crate::get_tree_sitter_language();
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| format!("Failed to set language: {}", e))?;

        Ok(Self { parser })
    }

    /// Execute source code and return the result
    pub fn execute(&mut self, source: &str) -> Result<Value, String> {
        // 1. Parse: Source → Tree-sitter AST
        let tree = self
            .parser
            .parse(source, None)
            .ok_or("Failed to parse source")?;

        let root = tree.root_node();

        // Check for syntax errors
        if let Some(error_msg) = catnip_tools::errors::find_errors(root, source) {
            return Err(error_msg);
        }

        // 2. Transform: AST → IRPure
        let ir = transform_pure(root, source)?;

        // 3. Semantic: IRPure → OpPure (validation + optimisation)
        let mut semantic = SemanticAnalyzer::new();
        let _optimized = semantic.analyze(&ir)?;

        // 4. Compile & Execute: IRPure → Op → Bytecode → VM
        // Pour préserver les variables entre statements, on compile et exécute
        // chaque statement séparément dans le même executor
        Python::attach(|py| {
            let mut executor = StandaloneExecutor::new();
            let mut last_result = Value::NIL;

            match &_optimized {
                IRPure::Program(items) if items.is_empty() => {
                    return Ok(Value::NIL);
                }
                IRPure::Program(items) => {
                    // Exécuter chaque statement séparément dans le même context
                    for item in items.iter() {
                        // Skip None/empty statements (e.g., comments)
                        if matches!(item, IRPure::None) {
                            continue;
                        }

                        let op = convert::irpure_to_op(py, item)
                            .map_err(|e| format!("Conversion error: {}", e))?;

                        let mut compiler = crate::vm::compiler::Compiler::new();
                        let code = compiler
                            .compile(py, op.bind(py))
                            .map_err(|e| format!("Compilation error: {}", e))?;

                        last_result = executor.execute(code)?;
                    }

                    // Convert PyObject result to native Value if possible
                    let final_result = if last_result.is_pyobj() {
                        unsafe {
                            if let Some(ptr) = last_result.as_pyobj_ptr() {
                                let bound = pyo3::Bound::from_borrowed_ptr(py, ptr);
                                Value::from_pyobject(py, &bound).unwrap_or(last_result)
                            } else {
                                last_result
                            }
                        }
                    } else {
                        last_result
                    };

                    Ok(final_result)
                }
                other => {
                    // Single statement
                    let op = convert::irpure_to_op(py, other)
                        .map_err(|e| format!("Conversion error: {}", e))?;

                    let mut compiler = crate::vm::compiler::Compiler::new();
                    let code = compiler
                        .compile(py, op.bind(py))
                        .map_err(|e| format!("Compilation error: {}", e))?;

                    executor.execute(code)
                }
            }
        })
    }

    /// Simplified version without debug output
    pub fn execute_quiet(&mut self, source: &str) -> Result<Value, String> {
        let tree = self
            .parser
            .parse(source, None)
            .ok_or("Failed to parse source")?;

        let root = tree.root_node();
        let ir = transform_pure(root, source)?;

        let mut semantic = SemanticAnalyzer::new();
        let optimized = semantic.analyze(&ir)?;

        // Extract first statement from Program (transform_pure returns Program of statements)
        let to_compile = match &optimized {
            IRPure::Program(items) if items.len() == 1 => &items[0],
            IRPure::Program(items) if items.is_empty() => {
                return Ok(Value::NIL); // Empty input
            }
            IRPure::Program(items) => {
                // Multiple statements: wrap in block
                &IRPure::op(crate::ir::IROpCode::OpBlock, items.clone())
            }
            other => other, // Single node (shouldn't happen from transform_pure)
        };

        // Compile + VM
        Python::attach(|py| {
            let op = convert::irpure_to_op(py, to_compile)
                .map_err(|e| format!("Conversion error: {}", e))?;

            let mut compiler = crate::vm::compiler::Compiler::new();
            let code = compiler
                .compile(py, op.bind(py))
                .map_err(|e| format!("Compilation error: {}", e))?;

            let mut executor = StandaloneExecutor::new();
            executor.execute(code)
        })
    }

    /// Execute with detailed per-phase timings
    pub fn execute_timed(&mut self, source: &str) -> Result<(Value, PipelineTimings), String> {
        let total_start = Instant::now();
        let mut timings = PipelineTimings::default();

        // Phase 1: Parse (tree-sitter)
        let parse_start = Instant::now();
        let tree = self
            .parser
            .parse(source, None)
            .ok_or("Failed to parse source")?;

        let root = tree.root_node();
        if let Some(error_msg) = catnip_tools::errors::find_errors(root, source) {
            return Err(error_msg);
        }
        timings.parse_us = parse_start.elapsed().as_micros() as u64;

        // Phase 2: Transform + Semantic (AST → IRPure → optimized)
        let compile_start = Instant::now();
        let ir = transform_pure(root, source)?;
        let mut semantic = SemanticAnalyzer::new();
        let optimized = semantic.analyze(&ir)?;
        timings.compile_us = compile_start.elapsed().as_micros() as u64;

        // Phase 3: Bytecode compilation + VM execution
        let exec_start = Instant::now();
        let result = Python::attach(|py| {
            let mut executor = StandaloneExecutor::new();
            let mut last_result = Value::NIL;

            match &optimized {
                IRPure::Program(items) if items.is_empty() => Ok(Value::NIL),
                IRPure::Program(items) => {
                    for item in items.iter() {
                        if matches!(item, IRPure::None) {
                            continue;
                        }
                        let op = convert::irpure_to_op(py, item)
                            .map_err(|e| format!("Conversion error: {}", e))?;
                        let mut compiler = crate::vm::compiler::Compiler::new();
                        let code = compiler
                            .compile(py, op.bind(py))
                            .map_err(|e| format!("Compilation error: {}", e))?;
                        last_result = executor.execute(code)?;
                    }

                    let final_result = if last_result.is_pyobj() {
                        unsafe {
                            if let Some(ptr) = last_result.as_pyobj_ptr() {
                                let bound = pyo3::Bound::from_borrowed_ptr(py, ptr);
                                Value::from_pyobject(py, &bound).unwrap_or(last_result)
                            } else {
                                last_result
                            }
                        }
                    } else {
                        last_result
                    };
                    Ok(final_result)
                }
                other => {
                    let op = convert::irpure_to_op(py, other)
                        .map_err(|e| format!("Conversion error: {}", e))?;
                    let mut compiler = crate::vm::compiler::Compiler::new();
                    let code = compiler
                        .compile(py, op.bind(py))
                        .map_err(|e| format!("Compilation error: {}", e))?;
                    executor.execute(code)
                }
            }
        })?;
        timings.execute_us = exec_start.elapsed().as_micros() as u64;

        timings.total_us = total_start.elapsed().as_micros() as u64;
        Ok((result, timings))
    }
}

impl Default for StandalonePipeline {
    fn default() -> Self {
        Self::new().expect("Failed to create standalone pipeline")
    }
}

/// PyO3 wrapper exposing StandalonePipeline to Python.
#[pyclass(name = "StandalonePipeline")]
pub struct PyStandalonePipeline {
    inner: StandalonePipeline,
}

#[pymethods]
impl PyStandalonePipeline {
    #[new]
    fn new() -> PyResult<Self> {
        Ok(Self {
            inner: StandalonePipeline::new()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e))?,
        })
    }

    /// Execute source code and return the result as a PyObject.
    fn execute(&mut self, py: Python, source: &str) -> PyResult<Py<PyAny>> {
        let value = self
            .inner
            .execute(source)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e))?;
        Ok(value.to_pyobject(py))
    }

    /// Version without debug output.
    fn execute_quiet(&mut self, py: Python, source: &str) -> PyResult<Py<PyAny>> {
        let value = self
            .inner
            .execute_quiet(source)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e))?;
        Ok(value.to_pyobject(py))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_creation() {
        let pipeline = StandalonePipeline::new();
        assert!(pipeline.is_ok());
    }

    #[test]
    fn test_parse_number() {
        let mut pipeline = StandalonePipeline::new().unwrap();
        let tree = pipeline.parser.parse("42", None).unwrap();
        let root = tree.root_node();
        let ir = transform_pure(root, "42").unwrap();

        // L'IR devrait être un Program contenant un Int(42)
        match ir {
            IRPure::Program(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0], IRPure::Int(42));
            }
            _ => panic!("Expected Program, got {:?}", ir),
        }
    }

    #[test]
    fn test_parse_addition() {
        let mut pipeline = StandalonePipeline::new().unwrap();
        let tree = pipeline.parser.parse("2 + 3", None).unwrap();
        let root = tree.root_node();
        let ir = transform_pure(root, "2 + 3").unwrap();

        match ir {
            IRPure::Program(items) => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    IRPure::Op { opcode, .. } => {
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
        let mut pipeline = StandalonePipeline::new().unwrap();
        let result = pipeline.execute_quiet("2 + 3").unwrap();
        assert!(result.is_int());
        assert_eq!(result.as_int(), Some(5));
    }

    #[test]
    fn test_end_to_end_number() {
        let mut pipeline = StandalonePipeline::new().unwrap();
        let result = pipeline.execute_quiet("42").unwrap();
        assert!(result.is_int());
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_end_to_end_complex() {
        let mut pipeline = StandalonePipeline::new().unwrap();
        // (10 - 3) * 2
        let result = pipeline.execute_quiet("(10 - 3) * 2").unwrap();
        assert!(result.is_int());
        assert_eq!(result.as_int(), Some(14));
    }

    #[test]
    fn test_end_to_end_for_loop() {
        let mut pipeline = StandalonePipeline::new().unwrap();

        // for i in list(1, 2, 3) { i }
        let result = pipeline.execute_quiet("for i in list(1, 2, 3) { i }");

        // For loop returns NIL
        assert!(result.is_ok(), "Execution failed: {:?}", result.err());
        assert!(result.unwrap().is_nil());
    }

    #[test]
    fn test_end_to_end_lambda() {
        let mut pipeline = StandalonePipeline::new().unwrap();

        // Lambda avec assignation puis appel
        let result = pipeline.execute("double = (x) => { x * 2 }; double(5)");

        assert!(result.is_ok(), "Execution failed: {:?}", result.err());
        let val = result.unwrap();

        assert!(val.is_int(), "Expected int, got {:?}", val);
        assert_eq!(val.as_int(), Some(10));
    }

    #[test]
    fn test_end_to_end_closure() {
        let mut pipeline = StandalonePipeline::new().unwrap();

        // Closure capturing variable
        let result = pipeline.execute_quiet("x = 5; add_x = (y) => { x + y }; add_x(3)");

        assert!(result.is_ok(), "Execution failed: {:?}", result.err());
        let val = result.unwrap();
        assert!(val.is_int(), "Expected int, got {:?}", val);
        assert_eq!(val.as_int(), Some(8));
    }
}
