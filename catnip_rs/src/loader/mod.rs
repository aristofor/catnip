// FILE: catnip_rs/src/loader/mod.rs
pub mod cache;
pub mod namespace;
pub mod native_plugin;
pub use catnip_core::loader::resolve;

use cache::ModuleCache;
use namespace::ModuleNamespace;
use resolve::{ModuleKind, PROTOCOLS};

use pyo3::PyTraverseError;
use pyo3::exceptions::{PyAttributeError, PyFileNotFoundError, PyRuntimeError, PyTypeError, PyValueError};
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule, PyTuple};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::constants;
use crate::vm::Value;
use crate::vm::frame::Globals;
use crate::vm::host::GlobalsProxy;

/// Rust replacement for `_ImportWrapper` + `ModuleLoader`.
/// Callable `#[pyclass]` injected as the `import` builtin.
#[pyclass(name = "ImportLoader", module = "catnip._rs", unsendable)]
pub struct ImportLoader {
    globals: Globals,
    policy: Option<Py<PyAny>>,
    cat_loader: Option<Py<PyAny>>,
    context: Option<Py<PyAny>>,
    cache: ModuleCache,
    #[allow(dead_code)]
    verbose: bool,
    native_suffix: String,
    /// Feeder struct registry id, so the extension `context.globals` proxy
    /// releases a displaced struct global struct-aware (`0` if unknown, e.g. a
    /// Python-constructed loader over a registry-less proxy).
    registry_id: u64,
    /// Registry of loaded native catnip_vm plugins (PureVM-only stdlib like
    /// `http`). Kept alive so the raw call pointers held by `NativePluginFn`
    /// stay valid for as long as the loader lives.
    plugin_registry: catnip_vm::plugin::SharedPluginRegistry,
}

#[pymethods]
impl ImportLoader {
    #[new]
    #[pyo3(signature = (globals_proxy, policy=None, cat_loader=None, verbose=false, context=None))]
    fn new(
        py: Python<'_>,
        globals_proxy: &GlobalsProxy,
        policy: Option<Py<PyAny>>,
        cat_loader: Option<Py<PyAny>>,
        verbose: bool,
        context: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        // Extract Globals Rc from the proxy
        let globals = globals_proxy.globals_rc();
        let registry_id = globals_proxy.registry_id();

        // Get the platform native extension suffix
        let sysconfig = py.import("sysconfig")?;
        let suffix: String = sysconfig
            .call_method1("get_config_var", ("EXT_SUFFIX",))?
            .extract()
            .unwrap_or_else(|_| ".so".to_string());

        Ok(Self {
            globals,
            policy,
            cat_loader,
            context,
            cache: ModuleCache::new(),
            verbose,
            native_suffix: suffix,
            plugin_registry: Rc::new(std::cell::RefCell::new(catnip_vm::plugin::PluginRegistry::new())),
            registry_id,
        })
    }

