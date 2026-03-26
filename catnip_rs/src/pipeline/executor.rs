// FILE: catnip_rs/src/pipeline/executor.rs
//! Standalone executor - Execute CodeObject with VMHost (no Python Context)

use crate::vm::VM;
use crate::vm::frame::{CodeObject, Globals, NativeClosureScope};
use crate::vm::host::{NdMode, VMHost};
use crate::vm::value::Value;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::rc::Rc;

/// Standalone executor with optional persistence across calls.
///
/// Uses `VMHost` instead of a Python Context. Builtins are provided
/// directly from Rust, no `py.import("catnip.context")` needed.
///
/// When reused across multiple `execute()` calls, the VM's struct_registry,
/// func_table, and the host's globals HashMap persist - variables, functions,
/// and struct types defined in earlier calls remain available.
pub struct Executor {
    vm: VM,
    host: Option<VMHost>,
    /// Source code for error snippet generation (bytes + filename)
    source: Option<(Vec<u8>, String)>,
}

impl Executor {
    /// Create new executor
    pub fn new() -> Self {
        Self {
            vm: VM::new(),
            host: None,
            source: None,
        }
    }

    /// Ensure host is initialized
    pub fn ensure_host(&mut self, py: Python<'_>) -> Result<(), String> {
        if self.host.is_none() {
            self.host = Some(VMHost::new(py).map_err(|e| format!("Failed to initialize host: {}", e))?);
        }
        Ok(())
    }

    /// Install thread-local struct registry for the VM.
    /// Must be called before any `Value::from_pyobject` that checks struct ownership.
    pub fn install_tables(&mut self) {
        crate::vm::value::set_struct_registry(&self.vm.struct_registry as *const _);
    }

    /// Access the host (if initialized).
    pub fn host(&self) -> Option<&VMHost> {
        self.host.as_ref()
    }

    /// Mutable access to the VM.
    pub fn vm_mut(&mut self) -> &mut VM {
        &mut self.vm
    }

    /// Set META.file on the host for relative import resolution.
    pub fn set_source_path(&mut self, py: Python<'_>, path: &str) -> Result<(), String> {
        self.ensure_host(py)?;
        self.host
            .as_ref()
            .unwrap()
            .set_meta_file(py, path)
            .map_err(|e| format!("Failed to set META.file: {}", e))
    }

    pub fn enable_jit(&mut self) {
        self.vm.enable_jit();
    }

    pub fn enable_jit_with_threshold(&mut self, threshold: u32) {
        self.vm.enable_jit_with_threshold(threshold);
    }

    pub fn disable_jit(&mut self) {
        self.vm.disable_jit();
    }

    /// Set ND broadcast parallelism mode.
    pub fn set_nd_mode(&mut self, mode: NdMode) {
        if let Some(ref mut host) = self.host {
            host.set_nd_mode(mode);
        }
    }

    /// Set ND memoization on/off.
    pub fn set_nd_memoize(&mut self, memoize: bool) {
        if let Some(ref mut host) = self.host {
            host.set_nd_memoize(memoize);
        }
    }

    /// Get a clone of the globals Arc (for GlobalsProxy).
    pub fn globals(&self) -> Option<&Globals> {
        self.host.as_ref().map(|h| h.globals())
    }

    /// Set the Python context on the host (enables pass_context and registry).
    pub fn set_context(&mut self, py: Python<'_>, context: pyo3::Py<pyo3::PyAny>) -> Result<(), String> {
        self.ensure_host(py)?;
        self.host.as_mut().unwrap().set_context(context);
        Ok(())
    }

    /// Bulk-inject all entries from a PyDict into the host's globals.
    pub fn inject_from_pydict(&mut self, py: Python<'_>, dict: &Bound<'_, PyDict>) -> Result<(), String> {
        self.ensure_host(py)?;
        let globals = self.host.as_ref().unwrap().globals();
        let mut g = globals.borrow_mut();
        for (key, value) in dict.iter() {
            if let Ok(name) = key.extract::<String>() {
                if let Ok(val) = Value::from_pyobject(py, &value) {
                    g.insert(name, val);
                }
            }
        }
        Ok(())
    }

    /// Export the host's globals into a PyDict.
    pub fn export_to_pydict(&self, py: Python<'_>, dict: &Bound<'_, PyDict>) -> Result<(), String> {
        if let Some(ref host) = self.host {
            let g = host.globals().borrow();
            for (key, value) in g.iter() {
                dict.set_item(key, value.to_pyobject(py))
                    .map_err(|e| format!("Failed to export '{}': {}", key, e))?;
            }
        }
        Ok(())
    }

