// FILE: catnip_vm/src/loader.rs
//! Pure Rust module loader for .cat file imports.
//!
//! Resolution via catnip_core::loader::resolve (no PyO3).
//! Each import creates a fresh PurePipeline, executes the module,
//! and returns a ModuleNamespace value. Caches results.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use catnip_core::loader::resolve::{self, ModuleKind};
use indexmap::IndexMap;

use crate::collections::ValueKey;
use crate::compiler::code_object::CodeObject;
use crate::error::{VMError, VMResult};
use crate::host::VmHost;
use crate::pipeline::PurePipeline;
use crate::value::{ModuleNamespace, Value};
use crate::vm::PureVM;
use crate::vm::closure::{PureClosureParent, PureClosureScope};
use crate::vm::func_table::PureFuncSlot;
use crate::vm::structs::StructCell;
use std::sync::Arc;

/// Parameters for an import() call with keyword arguments.
pub struct ImportParams<'a> {
    pub spec: &'a str,
    /// Selective import names: (original_name, alias).
    pub names: Vec<(String, String)>,
    /// Inject all public names into caller globals.
    pub wild: bool,
    /// Protocol filter ("cat", "py", "rs").
    pub protocol: Option<&'a str>,
}

/// Result of an import with potential injections.
pub enum ImportResult {
    /// Normal import: return namespace value.
    Namespace(Value),
    /// Wild or selective import: names injected into caller, return NIL.
    Injected(Vec<(String, Value)>),
}

/// Shared import cache (spec or abs path → module Value).
pub type ImportCache = Rc<RefCell<HashMap<String, Value>>>;

/// Shared set of modules currently being loaded (circular import detection).
pub type InProgressSet = Rc<RefCell<HashSet<String>>>;

/// Pure Rust import loader for .cat files and native plugins.
pub struct PureImportLoader {
    cache: ImportCache,
    in_progress: InProgressSet,
    source_dir: Option<PathBuf>,
    plugin_registry: Option<crate::plugin::SharedPluginRegistry>,
    /// Directories to search for stdlib plugin .so files.
    stdlib_paths: Vec<PathBuf>,
    /// Override for sys.argv (injected into sys module at load time).
    sys_argv: Option<Vec<String>>,
    /// Override for sys.executable (injected into sys module at load time).
    sys_executable: Option<String>,
    /// Module import policy (deny-wins).
    policy: Option<catnip_core::policy::ModulePolicyCore>,
    /// Qualified name of the current module context (e.g. "pkg" for pkg/lib.cat).
    /// Used to reconstruct fully qualified names for relative imports.
    module_name: Option<String>,
}

impl PureImportLoader {
    /// Create a new loader.
    pub fn new(source_dir: Option<PathBuf>) -> Self {
        Self {
            cache: Rc::new(RefCell::new(HashMap::new())),
            in_progress: Rc::new(RefCell::new(HashSet::new())),
            source_dir,
            plugin_registry: None,
            stdlib_paths: discover_stdlib_paths(),
            sys_argv: None,
            sys_executable: None,
            policy: None,
            module_name: None,
        }
    }

    /// Set the plugin registry for native plugin loading.
    pub fn set_plugin_registry(&mut self, registry: crate::plugin::SharedPluginRegistry) {
        self.plugin_registry = Some(registry);
    }

    /// Override sys.argv when the sys module is loaded.
    pub fn set_sys_argv(&mut self, argv: Vec<String>) {
        self.sys_argv = Some(argv);
    }

    /// Override sys.executable when the sys module is loaded.
    pub fn set_sys_executable(&mut self, exe: String) {
        self.sys_executable = Some(exe);
    }

    /// Set the module import policy.
    pub fn set_policy(&mut self, policy: catnip_core::policy::ModulePolicyCore) {
        self.policy = Some(policy);
    }

    /// Check if a module name is allowed by the current policy.
    fn check_policy(&self, name: &str) -> VMResult<()> {
        if let Some(ref policy) = self.policy {
            if !policy.check(name) {
                return Err(VMError::RuntimeError(format!("module '{}' blocked by policy", name)));
            }
        }
        Ok(())
    }

    /// Clear the module cache. Called by pipeline.reset() for isolation.
    pub fn clear_cache(&self) {
        let mut cache = self.cache.borrow_mut();
        for val in cache.values() {
            val.decref();
        }
        cache.clear();
    }

    /// Create a child loader sharing cache, in_progress, and plugin registry.
    /// `module_name` is the qualified name of the module being loaded (e.g. "pkg").
    fn child(&self, source_dir: PathBuf, module_name: String) -> Self {
        Self {
            cache: Rc::clone(&self.cache),
            in_progress: Rc::clone(&self.in_progress),
            source_dir: Some(source_dir),
            plugin_registry: self.plugin_registry.clone(),
            stdlib_paths: self.stdlib_paths.clone(),
            sys_argv: self.sys_argv.clone(),
            sys_executable: self.sys_executable.clone(),
            policy: self.policy.clone(),
            module_name: Some(module_name),
        }
    }

    /// Resolve a relative import spec to its fully qualified name
    /// using this loader's module context.
    ///
    /// `.secret` in module "pkg" → "pkg.secret"
    /// `..utils` in module "pkg.sub" → "pkg.utils"
    /// Absolute specs pass through unchanged.
    fn qualify_spec(&self, spec: &str) -> String {
        let (level, tail) = resolve::parse_relative_spec(spec);
        if level == 0 {
            return spec.to_string();
        }
        match &self.module_name {
            Some(prefix) => {
                let parts: Vec<&str> = prefix.split('.').collect();
                // level 1 = same package, level 2 = parent package, etc.
                let keep = parts.len().saturating_sub(level - 1);
                let parent = &parts[..keep.min(parts.len())];
                match (parent.is_empty(), tail.is_empty()) {
                    (true, _) => tail.to_string(),
                    (_, true) => parent.join("."),
                    _ => format!("{}.{}", parent.join("."), tail),
                }
            }
            None => tail.to_string(),
        }
    }

    /// Load a module by spec. `parent_vm` is used for func_table transplanting.
    pub fn load(&self, spec: &str, parent_vm: &mut PureVM) -> VMResult<Value> {
        // 1. Validate
        resolve::validate_spec(spec).map_err(VMError::ValueError)?;

        // 1b. Policy check (absolute imports checked immediately)
        let (level, _) = resolve::parse_relative_spec(spec);
        if level == 0 {
            self.check_policy(spec)?;
        }

        // 2. Cache hit by spec name
        if let Some(val) = self.cache.borrow().get(spec) {
            val.clone_refcount();
            return Ok(*val);
        }

        // 3. Stdlib modules (plugin discovery from .so files)
        if let Some(val) = self.try_load_stdlib_plugin(spec)? {
            return Ok(val);
        }

        // 4. Resolve to file path
        let (path, kind) = self.resolve_spec(spec, None)?;
        let abs_key = path.to_string_lossy().to_string();

        // 4b. Deferred policy check for relative imports.
        // Reconstruct the qualified name from the module context (e.g. ".secret" in "pkg" → "pkg.secret").
        if level > 0 {
            let qualified = self.qualify_spec(spec);
            self.check_policy(&qualified)?;
        }

        // Cache hit by absolute path
        if let Some(val) = self.cache.borrow().get(&abs_key) {
            val.clone_refcount();
            return Ok(*val);
        }

        // 4. Only .cat files supported in pure mode
        match kind {
            ModuleKind::Catnip | ModuleKind::Package => {}
            ModuleKind::Python => {
                return Err(VMError::RuntimeError(format!(
                    "cannot import '{}': Python modules require embedded Python mode",
                    spec
                )));
            }
            ModuleKind::Native => {
                return Err(VMError::RuntimeError(format!(
                    "cannot import '{}': native extensions not yet supported in pure mode",
                    spec
                )));
            }
        }

        // 5. Circular import detection
        if !self.in_progress.borrow_mut().insert(abs_key.clone()) {
            return Err(VMError::RuntimeError(format!("circular import detected: '{}'", spec)));
        }

        // 6. Load the module
        let result = self.load_cat_file(spec, &path, &abs_key, parent_vm);

        // Cleanup in_progress regardless of success
        self.in_progress.borrow_mut().remove(&abs_key);

        result
    }

    /// Load a module with extended import parameters (protocol, wild, selective).
    pub fn load_with_params(&self, params: ImportParams, parent_vm: &mut PureVM) -> VMResult<ImportResult> {
        // 1. Protocol validation
        match params.protocol {
            Some("cat") | Some("rs") | None => {}
            Some("py") => {
                return Err(VMError::RuntimeError("protocol 'py' not available in pure mode".into()));
            }
            Some(p) => {
                return Err(VMError::RuntimeError(format!("unknown protocol: '{}'", p)));
            }
        }

        // 2. Conflict check
        if params.wild && !params.names.is_empty() {
            return Err(VMError::ValueError("cannot combine wild and selective imports".into()));
        }

        // 3. Validate spec
        resolve::validate_spec(params.spec).map_err(VMError::ValueError)?;

        // 3b. Policy check (absolute imports checked immediately)
        let (level, _) = resolve::parse_relative_spec(params.spec);
        if level == 0 {
            self.check_policy(params.spec)?;
        }

        // 4. Cache hit by spec name
        if let Some(val) = self.cache.borrow().get(params.spec) {
            val.clone_refcount();
            return self.apply_import_mode(*val, &params);
        }

        // 5. Stdlib modules (plugin discovery from .so files)
        if let Some(val) = self.try_load_stdlib_plugin(params.spec)? {
            return self.apply_import_mode(val, &params);
        }

        // 6. Resolve to file path
        let (path, kind) = self.resolve_spec(params.spec, params.protocol)?;
        let abs_key = path.to_string_lossy().to_string();

        // 6b. Deferred policy check for relative imports.
        if level > 0 {
            let qualified = self.qualify_spec(params.spec);
            self.check_policy(&qualified)?;
        }

        // Cache hit by absolute path
        if let Some(val) = self.cache.borrow().get(&abs_key) {
            val.clone_refcount();
            return self.apply_import_mode(*val, &params);
        }

        // 7. Dispatch by module kind
        match kind {
            ModuleKind::Catnip | ModuleKind::Package => {}
            ModuleKind::Native => {
                // Native plugin: load via registry, cache, return
                let val = self.load_native_plugin(params.spec, &path)?;
                val.clone_refcount();
                self.cache.borrow_mut().insert(abs_key, val);
                return self.apply_import_mode(val, &params);
            }
            ModuleKind::Python => {
                return Err(VMError::RuntimeError(format!(
                    "cannot import '{}': Python modules require embedded Python mode",
                    params.spec
                )));
            }
        }

        // 8. Circular import detection
        if !self.in_progress.borrow_mut().insert(abs_key.clone()) {
            return Err(VMError::RuntimeError(format!(
                "circular import detected: '{}'",
                params.spec
            )));
        }

        // 9. Load the .cat module
        let result = self.load_cat_file(params.spec, &path, &abs_key, parent_vm);
        self.in_progress.borrow_mut().remove(&abs_key);

        match result {
            Ok(val) => self.apply_import_mode(val, &params),
            Err(e) => Err(e),
        }
    }

    /// Apply wild/selective/normal mode to a loaded module namespace.
    ///
    /// Ownership: consumes the caller's `namespace` ref -- the wild/selective
    /// branches release it once the injections hold their own refs (including
    /// the missing-name error path), the normal branch transfers it inside
    /// `ImportResult::Namespace`.
    ///
    /// Invariant: the namespace must outlive the closures it exported --
    /// injected closures late-bind their module globals through the namespace's
    /// `module_globals` Rc. This holds because every call site pins the
    /// namespace in `self.cache` BEFORE calling here (cache-hit, native-plugin
    /// and `.cat` paths alike), so the `decref`s below never release the last
    /// ref while an injected closure is live.
    fn apply_import_mode(&self, namespace: Value, params: &ImportParams) -> VMResult<ImportResult> {
        if params.wild {
            // Extract all public names
            let injections = self.extract_public_names(&namespace)?;
            namespace.decref();
            return Ok(ImportResult::Injected(injections));
        }

        if !params.names.is_empty() {
            // Extract specific names
            let mut injections = Vec::with_capacity(params.names.len());
            let attrs = self.get_module_attrs(&namespace)?;
            for (name, alias) in &params.names {
                match attrs.get(name.as_str()) {
                    Some(v) => {
                        v.clone_refcount();
                        injections.push((alias.clone(), *v));
                        continue;
                    }
                    None => {
                        for (_, v) in &injections {
                            v.decref();
                        }
                        namespace.decref();
                        return Err(VMError::RuntimeError(format!("'{}' not found in module", name)));
                    }
                };
            }
            namespace.decref();
            return Ok(ImportResult::Injected(injections));
        }

        Ok(ImportResult::Namespace(namespace))
    }