    /// Participate in CPython's cyclic GC. The loader is injected as the `import`
    /// builtin and held by `_ImportWrapper._rust_import` in the context globals,
    /// while it holds the context back via its `context` field -- a
    /// `Context <-> ImportLoader` cycle the collector cannot see (a Rust pyclass
    /// is opaque to it). Visiting the owned `Py` references lets the collector
    /// detect the cycle; `__clear__` breaks it. Without this, every `Catnip()`
    /// session leaks its context.
    ///
    /// Besides the directly owned `Option<Py<PyAny>>` fields, the VM `globals`
    /// (shared `Rc`) are surfaced too: `inject_globals` copied the context's
    /// builtins -- including the wrappers and `RUNTIME` that reference the
    /// context back -- into that map as `Value` handles, and the global
    /// `OBJECT_TABLE` holds the matching strong `Py` references invisibly to
    /// the collector. After the owning session dies, reporting them lets the
    /// collector see the whole context cycle and detach it -- even while
    /// leaked constant handles (CodeObject pools, cf. wip/DEFAUTS_SOURNOIS.md)
    /// still pin some slots. `try_borrow` guards against a re-entrant GC
    /// mid-execution.
    ///
    /// This report alone once cleared a LIVE pipeline's builtins (NameError
    /// moving with the GC's allocation thresholds, cf. check_doc_assertions on
    /// FOLD_GUIDE, 2026-07-02): the pipeline reaches the wrapper cluster only
    /// through the map (opaque Rust), so with no Python reference into it the
    /// cluster looked like a closed dead cycle. The counterpart is
    /// `PyPipeline::__traverse__`, which reports the same handles from the
    /// map's owner: an externally-referenced pipeline marks the cluster
    /// reachable, so the collector can never clear it under a live pipeline.
    /// A double report can only over-pin a cluster that is legitimately
    /// alive during that collection, never free a live one.
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        if let Some(ref policy) = self.policy {
            visit.call(policy)?;
        }
        if let Some(ref cat_loader) = self.cat_loader {
            visit.call(cat_loader)?;
        }
        if let Some(ref context) = self.context {
            visit.call(context)?;
        }
        if let Ok(globals) = self.globals.try_borrow() {
            // Dedup-by-slot lives in `visit_obj_handles`: several globals may
            // share one ObjectTable handle (a type bound under two names), and
            // the slot owns a single `Py` reference -- visiting it once per
            // handle would over-count and pin the cycle.
            crate::vm::value::visit_obj_handles(globals.values().copied(), &visit)?;
        }
        Ok(())
    }

    /// Break the `Context <-> ImportLoader` cycle by dropping the strong
    /// references reported by `__traverse__`. Only reached when the whole
    /// cluster -- pipeline included, or the pipeline already dead -- is
    /// unreachable (`PyPipeline::__traverse__` keeps it reachable otherwise).
    /// `Value` is `Copy` with manual refcounting, so each handle must be
    /// `decref`'d before the map is cleared, or the `OBJECT_TABLE` slots leak.
    ///
    /// Counterpart of the deduped `__traverse__`, but **must not** dedup: a
    /// slot reached by `k` aliased globals holds `k` handle refcounts, so all
    /// `k` must be released to reach 0. Idempotent with `VMHost`'s `Drop` /
    /// `gc_clear` drain (draining an already-drained map is a no-op).
    fn __clear__(&mut self) {
        self.policy = None;
        self.cat_loader = None;
        self.context = None;
        if let Ok(mut globals) = self.globals.try_borrow_mut() {
            for (_, value) in globals.drain(..) {
                value.decref();
            }
        }
    }

    /// Main entry point: import(spec, *names, wild=False, protocol=None, caller_dir=None)
    ///
    /// `caller_dir` overrides the META.file-derived directory; used by the AST
    /// executor whose META lives in the Python context, not in the VM globals.
    #[pyo3(signature = (spec, *names, wild=false, protocol=None, caller_dir=None))]
    fn __call__(
        &mut self,
        py: Python<'_>,
        spec: &str,
        names: &Bound<'_, PyTuple>,
        wild: bool,
        protocol: Option<&str>,
        caller_dir: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        // Validate spec
        let validated = resolve::validate_spec(spec).map_err(PyRuntimeError::new_err)?;

        // Validate protocol
        if let Some(proto) = protocol {
            if !PROTOCOLS.contains(&proto) {
                return Err(PyRuntimeError::new_err(format!(
                    "unknown protocol '{}' -- valid protocols: {}",
                    proto,
                    PROTOCOLS.join(", ")
                )));
            }
        }

        // Check selective + wild conflict
        if !names.is_empty() && wild {
            return Err(PyTypeError::new_err("cannot combine selective names with wild=True"));
        }

        // Explicit caller_dir wins; fall back to META.file in the VM globals
        let caller_dir = caller_dir.map(PathBuf::from).or_else(|| self.caller_dir(py));

        // Load the module namespace
        let namespace = if validated.starts_with('.') {
            self.load_relative(py, validated, caller_dir.as_deref(), protocol)?
        } else {
            self.load_bare(py, validated, caller_dir.as_deref(), protocol)?
        };

        // Handle selective imports
        if !names.is_empty() {
            let mut resolved = Vec::new();
            for item in names.iter() {
                let raw: String = item.extract()?;
                let (name, alias) = parse_import_name(&raw)?;
                let val = namespace
                    .bind(py)
                    .getattr(name.as_str())
                    .map_err(|_| PyAttributeError::new_err(format!("module '{}' has no attribute '{}'", spec, name)))?;
                resolved.push((alias, val.unbind()));
            }
            for (alias, value) in resolved {
                let val = Value::from_pyobject(py, value.bind(py)).map_err(PyValueError::new_err)?;
                self.globals.borrow_mut().insert(alias, val);
            }
            return Ok(py.None());
        }

        // Handle wild import
        if wild {
            let dir_list: Vec<String> = namespace.bind(py).dir()?.extract()?;
            for name in dir_list {
                if name.starts_with('_') || name == "META" {
                    continue;
                }
                if let Ok(val) = namespace.bind(py).getattr(name.as_str()) {
                    if let Ok(v) = Value::from_pyobject(py, &val) {
                        self.globals.borrow_mut().insert(name, v);
                    }
                }
            }
            return Ok(py.None());
        }

        Ok(namespace)
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        // Return a dummy for pickling support
        let builtins = py.import("builtins")?;
        let none_type = builtins.getattr("type")?.call1((py.None(),))?;
        Ok((none_type, PyTuple::empty(py)).into_pyobject(py)?.unbind().into())
    }
}

