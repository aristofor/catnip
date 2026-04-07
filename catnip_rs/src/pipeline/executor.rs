// FILE: catnip_rs/src/pipeline/executor.rs
//! Standalone executor - Execute CodeObject with VMHost (no Python Context)

use std::collections::HashMap;
use std::rc::Rc;

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::vm::VM;
use crate::vm::frame::{CodeObject, Globals, NativeClosureScope};
use crate::vm::host::{NdMode, VMHost};
use crate::vm::value::FuncSlot;
use crate::vm::value::Value;

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
    /// Module policy to pass to VMHost on creation.
    module_policy: Option<Py<PyAny>>,
}

impl Executor {
    /// Create new executor
    pub fn new() -> Self {
        Self {
            vm: VM::new(),
            host: None,
            source: None,
            module_policy: None,
        }
    }

    /// Ensure host is initialized
    pub fn ensure_host(&mut self, py: Python<'_>) -> Result<(), String> {
        if self.host.is_none() {
            self.host = Some(
                VMHost::new_with_policy(py, self.module_policy.as_ref().map(|p| p.clone_ref(py)))
                    .map_err(|e| format!("Failed to initialize host: {}", e))?,
            );
        }
        Ok(())
    }

    /// Set module policy (must be called before ensure_host).
    pub fn set_module_policy(&mut self, policy: Py<PyAny>) {
        self.module_policy = Some(policy);
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

    /// Mutable access to the host (if initialized).
    pub fn host_mut(&mut self) -> Option<&mut VMHost> {
        self.host.as_mut()
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
            // Install this VM's thread-locals so to_pyobject resolves symbols correctly
            // (may have been restored to a parent VM's after execute returned).
            let prev_sym = crate::vm::value::save_symbol_table();
            let prev_enum = crate::vm::value::save_enum_registry();
            crate::vm::value::set_symbol_table(&self.vm.symbol_table as *const _ as *mut _);
            crate::vm::value::set_enum_registry(&self.vm.enum_registry as *const _ as *mut _);

            let g = host.globals().borrow();
            for (key, value) in g.iter() {
                dict.set_item(key, value.to_pyobject(py))
                    .map_err(|e| format!("Failed to export '{}': {}", key, e))?;
            }

            crate::vm::value::restore_symbol_table(prev_sym);
            crate::vm::value::restore_enum_registry(prev_enum);
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

            // After execution, transplant our enum types to the parent VM
            // (if any). execute_with_host already restored parent TL pointers.
            self.transplant_enums_to_parent();

            // Clear thread-local to avoid polluting subsequent legacy executions
            crate::vm::host::clear_vm_globals();

            result
        })
        .map_err(|e| self.enrich_error(e))
    }

    /// Transplant this executor's enum types into the parent VM's registry,
    /// accessed via the thread-local pointers (restored by execute_with_host).
    /// Then remap symbol Values in our globals and closures to match parent IDs.
    fn transplant_enums_to_parent(&mut self) {
        // Access the parent's tables via thread-locals (restored after execute)
        let parent_sym_ptr = crate::vm::value::save_symbol_table();
        let parent_enum_ptr = crate::vm::value::save_enum_registry();
        if parent_sym_ptr.is_null() || parent_enum_ptr.is_null() {
            return; // No parent VM (top-level execution)
        }
        // Skip if this IS the parent (same pointer)
        if std::ptr::eq(parent_sym_ptr, &self.vm.symbol_table as *const _ as *mut _) {
            return;
        }
        if self.vm.enum_registry.get_type(0).is_none() {
            return; // No enums to transplant
        }
        // SAFETY: pointers are valid for the duration of this call (parent VM alive)
        let parent_sym = unsafe { &mut *parent_sym_ptr };
        let parent_enum = unsafe { &mut *parent_enum_ptr };

        // Build symbol remap: child_sym_id -> parent_sym_id
        let mut remap = HashMap::new();
        let mut type_id = 0u32;
        while let Some(ety) = self.vm.enum_registry.get_type(type_id) {
            for (_, child_sym_id) in &ety.variants {
                if let Some(qname) = self.vm.symbol_table.resolve(*child_sym_id) {
                    let parent_sym_id = parent_sym.intern(qname);
                    if parent_sym_id != *child_sym_id {
                        remap.insert(*child_sym_id, parent_sym_id);
                    }
                }
            }
            let variant_names: Vec<String> = ety.variants.iter().map(|(n, _)| n.clone()).collect();
            parent_enum.register(&ety.name, &variant_names, parent_sym);
            type_id += 1;
        }
        if remap.is_empty() {
            return;
        }
        // Remap symbol Values in our globals (closures point here via Rc)
        if let Some(ref host) = self.host {
            let mut g = host.globals().borrow_mut();
            for (_, value) in g.iter_mut() {
                remap_value_in_place(value, &remap);
            }
        }
        // Remap closure captures in func_table
        for slot in self.vm.func_table.iter_mut() {
            remap_func_slot_symbols(slot, &remap);
        }
    }
}

/// Rewrite a Value in-place if it's a symbol in the remap.
fn remap_value_in_place(value: &mut Value, remap: &HashMap<u32, u32>) {
    if value.is_symbol() {
        if let Some(child_sym) = value.as_symbol() {
            if let Some(&parent_sym) = remap.get(&child_sym) {
                *value = Value::from_symbol(parent_sym);
            }
        }
    }
}

/// Remap symbol Values in a FuncSlot's closure captures.
fn remap_func_slot_symbols(slot: &mut FuncSlot, remap: &HashMap<u32, u32>) {
    if let Some(ref scope) = slot.closure {
        remap_closure_scope_symbols(scope, remap);
    }
}

/// Recursively remap symbol Values in a NativeClosureScope and its parent chain.
fn remap_closure_scope_symbols(scope: &NativeClosureScope, remap: &HashMap<u32, u32>) {
    scope.remap_symbols(remap);
}

impl Drop for Executor {
    fn drop(&mut self) {
        // Only clear thread-local pointers if they belong to THIS executor's VM.
        // A child VM (from an import) may be dropped while the parent VM is still
        // executing -- unconditionally clearing would nuke the parent's TLs.
        let my_sym = &self.vm.symbol_table as *const _ as *mut _;
        let my_enum = &self.vm.enum_registry as *const _ as *mut _;
        let my_struct = &self.vm.struct_registry as *const _;
        let my_func = &self.vm.func_table as *const _;

        if crate::vm::value::save_func_table() == my_func {
            crate::vm::value::clear_func_table();
        }
        if crate::vm::value::save_struct_registry() == my_struct {
            crate::vm::value::clear_struct_registry();
        }
        if crate::vm::value::save_symbol_table() == my_sym {
            crate::vm::value::clear_symbol_table();
        }
        if crate::vm::value::save_enum_registry() == my_enum {
            crate::vm::value::clear_enum_registry();
        }
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