    /// Get the last error context from the VM (if any).
    pub fn take_last_error_context(&mut self) -> Option<crate::vm::core::ErrorContext> {
        self.vm.take_last_error_context()
    }

    /// Set source code for error snippet generation.
    pub fn set_source(&mut self, source: &[u8], filename: &str) {
        self.source = Some((source.to_vec(), filename.to_string()));
        self.vm.set_source(source.to_vec(), filename.to_string());
    }

    /// Enrich an error message with a source snippet if ErrorContext is available.
    /// Does NOT consume the ErrorContext -- it remains available for Python callers.
    fn enrich_error(&self, msg: String) -> String {
        let ctx = match self.vm.last_error_context.as_ref() {
            Some(c) => c,
            None => return msg,
        };
        let (ref source_bytes, ref filename) = match self.source {
            Some(ref s) => s,
            None => return msg,
        };
        if ctx.start_byte == 0 && msg.contains("parse") {
            return msg;
        }
        let mut sm = catnip_tools::sourcemap::SourceMap::new(source_bytes.clone(), filename.clone());
        let snippet = sm.get_snippet(ctx.start_byte as usize, ctx.start_byte as usize + 1, 0);
        format!("{msg}\n{snippet}")
    }

    /// Execute a function CodeObject with arguments and optional closure scope (for ND workers).
    pub fn execute_function(
        &mut self,
        code: std::sync::Arc<CodeObject>,
        args: &[Value],
        closure_scope: Option<NativeClosureScope>,
    ) -> Result<Value, String> {
        Python::attach(|py| {
            self.ensure_host(py)?;
            let host = self.host.as_ref().unwrap();
            crate::vm::host::set_vm_globals(Rc::clone(host.globals()));
            let result = self
                .vm
                .execute_with_host(py, code, args, host, closure_scope)
                .map_err(|e| format!("{e}"));
            crate::vm::host::clear_vm_globals();
            result
        })
        .map_err(|e| self.enrich_error(e))
    }

    /// Execute CodeObject and return result
    pub fn execute(&mut self, code: std::sync::Arc<CodeObject>) -> Result<Value, String> {
        Python::attach(|py| {
            self.ensure_host(py)?;

            let host = self.host.as_ref().unwrap();

            // Install globals for Python callbacks that re-enter Catnip
            // (e.g. cached() wrapper calling a VMFunction)
            crate::vm::host::set_vm_globals(Rc::clone(host.globals()));

            let result = self
                .vm
                .execute_with_host(py, code, &[], host, None)
                .map_err(|e| format!("{e}"));

            // Clear thread-local to avoid polluting subsequent legacy executions
            crate::vm::host::clear_vm_globals();

            result
        })
        .map_err(|e| self.enrich_error(e))
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        // Clear thread-local pointers to avoid dangling references
        // after the VM (and its func_table/struct_registry) is dropped.
        crate::vm::value::clear_func_table();
        crate::vm::value::clear_struct_registry();
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::IR;
    use crate::ir::IROpCode;
    use crate::vm::unified_compiler::UnifiedCompiler;

    #[test]
    fn test_executor_literal() {
        Python::attach(|py| {
            let ir = IR::Int(42);
            let mut compiler = UnifiedCompiler::new();
            let code = compiler.compile_pure(py, &ir).unwrap();

            let mut executor = Executor::new();
            executor.install_tables();
            let result = executor.execute(std::sync::Arc::new(code)).unwrap();

            assert!(result.is_int());
            assert_eq!(result.as_int(), Some(42));
        });
    }

    #[test]
    fn test_executor_addition() {
        Python::attach(|py| {
            let ir = IR::op(IROpCode::Add, vec![IR::Int(2), IR::Int(3)]);
            let mut compiler = UnifiedCompiler::new();
            let code = compiler.compile_pure(py, &ir).unwrap();

            let mut executor = Executor::new();
            executor.install_tables();
            let result = executor.execute(std::sync::Arc::new(code)).unwrap();

            assert!(result.is_int());
            assert_eq!(result.as_int(), Some(5));
        });
    }

    #[test]
    fn test_executor_complex_expression() {
        Python::attach(|py| {
            // (2 + 3) * 4
            let ir = IR::op(
                IROpCode::Mul,
                vec![IR::op(IROpCode::Add, vec![IR::Int(2), IR::Int(3)]), IR::Int(4)],
            );
            let mut compiler = UnifiedCompiler::new();
            let code = compiler.compile_pure(py, &ir).unwrap();

            let mut executor = Executor::new();
            executor.install_tables();
            let result = executor.execute(std::sync::Arc::new(code)).unwrap();

            assert!(result.is_int());
            assert_eq!(result.as_int(), Some(20));
        });
    }
}