impl ImportLoader {
    /// Create an ImportLoader from Rust (bypassing GlobalsProxy). `registry_id`
    /// is the feeder VM's struct registry id (see the struct field).
    pub fn create(
        py: Python<'_>,
        globals: Globals,
        policy: Option<Py<PyAny>>,
        cat_loader: Option<Py<PyAny>>,
        registry_id: u64,
    ) -> PyResult<Py<Self>> {
        let sysconfig = py.import("sysconfig")?;
        let suffix: String = sysconfig
            .call_method1("get_config_var", ("EXT_SUFFIX",))?
            .extract()
            .unwrap_or_else(|_| ".so".to_string());

        Py::new(
            py,
            Self {
                globals,
                policy,
                cat_loader,
                context: None,
                cache: ModuleCache::new(),
                verbose: false,
                native_suffix: suffix,
                plugin_registry: Rc::new(std::cell::RefCell::new(catnip_vm::plugin::PluginRegistry::new())),
                registry_id,
            },
        )
    }

    /// Set the module policy on an existing loader.
    pub fn set_policy_on(py: Python<'_>, loader: &Py<Self>, policy: Py<PyAny>) {
        let mut borrow = loader.borrow_mut(py);
        borrow.policy = Some(policy);
    }

    /// Get CWD via Python (respects mocks in tests).
    fn python_cwd(py: Python<'_>) -> Option<PathBuf> {
        let pathlib = py.import("pathlib").ok()?;
        let cwd = pathlib.getattr("Path").ok()?.call_method0("cwd").ok()?;
        let cwd_str: String = cwd.call_method0("__str__").ok()?.extract().ok()?;
        Some(PathBuf::from(cwd_str))
    }

    /// Extract caller_dir from META.file in globals.
    fn caller_dir(&self, py: Python<'_>) -> Option<PathBuf> {
        let g = self.globals.borrow();
        let meta_val = g.get("META")?;
        let meta_obj = meta_val.to_pyobject(py);
        let file_attr = meta_obj.bind(py).getattr("file").ok()?;
        let file_str: String = file_attr.extract().ok()?;
        Path::new(&file_str).parent().map(|p| p.to_path_buf())
    }

