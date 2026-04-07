// FILE: catnip_vm/src/plugin.rs
//! Native plugin loading via libloading (dlopen).
//!
//! Plugins export `extern "C" catnip_plugin_init() -> *const PluginDescriptor`.
//! ABI is internal and unstable -- plugins must be compiled against the same
//! catnip_vm version.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use indexmap::IndexMap;
use libloading::Library;

use crate::error::{VMError, VMResult};
use crate::value::{ModuleNamespace, Value};

// ---------------------------------------------------------------------------
// ABI constants
// ---------------------------------------------------------------------------

pub const PLUGIN_ABI_MAGIC: u32 = 0x434E_5054; // "CNPT"
pub const PLUGIN_ABI_VERSION: u32 = 2;

/// PluginResult flag: value is an opaque object handle, not a scalar.
pub const PLUGIN_RESULT_OBJECT: u32 = 1;

// ---------------------------------------------------------------------------
// ABI types (all #[repr(C)])
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct PluginResult {
    pub value: u64,
    pub error_code: u32,
    pub flags: u32,
    pub error_message: *const c_char,
}

pub type PluginCallFn =
    unsafe extern "C" fn(function_name: *const c_char, args: *const u64, argc: usize) -> PluginResult;

/// Method dispatch on an opaque object handle.
pub type PluginMethodFn =
    unsafe extern "C" fn(handle: u64, method: *const c_char, args: *const u64, argc: usize) -> PluginResult;

/// Attribute access on an opaque object handle.
pub type PluginGetAttrFn = unsafe extern "C" fn(handle: u64, attr: *const c_char) -> PluginResult;

/// Release an opaque object handle (refcount -> 0).
pub type PluginDropFn = unsafe extern "C" fn(handle: u64);

#[repr(C)]
pub struct PluginAttr {
    pub name: *const c_char,
    pub value: u64,
}

#[repr(C)]
pub struct PluginDescriptor {
    pub abi_magic: u32,
    pub abi_version: u32,
    pub module_name: *const c_char,
    pub module_version: *const c_char,
    pub num_attrs: u32,
    pub attrs: *const PluginAttr,
    pub num_functions: u32,
    pub functions: *const *const c_char,
    pub call: PluginCallFn,
    // ABI v2: object callbacks (NULL if plugin has no objects)
    pub method: Option<PluginMethodFn>,
    pub getattr: Option<PluginGetAttrFn>,
    pub drop: Option<PluginDropFn>,
}

pub type PluginInitFn = unsafe extern "C" fn() -> *const PluginDescriptor;

// ---------------------------------------------------------------------------
// Platform helper
// ---------------------------------------------------------------------------

pub fn native_suffix() -> &'static str {
    if cfg!(target_os = "macos") { ".dylib" } else { ".so" }
}

// ---------------------------------------------------------------------------
// Qualified name prefix
// ---------------------------------------------------------------------------

const PLUGIN_PREFIX: &str = "__plugin::";

/// Check whether a function name is a plugin-qualified name.
pub fn is_plugin_call(name: &str) -> bool {
    name.starts_with(PLUGIN_PREFIX)
}

// ---------------------------------------------------------------------------
// PluginRegistry
// ---------------------------------------------------------------------------

/// Shared handle to a PluginRegistry.
pub type SharedPluginRegistry = Rc<RefCell<PluginRegistry>>;

/// Object callbacks for a single plugin.
/// Retains an Arc<Library> to keep the .so loaded as long as objects exist.
#[derive(Clone)]
pub struct PluginObjectCallbacks {
    pub method: Option<PluginMethodFn>,
    pub getattr: Option<PluginGetAttrFn>,
    pub drop: Option<PluginDropFn>,
    /// Prevents dlclose while plugin objects are alive.
    _library: Option<Arc<Library>>,
}

