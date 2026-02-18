// FILE: catnip_rs/src/standalone/executor.rs
//! Standalone executor - Execute CodeObject with PyO3 embedded

use crate::vm::frame::CodeObject;
use crate::vm::value::Value;
use crate::vm::VM;
use pyo3::prelude::*;

/// Standalone executor — meant to be created per `StandalonePipeline::execute()` call.
///
/// The Context is lazily initialized on the first `execute()` then reused
/// for subsequent statements within the same program. No persistence across
/// instances: each `StandaloneExecutor::new()` starts from scratch.
pub struct StandaloneExecutor {
    vm: VM,
    context: Option<Py<PyAny>>,
}

impl StandaloneExecutor {
    /// Create new executor
    pub fn new() -> Self {
        Self {
            vm: VM::new(),
            context: None,
        }
    }

    /// Ensure context is initialized
    fn ensure_context(&mut self, py: Python<'_>) -> PyResult<()> {
        if self.context.is_none() {
            let locals = pyo3::types::PyDict::new(py);
            let context_module = py.import("catnip.context")?;
            let context_class = context_module.getattr("Context")?;
            let context = context_class.call0()?;
            context.setattr("locals", locals)?;

            // Create and attach Registry for pattern matching
            let registry_module = py.import("catnip._rs")?;
            let registry_class = registry_module.getattr("Registry")?;
            let registry = registry_class.call1((context.clone(),))?;
            context.setattr("_registry", registry)?;

            self.context = Some(context.unbind());
        }
        Ok(())
    }

    /// Get the context (for debugging)
    pub fn get_context(&self) -> Option<&Py<PyAny>> {
        self.context.as_ref()
    }

    /// Execute CodeObject and return result
    pub fn execute(&mut self, code: CodeObject) -> Result<Value, String> {
        Python::attach(|py| {
            // Ensure context exists and reuse it
            self.ensure_context(py)
                .map_err(|e| format!("Failed to initialize context: {}", e))?;

            // Set context in VM
            self.vm
                .set_context(self.context.as_ref().unwrap().clone_ref(py));

            // Execute with empty args
            let args: Vec<Value> = Vec::new();

            self.vm
                .execute(py, code, &args)
                .map_err(|e| format!("VM execution error: {:?}", e))
        })
    }
}

impl Default for StandaloneExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::IROpCode;
    use crate::ir::IRPure;
    use crate::standalone::convert;
    use crate::vm::compiler::Compiler;

    #[test]
    fn test_executor_literal() {
        Python::attach(|py| {
            let ir = IRPure::Int(42);
            let op = convert::irpure_to_op(py, &ir).unwrap();
            let mut compiler = Compiler::new();
            let code = compiler.compile(py, op.bind(py)).unwrap();

            let mut executor = StandaloneExecutor::new();
            let result = executor.execute(code).unwrap();

            assert!(result.is_int());
            assert_eq!(result.as_int(), Some(42));
        });
    }

    #[test]
    fn test_executor_addition() {
        Python::attach(|py| {
            let ir = IRPure::op(IROpCode::Add, vec![IRPure::Int(2), IRPure::Int(3)]);
            let op = convert::irpure_to_op(py, &ir).unwrap();
            let mut compiler = Compiler::new();
            let code = compiler.compile(py, op.bind(py)).unwrap();

            let mut executor = StandaloneExecutor::new();
            let result = executor.execute(code).unwrap();

            assert!(result.is_int());
            assert_eq!(result.as_int(), Some(5));
        });
    }

    #[test]
    fn test_executor_complex_expression() {
        Python::attach(|py| {
            // (2 + 3) * 4
            let ir = IRPure::op(
                IROpCode::Mul,
                vec![
                    IRPure::op(IROpCode::Add, vec![IRPure::Int(2), IRPure::Int(3)]),
                    IRPure::Int(4),
                ],
            );
            let op = convert::irpure_to_op(py, &ir).unwrap();
            let mut compiler = Compiler::new();
            let code = compiler.compile(py, op.bind(py)).unwrap();

            let mut executor = StandaloneExecutor::new();
            let result = executor.execute(code).unwrap();

            assert!(result.is_int());
            assert_eq!(result.as_int(), Some(20));
        });
    }
}