    /// Check module policy. Returns Ok if allowed, Err if blocked.
    fn check_policy(&self, py: Python<'_>, name: &str) -> PyResult<()> {
        if let Some(ref policy) = self.policy {
            let allowed: bool = policy.call_method1(py, "check", (name,))?.extract(py)?;
            if !allowed {
                return Err(PyRuntimeError::new_err(format!("module '{}' blocked by policy", name)));
            }
        }
        Ok(())
    }

    /// Load a relative import.
    fn load_relative(
        &mut self,
        py: Python<'_>,
        spec: &str,
        caller_dir: Option<&Path>,
        protocol: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let caller = caller_dir.ok_or_else(|| {
            PyRuntimeError::new_err(format!(
                "relative import '{}' requires source file context\n  relative imports are not available in REPL or -c mode",
                spec
            ))
        })?;

        // Policy check on the bare name
        let (_, bare) = resolve::parse_relative_spec(spec);
        if !bare.is_empty() {
            self.check_policy(py, bare)?;
        }

        let (path, kind) = resolve::resolve_relative(spec, caller, protocol, &self.native_suffix)
            .map_err(PyFileNotFoundError::new_err)?;

        // Check cache by resolved absolute path
        let abs_key = path.canonicalize().unwrap_or_else(|_| path.clone());
        let cache_key = abs_key.to_string_lossy().to_string();
        if let Some(cached) = self.cache.get(&cache_key) {
            return Ok(cached.clone_ref(py));
        }

        let ns = self.load_by_kind(py, &path, kind, bare)?;
        self.cache.insert(cache_key, ns.clone_ref(py));
        Ok(ns)
    }

    /// Load a bare name import.
    fn load_bare(
        &mut self,
        py: Python<'_>,
        name: &str,
        caller_dir: Option<&Path>,
        protocol: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        // Cache hit for importlib/stdlib modules (stored by name).
        // File-based modules are cached by resolved path below,
        // so this only matches context-independent modules.
        //
        // Rust stdlib modules (PyO3 C-extensions like `io`/`sys` and native
        // `.so` plugins like `http`) are excluded: they are cached under a
        // protocol-qualified `rs::<name>` key in steps 2/2b, so that a prior
        // `import('io', protocol='py')` (which caches Python's `io` under the
        // bare name) cannot shadow the Rust module here.
        if protocol.is_none() && !resolve::is_rust_stdlib(name) {
            if let Some(cached) = self.cache.get(name) {
                return Ok(cached.clone_ref(py));
            }
        }

        // Policy gate
        self.check_policy(py, name)?;

        // protocol='py' → importlib first
        if protocol == Some("py") {
            match self.try_importlib(py, name) {
                Ok(ns) => {
                    self.cache.insert(name.to_string(), ns.clone_ref(py));
                    return Ok(ns);
                }
                Err(e) => {
                    // Only swallow ModuleNotFoundError for the exact name
                    if !e.is_instance_of::<pyo3::exceptions::PyModuleNotFoundError>(py) {
                        return Err(e);
                    }
                }
            }
        }

        // 1. File search (caller_dir → CWD → CATNIP_PATH)
        // Cache by resolved absolute path so homonymous modules in different
        // directories don't collide.
        let cwd = Self::python_cwd(py);
        if let Some((path, kind)) =
            resolve::resolve_bare_name(name, caller_dir, protocol, &self.native_suffix, cwd.as_deref())
        {
            let abs_key = path
                .canonicalize()
                .unwrap_or_else(|_| path.clone())
                .to_string_lossy()
                .to_string();
            if let Some(cached) = self.cache.get(&abs_key) {
                return Ok(cached.clone_ref(py));
            }
            let ns = self.load_by_kind(py, &path, kind, name)?;
            self.cache.insert(abs_key, ns.clone_ref(py));
            return Ok(ns);
        }

        // 2. Stdlib modules with a PyO3 backend (loaded as `catnip.catnip_<name>`
        // C-extensions). Cached under `rs::<name>` for the same reason as 2b:
        // keep them isolated from the bare-name slot used by `protocol='py'`.
        if matches!(protocol, None | Some("rs")) {
            if let Some((import_name, needs_configure)) = resolve::lookup_stdlib(name) {
                let cache_key = format!("rs::{name}");
                if let Some(cached) = self.cache.get(&cache_key) {
                    return Ok(cached.clone_ref(py));
                }
                let ns = self.load_stdlib(py, name, import_name, needs_configure)?;
                self.cache.insert(cache_key, ns.clone_ref(py));
                return Ok(ns);
            }
        }

        // 2b. PureVM-only native stdlib plugins (catnip_vm `.so` via libloading).
        // These have no PyO3 backend, so they are bridged through native_plugin
        // rather than imported as Python modules.
        //
        // Cache under a protocol-qualified key (`rs::<name>`): these modules are
        // context-independent, but must not share the bare-name cache slot with
        // Python's same-named module (loaded via `protocol='py'`). The registry
        // rejects a second `load()` of the same `.so`, so serving repeats from
        // the cache is also what keeps the import idempotent.
        if matches!(protocol, None | Some("rs")) && resolve::is_native_stdlib(name) {
            let cache_key = format!("rs::{name}");
            if let Some(cached) = self.cache.get(&cache_key) {
                return Ok(cached.clone_ref(py));
            }
            let ns = native_plugin::load_native_module(py, &self.plugin_registry, name)?;
            self.cache.insert(cache_key, ns.clone_ref(py));
            return Ok(ns);
        }

        // 3. cat protocol blocks importlib fallback
        if protocol == Some("cat") {
            return Err(PyFileNotFoundError::new_err(format!(
                "Catnip module not found: '{}'",
                name
            )));
        }

        // 4. importlib fallback
        let ns = self.try_importlib(py, name)?;
        self.cache.insert(name.to_string(), ns.clone_ref(py));
        Ok(ns)
    }