/// Registry of loaded native plugins.
///
/// Owns `Library` handles (kept alive until drop) and maps qualified
/// function names (`__plugin::module::fn`) to their C call pointers.
pub struct PluginRegistry {
    libraries: HashMap<PathBuf, Arc<Library>>,
    calls: HashMap<String, PluginCallFn>,
    /// Object callbacks per plugin module name.
    object_callbacks: HashMap<String, PluginObjectCallbacks>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            libraries: HashMap::new(),
            calls: HashMap::new(),
            object_callbacks: HashMap::new(),
        }
    }

    /// Clear all loaded plugins. Library handles are dropped (dlclose).
    pub fn clear(&mut self) {
        self.calls.clear();
        self.object_callbacks.clear();
        self.libraries.clear();
    }

    /// Load a native plugin from `path`. Returns a ModuleNamespace.
    ///
    /// If the library was already loaded (same canonical path), returns an error.
    pub fn load(&mut self, path: &Path, expected_name: &str) -> VMResult<ModuleNamespace> {
        let canonical = path
            .canonicalize()
            .map_err(|e| VMError::RuntimeError(format!("cannot resolve plugin path '{}': {}", path.display(), e)))?;

        if self.libraries.contains_key(&canonical) {
            return Err(VMError::RuntimeError(format!(
                "plugin '{}' already loaded",
                expected_name
            )));
        }

        // 1. dlopen
        let lib = unsafe { Library::new(&canonical) }
            .map_err(|e| VMError::RuntimeError(format!("cannot load native plugin '{}': {}", path.display(), e)))?;

        // 2. Lookup init symbol
        let init_fn: libloading::Symbol<PluginInitFn> = unsafe { lib.get(b"catnip_plugin_init\0") }.map_err(|e| {
            VMError::RuntimeError(format!(
                "plugin '{}' missing symbol 'catnip_plugin_init': {}",
                expected_name, e
            ))
        })?;

        // 3. Call init, get descriptor
        let desc_ptr = unsafe { init_fn() };
        if desc_ptr.is_null() {
            return Err(VMError::RuntimeError(format!(
                "plugin '{}': catnip_plugin_init returned null",
                expected_name
            )));
        }
        let desc = unsafe { &*desc_ptr };

        // 4. Validate ABI
        if desc.abi_magic != PLUGIN_ABI_MAGIC {
            return Err(VMError::RuntimeError(format!(
                "plugin '{}': invalid ABI magic 0x{:08X} (expected 0x{:08X})",
                expected_name, desc.abi_magic, PLUGIN_ABI_MAGIC
            )));
        }
        if desc.abi_version != PLUGIN_ABI_VERSION {
            return Err(VMError::RuntimeError(format!(
                "plugin '{}': ABI version {} (expected {})",
                expected_name, desc.abi_version, PLUGIN_ABI_VERSION
            )));
        }

        // 5. Read module name from descriptor
        let module_name = unsafe { CStr::from_ptr(desc.module_name) }
            .to_str()
            .map_err(|_| VMError::RuntimeError(format!("plugin '{}': invalid UTF-8 in module_name", expected_name)))?;

        if module_name != expected_name {
            return Err(VMError::RuntimeError(format!(
                "plugin module name '{}' does not match import spec '{}'",
                module_name, expected_name
            )));
        }

        // 6. Build namespace attrs
        let mut attrs = IndexMap::new();

        // Static attributes
        for i in 0..desc.num_attrs as usize {
            let attr = unsafe { &*desc.attrs.add(i) };
            let name = unsafe { CStr::from_ptr(attr.name) }.to_str().map_err(|_| {
                VMError::RuntimeError(format!("plugin '{}': invalid UTF-8 in attr name", expected_name))
            })?;
            attrs.insert(name.to_string(), Value::from_raw(attr.value));
        }

        // Callable function entries (stored as qualified NativeStr)
        let call_fn = desc.call;
        for i in 0..desc.num_functions as usize {
            let name_ptr = unsafe { *desc.functions.add(i) };
            let fn_name = unsafe { CStr::from_ptr(name_ptr) }.to_str().map_err(|_| {
                VMError::RuntimeError(format!("plugin '{}': invalid UTF-8 in function name", expected_name))
            })?;

            let qualified = format!("{}{}{}{}", PLUGIN_PREFIX, module_name, "::", fn_name);
            self.calls.insert(qualified.clone(), call_fn);
            attrs.insert(fn_name.to_string(), Value::from_str(&qualified));
        }

        // 7. Wrap library in Arc (shared with plugin objects to prevent dlclose)
        let lib = Arc::new(lib);

        // 8. Register object callbacks (v2+)
        if desc.abi_version >= 2 {
            let cbs = PluginObjectCallbacks {
                method: desc.method,
                getattr: desc.getattr,
                drop: desc.drop,
                _library: Some(Arc::clone(&lib)),
            };
            self.object_callbacks.insert(module_name.to_string(), cbs);
        }

        // 9. Store library handle
        self.libraries.insert(canonical, lib);

        // Empty globals -- plugin namespace doesn't need closure scope
        let globals = Rc::new(RefCell::new(IndexMap::new()));

        Ok(ModuleNamespace {
            name: module_name.to_string(),
            attrs,
            module_globals: globals,
        })
    }

    /// Try to dispatch a plugin function call. Returns None if `name` is not
    /// a registered plugin function.
    pub fn try_call(&self, name: &str, args: &[Value]) -> Option<VMResult<Value>> {
        let call_fn = self.calls.get(name)?;
        Some(self.do_call(*call_fn, name, args))
    }

    /// Look up object callbacks for a plugin module.
    pub fn object_callbacks(&self, module_name: &str) -> Option<PluginObjectCallbacks> {
        self.object_callbacks.get(module_name).cloned()
    }

    fn do_call(&self, call_fn: PluginCallFn, name: &str, args: &[Value]) -> VMResult<Value> {
        // Extract module name and short function name from "__plugin::module::fn"
        let short_name = name.rsplit("::").next().unwrap_or(name);

        // Build C string for function name
        let c_name = std::ffi::CString::new(short_name)
            .map_err(|_| VMError::RuntimeError("plugin function name contains null byte".into()))?;

        // Args as raw u64 slice
        let raw_args: Vec<u64> = args.iter().map(|v| v.bits()).collect();

        // Call with panic catching
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            call_fn(c_name.as_ptr(), raw_args.as_ptr(), raw_args.len())
        }));

        match result {
            Ok(pr) => self.interpret_result(pr, name, short_name),
            Err(_) => Err(VMError::RuntimeError(format!(
                "plugin function '{}' panicked",
                short_name
            ))),
        }
    }

    /// Interpret a PluginResult, wrapping in PluginObject if flags indicate an object handle.
    fn interpret_result(&self, pr: PluginResult, qualified_name: &str, _display_name: &str) -> VMResult<Value> {
        if pr.error_code != 0 {
            // Special exit error code from sys.exit()
            if pr.error_code == 0x45584954 {
                return Err(VMError::Exit(pr.value as i32));
            }
            let msg = if pr.error_message.is_null() {
                format!("plugin error (code {})", pr.error_code)
            } else {
                let s = unsafe { CStr::from_ptr(pr.error_message) };
                s.to_string_lossy().into_owned()
            };
            return Err(VMError::RuntimeError(msg));
        }

        if pr.flags & PLUGIN_RESULT_OBJECT != 0 {
            // Extract module name from qualified name: "__plugin::module::fn"
            let module_name = qualified_name
                .strip_prefix(PLUGIN_PREFIX)
                .and_then(|rest| rest.split("::").next())
                .unwrap_or("");
            let cbs = self
                .object_callbacks
                .get(module_name)
                .cloned()
                .unwrap_or(PluginObjectCallbacks {
                    method: None,
                    getattr: None,
                    drop: None,
                    _library: None,
                });
            Ok(Value::from_plugin_object(pr.value, cbs))
        } else {
            Ok(Value::from_raw(pr.value))
        }
    }

    /// Dispatch a method call on a plugin object. Called from host.rs.
    pub fn call_method_on_object(
        &self,
        handle: u64,
        method_fn: PluginMethodFn,
        method: &str,
        args: &[Value],
        callbacks: &PluginObjectCallbacks,
    ) -> VMResult<Value> {
        let c_method = std::ffi::CString::new(method)
            .map_err(|_| VMError::RuntimeError("method name contains null byte".into()))?;
        let raw_args: Vec<u64> = args.iter().map(|v| v.bits()).collect();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            method_fn(handle, c_method.as_ptr(), raw_args.as_ptr(), raw_args.len())
        }));

        match result {
            Ok(pr) => self.interpret_method_result(pr, method, callbacks),
            Err(_) => Err(VMError::RuntimeError(format!("plugin method '{}' panicked", method))),
        }
    }

    /// Dispatch getattr on a plugin object. Called from host.rs.
    pub fn call_getattr_on_object(
        &self,
        handle: u64,
        getattr_fn: PluginGetAttrFn,
        attr: &str,
        callbacks: &PluginObjectCallbacks,
    ) -> VMResult<Value> {
        let c_attr = std::ffi::CString::new(attr)
            .map_err(|_| VMError::RuntimeError("attribute name contains null byte".into()))?;

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            getattr_fn(handle, c_attr.as_ptr())
        }));

        match result {
            Ok(pr) => self.interpret_method_result(pr, attr, callbacks),
            Err(_) => Err(VMError::RuntimeError(format!("plugin getattr '{}' panicked", attr))),
        }
    }

    /// Interpret a result from method/getattr -- same logic as interpret_result but
    /// reuses the same callbacks for returned objects (same plugin).
    fn interpret_method_result(
        &self,
        pr: PluginResult,
        _name: &str,
        callbacks: &PluginObjectCallbacks,
    ) -> VMResult<Value> {
        if pr.error_code != 0 {
            let msg = if pr.error_message.is_null() {
                format!("plugin error (code {})", pr.error_code)
            } else {
                let s = unsafe { CStr::from_ptr(pr.error_message) };
                s.to_string_lossy().into_owned()
            };
            return Err(VMError::RuntimeError(msg));
        }

        if pr.flags & PLUGIN_RESULT_OBJECT != 0 {
            Ok(Value::from_plugin_object(pr.value, callbacks.clone()))
        } else {
            Ok(Value::from_raw(pr.value))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    /// Build catnip_hello and return the path to the .so/.dylib.
    fn build_hello_plugin() -> PathBuf {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let status = Command::new("cargo")
            .args(["build", "-p", "catnip_hello"])
            .current_dir(&workspace)
            .status()
            .expect("failed to run cargo build");
        assert!(status.success(), "cargo build -p catnip_hello failed");

        let lib_name = if cfg!(target_os = "macos") {
            "libcatnip_hello.dylib"
        } else {
            "libcatnip_hello.so"
        };
        workspace.join("target/debug").join(lib_name)
    }

    #[test]
    fn test_plugin_load_and_call() {
        let so_path = build_hello_plugin();
        let mut registry = PluginRegistry::new();
        let ns = registry.load(&so_path, "hello").unwrap();

        assert_eq!(ns.name, "hello");
        assert!(ns.attrs.contains_key("VERSION"));
        assert!(ns.attrs.contains_key("greet"));
        assert!(ns.attrs.contains_key("add"));

        // VERSION attr
        let ver = ns.attrs["VERSION"];
        let ver_str = unsafe { ver.as_native_str_ref().unwrap() };
        assert_eq!(ver_str, "0.1.0");

        // greet()
        let greet_name = unsafe { ns.attrs["greet"].as_native_str_ref().unwrap() };
        assert_eq!(greet_name, "__plugin::hello::greet");
        let result = registry.try_call("__plugin::hello::greet", &[]).unwrap().unwrap();
        let result_str = unsafe { result.as_native_str_ref().unwrap() };
        assert_eq!(result_str, "hello!");

        // add(2, 3)
        let result = registry
            .try_call("__plugin::hello::add", &[Value::from_int(2), Value::from_int(3)])
            .unwrap()
            .unwrap();
        assert_eq!(result.as_int(), Some(5));
    }

    #[test]
    fn test_plugin_double_load_rejected() {
        let so_path = build_hello_plugin();
        let mut registry = PluginRegistry::new();
        registry.load(&so_path, "hello").unwrap();

        match registry.load(&so_path, "hello") {
            Err(VMError::RuntimeError(msg)) => assert!(msg.contains("already loaded"), "{msg}"),
            Ok(_) => panic!("expected already loaded error"),
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }

    #[test]
    fn test_plugin_not_found() {
        let mut registry = PluginRegistry::new();
        match registry.load(Path::new("/nonexistent/libfoo.so"), "foo") {
            Err(VMError::RuntimeError(msg)) => {
                assert!(msg.contains("cannot resolve") || msg.contains("cannot load"), "{msg}");
            }
            Ok(_) => panic!("expected error"),
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }

    #[test]
    fn test_plugin_unknown_function() {
        let so_path = build_hello_plugin();
        let mut registry = PluginRegistry::new();
        registry.load(&so_path, "hello").unwrap();

        // Unregistered qualified name
        assert!(registry.try_call("__plugin::hello::nope", &[]).is_none());
    }

    #[test]
    fn test_plugin_call_error() {
        let so_path = build_hello_plugin();
        let mut registry = PluginRegistry::new();
        registry.load(&so_path, "hello").unwrap();

        // add() with no args -> error_code 1
        let err = registry.try_call("__plugin::hello::add", &[]).unwrap().unwrap_err();
        assert!(format!("{err:?}").contains("requires 2 arguments"));
    }

    #[test]
    fn test_plugin_abi_validation() {
        // Missing symbol -- any random .so should fail
        let mut registry = PluginRegistry::new();
        let libc_path = "/usr/lib/x86_64-linux-gnu/libc.so.6";
        let path = Path::new(libc_path);
        if path.exists() {
            match registry.load(path, "libc") {
                Err(VMError::RuntimeError(msg)) => {
                    assert!(msg.contains("catnip_plugin_init"), "{msg}");
                }
                Ok(_) => panic!("expected missing symbol error"),
                Err(e) => panic!("unexpected error: {e:?}"),
            }
        }
    }

    #[test]
    fn test_native_suffix() {
        let suffix = native_suffix();
        if cfg!(target_os = "macos") {
            assert_eq!(suffix, ".dylib");
        } else {
            assert_eq!(suffix, ".so");
        }
    }

    #[test]
    fn test_is_plugin_call() {
        assert!(is_plugin_call("__plugin::hello::greet"));
        assert!(!is_plugin_call("abs"));
        assert!(!is_plugin_call("__io_print"));
    }

    // -- ABI v2 object tests (simulated, no cdylib needed) --

    use std::sync::atomic::{AtomicU64, Ordering};

    static MOCK_COUNTER: AtomicU64 = AtomicU64::new(0);
    static MOCK_DROPPED: AtomicU64 = AtomicU64::new(0);

    unsafe extern "C" fn mock_method(
        handle: u64,
        method: *const c_char,
        _args: *const u64,
        _argc: usize,
    ) -> PluginResult {
        let method = unsafe { CStr::from_ptr(method) }.to_bytes();
        match method {
            b"value" => PluginResult {
                value: Value::from_int(handle as i64).bits(),
                error_code: 0,
                flags: 0,
                error_message: std::ptr::null(),
            },
            b"child" => {
                // Return another object handle
                let new_handle = MOCK_COUNTER.fetch_add(1, Ordering::Relaxed) + 100;
                PluginResult {
                    value: new_handle,
                    error_code: 0,
                    flags: PLUGIN_RESULT_OBJECT,
                    error_message: std::ptr::null(),
                }
            }
            _ => PluginResult {
                value: 0,
                error_code: 1,
                flags: 0,
                error_message: b"unknown method\0".as_ptr() as *const c_char,
            },
        }
    }

    unsafe extern "C" fn mock_getattr(handle: u64, attr: *const c_char) -> PluginResult {
        let attr = unsafe { CStr::from_ptr(attr) }.to_bytes();
        match attr {
            b"id" => PluginResult {
                value: Value::from_int(handle as i64).bits(),
                error_code: 0,
                flags: 0,
                error_message: std::ptr::null(),
            },
            _ => PluginResult {
                value: 0,
                error_code: 1,
                flags: 0,
                error_message: b"unknown attr\0".as_ptr() as *const c_char,
            },
        }
    }

    unsafe extern "C" fn mock_drop(handle: u64) {
        MOCK_DROPPED.fetch_add(handle, Ordering::Relaxed);
    }

    fn mock_callbacks() -> PluginObjectCallbacks {
        PluginObjectCallbacks {
            method: Some(mock_method),
            getattr: Some(mock_getattr),
            drop: Some(mock_drop),
            _library: None,
        }
    }

    #[test]
    fn test_plugin_object_creation() {
        let cbs = mock_callbacks();
        let val = Value::from_plugin_object(42, cbs);
        assert!(val.is_plugin_object());
        let (handle, _) = unsafe { val.as_plugin_object_ref().unwrap() };
        assert_eq!(handle, 42);
    }

    #[test]
    fn test_plugin_object_method_dispatch() {
        let cbs = mock_callbacks();
        let registry = PluginRegistry::new();

        // Call "value" method on handle 7
        let result = registry
            .call_method_on_object(7, mock_method, "value", &[], &cbs)
            .unwrap();
        assert_eq!(result.as_int(), Some(7));
    }

    #[test]
    fn test_plugin_object_method_returns_object() {
        MOCK_COUNTER.store(0, Ordering::Relaxed);
        let cbs = mock_callbacks();
        let registry = PluginRegistry::new();

        // Call "child" method -- returns another plugin object
        let result = registry
            .call_method_on_object(1, mock_method, "child", &[], &cbs)
            .unwrap();
        assert!(result.is_plugin_object());
        let (handle, _) = unsafe { result.as_plugin_object_ref().unwrap() };
        assert_eq!(handle, 100);
    }

    #[test]
    fn test_plugin_object_getattr_dispatch() {
        let cbs = mock_callbacks();
        let registry = PluginRegistry::new();

        let result = registry.call_getattr_on_object(99, mock_getattr, "id", &cbs).unwrap();
        assert_eq!(result.as_int(), Some(99));
    }

    #[test]
    fn test_plugin_object_method_error() {
        let cbs = mock_callbacks();
        let registry = PluginRegistry::new();

        let err = registry
            .call_method_on_object(1, mock_method, "nope", &[], &cbs)
            .unwrap_err();
        assert!(format!("{err:?}").contains("unknown method"));
    }

    #[test]
    fn test_plugin_object_drop() {
        MOCK_DROPPED.store(0, Ordering::Relaxed);
        let cbs = mock_callbacks();
        let val = Value::from_plugin_object(5, cbs);
        // Value is Copy -- explicit decref triggers Arc drop -> ExtendedValue Drop -> drop_fn
        val.decref();
        assert_eq!(MOCK_DROPPED.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn test_plugin_result_flags() {
        assert_eq!(PLUGIN_RESULT_OBJECT, 1);
        assert_eq!(PLUGIN_ABI_VERSION, 2);
    }

    // -- IO plugin integration tests --

    fn build_io_plugin() -> PathBuf {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let status = Command::new("cargo")
            .args(["build", "-p", "catnip-io", "--no-default-features"])
            .current_dir(&workspace)
            .status()
            .expect("failed to run cargo build");
        assert!(status.success(), "cargo build -p catnip-io failed");

        let lib_name = if cfg!(target_os = "macos") {
            "libcatnip_io.dylib"
        } else {
            "libcatnip_io.so"
        };
        workspace.join("target/debug").join(lib_name)
    }

    #[test]
    fn test_io_plugin_load() {
        let so_path = build_io_plugin();
        let mut registry = PluginRegistry::new();
        let ns = registry.load(&so_path, "io").unwrap();

        assert_eq!(ns.name, "io");
        assert!(ns.attrs.contains_key("PROTOCOL"));
        assert!(ns.attrs.contains_key("VERSION"));
        assert!(ns.attrs.contains_key("print"));
        assert!(ns.attrs.contains_key("open"));

        let proto = ns.attrs["PROTOCOL"];
        let proto_str = unsafe { proto.as_native_str_ref().unwrap() };
        assert_eq!(proto_str, "rust");
    }

    #[test]
    fn test_io_plugin_print() {
        let so_path = build_io_plugin();
        let mut registry = PluginRegistry::new();
        registry.load(&so_path, "io").unwrap();

        // print("hello") should succeed (output goes to stdout)
        let result = registry
            .try_call("__plugin::io::print", &[Value::from_str("hello")])
            .unwrap()
            .unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn test_io_plugin_open_read_close() {
        use std::io::Write as _;
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        {
            let mut f = std::fs::File::create(&file_path).unwrap();
            f.write_all(b"hello world").unwrap();
        }

        let so_path = build_io_plugin();
        let mut registry = PluginRegistry::new();
        registry.load(&so_path, "io").unwrap();

        // open(path, "r") -> PluginObject
        let path_val = Value::from_string(file_path.to_string_lossy().to_string());
        let result = registry.try_call("__plugin::io::open", &[path_val]).unwrap().unwrap();
        assert!(
            result.is_plugin_object(),
            "expected plugin object, got: {}",
            result.display_string()
        );

        let (handle, cbs) = unsafe { result.as_plugin_object_ref().unwrap() };

        // getattr "name"
        let name = registry
            .call_getattr_on_object(handle, cbs.getattr.unwrap(), "name", &cbs)
            .unwrap();
        let name_str = unsafe { name.as_native_str_ref().unwrap() };
        assert!(name_str.contains("test.txt"));

        // getattr "closed" -> false
        let closed = registry
            .call_getattr_on_object(handle, cbs.getattr.unwrap(), "closed", &cbs)
            .unwrap();
        assert_eq!(closed.as_bool(), Some(false));

        // method "read"
        let content = registry
            .call_method_on_object(handle, cbs.method.unwrap(), "read", &[], &cbs)
            .unwrap();
        let content_str = unsafe { content.as_native_str_ref().unwrap() };
        assert_eq!(content_str, "hello world");

        // method "close"
        let _ = registry
            .call_method_on_object(handle, cbs.method.unwrap(), "close", &[], &cbs)
            .unwrap();

        // getattr "closed" -> true
        let closed = registry
            .call_getattr_on_object(handle, cbs.getattr.unwrap(), "closed", &cbs)
            .unwrap();
        assert_eq!(closed.as_bool(), Some(true));

        result.decref();
    }

    #[test]
    fn test_io_plugin_open_write_read_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("out.txt");

        let so_path = build_io_plugin();
        let mut registry = PluginRegistry::new();
        registry.load(&so_path, "io").unwrap();

        // open(path, "w")
        let path_val = Value::from_string(file_path.to_string_lossy().to_string());
        let mode_val = Value::from_str("w");
        let wfile = registry
            .try_call("__plugin::io::open", &[path_val, mode_val])
            .unwrap()
            .unwrap();
        let (wh, wcbs) = unsafe { wfile.as_plugin_object_ref().unwrap() };

        // write "hello"
        let n = registry
            .call_method_on_object(wh, wcbs.method.unwrap(), "write", &[Value::from_str("hello")], &wcbs)
            .unwrap();
        assert_eq!(n.as_int(), Some(5));

        // close
        let _ = registry
            .call_method_on_object(wh, wcbs.method.unwrap(), "close", &[], &wcbs)
            .unwrap();
        wfile.decref();

        // open(path, "r")
        let path_val2 = Value::from_string(file_path.to_string_lossy().to_string());
        let rfile = registry.try_call("__plugin::io::open", &[path_val2]).unwrap().unwrap();
        let (rh, rcbs) = unsafe { rfile.as_plugin_object_ref().unwrap() };

        let content = registry
            .call_method_on_object(rh, rcbs.method.unwrap(), "read", &[], &rcbs)
            .unwrap();
        let s = unsafe { content.as_native_str_ref().unwrap() };
        assert_eq!(s, "hello");

        rfile.decref();
    }

    // -- sys plugin integration tests --

    fn build_sys_plugin() -> PathBuf {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let status = Command::new("cargo")
            .args(["build", "-p", "catnip-sys", "--no-default-features"])
            .current_dir(&workspace)
            .status()
            .expect("failed to run cargo build");
        assert!(status.success(), "cargo build -p catnip-sys failed");

        let lib_name = if cfg!(target_os = "macos") {
            "libcatnip_sys.dylib"
        } else {
            "libcatnip_sys.so"
        };
        workspace.join("target/debug").join(lib_name)
    }

    #[test]
    fn test_sys_plugin_load() {
        let so_path = build_sys_plugin();
        let mut registry = PluginRegistry::new();
        let ns = registry.load(&so_path, "sys").unwrap();

        assert_eq!(ns.name, "sys");
        assert!(ns.attrs.contains_key("PROTOCOL"));
        assert!(ns.attrs.contains_key("argv"));
        assert!(ns.attrs.contains_key("platform"));
        assert!(ns.attrs.contains_key("version"));
        assert!(ns.attrs.contains_key("cpu_count"));
        assert!(ns.attrs.contains_key("exit"));

        // platform should be a string
        let platform = ns.attrs["platform"];
        let p = unsafe { platform.as_native_str_ref().unwrap() };
        assert!(!p.is_empty());

        // cpu_count should be > 0
        let cpus = ns.attrs["cpu_count"];
        assert!(cpus.as_int().unwrap() > 0);
    }

    #[test]
    fn test_sys_plugin_exit() {
        let so_path = build_sys_plugin();
        let mut registry = PluginRegistry::new();
        registry.load(&so_path, "sys").unwrap();

        // exit(42) should produce VMError::Exit(42)
        let result = registry
            .try_call("__plugin::sys::exit", &[Value::from_int(42)])
            .unwrap();
        match result {
            Err(VMError::Exit(code)) => assert_eq!(code, 42),
            other => panic!("expected Exit(42), got {:?}", other),
        }
    }
}