    /// Extract all public names from a module namespace.
    fn extract_public_names(&self, namespace: &Value) -> VMResult<Vec<(String, Value)>> {
        let attrs = self.get_module_attrs(namespace)?;
        let mut result = Vec::new();
        for (name, val) in attrs {
            if !name.starts_with('_') {
                val.clone_refcount();
                result.push((name.clone(), *val));
            }
        }
        Ok(result)
    }

    /// Get the attrs IndexMap from a module namespace value.
    fn get_module_attrs<'a>(&self, namespace: &'a Value) -> VMResult<&'a IndexMap<String, Value>> {
        // SAFETY: `as_module_ref` checks the module tag internally and returns
        // None on mismatch, so the borrow is only taken when the payload is a live
        // module Arc; the returned reference is tied to `namespace`'s lifetime.
        if let Some(ns) = unsafe { namespace.as_module_ref() } {
            Ok(&ns.attrs)
        } else {
            Err(VMError::TypeError("expected module namespace".into()))
        }
    }

    /// Resolve a spec to a file path, optionally filtered by protocol.
    fn resolve_spec(&self, spec: &str, protocol: Option<&str>) -> VMResult<(PathBuf, ModuleKind)> {
        let (level, _) = resolve::parse_relative_spec(spec);
        let suffix = if protocol == Some("rs") {
            crate::plugin::native_suffix()
        } else {
            ""
        };

        if level > 0 {
            // Relative import
            let caller_dir = self.source_dir.as_deref().ok_or_else(|| {
                VMError::RuntimeError(format!("relative import '{}' requires a source file context", spec))
            })?;
            resolve::resolve_relative(spec, caller_dir, protocol, suffix).map_err(VMError::RuntimeError)
        } else {
            // Bare name
            resolve::resolve_bare_name(
                spec,
                self.source_dir.as_deref(),
                protocol,
                suffix,
                None, // use env CWD
            )
            .ok_or_else(|| VMError::RuntimeError(format!("module '{}' not found", spec)))
        }
    }

    /// Load a native plugin via the plugin registry.
    fn load_native_plugin(&self, spec: &str, path: &Path) -> VMResult<Value> {
        let registry = self
            .plugin_registry
            .as_ref()
            .ok_or_else(|| VMError::RuntimeError("native plugin loading requires a plugin registry".into()))?;
        let ns = registry.borrow_mut().load(path, spec)?;
        Ok(Value::from_module(ns))
    }

    /// Load a .cat file (or package), transplant functions, build namespace.
    fn load_cat_file(&self, spec: &str, path: &Path, abs_key: &str, parent_vm: &mut PureVM) -> VMResult<Value> {
        // For packages, find the entry point and optional export filter
        let (source_path, pkg_exports) = if path.is_dir() {
            self.resolve_package_entry(path)?
        } else {
            (path.to_path_buf(), None)
        };

        // Read source
        let source = std::fs::read_to_string(&source_path)
            .map_err(|e| VMError::RuntimeError(format!("cannot read '{}': {}", source_path.display(), e)))?;

        // Create child pipeline
        let mut child = PurePipeline::new().map_err(|e| VMError::RuntimeError(format!("pipeline init failed: {e}")))?;
        // The child's runtime closures escape into the parent (module namespace);
        // its Drop must not clear their captured maps (would break exported letrec
        // closures). Their letrec cycle-break handles are transferred to the
        // parent VM after execution instead (see below), so the parent's own
        // reset drain reclaims an exported cyclic group.
        child.vm_mut().drain_closures_on_drop = false;

        // Set up child's import loader (shared cache, new source_dir).
        // Propagate the qualified module name so relative imports resolve correctly.
        let child_dir = source_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let qualified = self.qualify_spec(spec);
        let child_loader = self.child(child_dir, qualified);
        child.set_import_loader(child_loader);

        // Inject META with file path, protocol, and main flag before execution
        {
            let meta = crate::value::NativeMeta::new();
            meta.set("file", Value::from_string(source_path.to_string_lossy().into_owned()));
            meta.set("protocol", Value::from_str("cat"));
            meta.set("main", Value::FALSE);
            child.host().store_global("META", Value::from_meta(meta));
        }

        // Snapshot baseline globals (keys + raw bits) before execution.
        // Used for heuristic export: only names that are new or changed get exported.
        let baseline: HashMap<String, u64> = child
            .host()
            .globals()
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.to_raw()))
            .collect();

        // Execute module. The returned value (the module's last statement) is
        // owned by the caller; ignoring it leaks one ref per import whenever
        // that statement yields a heap value (`r = Reg(42)` as the final line
        // retained its StructCell past reset).
        match child.execute(&source) {
            Ok(val) => val.decref(),
            Err(e) => {
                child.vm_mut().drain_closures_on_drop = true;
                child.host().clear_globals();
                return Err(VMError::RuntimeError(format!("error in module '{}': {}", spec, e)));
            }
        }

        // Transfer the child's letrec cycle-break handles (weak) to the parent:
        // a sibling-defined closure group carries strong cross-captures (the
        // backward sibling capture plus the forward PatchClosure) that only the
        // VM's reset drain breaks. The child's drain is off (its closures
        // escape into the namespace), and since the namespace drains
        // module_globals at drop, those cycles are the one thing still pinning
        // an exported group -- the parent's drain now reaches them.
        {
            let mut transferred = std::mem::take(&mut child.vm_mut().runtime_closures);
            parent_vm.runtime_closures.append(&mut transferred);
        }

        // Compute both bases before transplanting so all remaps are complete
        let func_base = parent_vm.func_table.len() as u32;
        let mut bases = RemapBases {
            func: func_base,
            stype_remap: std::collections::HashMap::new(),
            symbol_remap: std::collections::HashMap::new(),
            etype_remap: std::collections::HashMap::new(),
        };

        // Build remap tables first (structs, enums), then transplant functions
        transplant_structs(child.vm(), parent_vm, &mut bases);
        transplant_enums(child.vm(), parent_vm, &mut bases);
        let code_remap = transplant_functions(child.vm(), parent_vm, &bases);

        // Remap VMFunc/StructType/Enum indices in child globals so closures resolve
        // correctly. Runtime closures are deep-remapped (code + captures + scope
        // chain) so their child-baked constant indices point at the parent's
        // transplanted templates, and the walk recurses into exported
        // list/tuple/dict containers so a closure nested in one is remapped too.
        if bases.func > 0
            || !bases.stype_remap.is_empty()
            || !bases.symbol_remap.is_empty()
            || !bases.etype_remap.is_empty()
        {
            let mut ctx = ClosureGraphRemap::new(&bases, &code_remap);
            let mut child_globals_mut = child.host().globals().borrow_mut();
            // Snapshot the old refs (bit-copies): the remap borrows its inputs, so
            // the old pointers must stay valid (memoization keys) until every
            // global is rebuilt -- release them only afterwards.
            let olds: Vec<Value> = child_globals_mut.values().copied().collect();
            for val in child_globals_mut.values_mut() {
                *val = ctx.value(*val);
            }
            drop(child_globals_mut);
            // Break the *old* exported closures' cycles before releasing them: a
            // mutual-recursion group replaced by remapped copies is otherwise
            // pinned by its own strong captures and leaks. Each old global closure
            // keeps its ref (still in `olds`), so it stays live through this pass;
            // clearing its captures cascades a decref into non-global cycle members
            // (their `Drop` drains in turn). Then release the globals' refs.
            for old in &olds {
                if old.is_closure() {
                    // SAFETY: is_closure() proves a live Arc<PureFuncSlot> (held by
                    // its still-live `olds` ref); the borrow ends with this block.
                    if let Some(slot) = unsafe { old.as_closure_ref() } {
                        // Never clear a bound method's scope -- it is shared with
                        // its (template) method, like the VM drain's guard. Only
                        // MakeFunction closures own a unique scope. (Templates are
                        // `closure: None` today, so this is defensive.)
                        if slot.bound_self.is_none() {
                            if let Some(cs) = &slot.closure {
                                cs.clear_captured();
                            }
                        }
                    }
                }
            }
            for old in olds {
                old.decref();
            }
            // The rebuilt closures recreate the group's cross-capture cycles;
            // hand their cycle-break handles to the parent's reset drain (the
            // transferred child handles above only cover the pre-remap slots,
            // which the rebuild just released).
            parent_vm.runtime_closures.append(&mut ctx.cyclic);
            ctx.finish();
        }

        // Build exports: META.exports > __all__ > default, then lib.exports.include post-filters
        let child_globals = child.host().globals().borrow();

        // Step 0: Check META.exports (highest priority)
        let meta_exports_val = child_globals.get("META").filter(|v| v.is_meta()).and_then(|v| {
            // SAFETY: `v.is_meta()` was checked by the preceding filter, so the
            // payload is a live meta Arc; `as_meta_ref` borrows `v`, kept alive by
            // the closure for the returned reference's use.
            let m = unsafe { v.as_meta_ref()? };
            m.get("exports")
        });
        let meta_exports = if let Some(ref val) = meta_exports_val {
            match extract_string_list(val) {
                Ok(list) => Some(list),
                Err(e) => {
                    drop(child_globals);
                    child.host().clear_globals();
                    return Err(VMError::TypeError(format!("META.exports in module '{}': {}", spec, e)));
                }
            }
        } else {
            None
        };

        // Step 1: Build base namespace using META.exports or __all__ or default exclusion rules
        let all_filter = if meta_exports.is_some() {
            None // META.exports takes priority
        } else {
            child_globals.get("__all__").and_then(|v| extract_string_list(v).ok())
        };

        let mut attrs = IndexMap::new();
        if let Some(ref meta_filter) = meta_exports {
            // META.exports: pick exactly those names, error on missing
            for name in meta_filter {
                if let Some(val) = child_globals.get(name) {
                    val.clone_refcount();
                    attrs.insert(name.clone(), *val);
                } else {
                    for (_, v) in attrs.iter() {
                        v.decref();
                    }
                    drop(child_globals);
                    child.host().clear_globals();
                    return Err(VMError::NameError(format!(
                        "META.exports references undefined name '{}' in module '{}'",
                        name, spec
                    )));
                }
            }
        } else {
            for (name, val) in child_globals.iter() {
                if name == "META" {
                    continue;
                }
                if let Some(ref filter) = all_filter {
                    if !filter.contains(name) {
                        continue;
                    }
                } else if name.starts_with('_') {
                    continue;
                } else {
                    // Baseline comparison: skip names unchanged from pre-execution state.
                    // This matches Python's heuristic and allows exporting redefined builtins.
                    match baseline.get(name) {
                        Some(old_raw) if *old_raw == val.to_raw() => continue,
                        _ => {}
                    }
                }
                val.clone_refcount();
                attrs.insert(name.clone(), *val);
            }
        }

        // Step 2: If lib.exports.include is present, restrict to that subset
        if let Some(ref include_list) = pkg_exports {
            attrs.retain(|name, val| {
                if include_list.contains(name) {
                    true
                } else {
                    val.decref();
                    false
                }
            });
        }

        // Derive module name from spec
        let module_name = spec.trim_start_matches('.').to_string();

        // Keep child globals alive for closure scopes
        let module_globals = Rc::clone(child.host().globals());

        let ns = ModuleNamespace {
            name: module_name,
            attrs,
            module_globals,
        };
        let val = Value::from_module(ns);

        // Cache by absolute path only (spec-keyed aliases would pollute the
        // shared cache when child loaders have different source_dir).
        val.clone_refcount();
        self.cache.borrow_mut().insert(abs_key.to_string(), val);

        Ok(val)
    }

    /// Resolve a package directory to its entry point .cat file and optional export filter.
    fn resolve_package_entry(&self, pkg_dir: &Path) -> VMResult<(PathBuf, Option<Vec<String>>)> {
        let lib_toml = pkg_dir.join("lib.toml");
        let content = std::fs::read_to_string(&lib_toml)
            .map_err(|e| VMError::RuntimeError(format!("cannot read {}: {}", lib_toml.display(), e)))?;

        let mut entry_name: Option<String> = None;
        let mut exports_include: Option<Vec<String>> = None;

        // Track current TOML section to scope key parsing
        let mut current_section = ""; // "" = top-level

        for line in content.lines() {
            let trimmed = line.trim();

            // Track TOML sections
            if trimmed.starts_with('[') {
                if trimmed == "[lib]" {
                    current_section = "lib";
                } else if trimmed == "[lib.exports]" {
                    current_section = "lib.exports";
                } else {
                    current_section = "other";
                }
                continue;
            }

            // Parse entry = "filename" only under [lib]
            if entry_name.is_none() && current_section == "lib" {
                if let Some(rest) = trimmed.strip_prefix("entry") {
                    let rest = rest.trim();
                    if let Some(rest) = rest.strip_prefix('=') {
                        entry_name = Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
                    }
                }
            }

            // Parse include = [...] only under [lib.exports] or [exports]
            if current_section == "lib.exports" {
                if let Some(rest) = trimmed.strip_prefix("include") {
                    let rest = rest.trim();
                    if let Some(rest) = rest.strip_prefix('=') {
                        exports_include = Some(parse_toml_string_array(rest.trim()));
                    }
                }
            }
        }

        let entry_path = if let Some(name) = entry_name {
            let entry_path = pkg_dir.join(&name);
            // Path traversal guard
            if let (Ok(resolved), Ok(pkg_resolved)) = (entry_path.canonicalize(), pkg_dir.canonicalize()) {
                if !resolved.starts_with(&pkg_resolved) {
                    return Err(VMError::RuntimeError(format!(
                        "package entry '{}' escapes package directory",
                        name
                    )));
                }
            }
            entry_path
        } else {
            // Default: main.cat
            let init = pkg_dir.join("main.cat");
            if init.is_file() {
                init
            } else {
                return Err(VMError::RuntimeError(format!(
                    "package '{}' has no entry point (no 'entry' in lib.toml, no __init__.cat)",
                    pkg_dir.display()
                )));
            }
        };

        Ok((entry_path, exports_include))
    }
}

