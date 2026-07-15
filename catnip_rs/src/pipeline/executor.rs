// FILE: catnip_rs/src/pipeline/executor.rs
//! Standalone executor - Execute CodeObject with VMHost (no Python Context)

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use crate::vm::VM;
use crate::vm::frame::{CodeObject, Globals, NativeClosureScope, VMFunction};
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
            // Serialize the first `catnip` import across test threads
            // (importlib raises _DeadlockError under parallel first-imports).
            // The GIL is released while waiting so the warming thread can run.
            #[cfg(test)]
            py.detach(crate::test_support::init_python);
            // Make the registry resolvable by id (idempotent) so the import and
            // extension `GlobalsProxy`s can release a displaced struct global
            // struct-aware, even if no struct proxy was ever materialized.
            self.register_struct_registry();
            let registry_id = self.vm.struct_registry.id();
            self.host = Some(
                VMHost::new_with_policy(py, self.module_policy.as_ref().map(|p| p.clone_ref(py)), registry_id)
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

    /// This VM's struct registry id -- lets a `GlobalsProxy` over this host's
    /// globals release a displaced TAG_STRUCT against the registry that owns it.
    pub fn struct_registry_id(&self) -> u64 {
        self.vm.struct_registry.id()
    }

    /// Enter this VM's struct registry into the identity table so a `GlobalsProxy`
    /// can resolve it by id (`proxy_registry_decref` / `with_struct_registry_installed`)
    /// even when no struct proxy was ever materialized to seed the table.
    /// Idempotent; the registry removes itself on `Drop`.
    pub fn register_struct_registry(&self) {
        crate::vm::value::register_struct_registry(self.vm.struct_registry.id(), &self.vm.struct_registry as *const _);
    }

    /// Set the Python context on the host (enables pass_context and registry).
    pub fn set_context(&mut self, py: Python<'_>, context: pyo3::Py<pyo3::PyAny>) -> Result<(), String> {
        self.ensure_host(py)?;
        self.host.as_mut().unwrap().set_context(context);
        Ok(())
    }

    /// GC traverse: the executor's own policy ref, then the host's owned
    /// references (context, policy, globals handles). See `VMHost::gc_traverse`.
    pub fn gc_traverse(&self, visit: &pyo3::gc::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        if let Some(ref policy) = self.module_policy {
            visit.call(policy)?;
        }
        if let Some(host) = &self.host {
            host.gc_traverse(visit)?;
        }
        self.vm.gc_traverse(visit)?;
        Ok(())
    }

    /// GC clear: drop the references reported by `gc_traverse`.
    pub fn gc_clear(&mut self) {
        self.module_policy = None;
        if let Some(host) = &mut self.host {
            host.gc_clear();
        }
        self.vm.gc_clear();
    }

    /// Ledger probe: live instance slots of this VM's struct registry
    /// (idx, refcount, type name).
    pub fn debug_instance_slots(&self) -> Vec<(u32, u32, String)> {
        self.vm.struct_registry.debug_instance_slots()
    }

    /// Ledger probe: summed refcount of this VM's live struct instance slots
    /// (the struct counterpart of `OBJECT_TABLE`'s `refs`).
    pub fn debug_instance_rc_sum(&self) -> u64 {
        self.vm.struct_registry.debug_instance_rc_sum()
    }

    /// Release an owned `Value` against THIS VM's struct registry (struct-aware:
    /// a TAG_STRUCT needs the registry in hand, `Value::decref` is a deliberate
    /// no-op on it and leaked one registry count otherwise -- the bug documented
    /// on `VMHost::store_global`). Used for both a value displaced from a map
    /// (globals overwrite) and a Halt-popped result (`consume_result`) -- hence
    /// not named `release_displaced`.
    pub fn release_owned(&mut self, val: Value) {
        crate::vm::core::decref_discard(&self.vm.struct_registry, val);
    }

    /// Bulk-inject all entries from a PyDict into the host's globals.
    pub fn inject_from_pydict(&mut self, py: Python<'_>, dict: &Bound<'_, PyDict>) -> Result<(), String> {
        self.ensure_host(py)?;
        let globals = std::rc::Rc::clone(self.host.as_ref().unwrap().globals());
        // `Value` is `Copy` with manual refcounting: a displaced entry must be
        // released or its ref leaks. This bulk inject overwrites the host's
        // pre-seeded builtins (RUNTIME, import, jit, ...) and, on a reused
        // pipeline, the previous run's globals -- including struct instances
        // re-injected as proxies (`from_pyobject` increfs the slot), which a
        // plain `decref` would orphan (+1 registry count per run). Displaced
        // values are released after the borrow ends: a pyobj release can run
        // arbitrary `__del__` code that may re-enter the globals map.
        let mut displaced: Vec<Value> = Vec::new();
        {
            let mut g = globals.borrow_mut();
            for (key, value) in dict.iter() {
                if let Ok(name) = key.extract::<String>() {
                    if let Ok(val) = Value::from_pyobject(py, &value) {
                        if let Some(old) = g.insert(name, val) {
                            displaced.push(old);
                        }
                    }
                }
            }
        }
        for old in displaced {
            self.release_owned(old);
        }
        Ok(())
    }

    /// Install frozen parent globals into the host, thawing each against this
    /// VM's already-registered struct types. Same ownership discipline as
    /// `inject_from_pydict`: the map takes the ref `frozen_to_value` mints (no
    /// extra `clone_refcount`), and a value displaced on a reused worker
    /// pipeline is released registry-aware after the borrow ends -- a struct
    /// needs the registry in hand, and a pyobj release can run `__del__` that
    /// re-enters the map. Used by the native ND process worker, which reinstalls
    /// the same global names on every task and would otherwise strand one ref
    /// per refcounted global per task.
    pub fn install_frozen_globals(
        &mut self,
        py: Python<'_>,
        globals: &[(String, catnip_core::freeze::FrozenValue)],
    ) -> Result<(), String> {
        self.ensure_host(py)?;
        // Thaw and release must target the same registry: struct_from_frozen
        // creates into the thread-local one, release_owned frees from this VM's.
        // ensure_host (module imports) may have moved the thread-local, so pin
        // it back to this VM, as ensure_executor does before every run.
        self.install_tables();
        let map = Rc::clone(self.host.as_ref().unwrap().globals());
        let mut displaced: Vec<Value> = Vec::new();
        {
            let mut g = map.borrow_mut();
            for (name, fg) in globals {
                let val = crate::freeze::frozen_to_value(py, fg);
                if let Some(old) = g.insert(name.clone(), val) {
                    displaced.push(old);
                }
            }
        }
        for old in displaced {
            self.release_owned(old);
        }
        Ok(())
    }

    /// Release the struct instances captured by a thawed closure scope. The
    /// scope's `Drop` releases pyobj/bigint/complex captures but deliberately
    /// leaves struct captures (no registry in hand at teardown, see
    /// `ClosureScopeInner`); a worker that thaws a fresh scope per task would
    /// strand one registry slot per struct capture per task. Call after the
    /// result is frozen (a result aliasing a captured struct must be read
    /// first) and before the scope is dropped.
    pub fn release_captured_structs(&mut self, scope: &NativeClosureScope) {
        scope.release_captured_structs(&self.vm.struct_registry);
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

    /// Convert a result Value to a PyObject with this VM's thread-locals
    /// installed. After execute() returns they are restored (null at top
    /// level), so a bare `to_pyobject` would fail to resolve symbols and
    /// silently convert an enum/union variant to its integer index.
    pub fn value_to_pyobject(&self, py: Python<'_>, value: Value) -> Py<PyAny> {
        let prev_reg = crate::vm::value::save_struct_registry();
        let prev_func = crate::vm::value::save_func_table();
        let prev_sym = crate::vm::value::save_symbol_table();
        let prev_enum = crate::vm::value::save_enum_registry();
        crate::vm::value::set_struct_registry(&self.vm.struct_registry as *const _);
        crate::vm::value::set_func_table(&self.vm.func_table as *const _);
        crate::vm::value::set_symbol_table(&self.vm.symbol_table as *const _ as *mut _);
        crate::vm::value::set_enum_registry(&self.vm.enum_registry as *const _ as *mut _);
        let result = value.to_pyobject(py);
        crate::vm::value::restore_struct_registry(prev_reg);
        crate::vm::value::restore_func_table(prev_func);
        crate::vm::value::restore_symbol_table(prev_sym);
        crate::vm::value::restore_enum_registry(prev_enum);
        result
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

            // Capture the parent VM's func_table pointer (if we are nested inside
            // an import). execute_with_host installs our func_table and -- unlike
            // symbol/enum -- never restores the previous one, so it must be read
            // before, not after.
            //
            // Only trust the pointer when a VM dispatch is genuinely active on
            // this thread (depth > 0): an import suspends the parent in its own
            // dispatch, keeping its func_table alive. At top level the leftover
            // pointer may name a sibling Executor that Python is about to GC, so
            // transplanting into it would be a use-after-free.
            let parent_func_ptr = if crate::vm::value::vm_depth() > 0 {
                crate::vm::value::save_func_table()
            } else {
                std::ptr::null()
            };

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

            // Transplant our func_table into the parent VM so exported functions
            // (and the by-name sibling calls they resolve through the module
            // globals) stay valid after this child VM is dropped. Must run after
            // the enum pass: it transplants the symbol-remapped closures.
            self.transplant_functions_to_parent(py, parent_func_ptr);

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
        // SAFETY: parent_enum_ptr is non-null (checked above) and points to the parent
        // VM's enum registry, alive for this call (parent suspended in import under the GIL).
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

    /// Transplant this executor's func_table slots into the parent VM's table,
    /// accessed via the thread-local pointer (restored by execute_with_host).
    ///
    /// In the embedded VM, `.cat` modules run in a child VM whose func_table is
    /// dropped once loading returns. Exported functions resolve their siblings
    /// by name through the module globals (closure parent chain), which hold raw
    /// `VMFunc(child_idx)` values. Appending the child slots to the parent table
    /// at offset `func_base` and shifting every `VMFunc` value (module globals +
    /// closure captures) by that base keeps those calls valid in the parent.
    fn transplant_functions_to_parent(
        &mut self,
        py: Python<'_>,
        parent_func_ptr: *const crate::vm::value::FunctionTable,
    ) {
        if self.vm.func_table.slots.is_empty() {
            return;
        }
        if parent_func_ptr.is_null() {
            return; // No parent VM (top-level execution)
        }
        // Skip if this IS the parent (same pointer)
        if std::ptr::eq(parent_func_ptr, &self.vm.func_table as *const _) {
            return;
        }
        // SAFETY: pointer is valid for the duration of this call (parent VM alive,
        // single-threaded under the GIL). The parent VM is suspended in the import
        // call and holds no live borrow into its func_table.
        let parent_func = unsafe { &mut *(parent_func_ptr as *mut crate::vm::value::FunctionTable) };
        let func_base = parent_func.slots.len() as u32;
        // Child slot count, captured before the append below (which only reads).
        // A container VMFunction addresses a child slot, so its index is < child_len;
        // a VMFunction pyobj carrying a foreign index (parent-relative, e.g. stashed
        // through a Python module) has index >= child_len and must NOT be shifted.
        let child_len = self.vm.func_table.slots.len() as u32;

        // Append each child slot to the parent, shifting closure-captured VMFunc
        // indices by func_base (letrec self/sibling references).
        for slot in self.vm.func_table.slots.iter() {
            if let Some(ref closure) = slot.closure {
                closure.remap_vmfuncs(func_base);
            }
            parent_func.insert(FuncSlot {
                code: std::sync::Arc::clone(&slot.code),
                closure: slot.closure.clone(),
                code_py: slot.code_py.clone_ref(py),
                context: slot.context.as_ref().map(|c| c.clone_ref(py)),
            });
        }

        // Shift VMFunc Values in our globals (exports and the closure parent
        // chain read these via the shared Rc) by func_base. A closure exported
        // *inside* a container (`handlers = [f]`) is stored as a Python
        // VMFunction whose index is child-relative, so walk the exported
        // list/tuple/dict globals and shift those in place too. A struct
        // exported as a bare global is a native TAG_STRUCT whose fields live in
        // the registry (not a Python proxy), so shift those field values too.
        if func_base > 0 {
            if let Some(ref host) = self.host {
                let mut visited: HashSet<usize> = HashSet::new();
                let mut struct_visited: HashSet<u32> = HashSet::new();
                let mut g = host.globals().borrow_mut();
                for (_, value) in g.iter_mut() {
                    if value.is_vmfunc() && !value.is_invalid() {
                        *value = Value::from_vmfunc(value.as_vmfunc_idx() + func_base);
                    } else if value.is_struct_instance() {
                        if let Some(inst_idx) = value.as_struct_instance_idx() {
                            shift_struct_fields(
                                &self.vm.struct_registry,
                                inst_idx,
                                func_base,
                                child_len,
                                py,
                                &mut visited,
                                &mut struct_visited,
                            );
                        }
                    } else if let Some(obj) = value.as_pyobject(py) {
                        shift_container_vmfuncs(obj.bind(py), func_base, child_len, &mut visited);
                    }
                }
            }
        }

        // Reinstall the parent func_table as the thread-local so reading the
        // module exports (and the parent VM's subsequent value conversions)
        // resolves the transplanted slots. execute_with_host left ours installed.
        crate::vm::value::restore_func_table(parent_func_ptr);
    }
}

/// Shift the `func_table_idx` of every `VMFunction` reachable through an
/// exported Python container (list / tuple / dict, arbitrarily nested) by
/// `func_base`, mirroring the direct-global shift in
/// `transplant_functions_to_parent`. A module exporting a closure *inside* a
/// container stores it as a Python `VMFunction` whose index is child-relative;
/// without this walk the parent reads the unshifted index and collides with an
/// unrelated parent slot (a silent wrong result).
///
/// `visited` (object identity) shifts each object exactly once, so a closure
/// shared by two globals -- or reached through a cyclic container -- is never
/// double-shifted. Only a child-relative index (`idx < child_len`) is shifted:
/// a `VMFunction` carrying a foreign index (e.g. a parent function stashed
/// through a Python module) keeps it. Mutation is in place: the `VMFunction` is
/// only an index carrier (dispatch re-reads the parent table by index via
/// `from_pyobject`), so no container rebuild or refcount juggling is needed.
fn shift_container_vmfuncs(obj: &Bound<'_, PyAny>, func_base: u32, child_len: u32, visited: &mut HashSet<usize>) {
    if !visited.insert(obj.as_ptr() as usize) {
        return;
    }
    if let Ok(vmfunc) = obj.cast::<VMFunction>() {
        let mut f = vmfunc.borrow_mut();
        if let Some(idx) = f.func_table_idx {
            if idx < child_len {
                f.func_table_idx = Some(idx + func_base);
            }
        }
        return;
    }
    if let Ok(proxy) = obj.cast::<crate::vm::structs::CatnipStructProxy>() {
        // A closure captured in a struct field is stored as a Python VMFunction
        // in `field_values`, the same index-carrier as a list element. Clone the
        // field refs out first so the recursion holds no borrow of the proxy.
        let py = obj.py();
        let fields: Vec<Py<PyAny>> = proxy.borrow().field_values.iter().map(|v| v.clone_ref(py)).collect();
        for f in &fields {
            shift_container_vmfuncs(f.bind(py), func_base, child_len, visited);
        }
        return;
    }
    if let Ok(list) = obj.cast::<PyList>() {
        for item in list.iter() {
            shift_container_vmfuncs(&item, func_base, child_len, visited);
        }
    } else if let Ok(tuple) = obj.cast::<PyTuple>() {
        for item in tuple.iter() {
            shift_container_vmfuncs(&item, func_base, child_len, visited);
        }
    } else if let Ok(dict) = obj.cast::<PyDict>() {
        for (_, v) in dict.iter() {
            shift_container_vmfuncs(&v, func_base, child_len, visited);
        }
    }
}

/// Shift the child-relative VMFunc field values of a native struct instance
/// (and any nested struct / Python container reachable through its fields) by
/// `func_base`. A struct exported as a bare top-level global is a native
/// TAG_STRUCT whose fields live in the registry, not a Python proxy, so the
/// container walk never reaches it; this covers that path.
///
/// `struct_visited` (by instance index) shifts each instance exactly once
/// (shared or cyclic instances); `py_visited` is shared with the container
/// walk so a proxy reachable through both a container and a struct field is
/// shifted once. Shifting a VMFunc value only rewrites its index bits, so the
/// operation is refcount-neutral (no clone/decref of the fields).
fn shift_struct_fields(
    registry: &crate::vm::structs::StructRegistry,
    idx: u32,
    func_base: u32,
    child_len: u32,
    py: Python<'_>,
    py_visited: &mut HashSet<usize>,
    struct_visited: &mut HashSet<u32>,
) {
    if !struct_visited.insert(idx) {
        return;
    }
    // Snapshot the field bits (Value is Copy) so no registry borrow is held
    // across the recursion and write-back below.
    let fields: Vec<Value> = match registry.with_instance(idx, |inst| inst.fields.clone()) {
        Some(fields) => fields,
        None => return,
    };
    for (i, &fv) in fields.iter().enumerate() {
        if fv.is_vmfunc() && !fv.is_invalid() && fv.as_vmfunc_idx() < child_len {
            registry.with_instance_mut(idx, |inst| {
                inst.fields[i] = Value::from_vmfunc(fv.as_vmfunc_idx() + func_base);
            });
        } else if fv.is_struct_instance() {
            if let Some(nested) = fv.as_struct_instance_idx() {
                shift_struct_fields(registry, nested, func_base, child_len, py, py_visited, struct_visited);
            }
        } else if let Some(obj) = fv.as_pyobject(py) {
            shift_container_vmfuncs(obj.bind(py), func_base, child_len, py_visited);
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
    fn install_frozen_globals_reinstall_releases_displaced() {
        use crate::vm::structs::StructField;
        use catnip_core::freeze::FrozenValue;
        use catnip_core::vm::opcode::ParamCheck;
        Python::attach(|py| {
            let mut executor = Executor::new();
            executor.install_tables();
            executor.ensure_host(py).unwrap();
            // A struct global thaws into a registry slot (measurable, unlike a
            // list which NaN-boxes into a NativeList). Register the type so
            // struct_from_frozen can reconstruct it against this VM's registry.
            executor.vm_mut().struct_registry.register_type(
                "P".into(),
                vec![StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: ParamCheck::None,
                }],
                indexmap::IndexMap::new(),
                vec![],
                vec!["P".into()],
            );
            let make = |v: i64| {
                vec![(
                    "G".to_string(),
                    FrozenValue::Struct {
                        type_name: "P".into(),
                        fields: vec![("x".to_string(), FrozenValue::Int(v))],
                    },
                )]
            };

            // Warm up so the slot exists, then measure across REINSTALLS: the
            // worker overwrites the same global name every task, so the
            // displaced instance must be released or one slot strands per task
            // on the reused pipeline.
            executor.install_frozen_globals(py, &make(0)).unwrap();
            let before = executor.debug_instance_slots().len();
            for i in 1..=50 {
                executor.install_frozen_globals(py, &make(i)).unwrap();
            }
            let after = executor.debug_instance_slots().len();
            assert_eq!(
                after, before,
                "reinstalling a struct global must release the displaced slot (no per-task leak)"
            );
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