    /// Load a module by resolved path and kind.
    fn load_by_kind(&mut self, py: Python<'_>, path: &Path, kind: ModuleKind, name: &str) -> PyResult<Py<PyAny>> {
        match kind {
            ModuleKind::Catnip => self.load_catnip(py, path, name),
            ModuleKind::Python | ModuleKind::Native => self.load_python_file(py, path, name),
            ModuleKind::Package => self.load_package(py, path, name),
        }
    }

    /// Load a .cat module via the Python callback.
    fn load_catnip(&self, py: Python<'_>, path: &Path, name: &str) -> PyResult<Py<PyAny>> {
        let callback = self
            .cat_loader
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("no .cat loader available (cat_loader callback not set)"))?;
        let path_str = path.to_string_lossy().to_string();
        let result = callback.call1(py, (path_str, name))?;
        Ok(result)
    }

    /// Load a Python file (.py, .so) via importlib.
    fn load_python_file(&self, py: Python<'_>, path: &Path, name: &str) -> PyResult<Py<PyAny>> {
        let importlib_util = py.import("importlib.util")?;
        let namespace_key = format!("_catnip_local.{}.{}", name, path.display());

        let spec = importlib_util.call_method1(
            "spec_from_file_location",
            (namespace_key.as_str(), path.to_string_lossy().as_ref()),
        )?;

        if spec.is_none() {
            return Err(PyFileNotFoundError::new_err(format!(
                "cannot load module from '{}'",
                path.display()
            )));
        }

        let module = importlib_util.call_method1("module_from_spec", (&spec,))?;

        // Insert into sys.modules before exec_module (standard importlib protocol)
        let sys = py.import("sys")?;
        let sys_modules = sys.getattr("modules")?;
        sys_modules.set_item(&namespace_key, &module)?;

        let loader_attr = spec.getattr("loader")?;
        let exec_result = loader_attr.call_method1("exec_module", (&module,));

        if let Err(e) = exec_result {
            // Cleanup partially initialized module
            let _ = sys_modules.del_item(&namespace_key);
            return Err(e);
        }

        self.maybe_load_extension(py, &module)?;

        let pymod: &Bound<'_, PyModule> = module.cast()?;
        let ns = ModuleNamespace::from_pymodule(py, pymod, Some(name))?;
        Ok(Py::new(py, ns)?.into_any())
    }

    /// Load a stdlib native module (io, sys).
    fn load_stdlib(&self, py: Python<'_>, name: &str, import_name: &str, needs_configure: bool) -> PyResult<Py<PyAny>> {
        let full_name = format!("catnip.{}", import_name);
        let module = py.import(full_name.as_str())?;

        if needs_configure {
            self.configure_sys(py, &module)?;
        }

        let ns = ModuleNamespace::from_pymodule(py, &module, Some(name))?;
        Ok(Py::new(py, ns)?.into_any())
    }

    /// Configure sys module with context values.
    fn configure_sys(&self, py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
        let g = self.globals.borrow();
        if let Some(argv_val) = g.get("argv") {
            let argv_py = argv_val.to_pyobject(py);
            module.setattr("argv", argv_py.bind(py))?;
        }
        if let Some(exe_val) = g.get("_executable") {
            let exe_py = exe_val.to_pyobject(py);
            module.setattr("executable", exe_py.bind(py))?;
        }
        Ok(())
    }

    /// Load a package directory (lib.toml).
    fn load_package(&mut self, py: Python<'_>, dir: &Path, name: &str) -> PyResult<Py<PyAny>> {
        let lib_toml = dir.join("lib.toml");
        let content = std::fs::read_to_string(&lib_toml)
            .map_err(|e| PyFileNotFoundError::new_err(format!("cannot read {}: {}", lib_toml.display(), e)))?;

        // Parse TOML
        let table: toml::Table = content
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid lib.toml in '{}': {}", name, e)))?;

        let lib = table.get("lib").and_then(|v| v.as_table());
        let entry = lib
            .and_then(|l| l.get("entry"))
            .and_then(|v| v.as_str())
            .unwrap_or("main.cat");

        let pkg_root = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        let entry_path = dir.join(entry).canonicalize().map_err(|_| {
            PyFileNotFoundError::new_err(format!(
                "package '{}': entry point '{}' not found in {}",
                name,
                entry,
                dir.display()
            ))
        })?;

        // Security: entry point must stay inside package
        if !entry_path.starts_with(&pkg_root) {
            return Err(PyValueError::new_err(format!(
                "package '{}': entry point '{}' escapes package directory",
                name, entry
            )));
        }

        // Load the entry point .cat file
        let ns_obj = self.load_catnip(py, &entry_path, name)?;

        // Apply export filtering if specified
        let include: Option<Vec<String>> = lib
            .and_then(|l| l.get("exports"))
            .and_then(|v| v.as_table())
            .and_then(|e| e.get("include"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect());

        if let Some(include_list) = include {
            // Filter namespace to only exported symbols
            let source = ns_obj.bind(py);
            let mut ns = ModuleNamespace::new(name.to_string());
            for sym in include_list {
                if let Ok(val) = source.getattr(sym.as_str()) {
                    ns.set_attr(sym, val.unbind());
                }
            }
            return Ok(Py::new(py, ns)?.into_any());
        }

        Ok(ns_obj)
    }

    /// Try loading via importlib (Python ecosystem).
    fn try_importlib(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let importlib = py.import("importlib")?;
        let module = importlib.call_method1("import_module", (name,))?;
        self.maybe_load_extension(py, &module)?;
        let pymod: &Bound<'_, PyModule> = module.cast()?;
        let display_name = name.rsplit('.').next().unwrap_or(name);
        let ns = ModuleNamespace::from_pymodule(py, pymod, Some(display_name))?;
        Ok(Py::new(py, ns)?.into_any())
    }

    /// If a module has `__catnip_extension__`, load it via the Python extension system.
    fn maybe_load_extension(&self, py: Python<'_>, module: &Bound<'_, PyAny>) -> PyResult<()> {
        let ext_meta = match module.getattr("__catnip_extension__") {
            Ok(meta) if !meta.is_none() => meta,
            _ => return Ok(()),
        };
        if !ext_meta.is_instance_of::<PyDict>() {
            return Ok(());
        }

        let ext_mod = py.import(constants::PY_MOD_EXTENSIONS)?;

        // Build the context proxy passed to register(). Its `globals` is a
        // bidirectional GlobalsProxy: every write lands both in the VM IndexMap
        // (read by the VM) and in the real `context.globals` (read by the AST
        // executor), so register-time mutations -- and the exports injected via
        // `context.globals.update(...)` -- are consistent across both runtimes.
        // Other attribute reads delegate to the real context.
        let (proxy_globals, extensions): (GlobalsProxy, Py<PyAny>) = if let Some(ref real_ctx) = self.context {
            let bound = real_ctx.bind(py);
            let py_globals = bound.getattr("globals")?;
            let extensions = match bound.getattr("_extensions") {
                Ok(ext) => ext.unbind(),
                Err(_) => PyDict::new(py).into_any().unbind(),
            };
            (
                GlobalsProxy::with_mirror_registry(Rc::clone(&self.globals), py_globals.unbind(), self.registry_id),
                extensions,
            )
        } else {
            (
                GlobalsProxy::with_registry(Rc::clone(&self.globals), self.registry_id),
                PyDict::new(py).into_any().unbind(),
            )
        };

        let ctx = ExtensionContextProxy {
            context: self.context.as_ref().map(|c| c.clone_ref(py)),
            globals: Py::new(py, proxy_globals)?,
            extensions,
        };
        let ctx_obj = Py::new(py, ctx)?;

        ext_mod.call_method1("load_extension", (module, ctx_obj))?;

        Ok(())
    }
}