/// Transplant function slots from child VM to parent VM. Returns a map from each
/// child template's code pointer to the parent's (possibly remapped) code, so an
/// exported runtime closure -- which shares its template's code `Arc` -- can have
/// its child-baked constant indices swapped for the parent's remapped code.
fn transplant_functions(
    child_vm: &PureVM,
    parent_vm: &mut PureVM,
    bases: &RemapBases,
) -> std::collections::HashMap<usize, std::sync::Arc<crate::compiler::code_object::CodeObject>> {
    let mut code_remap = std::collections::HashMap::new();
    let child_len = child_vm.func_table.len();
    if child_len == 0 {
        return code_remap;
    }

    let needs_remap = bases.func > 0
        || !bases.stype_remap.is_empty()
        || !bases.symbol_remap.is_empty()
        || !bases.etype_remap.is_empty();
    for i in 0..child_len {
        let slot = child_vm.func_table.get(i as u32).unwrap();
        let child_code_ptr = std::sync::Arc::as_ptr(&slot.code) as usize;
        let new_code =
            if needs_remap
                && slot.code.constants.iter().any(|c| {
                    (c.is_vmfunc() && !c.is_invalid()) || c.is_struct_type() || c.is_symbol() || c.is_enum_type()
                })
            {
                let mut code = (*slot.code).clone();
                for c in &mut code.constants {
                    *c = remap_value(*c, bases);
                }
                std::sync::Arc::new(code)
            } else {
                std::sync::Arc::clone(&slot.code)
            };
        code_remap.insert(child_code_ptr, std::sync::Arc::clone(&new_code));
        let new_closure = if needs_remap {
            slot.closure.as_ref().map(|cs| remap_closure(cs, bases))
        } else {
            slot.closure.clone()
        };
        // Transplant carries function definitions (templates), not per-call bound
        // methods, so no curried receiver.
        parent_vm
            .func_table
            .insert(crate::vm::func_table::PureFuncSlot::template(new_code, new_closure));
    }
    code_remap
}

/// Remap VMFunc/StructType/Symbol/EnumType indices in a closure scope's captured values.
fn remap_closure(
    scope: &crate::vm::closure::PureClosureScope,
    bases: &RemapBases,
) -> crate::vm::closure::PureClosureScope {
    for (name, val) in scope.captured_entries() {
        let remapped = remap_value(val, bases);
        if remapped.to_raw() != val.to_raw() {
            scope.set(&name, remapped);
        }
    }
    scope.clone()
}

/// Deep-remaps an exported *runtime closure* graph so the child func_table
/// indices baked in its code constants (and reachable through its captures and
/// scope chain) point at the parent's transplanted templates. `remap_value`
/// leaves a `TAG_CLOSURE` untouched -- an exported closure would otherwise carry
/// child-baked constants and read the wrong parent template (a silent wrong
/// result). Runtime slots and closure scopes are memoized (by pointer) so a
/// mutual-recursion capture cycle and a shared enclosing scope are each rebuilt
/// exactly once.
///
/// Refcount contract: every method **borrows** its input (does not decref) and
/// returns an **owned** ref. `slot_memo` holds one owned ref per rebuilt closure;
/// `finish` drains it. Captured refs are transferred into the new scopes.
struct ClosureGraphRemap<'a> {
    bases: &'a RemapBases,
    code_remap: &'a HashMap<usize, Arc<CodeObject>>,
    slot_memo: HashMap<usize, Value>,
    scope_memo: HashMap<usize, PureClosureScope>,
    container_memo: HashMap<usize, Value>,
    container_visiting: HashSet<usize>,
    /// Rebuilt closures holding a strong closure capture: the rebuild recreates
    /// a sibling group's cross-capture cycle, so these need the same reset
    /// cycle-break as a live `PatchClosure` target. The driver appends them to
    /// the parent VM's `runtime_closures` (weak, pins nothing).
    cyclic: Vec<std::sync::Weak<PureFuncSlot>>,
}

impl<'a> ClosureGraphRemap<'a> {
    fn new(bases: &'a RemapBases, code_remap: &'a HashMap<usize, Arc<CodeObject>>) -> Self {
        Self {
            bases,
            code_remap,
            slot_memo: HashMap::new(),
            scope_memo: HashMap::new(),
            container_memo: HashMap::new(),
            container_visiting: HashSet::new(),
            cyclic: Vec::new(),
        }
    }

    /// Remap any value: recurse into a runtime closure, a container or a struct
    /// instance (a closure may hide inside an exported list/tuple/dict or a
    /// struct field), otherwise delegate to `remap_value` (guarded with a clone
    /// so the borrowed input keeps its ref).
    fn value(&mut self, val: Value) -> Value {
        if val.is_closure() {
            self.closure(val)
        } else if val.is_native_list() || val.is_native_tuple() || val.is_native_dict() {
            self.container(val)
        } else if val.is_struct_instance() {
            self.struct_instance(val)
        } else {
            val.clone_refcount();
            remap_value(val, self.bases)
        }
    }

    /// Rebuild a runtime closure with parent-remapped code, scope and bound_self.
    fn closure(&mut self, val: Value) -> Value {
        // SAFETY: is_closure() (checked by the caller) proves a live Arc<PureFuncSlot>;
        // the borrow does not outlive this extraction.
        let ptr = unsafe { val.as_closure_ref().unwrap() as *const PureFuncSlot as usize };
        if let Some(&existing) = self.slot_memo.get(&ptr) {
            existing.clone_refcount();
            return existing;
        }
        // Extract everything owned up front so no borrow of the slot is held
        // across the recursive calls below.
        let (new_code, parent_link, self_name, bound_self, captures) = {
            // SAFETY: same live-Arc guarantee; borrow ends with this block.
            let slot = unsafe { val.as_closure_ref().unwrap() };
            let child_code_ptr = Arc::as_ptr(&slot.code) as usize;
            let new_code = self
                .code_remap
                .get(&child_code_ptr)
                .cloned()
                .unwrap_or_else(|| Arc::clone(&slot.code));
            let parent_link = slot.closure.as_ref().map(|cs| cs.parent());
            let self_name = slot.closure.as_ref().and_then(|cs| cs.self_ref_name());
            let captures = slot.closure.as_ref().map(|cs| cs.captured_entries());
            (new_code, parent_link, self_name, slot.bound_self, captures)
        };
        // Shell scope: remapped parent, empty captures (filled after memoizing so
        // a cyclic capture resolves through slot_memo).
        let new_scope = parent_link.map(|p| PureClosureScope::new(IndexMap::new(), self.parent(p)));
        let new_bound_self = bound_self.map(|bs| self.value(bs));
        let arc = PureFuncSlot::new_runtime(new_code, new_scope.clone(), new_bound_self).into_arc();
        if let (Some(ns), Some(name)) = (&new_scope, self_name) {
            ns.set_self_ref(name, Arc::downgrade(&arc));
        }
        let weak = Arc::downgrade(&arc);
        let new_val = Value::from_arc_closure(arc);
        self.slot_memo.insert(ptr, new_val);
        if let (Some(ns), Some(caps)) = (&new_scope, captures) {
            let mut holds_closure_capture = false;
            for (name, cap) in caps {
                let remapped = self.value(cap);
                holds_closure_capture |= remapped.is_closure();
                ns.insert_captured(&name, remapped);
            }
            if holds_closure_capture {
                self.cyclic.push(weak);
            }
        }
        new_val.clone_refcount();
        new_val
    }

    /// Remap a parent link: reuse the (already in-place remapped) module globals,
    /// recurse on an enclosing scope, or keep `None`.
    fn parent(&mut self, p: PureClosureParent) -> PureClosureParent {
        match p {
            PureClosureParent::None => PureClosureParent::None,
            PureClosureParent::Globals(g) => PureClosureParent::Globals(g),
            PureClosureParent::Scope(s) => PureClosureParent::Scope(self.scope(s)),
        }
    }

    /// Rebuild an enclosing scope (memoized, shell-then-fill). Its self-ref (to a
    /// now-dead enclosing frame) is dropped -- its upgrade would fail anyway.
    fn scope(&mut self, s: PureClosureScope) -> PureClosureScope {
        let sptr = s.inner_ptr();
        if let Some(existing) = self.scope_memo.get(&sptr) {
            return existing.clone();
        }
        let parent = self.parent(s.parent());
        let new_s = PureClosureScope::new(IndexMap::new(), parent);
        self.scope_memo.insert(sptr, new_s.clone());
        for (name, cap) in s.captured_entries() {
            let remapped = self.value(cap);
            new_s.insert_captured(&name, remapped);
        }
        new_s
    }

    /// Rebuild a list/tuple/dict whose elements transitively reference a runtime
    /// closure or a remappable type, so a closure nested in an exported container
    /// resolves the parent's transplanted templates instead of its child-baked
    /// constant indices (a silent wrong result otherwise). Identity is preserved:
    /// an unchanged container returns its original Arc, and a container shared by
    /// several globals is rebuilt once (pointer-memoized). Struct instances take
    /// the sibling path `struct_instance` (same discipline, plus the `type_id`
    /// remap).
    ///
    /// Refcount contract mirrors `closure`: borrows its input, returns an owned
    /// ref; `container_memo` holds one owned ref per rebuilt container (drained by
    /// `finish`). A cyclic mutable container's back-edge keeps the original
    /// element unremapped (memory-safe; a closure reachable only through the cycle
    /// stays on its child template -- an accepted narrow limitation).
    fn container(&mut self, val: Value) -> Value {
        let ptr = Self::container_ptr(val);
        if let Some(&existing) = self.container_memo.get(&ptr) {
            existing.clone_refcount();
            return existing;
        }
        if self.container_visiting.contains(&ptr) {
            val.clone_refcount();
            return val;
        }
        self.container_visiting.insert(ptr);
        let result = if val.is_native_dict() {
            self.rebuild_dict(val)
        } else {
            self.rebuild_sequence(val)
        };
        self.container_visiting.remove(&ptr);
        self.container_memo.insert(ptr, result);
        result.clone_refcount();
        result
    }

