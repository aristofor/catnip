// FILE: catnip_rs/src/loader/mod.rs
pub mod cache;
pub mod namespace;
pub use catnip_core::loader::resolve;

use cache::ModuleCache;
use namespace::ModuleNamespace;
use resolve::{ModuleKind, PROTOCOLS};

use pyo3::exceptions::{PyAttributeError, PyFileNotFoundError, PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyModule, PyTuple};
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
        })
    }

    /// Main entry point: import(spec, *names, wild=False, protocol=None)
    #[pyo3(signature = (spec, *names, wild=false, protocol=None))]
    fn __call__(
        &mut self,
        py: Python<'_>,
        spec: &str,
        names: &Bound<'_, PyTuple>,
        wild: bool,
        protocol: Option<&str>,
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

        // Get caller_dir from META.file
        let caller_dir = self.caller_dir(py);

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
    /// Create an ImportLoader from Rust (bypassing GlobalsProxy).
    pub fn create(
        py: Python<'_>,
        globals: Globals,
        policy: Option<Py<PyAny>>,
        cat_loader: Option<Py<PyAny>>,
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
        if protocol.is_none() {
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

        // 2. Stdlib native modules
        if matches!(protocol, None | Some("rs")) {
            if let Some((import_name, needs_configure)) = resolve::lookup_stdlib(name) {
                let ns = self.load_stdlib(py, name, import_name, needs_configure)?;
                self.cache.insert(name.to_string(), ns.clone_ref(py));
                return Ok(ns);
            }
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
        if !ext_meta.is_instance_of::<pyo3::types::PyDict>() {
            return Ok(());
        }

        let ext_mod = py.import(constants::PY_MOD_EXTENSIONS)?;

        // Build a proxy context that writes directly to Rust Globals.
        // This ensures exports and register() side effects land in the VM's
        // globals (IndexMap), not just in the Python context's PyDict.
        // The real context is used for _extensions tracking.
        {
            let proxy = GlobalsProxy::new(Rc::clone(&self.globals));
            let proxy_obj = Py::new(py, proxy)?;
            let types = py.import("types")?;
            let ctx = types.getattr("SimpleNamespace")?.call0()?;
            ctx.setattr("globals", proxy_obj)?;

            // Share _extensions with the real context if available
            if let Some(ref real_ctx) = self.context {
                if let Ok(ext_dict) = real_ctx.bind(py).getattr("_extensions") {
                    ctx.setattr("_extensions", ext_dict)?;
                } else {
                    ctx.setattr("_extensions", pyo3::types::PyDict::new(py))?;
                }
            } else {
                ctx.setattr("_extensions", pyo3::types::PyDict::new(py))?;
            }

            ext_mod.call_method1("load_extension", (module, &ctx))?;
        }

        Ok(())
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