/// Context proxy passed to an extension's `register(context)` hook.
///
/// `globals` returns a bidirectional `GlobalsProxy` (writes reach both the VM
/// IndexMap and the Python `context.globals`); `_extensions` is the tracking
/// dict (the real context's, or a fresh one when no context is attached); every
/// other attribute delegates to the real `Context` so register hooks can read
/// its full API. Without this delegation, register hooks would only see the
/// stripped-down namespace and diverge between VM and AST execution.
#[pyclass(unsendable)]
struct ExtensionContextProxy {
    context: Option<Py<PyAny>>,
    globals: Py<GlobalsProxy>,
    extensions: Py<PyAny>,
}

#[pymethods]
impl ExtensionContextProxy {
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        match name {
            "globals" => Ok(self.globals.clone_ref(py).into_any()),
            "_extensions" => Ok(self.extensions.clone_ref(py)),
            _ => match &self.context {
                Some(ctx) => ctx.bind(py).getattr(name).map(|v| v.unbind()),
                None => Err(PyAttributeError::new_err(format!(
                    "'ExtensionContextProxy' object has no attribute '{name}'"
                ))),
            },
        }
    }
}

/// Parse a selective import name spec: "name" or "name:alias".
fn parse_import_name(raw: &str) -> PyResult<(String, String)> {
    if raw.is_empty() {
        return Err(PyValueError::new_err("import name cannot be empty"));
    }
    match raw.split_once(':') {
        Some((name, alias)) => {
            if name.is_empty() {
                return Err(PyValueError::new_err(format!("empty name in import spec '{}'", raw)));
            }
            if alias.is_empty() {
                return Err(PyValueError::new_err(format!("empty alias in import spec '{}'", raw)));
            }
            Ok((name.to_string(), alias.to_string()))
        }
        None => Ok((raw.to_string(), raw.to_string())),
    }
}