    /// The NaN-box payload is the shared Arc's data pointer, so two Values backed
    /// by the same container yield the same memo key.
    fn container_ptr(val: Value) -> usize {
        if val.is_native_list() {
            // SAFETY: is_native_list() proves a live Arc<NativeList>; the borrow
            // ends with the cast to a bare address.
            unsafe { val.as_native_list_ref().unwrap() as *const _ as usize }
        } else if val.is_native_tuple() {
            // SAFETY: is_native_tuple() proves a live Arc<NativeTuple>.
            unsafe { val.as_native_tuple_ref().unwrap() as *const _ as usize }
        } else {
            // SAFETY: is_native_dict() proves a live Arc<NativeDict>.
            unsafe { val.as_native_dict_ref().unwrap() as *const _ as usize }
        }
    }

    /// Remap a sequence of borrowed element bits. Returns `Some(remapped)` (owned
    /// refs, ready to hand to a `from_*` constructor) when at least one element
    /// changed, or `None` when all were unchanged -- in the `None` case the owned
    /// copies `value()` produced are released here, so the caller keeps the
    /// container's original Arc. Single point for the unchanged-path `decref`, so
    /// list/tuple/dict share one refcount-sensitive branch instead of three.
    fn remap_elems(&mut self, elems: &[Value]) -> Option<Vec<Value>> {
        let mut remapped = Vec::with_capacity(elems.len());
        let mut changed = false;
        for e in elems {
            let r = self.value(*e);
            changed |= r.to_raw() != e.to_raw();
            remapped.push(r);
        }
        if changed {
            Some(remapped)
        } else {
            for r in remapped {
                r.decref();
            }
            None
        }
    }

    /// Rebuild a list or tuple element-wise. Returns a fresh container owning the
    /// remapped elements only if one changed; otherwise the original Arc.
    fn rebuild_sequence(&mut self, val: Value) -> Value {
        let is_list = val.is_native_list();
        let elems: Vec<Value> = if is_list {
            // SAFETY: is_native_list() proves a live Arc<NativeList>; the snapshot
            // copies bits into an owned Vec, so no borrow crosses the recursion.
            unsafe { val.as_native_list_ref().unwrap() }.snapshot_items()
        } else {
            // SAFETY: is_native_tuple() proves a live Arc<NativeTuple>.
            unsafe { val.as_native_tuple_ref().unwrap() }.as_slice().to_vec()
        };
        match self.remap_elems(&elems) {
            Some(remapped) if is_list => Value::from_list(remapped),
            Some(remapped) => Value::from_tuple(remapped),
            None => {
                val.clone_refcount();
                val
            }
        }
    }

    /// Rebuild a dict value-wise (keys are immutable primitives -- never a
    /// closure -- so only values recurse; a key carrying a remappable type/symbol
    /// id is a pre-existing gap, orthogonal to this walk). Fresh dict only on
    /// change.
    fn rebuild_dict(&mut self, val: Value) -> Value {
        let (keys, vals): (Vec<ValueKey>, Vec<Value>) = {
            // SAFETY: is_native_dict() proves a live Arc<NativeDict>; both
            // snapshots detach from the borrow before the recursion below.
            let d = unsafe { val.as_native_dict_ref().unwrap() };
            (d.keys_cloned(), d.snapshot_values())
        };
        match self.remap_elems(&vals) {
            Some(remapped) => Value::from_dict(keys.into_iter().zip(remapped).collect()),
            None => {
                val.clone_refcount();
                val
            }
        }
    }

    /// Rebuild a struct instance whose fields transitively reference a runtime
    /// closure, a container or a remappable type, and remap its `type_id` to the
    /// parent's transplanted registry (the child-baked id resolves the wrong
    /// type once the registries diverge -- same class of silent wrong result as
    /// a stale VMFunc index). Same discipline as `container`: pointer-memoized
    /// identity, cycle guard on the shared visiting set (struct fields are
    /// `Cell`s, a self-referential instance is legal), borrowed input / owned
    /// output, memo drained by `finish`.
    fn struct_instance(&mut self, val: Value) -> Value {
        // SAFETY: is_struct_instance() (checked by the caller) proves a live
        // Arc<StructCell>; the borrow ends with the cast to a bare address.
        let ptr = unsafe { val.as_struct_ref().unwrap() as *const StructCell as usize };
        if let Some(&existing) = self.container_memo.get(&ptr) {
            existing.clone_refcount();
            return existing;
        }
        if self.container_visiting.contains(&ptr) {
            val.clone_refcount();
            return val;
        }
        self.container_visiting.insert(ptr);
        let (type_id, frozen, fields) = {
            // SAFETY: same live-Arc guarantee; the snapshot copies bits into an
            // owned Vec, so no borrow crosses the recursion below.
            let cell = unsafe { val.as_struct_ref().unwrap() };
            (cell.type_id, cell.frozen.get(), cell.field_values())
        };
        let new_type_id = self.bases.stype_remap.get(&type_id).copied().unwrap_or(type_id);
        let rebuilt_fields = match self.remap_elems(&fields) {
            Some(remapped) => Some(remapped),
            // Fields untouched but the type moved: rebuild with owned copies of
            // the original fields (`StructCell::new` takes ownership).
            None if new_type_id != type_id => Some(
                fields
                    .iter()
                    .map(|f| {
                        f.clone_refcount();
                        *f
                    })
                    .collect(),
            ),
            None => None,
        };
        let result = match rebuilt_fields {
            Some(fields) => {
                let cell = StructCell::new(new_type_id, fields);
                // A hashed (dict/set key) instance must stay mutation-frozen.
                cell.frozen.set(frozen);
                Value::from_struct_instance(cell)
            }
            None => {
                val.clone_refcount();
                val
            }
        };
        self.container_visiting.remove(&ptr);
        self.container_memo.insert(ptr, result);
        result.clone_refcount();
        result
    }

    /// Release the memo's owning refs once the graph is rebuilt (each rebuilt
    /// closure or container is still held by a global or a capturing scope).
    fn finish(mut self) {
        for (_, v) in self.slot_memo.drain() {
            v.decref();
        }
        for (_, v) in self.container_memo.drain() {
            v.decref();
        }
    }
}

/// Transplant struct types from child to parent (types only, not instances).
/// For MVP: struct types exported by modules work when called through
/// module functions. Direct construction from parent scope is a future enhancement.
/// Transplant struct types from child to parent.
fn transplant_structs(child_vm: &PureVM, parent_vm: &mut PureVM, bases: &mut RemapBases) {
    use catnip_core::exception::ExceptionKind;

    let child_types = &child_vm.struct_registry;
    let mut child_type_id = 0u32;
    while let Some(ty) = child_types.get_type(child_type_id) {
        // Built-in exception types exist in both parent and child with the same name.
        // Map child type_id to the parent's existing type_id instead of transplanting.
        if let Some(parent_id) = parent_vm.struct_registry.find_type_id(&ty.name) {
            if ExceptionKind::from_name(&ty.name).is_some() {
                if parent_id != child_type_id {
                    bases.stype_remap.insert(child_type_id, parent_id);
                }
                child_type_id += 1;
                continue;
            }
        }

        let mut new_ty = ty.clone();
        // The clone bit-copied `defaults` (Value is Copy): the parent's copy
        // takes its own ref per heap default, so the child registry's Drop
        // (post-import) and the parent's each release exactly their own.
        for &d in &new_ty.defaults {
            d.clone_refcount();
        }
        for idx in new_ty.methods.values_mut() {
            *idx += bases.func;
        }
        for idx in new_ty.static_methods.values_mut() {
            *idx += bases.func;
        }
        if let Some(ref mut init) = new_ty.init_fn {
            *init += bases.func;
        }
        let parent_type_id = parent_vm.struct_registry.register_type(new_ty);
        // Carry the generic-union payload templates: an imported `union Option[T]`
        // must still enforce an `Option[int]` payload. Templates are keyed by
        // type_id, which is remapped on transplant, so re-key to the parent id.
        if let Some(templates) = child_types.variant_templates(child_type_id) {
            let templates = templates.to_vec();
            parent_vm
                .struct_registry
                .set_variant_templates(parent_type_id, templates);
        }
        if parent_type_id != child_type_id {
            bases.stype_remap.insert(child_type_id, parent_type_id);
        }
        child_type_id += 1;
    }
}

/// Transplant enum types and symbols from child VM to parent VM.
/// Re-interns symbol names in parent's SymbolTable (indices may differ).
fn transplant_enums(child_vm: &PureVM, parent_vm: &mut PureVM, bases: &mut RemapBases) {
    // Collect child enum data first (to avoid borrow conflicts)
    let mut child_data: Vec<(String, Vec<(String, u32)>)> = Vec::new();
    {
        let child_enums = &child_vm.enum_registry;
        let child_symbols = &child_vm.symbol_table;
        let mut type_id = 0u32;
        while let Some(ety) = child_enums.get_type(type_id) {
            let mut variants_with_qnames = Vec::new();
            for (vname, child_sym_id) in &ety.variants {
                variants_with_qnames.push((vname.clone(), *child_sym_id));
                // Pre-compute symbol remap
                if let Some(qname) = child_symbols.resolve(*child_sym_id) {
                    let parent_sym_id = parent_vm.symbol_table.intern(qname);
                    if parent_sym_id != *child_sym_id {
                        bases.symbol_remap.insert(*child_sym_id, parent_sym_id);
                    }
                }
            }
            child_data.push((ety.name.clone(), variants_with_qnames));
            type_id += 1;
        }
    }

    // Register in parent
    for (child_type_id, (name, variants)) in child_data.iter().enumerate() {
        let variant_names: Vec<String> = variants.iter().map(|(n, _)| n.clone()).collect();
        let parent_type_id = parent_vm
            .enum_registry
            .register(name, &variant_names, &mut parent_vm.symbol_table);
        if parent_type_id != child_type_id as u32 {
            bases.etype_remap.insert(child_type_id as u32, parent_type_id);
        }
        parent_vm.enum_type_names.insert(name.clone(), parent_type_id);
    }

    // Re-intern all remaining child symbols: union nullary variants live
    // outside the enum registry but still flow through exported values.
    let mut sym_id = 0u32;
    while let Some(qname) = child_vm.symbol_table.resolve(sym_id) {
        let parent_sym = parent_vm.symbol_table.intern(qname);
        if parent_sym != sym_id {
            bases.symbol_remap.entry(sym_id).or_insert(parent_sym);
        }
        sym_id += 1;
    }
}

/// Parse a selective import name: "name" or "name:alias".
pub(crate) fn parse_import_name(raw: &str) -> VMResult<(String, String)> {
    if raw.is_empty() {
        return Err(VMError::ValueError("import name cannot be empty".into()));
    }
    match raw.split_once(':') {
        Some((name, alias)) => {
            if name.is_empty() {
                return Err(VMError::ValueError(format!("empty name in import spec '{}'", raw)));
            }
            if alias.is_empty() {
                return Err(VMError::ValueError(format!("empty alias in import spec '{}'", raw)));
            }
            Ok((name.to_string(), alias.to_string()))
        }
        None => Ok((raw.to_string(), raw.to_string())),
    }
}

/// Extract a list of strings from a Value (list, tuple, or set).
/// Returns Err if the value is not a container or contains non-string entries.
fn extract_string_list(v: &Value) -> Result<Vec<String>, String> {
    let require_str = |item: &Value, idx: usize| -> Result<String, String> {
        if item.is_native_str() {
            // SAFETY: the `is_native_str()` guard holds, so the payload is a live
            // native-string Arc and `as_native_str_ref` returns Some.
            Ok(unsafe { item.as_native_str_ref().unwrap() }.to_string())
        } else {
            Err(format!("expected string at index {}, got {}", idx, item.type_name()))
        }
    };

    if v.is_native_list() {
        // SAFETY: the `is_native_list()` guard holds, so the payload is a live
        // list Arc and `as_native_list_ref` returns Some.
        let items = unsafe { v.as_native_list_ref().unwrap() };
        let cloned = items.as_slice_cloned();
        cloned
            .iter()
            .enumerate()
            .map(|(i, item)| require_str(item, i))
            .collect()
    } else if v.is_native_tuple() {
        // SAFETY: the `is_native_tuple()` guard holds, so the payload is a live
        // tuple Arc and `as_native_tuple_ref` returns Some.
        let tuple = unsafe { v.as_native_tuple_ref().unwrap() };
        tuple
            .as_slice()
            .iter()
            .enumerate()
            .map(|(i, item)| require_str(item, i))
            .collect()
    } else if v.is_native_set() {
        // SAFETY: the `is_native_set()` guard holds, so the payload is a live
        // set Arc and `as_native_set_ref` returns Some.
        let set = unsafe { v.as_native_set_ref().unwrap() };
        let vals = set.to_values();
        vals.iter().enumerate().map(|(i, item)| require_str(item, i)).collect()
    } else {
        Err(format!(
            "must be a list, tuple or set of strings, got {}",
            v.type_name()
        ))
    }
}

/// Parse a simple TOML string array like `["a", "b", "c"]`.
fn parse_toml_string_array(s: &str) -> Vec<String> {
    let s = s.trim();
    let s = s.strip_prefix('[').unwrap_or(s);
    let s = s.strip_suffix(']').unwrap_or(s);
    s.split(',')
        .map(|item| item.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Offsets for remapping transplanted values.
struct RemapBases {
    func: u32,
    /// Struct type remapping: child type_id -> parent type_id
    stype_remap: std::collections::HashMap<u32, u32>,
    /// Symbol remapping: child symbol_id -> parent symbol_id
    symbol_remap: std::collections::HashMap<u32, u32>,
    /// Enum type remapping: child type_id -> parent type_id
    etype_remap: std::collections::HashMap<u32, u32>,
}

/// Remap a Value's VMFunc/StructType/Symbol/EnumType index.
///
/// Ownership contract: consumes the caller's reference to `val` and returns
/// an owned value. Callers overwrite the source slot with the result, so a
/// rebuilt enum/union (fresh Arc) must release the old one here -- the three
/// call sites (child globals, cloned constants, closure captures) all own one
/// reference per slot.
fn remap_value(val: Value, bases: &RemapBases) -> Value {
    if val.is_vmfunc() && !val.is_invalid() {
        if bases.func == 0 {
            return val;
        }
        Value::from_vmfunc(val.as_vmfunc_idx() + bases.func)
    } else if val.is_struct_type() {
        if let Some(child_type_id) = val.as_struct_type_id() {
            if let Some(&parent_type_id) = bases.stype_remap.get(&child_type_id) {
                return Value::from_struct_type(parent_type_id);
            }
        }
        val
    } else if val.is_symbol() {
        if let Some(child_sym) = val.as_symbol() {
            if let Some(&parent_sym) = bases.symbol_remap.get(&child_sym) {
                return Value::from_symbol(parent_sym);
            }
        }
        val
    } else if val.is_enum_type() {
        if let Some(child_type_id) = val.as_enum_type_id() {
            if let Some(&parent_type_id) = bases.etype_remap.get(&child_type_id) {
                let remapped = Value::from_enum_type(parent_type_id);
                val.decref();
                return remapped;
            }
        }
        val
    } else if val.is_union_type() {
        // Rebuild the namespace only when a variant binding actually moves:
        // variant bindings carry child struct type ids and symbol ids that
        // must be remapped like direct exports (both flavors are immediates,
        // so the recursive call never touches a refcount).
        // SAFETY: the `is_union_type()` guard holds, so the payload is a live
        // union namespace Arc and `as_union_ref` returns Some.
        let ns = unsafe { val.as_union_ref().unwrap() };
        let variants: Vec<(String, Value)> = ns
            .variants
            .iter()
            .map(|(n, v)| (n.clone(), remap_value(*v, bases)))
            .collect();
        let unchanged = variants
            .iter()
            .zip(ns.variants.iter())
            .all(|((_, new), (_, old))| new.to_raw() == old.to_raw());
        if unchanged {
            return val;
        }
        let remapped = Value::from_union_type(crate::value::UnionNamespace {
            name: ns.name.clone(),
            type_params: ns.type_params.clone(),
            variants,
        });
        val.decref();
        remapped
    } else {
        val
    }
}

// ---------------------------------------------------------------------------
// Stdlib plugin discovery
// ---------------------------------------------------------------------------

/// Build the list of directories to search for stdlib plugin .so files.
///
/// Priority: $CATNIP_STDLIB_PATH > exe dir > target/debug (dev).
fn discover_stdlib_paths() -> Vec<PathBuf> {
    // 1. Environment variable (colon-separated)
    let mut paths = catnip_core::paths::stdlib_env_paths();

    // 2. Next to the executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.is_dir() {
                paths.push(dir.to_path_buf());
            }
            // Also check lib/ subdir
            let lib_dir = dir.join("lib");
            if lib_dir.is_dir() {
                paths.push(lib_dir);
            }
        }
    }

    // 3. Dev mode: workspace target/debug
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let workspace = PathBuf::from(manifest_dir).parent().map(|p| p.to_path_buf());
        if let Some(ws) = workspace {
            let debug_dir = ws.join("target/debug");
            if debug_dir.is_dir() {
                paths.push(debug_dir);
            }
        }
    }

    paths
}

impl PureImportLoader {
    /// Try to load a stdlib module as a native plugin.
    /// Searches stdlib_paths for `libcatnip_{name}.so`.
    /// Override argv/executable in a sys module namespace after plugin load.
    fn configure_sys(&self, ns: &mut ModuleNamespace) {
        // The plugin load path incref's every static attr (argv/executable are
        // exported by the sys descriptor), so a key we overwrite here owns a ref.
        // Decref the displaced value -- symmetric with ModuleNamespace::drop --
        // otherwise the orphaned ref leaks (it never re-enters attrs to be dropped).
        if let Some(ref argv) = self.sys_argv {
            let items: Vec<Value> = argv.iter().map(|s| Value::from_string(s.clone())).collect();
            if let Some(old) = ns.attrs.insert("argv".to_string(), Value::from_list(items)) {
                old.decref();
            }
        }
        if let Some(ref exe) = self.sys_executable {
            if let Some(old) = ns
                .attrs
                .insert("executable".to_string(), Value::from_string(exe.clone()))
            {
                old.decref();
            }
        }
    }

    fn try_load_stdlib_plugin(&self, name: &str) -> VMResult<Option<Value>> {
        let registry = match &self.plugin_registry {
            Some(r) => r,
            None => return Ok(None),
        };

        // Already loaded?
        {
            let reg = registry.borrow();
            if reg.object_callbacks(name).is_some()
                || reg.try_call(&format!("__plugin::{}::__probe", name), &[]).is_some()
            {
                // Module was loaded before -- but we lost the namespace. This shouldn't happen
                // because the cache should have caught it. Return None to fall through.
                return Ok(None);
            }
        }

        // Unit tests dlopen these plugins from target/debug, but nothing in a
        // `cargo test -p catnip_vm` run builds them (workspace members, not
        // dependencies). Build lazily, once per process, instead of depending
        // on the plugin fixture tests having run first.
        #[cfg(test)]
        crate::test_support::ensure_stdlib_plugin(name);

        let lib_name = format!("libcatnip_{}{}", name, crate::plugin::native_suffix());

        let needs_configure = resolve::lookup_stdlib(name).map(|(_, nc)| nc).unwrap_or(false);

        for dir in &self.stdlib_paths {
            let candidate = dir.join(&lib_name);
            if candidate.is_file() {
                let mut ns = registry.borrow_mut().load(&candidate, name)?;
                if needs_configure {
                    self.configure_sys(&mut ns);
                }
                let val = Value::from_module(ns);
                val.clone_refcount();
                self.cache.borrow_mut().insert(name.to_string(), val);
                return Ok(Some(val));
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_module(dir: &Path, name: &str, content: &str) {
        let path = dir.join(format!("{}.cat", name));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn test_import_basic() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "utils", "add = (a, b) => { a + b }\nx = 42");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('utils')\nutils.x").unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_import_function_call() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "math_utils", "double = (x) => { x * 2 }");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('math_utils')\nmath_utils.double(21)").unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_import_closure() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "state", "x = 42\nget_x = () => { x }");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('state')\nstate.get_x()").unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_import_cache() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "counter", "val = 1");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        // Import twice -- should get same cached module
        let result = pipeline
            .execute("import('counter')\nimport('counter')\ncounter.val")
            .unwrap();
        assert_eq!(result.as_int(), Some(1));
    }

    #[test]
    fn test_import_union_across_module() {
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "shapes",
            "union Shape { Circle(r); Origin; }\n\
             is_origin = (s) => {\n    match s {\n        Shape.Circle{r} => { 0 }\n        Shape.Origin => { 1 }\n    }\n}",
        );

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        pipeline.execute("import('shapes')").unwrap();
        // Payload variant: construction + match through the module boundary.
        assert_eq!(
            pipeline
                .execute("shapes.is_origin(shapes.Shape.Circle(1))")
                .unwrap()
                .as_int(),
            Some(0)
        );
        // Nullary variant: the exported symbol must match the pattern
        // compiled inside the module (symbol remap).
        assert_eq!(
            pipeline
                .execute("shapes.is_origin(shapes.Shape.Origin)")
                .unwrap()
                .as_int(),
            Some(1)
        );
        // Structural equality across the boundary.
        assert_eq!(
            pipeline
                .execute("shapes.Shape.Circle(2) == shapes.Shape.Circle(2)")
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            pipeline
                .execute("shapes.Shape.Origin == shapes.Shape.Origin")
                .unwrap()
                .as_bool(),
            Some(true)
        );
    }

    #[test]
    fn test_import_circular_error() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "a", "import('b')");
        make_module(dir.path(), "b", "import('a')");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('a')");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("circular import"), "got: {}", err);
    }

    #[test]
    fn test_import_not_found() {
        let dir = TempDir::new().unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('nonexistent')");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "got: {}", err);
    }

    #[test]
    fn test_import_transitive() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "base", "val = 10");
        make_module(dir.path(), "mid", "import('base')\nresult = base.val + 5");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('mid')\nmid.result").unwrap();
        assert_eq!(result.as_int(), Some(15));
    }

    #[test]
    fn test_import_relative() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("pkg");
        std::fs::create_dir(&sub).unwrap();
        make_module(&sub, "helper", "val = 99");

        // Create a main.cat that does a relative import
        // (relative imports need source_dir context)
        make_module(dir.path(), "main_mod", "import('pkg.helper')");

        // For now test that a bare dotted name resolves via subdirectory
        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        // dotted names are resolved as path segments
        // import('pkg.helper') looks for pkg/helper.cat
        // But the auto-name derivation from desugaring might fail for dotted names
        // Test manual assignment instead
        let result = pipeline.execute("helper = import('pkg.helper')\nhelper.val").unwrap();
        assert_eq!(result.as_int(), Some(99));
    }

    #[test]
    fn test_import_cross_call() {
        // Regression: module where f() calls g(), with parent having pre-existing functions.
        // Tests that VMFunc indices are correctly remapped in child globals.
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "cross",
            "double = (x) => { x * 2 }\napply = (x) => { double(x) + 1 }",
        );

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        // Define a function in parent first to ensure base > 0
        pipeline.execute("parent_fn = (x) => { x }").unwrap();

        let result = pipeline.execute("import('cross')\ncross.apply(5)").unwrap();
        // apply(5) = double(5) + 1 = 10 + 1 = 11
        assert_eq!(result.as_int(), Some(11));
    }

    #[test]
    fn regression_import_nested_lambda() {
        // Exported fn with a nested lambda (baked template constant) + base>0.
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "nested",
            "outer = (x) => { inner = (y) => { y + 1 }\n inner(x) }",
        );
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        pipeline.execute("parent_fn = (x) => { x }").unwrap().decref(); // base > 0
        let result = pipeline.execute("import('nested')\nnested.outer(5)").unwrap();
        assert_eq!(result.as_int(), Some(6), "nested lambda in exported fn");
    }

    #[test]
    fn regression_import_letrec_factory_c2() {
        // C2 regression guard: a module exporting a letrec closure (mutual group in
        // a factory) must not have its captured map cleared by the child VM's drop
        // drain. Fixed by gating drain_closures_on_drop off for child module VMs.
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "muts",
            "pair = () => { even = (n) => { if n == 0 { 1 } else { odd(n - 1) } }\n odd = (n) => { if n == 0 { 0 } else { even(n - 1) } }\n even }\nis_even = pair()",
        );
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        let result = pipeline.execute("import('muts')\nmuts.is_even(4)").unwrap();
        assert_eq!(
            result.as_int(),
            Some(1),
            "exported letrec closure lost its mutual capture"
        );
    }

    #[test]
    fn regression_import_nested_lambda_in_factory_closure() {
        // C1, harder: a factory-returned closure (parent = enclosing Scope, not
        // Globals) whose OWN code bakes a nested-lambda template index. The deep
        // remap must swap its code AND rebuild its scope chain. base>0.
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "nestfac",
            "mk = () => { helper = (x) => { add1 = (y) => { y + 1 }\n add1(x) }\n helper }\nboxed = mk()",
        );
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        pipeline.execute("parent_fn = (x) => { x }").unwrap().decref();
        let result = pipeline.execute("import('nestfac')\nnestfac.boxed(5)").unwrap();
        assert_eq!(
            result.as_int(),
            Some(6),
            "nested lambda in a factory closure resolved wrong template"
        );
    }

    #[test]
    fn regression_import_mutual_recursion_with_nested_lambda() {
        // C1 + C2 together: exported mutual-recursion group (capture cycle, needs
        // slot memoization) where one member ALSO bakes a nested lambda (code swap
        // through the cyclic graph). base>0.
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "mutnest",
            "mk = () => { even = (n) => { chk = (m) => { m == 0 }\n if chk(n) { 1 } else { odd(n - 1) } }\n odd = (n) => { if n == 0 { 0 } else { even(n - 1) } }\n even }\nis_even = mk()",
        );
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        pipeline.execute("parent_fn = (x) => { x }").unwrap().decref();
        let result = pipeline.execute("import('mutnest')\nmutnest.is_even(4)").unwrap();
        assert_eq!(
            result.as_int(),
            Some(1),
            "mutual recursion + nested lambda export failed"
        );
    }

    #[test]
    fn regression_import_closure_capturing_struct() {
        // Refcount balance: an exported closure captures a heap struct. If the deep
        // remap double-frees the capture, `get()` reads a freed instance (None /
        // UAF); if it leaks, the struct simply stays (harmless, cached). base>0.
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "capstruct",
            "struct P { v }\nmake = () => { p = P(42)\n () => { p.v } }\nget = make()",
        );
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        pipeline.execute("parent_fn = (x) => { x }").unwrap().decref();
        let result = pipeline.execute("import('capstruct')\ncapstruct.get()").unwrap();
        assert_eq!(
            result.as_int(),
            Some(42),
            "closure-captured struct was freed by the remap"
        );
    }

    #[test]
    fn regression_import_nested_lambda_in_exported_list() {
        // The closure-graph remap recurses into exported list containers, so a
        // closure baking a nested-lambda template index resolves the parent's
        // transplanted code instead of its child-baked constant. base>0.
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "listfac",
            "f = (x) => { inc = (y) => { y + 1 }\n inc(x) }\nhandlers = [f]",
        );
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        pipeline.execute("parent_fn = (x) => { x }").unwrap().decref();
        let result = pipeline.execute("import('listfac')\nlistfac.handlers[0](5)").unwrap();
        assert_eq!(
            result.as_int(),
            Some(6),
            "closure in an exported list resolved the wrong template"
        );
    }

    #[test]
    fn regression_import_nested_lambda_in_exported_tuple() {
        // Same as the list case for an immutable tuple container.
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "tupfac",
            "f = (x) => { inc = (y) => { y + 1 }\n inc(x) }\nhandlers = tuple(f, f)",
        );
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        pipeline.execute("parent_fn = (x) => { x }").unwrap().decref();
        let result = pipeline.execute("import('tupfac')\ntupfac.handlers[1](5)").unwrap();
        assert_eq!(
            result.as_int(),
            Some(6),
            "closure in an exported tuple resolved the wrong template"
        );
    }

    #[test]
    fn regression_import_nested_lambda_in_exported_dict() {
        // Dict values recurse (keys are primitives, never closures).
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "dictfac",
            "f = (x) => { inc = (y) => { y + 1 }\n inc(x) }\nhandlers = {'go': f}",
        );
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        pipeline.execute("parent_fn = (x) => { x }").unwrap().decref();
        let result = pipeline
            .execute("import('dictfac')\ndictfac.handlers['go'](5)")
            .unwrap();
        assert_eq!(
            result.as_int(),
            Some(6),
            "closure in an exported dict resolved the wrong template"
        );
    }

    #[test]
    fn regression_import_nested_list_shares_closure_identity() {
        // A closure exported both directly and inside a container is remapped
        // once (shared slot_memo), so the two exports stay the same function.
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "sharefac",
            "f = (x) => { inc = (y) => { y + 1 }\n inc(x) }\nhandlers = [f]",
        );
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        pipeline.execute("parent_fn = (x) => { x }").unwrap().decref();
        // Both paths must resolve the same transplanted template.
        let direct = pipeline.execute("import('sharefac')\nsharefac.f(5)").unwrap();
        assert_eq!(direct.as_int(), Some(6));
        let nested = pipeline.execute("sharefac.handlers[0](5)").unwrap();
        assert_eq!(nested.as_int(), Some(6));
    }

    #[test]
    fn regression_import_nested_container_closure_is_refcount_neutral() {
        // The container remap must be refcount-neutral: rebuilding an exported
        // list that holds a closure must leak no *more* runtime slots than
        // exporting the same closure directly. (The absolute per-import leak
        // this once tolerated is closed -- see
        // regression_import_closure_slots_are_reclaimed_at_reset; this oracle
        // still pins that the container rebuild adds none on top.)
        use crate::vm::func_table::live_func_slots;
        let nested = "f = (x) => { inc = (y) => { y + 1 }\n inc(x) }";
        let import_and_call = |body: String, name: &str, call: &str| -> usize {
            let dir = TempDir::new().unwrap();
            make_module(dir.path(), name, &body);
            let mut pipeline = PurePipeline::new().unwrap();
            pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
            let before = live_func_slots();
            pipeline.execute("parent_fn = (x) => { x }").unwrap().decref();
            pipeline.execute(call).unwrap().decref();
            pipeline.reset();
            live_func_slots() - before
        };
        let direct = import_and_call(
            nested.to_string(),
            "neutdirect",
            "import('neutdirect')\nneutdirect.f(5)",
        );
        let container = import_and_call(
            format!("{nested}\nhandlers = [f]"),
            "neutlist",
            "import('neutlist')\nneutlist.handlers[0](5)",
        );
        assert_eq!(
            container,
            direct,
            "container remap leaked {} slot(s) more than the direct export",
            container.saturating_sub(direct)
        );
    }

    #[test]
    fn regression_import_struct_instance_is_reclaimed_at_reset() {
        // Exporting any struct instance used to retain one StructCell past
        // pipeline.reset(): the child's globals map (kept alive through the
        // ModuleNamespace's `module_globals` Rc) owned one ref per entry that
        // no teardown path ever released -- the child host drops without
        // draining and the parent's clear_globals never sees this map.
        // ModuleNamespace::drop now drains it.
        use crate::vm::structs::live_struct_instances;
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "regmod", "struct Reg { v }\nr = Reg(42)");
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        let before = live_struct_instances();
        pipeline.execute("import('regmod')\nregmod.r.v").unwrap().decref();
        pipeline.reset();
        assert_eq!(live_struct_instances(), before, "struct instance leaked past reset");
    }

    #[test]
    fn regression_wild_import_struct_instance_is_reclaimed_at_reset() {
        // Wild import (C2 fix): `import('mod', wild=true)` used to drop the
        // ModuleNamespace without decref, leaking every exported attr and
        // every module-global entry (struct instances, closures, lists).
        use crate::vm::structs::live_struct_instances;
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "rmod", "struct Reg { v }\nr = Reg(42)");
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        let before = live_struct_instances();
        pipeline.execute("import('rmod', wild=true)\nr.v").unwrap().decref();
        pipeline.reset();
        assert_eq!(
            live_struct_instances(),
            before,
            "wild import leaked ModuleNamespace (C2)"
        );
    }

    #[test]
    fn regression_selective_import_struct_instance_is_reclaimed_at_reset() {
        // Selective import (C2 fix): same as wild — `Injected` path dropped
        // the namespace without decref.
        use crate::vm::structs::live_struct_instances;
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "smod", "struct Reg { v }\nr = Reg(42)\ns = Reg(99)");
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        let before = live_struct_instances();
        pipeline.execute("import('smod', 'r')\nr.v").unwrap().decref();
        pipeline.reset();
        assert_eq!(
            live_struct_instances(),
            before,
            "selective import leaked ModuleNamespace (C2)"
        );
    }

    #[test]
    fn regression_import_closure_slots_are_reclaimed_at_reset() {
        // Same mechanism as the struct-instance leak: the exported closure's
        // child-globals ref was never released, retaining its runtime slot
        // (and transitively its nested lambda) past reset.
        use crate::vm::func_table::live_func_slots;
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "slotmod", "f = (x) => { inc = (y) => { y + 1 }\n inc(x) }");
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        let before = live_func_slots();
        pipeline.execute("import('slotmod')\nslotmod.f(5)").unwrap().decref();
        pipeline.reset();
        assert_eq!(live_func_slots(), before, "runtime slots leaked past reset");
    }

    #[test]
    fn regression_import_sibling_closure_group_reclaimed_at_reset() {
        // ABSOLUTE oracle (the differential twin below is blind to a leak that
        // hits both sides): a sibling-defined closure group carries strong
        // cross-captures (backward sibling capture + forward PatchClosure),
        // and only a VM reset drain breaks that cycle. The child's drain is
        // off, so its handles must be transferred to the parent. Covers both
        // a real letrec and a non-recursive pair (the patch is emitted for
        // every sibling pair, recursion or not).
        use crate::vm::func_table::live_func_slots;
        let run = |body: &str, name: &str| -> usize {
            let dir = TempDir::new().unwrap();
            make_module(dir.path(), name, body);
            let mut p = PurePipeline::new().unwrap();
            p.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
            let before = live_func_slots();
            p.execute(&format!("import('{name}')")).unwrap().decref();
            p.reset();
            live_func_slots() - before
        };
        let pair = run(
            "make = () => { f = (x) => { x + 1 }\n g = (x) => { x + 2 }\n [f, g] }\nh = make()",
            "sibpair",
        );
        assert_eq!(pair, 0, "non-recursive sibling group leaked {pair} slot(s)");
        let letrec = run(
            "make = () => { f = () => { g() }\n g = () => { f() }\n [f, g] }\nh = make()",
            "sibrec",
        );
        assert_eq!(letrec, 0, "letrec group leaked {letrec} slot(s)");
    }

    #[test]
    fn regression_import_remapped_letrec_group_reclaimed_at_reset() {
        // Same oracle through the deep-remap path (parent defines a function
        // first so func_base > 0): the rebuilt closures recreate the group's
        // cycles and must register with the parent's reset drain. Also checks
        // the remapped group still WORKS before reset (correctness first).
        use crate::vm::func_table::live_func_slots;
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "remaprec",
            "make = () => {\n even = (n) => { if n == 0 { true } else { odd(n - 1) } }\n \
             odd = (n) => { if n == 0 { false } else { even(n - 1) } }\n [even, odd] }\nh = make()",
        );
        let mut p = PurePipeline::new().unwrap();
        p.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        let before = live_func_slots();
        p.execute("parent_fn = (x) => { x }").unwrap().decref();
        let r = p.execute("m = import('remaprec')\nm.h[0](4)").unwrap();
        assert_eq!(
            r.as_bool(),
            Some(true),
            "remapped letrec closure returned the wrong result"
        );
        r.decref();
        p.reset();
        assert_eq!(live_func_slots() - before, 0, "remapped letrec group leaked past reset");
    }

    #[test]
    fn regression_import_mutual_recursion_in_container_no_extra_leak() {
        // A mutually-recursive closure group exported only inside a container must
        // not leak its old (pre-remap) copies. The rebuild replaces the container
        // and drops the old one; the old group's cross-captures form a strong Arc
        // cycle that the driver's cycle-break must reach *inside* the old
        // container, not only at top level. Differential vs a non-recursive group
        // in the same shape isolates the cycle leak.
        use crate::vm::func_table::live_func_slots;
        let import = |body: String, name: &str| -> usize {
            let dir = TempDir::new().unwrap();
            make_module(dir.path(), name, &body);
            let mut p = PurePipeline::new().unwrap();
            p.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
            let before = live_func_slots();
            p.execute("parent_fn = (x) => { x }").unwrap().decref();
            p.execute(&format!("import('{name}')")).unwrap().decref();
            p.reset();
            live_func_slots() - before
        };
        let noncyclic = import(
            "make = () => { f = (x) => { x + 1 }\n g = (x) => { x + 2 }\n [f, g] }\nh = make()".to_string(),
            "mrnon",
        );
        let cyclic = import(
            "make = () => { f = () => { g() }\n g = () => { f() }\n [f, g] }\nh = make()".to_string(),
            "mrcyc",
        );
        assert_eq!(
            cyclic,
            noncyclic,
            "mutual-recursion group nested in a container leaked {} old slot(s)",
            cyclic.saturating_sub(noncyclic)
        );
    }

    #[test]
    fn regression_import_nested_lambda_in_exported_struct_field() {
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "structfac",
            "struct Reg { h }\nf = (x) => { inc = (y) => { y + 1 }\n inc(x) }\nr = Reg(f)",
        );
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        pipeline.execute("parent_fn = (x) => { x }").unwrap().decref();
        let result = pipeline.execute("import('structfac')\nstructfac.r.h(5)").unwrap();
        assert_eq!(
            result.as_int(),
            Some(6),
            "closure in an exported struct field resolved the wrong template"
        );
    }

    #[test]
    fn regression_import_struct_instance_type_id_is_remapped() {
        // A parent-defined struct type shifts the transplant ids: without the
        // instance remap, the exported instance's child-baked type_id resolves
        // the parent's `Pad` (silent wrong type / missing field).
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "shiftmod", "struct Reg { v }\nr = Reg(42)");
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        pipeline.execute("struct Pad { x }").unwrap().decref();
        let result = pipeline.execute("import('shiftmod')\nshiftmod.r.v").unwrap();
        assert_eq!(
            result.as_int(),
            Some(42),
            "imported struct instance kept its child-baked type_id"
        );
    }

    #[test]
    fn regression_import_rebuilt_struct_instance_is_reclaimed_at_reset() {
        // ABSOLUTE oracle: the struct-instance rebuild must stay refcount-
        // neutral. An instance forced through the rebuild on both axes
        // (closure field -> remap_elems change, parent struct -> type_id
        // shift) is reclaimed at reset like a plain exported one, and the
        // closure it carries frees its runtime slot.
        use crate::vm::func_table::live_func_slots;
        use crate::vm::structs::live_struct_instances;
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "rebmod",
            "struct Reg { h }\nf = (x) => { inc = (y) => { y + 1 }\n inc(x) }\nr = Reg(f)",
        );
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        let cells_before = live_struct_instances();
        let slots_before = live_func_slots();
        pipeline
            .execute("struct Pad { x }\nparent_fn = (x) => { x }")
            .unwrap()
            .decref();
        let result = pipeline.execute("import('rebmod')\nrebmod.r.h(5)").unwrap();
        assert_eq!(
            result.as_int(),
            Some(6),
            "rebuilt struct-field closure resolved the wrong template"
        );
        pipeline.reset();
        assert_eq!(
            live_struct_instances(),
            cells_before,
            "rebuilt struct instance leaked past reset"
        );
        assert_eq!(
            live_func_slots(),
            slots_before,
            "struct-field closure slot leaked past reset"
        );
    }

    #[test]
    fn test_import_cache_is_scoped_by_child_source_dir() {
        let dir = TempDir::new().unwrap();
        let dir_a = dir.path().join("a");
        let dir_b = dir.path().join("b");
        std::fs::create_dir(&dir_a).unwrap();
        std::fs::create_dir(&dir_b).unwrap();
        make_module(&dir_a, "helper", "val = 10");
        make_module(&dir_b, "helper", "val = 20");
        make_module(&dir_a, "main", "helper = import('helper')\nvalue = helper.val");
        make_module(&dir_b, "main", "helper = import('helper')\nvalue = helper.val");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline
            .execute("a = import('a.main')\nb = import('b.main')\na.value * 100 + b.value")
            .unwrap();
        assert_eq!(result.as_int(), Some(1020));
    }

    #[test]
    fn test_package_path_traversal() {
        let dir = TempDir::new().unwrap();
        let pkg = dir.path().join("evil_pkg");
        std::fs::create_dir(&pkg).unwrap();

        // lib.toml with path traversal under [lib]
        let lib_toml = pkg.join("lib.toml");
        std::fs::write(&lib_toml, "[lib]\nentry = \"../outside.cat\"").unwrap();

        // Create the target file outside the package
        make_module(dir.path(), "outside", "val = 666");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('evil_pkg')");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("escapes"), "got: {}", err);
    }

    #[test]
    fn test_reset_preserves_loader() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "mod1", "x = 1");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        // Reset clears state but preserves import loader
        pipeline.reset();

        let result = pipeline.execute("import('mod1')\nmod1.x").unwrap();
        assert_eq!(result.as_int(), Some(1));
    }

    // -----------------------------------------------------------------------
    // Import parity tests (protocol, wild, selective, __all__, exports)
    // -----------------------------------------------------------------------

    #[test]
    fn test_import_protocol_cat() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "utils", "x = 42");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("m = import('utils', protocol=\"cat\")\nm.x").unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_import_protocol_py_error() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "utils", "x = 1");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('utils', protocol=\"py\")");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not available"), "got: {}", err);
    }

    #[test]
    fn test_import_protocol_unknown() {
        let dir = TempDir::new().unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('utils', protocol=\"java\")");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown protocol"), "got: {}", err);
    }

    #[test]
    fn test_import_generic_union_enforces_payload() {
        // Regression: a generic union imported from a module keeps enforcing its
        // payload -- `variant_templates` must travel with the transplanted types
        // (they are keyed by type_id, remapped on transplant).
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "opt", "union Option[T] { Some(value: T); None }\n");

        let mut good = PurePipeline::new().unwrap();
        good.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        assert!(
            good.execute("import('opt', wild=true)\nf = (x: Option[int]) => { 1 }\nf(Option.Some(1))")
                .is_ok(),
            "good payload should pass across the import"
        );

        let mut bad = PurePipeline::new().unwrap();
        bad.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        assert!(
            bad.execute("import('opt', wild=true)\nf = (x: Option[int]) => { 1 }\nf(Option.Some(\"s\"))")
                .is_err(),
            "mistyped payload must be rejected across the import (templates transplanted)"
        );
    }

    #[test]
    fn test_import_wild() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "helpers", "x = 10\ny = 20\n_private = 99");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        // Wild import injects x and y, but not _private
        let result = pipeline.execute("import('helpers', wild=true)\nx + y").unwrap();
        assert_eq!(result.as_int(), Some(30));
    }

    #[test]
    fn test_import_wild_excludes_private() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "priv", "_secret = 42\npublic = 1");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        pipeline.execute("import('priv', wild=true)").unwrap();
        // _secret should not be accessible
        let result = pipeline.execute("_secret");
        assert!(result.is_err(), "_secret should not be injected by wild import");
    }

    #[test]
    fn test_import_wild_selective_conflict() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "stuff", "a = 1");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('stuff', 'a', wild=true)");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cannot combine"), "got: {}", err);
    }

    #[test]
    fn test_import_selective() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "math_lib", "pi = 314\ne = 271\ntau = 628");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('math_lib', 'pi', 'e')\npi + e").unwrap();
        assert_eq!(result.as_int(), Some(585));
    }

    #[test]
    fn test_import_selective_alias() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "data", "value = 42");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('data', 'value:v')\nv").unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_import_selective_missing() {
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "small", "x = 1");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('small', 'nonexistent')");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "got: {}", err);
    }

    #[test]
    fn test_import_all_filtering() {
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "filtered",
            "__all__ = list(\"public_a\", \"public_b\")\npublic_a = 10\npublic_b = 20\nhidden = 99",
        );

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        // Normal import: __all__ filters namespace
        let result = pipeline
            .execute("m = import('filtered')\nm.public_a + m.public_b")
            .unwrap();
        assert_eq!(result.as_int(), Some(30));
    }

    #[test]
    fn test_import_wild_respects_all() {
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "restricted",
            "__all__ = list(\"exported\")\nexported = 42\ninternal = 99",
        );

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('restricted', wild=true)\nexported").unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_package_exports_include() {
        let dir = TempDir::new().unwrap();
        let pkg = dir.path().join("mypkg");
        std::fs::create_dir(&pkg).unwrap();

        // lib.toml with exports.include
        std::fs::write(
            pkg.join("lib.toml"),
            "[lib]\nentry = \"main.cat\"\n\n[lib.exports]\ninclude = [\"api_func\", \"version\"]\n",
        )
        .unwrap();
        make_module(&pkg, "main", "api_func = 100\nversion = 1\n_internal = 999\nhidden = 0");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("m = import('mypkg')\nm.api_func + m.version").unwrap();
        assert_eq!(result.as_int(), Some(101));
    }

    #[test]
    fn test_package_exports_include_intersects_all() {
        // Regression: lib.exports.include must post-filter __all__, not replace it.
        // If __all__ hides "hidden" and include asks for it, it stays hidden.
        let dir = TempDir::new().unwrap();
        let pkg = dir.path().join("both");
        std::fs::create_dir(&pkg).unwrap();

        std::fs::write(
            pkg.join("lib.toml"),
            "[lib]\nentry = \"main.cat\"\n\n[lib.exports]\ninclude = [\"a\", \"hidden\"]\n",
        )
        .unwrap();
        // __all__ only exports "a" and "b", not "hidden"
        make_module(&pkg, "main", "__all__ = list(\"a\", \"b\")\na = 1\nb = 2\nhidden = 99");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        // "a" is in both __all__ and include → visible
        let result = pipeline.execute("m = import('both')\nm.a").unwrap();
        assert_eq!(result.as_int(), Some(1));

        // "hidden" is in include but NOT in __all__ → must stay hidden
        let err = pipeline.execute("m.hidden");
        assert!(err.is_err(), "hidden should not be accessible when __all__ excludes it");
    }

    #[test]
    fn test_package_entry_scoped_to_lib_section() {
        // Regression: entry = ... in unrelated sections must be ignored.
        let dir = TempDir::new().unwrap();
        let pkg = dir.path().join("scoped");
        std::fs::create_dir(&pkg).unwrap();

        // [tool.release] has its own "entry" key -- must not be picked up
        std::fs::write(
            pkg.join("lib.toml"),
            "[tool.release]\nentry = \"wrong.cat\"\n\n[lib]\nentry = \"right.cat\"\n",
        )
        .unwrap();
        make_module(&pkg, "right", "val = 42");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("m = import('scoped')\nm.val").unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_package_entry_top_level_ignored() {
        // Regression: top-level entry = ... (before any section) must be ignored.
        // Only [lib].entry is valid, matching the Python loader behavior.
        let dir = TempDir::new().unwrap();
        let pkg = dir.path().join("toplevel");
        std::fs::create_dir(&pkg).unwrap();

        // Top-level entry should be ignored; [lib] section has the real one
        std::fs::write(
            pkg.join("lib.toml"),
            "entry = \"wrong.cat\"\n\n[lib]\nentry = \"right.cat\"\n",
        )
        .unwrap();
        make_module(&pkg, "right", "val = 7");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("m = import('toplevel')\nm.val").unwrap();
        assert_eq!(result.as_int(), Some(7));
    }

    #[test]
    fn test_parse_import_name() {
        // Basic name
        let (name, alias) = parse_import_name("foo").unwrap();
        assert_eq!(name, "foo");
        assert_eq!(alias, "foo");

        // Name with alias
        let (name, alias) = parse_import_name("foo:bar").unwrap();
        assert_eq!(name, "foo");
        assert_eq!(alias, "bar");

        // Empty string
        assert!(parse_import_name("").is_err());

        // Empty name
        assert!(parse_import_name(":bar").is_err());

        // Empty alias
        assert!(parse_import_name("foo:").is_err());
    }

    // -- Native plugin integration tests --

    fn build_hello_plugin() -> PathBuf {
        crate::test_support::hello_plugin()
    }

    #[test]
    fn test_import_native_plugin_via_pipeline() {
        let so_path = build_hello_plugin();

        // Put symlink in a temp dir so the resolver can find it
        let dir = TempDir::new().unwrap();
        let link = dir.path().join(format!("hello{}", crate::plugin::native_suffix()));
        std::os::unix::fs::symlink(&so_path, &link).unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let mut loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        loader.set_plugin_registry(pipeline.plugin_registry().clone());
        pipeline.set_import_loader(loader);

        // greet
        let result = pipeline
            .execute("hello = import('hello', protocol='rs')\nhello.greet()")
            .unwrap();
        let s = unsafe { result.as_native_str_ref().unwrap() };
        assert_eq!(s, "hello!");

        // add
        let result = pipeline.execute("hello.add(10, 32)").unwrap();
        assert_eq!(result.as_int(), Some(42));

        // VERSION
        let result = pipeline.execute("hello.VERSION").unwrap();
        let v = unsafe { result.as_native_str_ref().unwrap() };
        assert_eq!(v, "0.1.0");
    }

    #[test]
    fn test_import_unknown_module_fails() {
        let dir = TempDir::new().unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        // No .cat, no .so, no stdlib -> fails
        let result = pipeline.execute("import('nonexistent_xyz')");
        assert!(result.is_err());
    }

    #[test]
    fn test_set_import_loader_inherits_plugin_registry() {
        let so_path = build_hello_plugin();
        let dir = TempDir::new().unwrap();
        let link = dir.path().join(format!("hello{}", crate::plugin::native_suffix()));
        std::os::unix::fs::symlink(&so_path, &link).unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        // set_import_loader should auto-bind the pipeline's registry
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline
            .execute("hello = import('hello', protocol='rs')\nhello.greet()")
            .unwrap();
        let s = unsafe { result.as_native_str_ref().unwrap() };
        assert_eq!(s, "hello!");
    }

    #[test]
    fn test_reset_allows_reimport_native_plugin() {
        let so_path = build_hello_plugin();
        let dir = TempDir::new().unwrap();
        let link = dir.path().join(format!("hello{}", crate::plugin::native_suffix()));
        std::os::unix::fs::symlink(&so_path, &link).unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        // First import
        pipeline
            .execute("hello = import('hello', protocol='rs')\nhello.greet()")
            .unwrap();

        // Reset and reimport -- must not fail with "already loaded"
        pipeline.reset();
        let loader2 = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader2);

        let result = pipeline
            .execute("hello = import('hello', protocol='rs')\nhello.add(1, 2)")
            .unwrap();
        assert_eq!(result.as_int(), Some(3));
    }

    #[test]
    fn test_qualify_spec_absolute() {
        let loader = PureImportLoader::new(None);
        assert_eq!(loader.qualify_spec("math"), "math");
        assert_eq!(loader.qualify_spec("pkg.sub"), "pkg.sub");
    }

    #[test]
    fn test_qualify_spec_relative_with_context() {
        let mut loader = PureImportLoader::new(None);
        loader.module_name = Some("pkg".to_string());

        // .secret in pkg → pkg.secret
        assert_eq!(loader.qualify_spec(".secret"), "pkg.secret");

        // ..utils in pkg → utils (goes above pkg)
        assert_eq!(loader.qualify_spec("..utils"), "utils");
    }

    #[test]
    fn test_qualify_spec_relative_nested() {
        let mut loader = PureImportLoader::new(None);
        loader.module_name = Some("pkg.sub".to_string());

        // .foo in pkg.sub → pkg.sub.foo
        assert_eq!(loader.qualify_spec(".foo"), "pkg.sub.foo");

        // ..utils in pkg.sub → pkg.utils
        assert_eq!(loader.qualify_spec("..utils"), "pkg.utils");

        // ...deep in pkg.sub → deep (goes above pkg)
        assert_eq!(loader.qualify_spec("...deep"), "deep");
    }

    #[test]
    fn test_qualify_spec_no_context() {
        let loader = PureImportLoader::new(None);
        // No module_name: fallback to bare tail
        assert_eq!(loader.qualify_spec(".secret"), "secret");
    }

    #[test]
    fn test_relative_import_policy_check() {
        let dir = TempDir::new().unwrap();
        let pkg = dir.path().join("pkg");
        std::fs::create_dir(&pkg).unwrap();
        make_module(&pkg, "allowed", "x = 1");
        make_module(&pkg, "denied", "x = 2");
        make_module(&pkg, "main", "import('.denied')");

        let mut pipeline = PurePipeline::new().unwrap();
        let mut loader = PureImportLoader::new(Some(pkg.to_path_buf()));
        loader.module_name = Some("pkg".to_string());
        let policy =
            catnip_core::policy::ModulePolicyCore::create("allow", vec![], vec!["pkg.denied".to_string()]).unwrap();
        loader.set_policy(policy);
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('.denied')");
        assert!(result.is_err(), "relative import should be blocked by policy");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("blocked"), "got: {}", err);
    }

    // -----------------------------------------------------------------------
    // Parity tests: META.protocol, META.exports, baseline heuristic
    // -----------------------------------------------------------------------

    #[test]
    fn test_meta_protocol_set() {
        // META.protocol must be 'cat' inside an imported module.
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "probe", "proto = META.protocol");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("m = import('probe')\nm.proto").unwrap();
        let s = unsafe { result.as_native_str_ref().unwrap() };
        assert_eq!(s, "cat");
    }

    #[test]
    fn test_meta_exports_priority() {
        // META.exports restricts exports to exactly the listed names.
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "restricted",
            "x = 10\ny = 20\nsecret = 99\nMETA.exports = list(\"x\", \"y\")",
        );

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("m = import('restricted')\nm.x + m.y").unwrap();
        assert_eq!(result.as_int(), Some(30));

        // 'secret' must not be accessible
        let err = pipeline.execute("m.secret");
        assert!(err.is_err(), "secret should not be exported");
    }

    #[test]
    fn test_meta_exports_over_all() {
        // META.exports takes priority over __all__.
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "prio",
            "__all__ = list(\"a\", \"b\")\na = 1\nb = 2\nc = 3\nMETA.exports = list(\"c\")",
        );

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("m = import('prio')\nm.c").unwrap();
        assert_eq!(result.as_int(), Some(3));

        // 'a' is in __all__ but not in META.exports -> hidden
        let err = pipeline.execute("m.a");
        assert!(
            err.is_err(),
            "a should not be exported when META.exports overrides __all__"
        );
    }

    #[test]
    fn test_builtin_redefined_exported() {
        // A module that redefines a builtin name should export the new value.
        // This tests the baseline heuristic (compare before/after execution).
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "shadow", "len = 42");

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("m = import('shadow')\nm.len").unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    // --- regression: META.exports wrong type (C1) ---
    //
    // When META.exports is not a string list (e.g. META.exports = 42),
    // extract_string_list returns Err and the ? early-returned without
    // draining the child's globals, leaking one ref per module-level
    // heap value. Fixed: drain + clear_globals on the error path.

    #[test]
    fn regression_meta_exports_wrong_type_drains_globals() {
        use crate::vm::structs::live_struct_instances;
        let dir = TempDir::new().unwrap();
        make_module(
            dir.path(),
            "metawrong",
            "struct Reg { v }\nr = Reg(42)\nMETA.exports = 42",
        );
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        let before = live_struct_instances();
        let result = pipeline.execute("import('metawrong')");
        assert!(result.is_err(), "META.exports = 42 must raise TypeError");
        pipeline.reset();
        assert_eq!(
            live_struct_instances(),
            before,
            "META.exports wrong-type error leaked struct instance (C1)"
        );
    }

    // --- regression: selective import missing name (C1-bis) ---
    //
    // When a selective import names an absent symbol after one that
    // exists, the already clone_refcount'd injections leaked plus the
    // namespace was never decref'd. E.g. import('mod', 'r', 'nope').

    #[test]
    fn regression_selective_import_missing_name_reclaims() {
        use crate::vm::structs::live_struct_instances;
        let dir = TempDir::new().unwrap();
        make_module(dir.path(), "c1bmod", "struct Reg { v }\nr = Reg(42)\ns = Reg(99)");
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        let before = live_struct_instances();
        // 'r' exists, 'nope' does not
        let result = pipeline.execute("import('c1bmod', 'r', 'nope')");
        assert!(result.is_err(), "selective import of missing name must fail");
        pipeline.reset();
        assert_eq!(
            live_struct_instances(),
            before,
            "selective import missing-name error leaked namespace or injections (C1-bis)"
        );
    }

    // --- regression: execute error path drains closure cycles (C1-ter) ---
    //
    // When a module's execute raises an error after defining sibling
    // closures, clear_globals was called but drain_closures_on_drop was
    // still false. Sibling closures carry strong cross-captures
    // (backward + forward PatchClosure) forming an Rc cycle that only
    // the Drop drain breaks. Fixed: set drain_closures_on_drop = true
    // on the error branch.

    #[test]
    fn regression_execute_error_drains_sibling_closure_cycle() {
        use crate::vm::func_table::live_func_slots;
        let dir = TempDir::new().unwrap();
        // Sibling closures with mutual capture (strong cross-captures),
        // then a guaranteed runtime error
        make_module(
            dir.path(),
            "siberr",
            "make = () => { f = () => { g() }\n g = () => { f() }\n [f, g] }\nh = make()\nraise 'boom'",
        );
        let mut pipeline = PurePipeline::new().unwrap();
        pipeline.set_import_loader(PureImportLoader::new(Some(dir.path().to_path_buf())));
        let before = live_func_slots();
        let result = pipeline.execute("import('siberr')");
        assert!(result.is_err(), "module with raise must error");
        pipeline.reset();
        assert_eq!(
            live_func_slots(),
            before,
            "execute-error path leaked sibling closure cycle (C1-ter)"
        );
    }

    // --- remap_value ownership (owned in, owned out) ---
    //
    // The call sites own one reference per slot and overwrite it with the
    // result, so a rebuilt enum/union must release the input reference.
    // Oracle: extended_strong_count on a probe copy (same pattern as the
    // CodeObject pool tests).

    fn union_value(variants: Vec<(String, Value)>) -> Value {
        Value::from_union_type(crate::value::UnionNamespace {
            name: "Shape".into(),
            type_params: vec![],
            variants,
        })
    }

    #[test]
    fn remap_value_union_releases_old_namespace() {
        let mut bases = RemapBases {
            func: 0,
            stype_remap: std::collections::HashMap::new(),
            symbol_remap: std::collections::HashMap::new(),
            etype_remap: std::collections::HashMap::new(),
        };
        bases.stype_remap.insert(3, 10);

        let old = union_value(vec![
            ("Circle".into(), Value::from_struct_type(3)),
            ("Origin".into(), Value::from_symbol(7)),
        ]);
        old.clone_refcount(); // probe ref, on top of the slot ref consumed below
        assert_eq!(old.extended_strong_count(), 2);

        let new = remap_value(old, &bases);
        assert_ne!(new.to_raw(), old.to_raw(), "remapped variant forces a rebuild");
        assert_eq!(
            old.extended_strong_count(),
            1,
            "rebuild must release the input reference (probe only)"
        );
        // SAFETY: the probe ref keeps the old namespace alive; the new value owns its Arc.
        let ns = unsafe { new.as_union_ref().unwrap() };
        assert_eq!(ns.variant("Circle").unwrap().as_struct_type_id(), Some(10));
        assert_eq!(ns.variant("Origin").unwrap().as_symbol(), Some(7));

        old.decref();
        new.decref();
    }

    #[test]
    fn remap_value_union_unchanged_returns_same_arc() {
        let mut bases = RemapBases {
            func: 0,
            stype_remap: std::collections::HashMap::new(),
            symbol_remap: std::collections::HashMap::new(),
            etype_remap: std::collections::HashMap::new(),
        };
        bases.stype_remap.insert(99, 100); // applies to no variant

        let old = union_value(vec![
            ("Circle".into(), Value::from_struct_type(3)),
            ("Origin".into(), Value::from_symbol(7)),
        ]);
        let new = remap_value(old, &bases);
        assert_eq!(new.to_raw(), old.to_raw(), "no remapped variant, no rebuild");
        assert_eq!(old.extended_strong_count(), 1, "refcount untouched");

        old.decref();
    }

    #[test]
    fn remap_value_enum_releases_old() {
        let mut bases = RemapBases {
            func: 0,
            stype_remap: std::collections::HashMap::new(),
            symbol_remap: std::collections::HashMap::new(),
            etype_remap: std::collections::HashMap::new(),
        };
        bases.etype_remap.insert(5, 9);

        let old = Value::from_enum_type(5);
        old.clone_refcount(); // probe
        assert_eq!(old.extended_strong_count(), 2);

        let new = remap_value(old, &bases);
        assert_eq!(new.as_enum_type_id(), Some(9));
        assert_eq!(
            old.extended_strong_count(),
            1,
            "rebuild must release the input reference (probe only)"
        );

        old.decref();
        new.decref();
    }
}
