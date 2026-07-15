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
pub const PLUGIN_ABI_VERSION: u32 = 5;

/// PluginResult flag: value is an opaque object handle, not a scalar.
pub const PLUGIN_RESULT_OBJECT: u32 = 1;

/// PluginResult flag (ABI v4): `value` was built by the host's own value-builder
/// callbacks (`PluginHostApi`), so the pointer lives in the host heap and is
/// trusted as-is. Lets a plugin return structured data without the host ever
/// dereferencing a plugin-owned Arc.
pub const PLUGIN_RESULT_HOSTVALUE: u32 = 2;

/// PluginAttr flag (ABI v5): `value` is a host-built pointer (produced by a
/// `PluginHostApi` builder), so it lives in the host heap and is trusted as-is.
/// Mirror of `PLUGIN_RESULT_HOSTVALUE` for the static-attribute boundary: without
/// this flag an attr must be an inline scalar (enforced by `from_raw_scalar`), so
/// a plugin cannot hand the host a raw pointer into its own heap undeclared.
pub const PLUGIN_ATTR_HOSTVALUE: u32 = 1;

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

/// Membership probe on an opaque object handle: returns non-zero iff `name` is
/// a readable attribute or callable method of the object.
///
/// Pure side-channel: it must not mutate the object or block. Used by the AST
/// bridge to decide attribute-vs-method without invoking the member (the AST
/// lowers `obj.m(...)` to getattr-then-call, so it cannot tell them apart).
pub type PluginHasMemberFn = unsafe extern "C" fn(handle: u64, name: *const c_char) -> u8;

/// Release an opaque object handle (refcount -> 0).
pub type PluginDropFn = unsafe extern "C" fn(handle: u64);

#[repr(C)]
pub struct PluginAttr {
    pub name: *const c_char,
    pub value: u64,
    /// ABI v5: `PLUGIN_ATTR_HOSTVALUE` when `value` is a host-built pointer.
    pub flags: u32,
}

impl PluginAttr {
    /// A host-built pointer attr (string/list/dict from a `PluginHostApi`
    /// builder), trusted by the host at admission.
    pub const fn host_value(name: *const c_char, value: u64) -> Self {
        Self {
            name,
            value,
            flags: PLUGIN_ATTR_HOSTVALUE,
        }
    }

    /// An inline scalar attr (int/bool/float), validated by `from_raw_scalar`.
    pub const fn scalar(name: *const c_char, value: u64) -> Self {
        Self { name, value, flags: 0 }
    }
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
    // ABI v3: membership probe (NULL keeps the v2 optimistic fallback)
    pub has_member: Option<PluginHasMemberFn>,
}

// ---------------------------------------------------------------------------
// ABI v4: host value-builder API
// ---------------------------------------------------------------------------
//
// A plugin returning structured data (string, bytes, list, dict) must build it
// in the HOST heap through these callbacks instead of passing a pointer into
// its own heap. The host then never dereferences a plugin-owned Arc, so the
// boundary is unconditionally safe rather than safe-by-contract.
//
// Each callback returns an owned host Value token (refcount 1). The composite
// builders (`make_list`/`make_dict`) CONSUME the tokens passed to them (no
// incref), so a token must not be reused after handing it off.

pub type HostMakeStringFn = unsafe extern "C" fn(ptr: *const u8, len: usize) -> u64;
pub type HostMakeBytesFn = unsafe extern "C" fn(ptr: *const u8, len: usize) -> u64;
pub type HostMakeListFn = unsafe extern "C" fn(items: *const u64, len: usize) -> u64;
pub type HostMakeDictFn = unsafe extern "C" fn(keys: *const u64, vals: *const u64, len: usize) -> u64;
/// `decimal` points at the base-10 ASCII digits of the integer.
pub type HostMakeBigintFn = unsafe extern "C" fn(decimal: *const u8, len: usize) -> u64;

#[repr(C)]
pub struct PluginHostApi {
    pub make_string: HostMakeStringFn,
    pub make_bytes: HostMakeBytesFn,
    pub make_list: HostMakeListFn,
    pub make_dict: HostMakeDictFn,
    pub make_bigint: HostMakeBigintFn,
}

unsafe extern "C" fn host_make_string(ptr: *const u8, len: usize) -> u64 {
    let s = if ptr.is_null() {
        String::new()
    } else {
        // SAFETY: this is a plugin-invoked host callback; `ptr` is non-null (just
        // checked) and the plugin guarantees `len` readable bytes for the call.
        String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(ptr, len) }).into_owned()
    };
    Value::from_string(s).bits()
}

unsafe extern "C" fn host_make_bytes(ptr: *const u8, len: usize) -> u64 {
    let data = if ptr.is_null() {
        Vec::new()
    } else {
        // SAFETY: plugin-invoked host callback; `ptr` is non-null (just checked)
        // and the plugin guarantees `len` readable bytes for the call.
        unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
    };
    Value::from_bytes(data).bits()
}

unsafe extern "C" fn host_make_list(items: *const u64, len: usize) -> u64 {
    // Each token is consumed: from_raw takes ownership without an incref.
    let vals: Vec<Value> = if items.is_null() || len == 0 {
        Vec::new()
    } else {
        // SAFETY: `items` is non-null and `len != 0` (just checked); the plugin
        // hands `len` u64 value tokens it owns, readable for the call.
        unsafe { std::slice::from_raw_parts(items, len) }
            .iter()
            .map(|&b| Value::from_raw(b))
            .collect()
    };
    Value::from_list(vals).bits()
}

unsafe extern "C" fn host_make_dict(keys: *const u64, vals: *const u64, len: usize) -> u64 {
    let mut map = indexmap::IndexMap::with_capacity(len);
    if !keys.is_null() && !vals.is_null() {
        // SAFETY: `keys` is non-null (just checked) and points to `len` u64 value
        // tokens the plugin owns, readable for the call.
        let ks = unsafe { std::slice::from_raw_parts(keys, len) };
        // SAFETY: `vals` is non-null (just checked) and points to `len` u64 value
        // tokens the plugin owns, readable for the call.
        let vs = unsafe { std::slice::from_raw_parts(vals, len) };
        for i in 0..len {
            let val = Value::from_raw(vs[i]);
            let key_tok = Value::from_raw(ks[i]);
            match key_tok.to_key() {
                Ok(k) => {
                    // to_key takes an independent ref for pointer keys; the plugin
                    // transferred ownership of the key token, so release it
                    // (mirror of BuildDict's `chunk[0].decref()`).
                    key_tok.decref();
                    // A duplicate key evicts the previous value; release it.
                    if let Some(old) = map.insert(k, val) {
                        old.decref();
                    }
                }
                Err(_) => {
                    // Non-hashable key: release every token this call owns. The
                    // bare IndexMap drop frees the ValueKeys but not the Values
                    // (Value is Copy), so drain and decref them explicitly.
                    val.decref();
                    key_tok.decref();
                    for (_, v) in map.drain(..) {
                        v.decref();
                    }
                    for &b in &vs[i + 1..] {
                        Value::from_raw(b).decref();
                    }
                    for &b in &ks[i + 1..] {
                        Value::from_raw(b).decref();
                    }
                    return Value::INVALID.bits();
                }
            }
        }
    }
    Value::from_dict(map).bits()
}

unsafe extern "C" fn host_make_bigint(decimal: *const u8, len: usize) -> u64 {
    if decimal.is_null() {
        return Value::from_int(0).bits();
    }
    // SAFETY: `decimal` is non-null (the null case returned above) and points to
    // `len` base-10 ASCII bytes supplied by the plugin, readable for the call.
    let s = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(decimal, len) });
    match rug::Integer::from_str_radix(s.trim(), 10) {
        Ok(n) => Value::from_bigint_or_demote(n).bits(),
        Err(_) => Value::from_int(0).bits(),
    }
}

/// Host API instance handed to every plugin at init. The function pointers are
/// `'static` host code, so this is valid for the whole process lifetime.
pub static PLUGIN_HOST_API: PluginHostApi = PluginHostApi {
    make_string: host_make_string,
    make_bytes: host_make_bytes,
    make_list: host_make_list,
    make_dict: host_make_dict,
    make_bigint: host_make_bigint,
};

/// ABI v4: init receives the host value-builder API. A v3 plugin (zero-arg
/// init) ignores the extra argument harmlessly under the C calling convention.
pub type PluginInitFn = unsafe extern "C" fn(host: *const PluginHostApi) -> *const PluginDescriptor;

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
    pub has_member: Option<PluginHasMemberFn>,
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

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
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
        // SAFETY: `canonical` is a resolved filesystem path; dlopen runs the
        // library's platform initializers, the unavoidable risk of loading native
        // code (the path is operator-controlled, not attacker input).
        let lib = unsafe { Library::new(&canonical) }
            .map_err(|e| VMError::RuntimeError(format!("cannot load native plugin '{}': {}", path.display(), e)))?;

        // 2. Lookup init symbol
        // SAFETY: resolves the `catnip_plugin_init` symbol; the returned Symbol
        // borrows `lib`, so it cannot outlive the library, and the asserted
        // `PluginInitFn` signature is checked against the ABI version right after.
        let init_fn: libloading::Symbol<PluginInitFn> = unsafe { lib.get(b"catnip_plugin_init\0") }.map_err(|e| {
            VMError::RuntimeError(format!(
                "plugin '{}' missing symbol 'catnip_plugin_init': {}",
                expected_name, e
            ))
        })?;

        // 3. Call init, get descriptor (ABI v4 passes the host value-builder API)
        // SAFETY: calls the plugin's init entry; `PLUGIN_HOST_API` is a 'static
        // host struct valid for the whole process, satisfying the `host` pointer
        // contract. The returned descriptor pointer is null-checked and
        // ABI-validated below before any field is dereferenced.
        let desc_ptr = unsafe { init_fn(&PLUGIN_HOST_API as *const PluginHostApi) };
        if desc_ptr.is_null() {
            return Err(VMError::RuntimeError(format!(
                "plugin '{}': catnip_plugin_init returned null",
                expected_name
            )));
        }
        // SAFETY: `desc_ptr` was null-checked just above; the plugin returns a
        // pointer to its own #[repr(C)] static PluginDescriptor, valid for the
        // library's lifetime (kept alive in `self.libraries`).
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
        // SAFETY: ABI magic and version were validated above; per the ABI
        // `desc.module_name` is a non-null, NUL-terminated C string in the
        // plugin's static data, and `to_str` validates UTF-8.
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
        if desc.num_attrs > 0 && desc.attrs.is_null() {
            return Err(VMError::RuntimeError(format!(
                "plugin '{}': num_attrs={} but attrs pointer is null",
                expected_name, desc.num_attrs
            )));
        }
        for i in 0..desc.num_attrs as usize {
            // SAFETY: `desc.attrs` is non-null when num_attrs > 0 (checked above)
            // and points to `num_attrs` PluginAttr; `i < num_attrs`, so `add(i)`
            // is in bounds and the element is a live #[repr(C)] struct.
            let attr = unsafe { &*desc.attrs.add(i) };
            // SAFETY: per the ABI `attr.name` is a non-null, NUL-terminated C
            // string in the plugin's static data; `to_str` validates UTF-8.
            let name = unsafe { CStr::from_ptr(attr.name) }.to_str().map_err(|_| {
                VMError::RuntimeError(format!("plugin '{}': invalid UTF-8 in attr name", expected_name))
            })?;
            // Admit the attr value before trusting its bits: a host-built pointer
            // (PLUGIN_ATTR_HOSTVALUE) lives in the host heap and is taken as-is;
            // anything else must be an inline scalar. A plugin may not hand the
            // host a raw pointer into its own heap without declaring it host-built
            // (mirror of admit_plugin_scalar for the static-attr boundary).
            let value = admit_plugin_attr(attr, name, expected_name)?;
            // The descriptor is a plugin-side static that retains ownership of its
            // attr values across every load: the namespace only borrows them. Take
            // an independent ref so `attrs` owns one refcount per attr, symmetric
            // with ModuleNamespace::drop (otherwise dropping one namespace would
            // free a value other namespaces still alias).
            value.clone_refcount();
            attrs.insert(name.to_string(), value);
        }

        // Callable function entries (stored as qualified NativeStr)
        let call_fn = desc.call;
        if desc.num_functions > 0 && desc.functions.is_null() {
            return Err(VMError::RuntimeError(format!(
                "plugin '{}': num_functions={} but functions pointer is null",
                expected_name, desc.num_functions
            )));
        }
        for i in 0..desc.num_functions as usize {
            // SAFETY: `desc.functions` is non-null when num_functions > 0 (checked
            // above) and points to `num_functions` `*const c_char`; `i` is in
            // bounds, so reading the pointer element at `add(i)` is valid.
            let name_ptr = unsafe { *desc.functions.add(i) };
            // SAFETY: `name_ptr` is a non-null, NUL-terminated C string from the
            // plugin's function-name table (ABI); `to_str` validates UTF-8.
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
                has_member: desc.has_member,
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

    /// Look up the raw call pointer for a qualified function name
    /// (`__plugin::module::fn`). Used by the PyO3 bridge.
    pub fn call_fn(&self, qualified_name: &str) -> Option<PluginCallFn> {
        self.calls.get(qualified_name).copied()
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
        // SAFETY: `call_fn` comes from a descriptor whose ABI magic/version were
        // validated at load; `c_name` is a valid NUL-terminated string and
        // `raw_args` a live slice, both outliving the call; catch_unwind contains
        // any plugin-side panic crossing the FFI boundary.
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
                // SAFETY: `pr.error_message` is non-null (just checked) and points
                // to a NUL-terminated C string owned by the plugin; copied into an
                // owned String via lossy conversion before returning.
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
                    has_member: None,
                    _library: None,
                });
            Ok(Value::from_plugin_object(pr.value, cbs))
        } else {
            admit_plugin_scalar(&pr)
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

        // SAFETY: `method_fn` is a callback from the plugin's validated descriptor;
        // `handle` is an opaque object token previously issued by that same plugin,
        // and `c_method`/`raw_args` stay live for the call; catch_unwind contains
        // any plugin-side panic.
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

        // SAFETY: `getattr_fn` is a callback from the plugin's validated descriptor;
        // `handle` is an opaque object token previously issued by that same plugin,
        // and `c_attr` stays live for the call; catch_unwind contains any panic.
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
                // SAFETY: `pr.error_message` is non-null (just checked) and points
                // to a NUL-terminated C string owned by the plugin; copied into an
                // owned String via lossy conversion before returning.
                let s = unsafe { CStr::from_ptr(pr.error_message) };
                s.to_string_lossy().into_owned()
            };
            return Err(VMError::RuntimeError(msg));
        }

        if pr.flags & PLUGIN_RESULT_OBJECT != 0 {
            Ok(Value::from_plugin_object(pr.value, callbacks.clone()))
        } else {
            admit_plugin_scalar(&pr)
        }
    }
}

// ---------------------------------------------------------------------------
// Standalone invocation helpers
// ---------------------------------------------------------------------------
//
// These mirror the `PluginRegistry` methods but take raw fn pointers and
// callbacks directly, so callers that hold those (the PyO3 bridge in
// catnip_rs) can invoke plugins without borrowing a registry.

impl PluginObjectCallbacks {
    /// Callbacks for a plugin that exposes no objects (ABI v1).
    pub fn empty() -> Self {
        Self {
            method: None,
            getattr: None,
            drop: None,
            has_member: None,
            _library: None,
        }
    }
}

/// Admit the non-object result of a plugin call (ABI v4 boundary lock).
///
/// A host-built value (`PLUGIN_RESULT_HOSTVALUE`) lives in the host heap and is
/// trusted as-is. Anything else must be an inline scalar: a plugin may not hand
/// the host a raw pointer into its own heap. Enforces `from_raw_scalar`
/// (proven in `CatnipBoundaryProof`).
fn admit_plugin_scalar(pr: &PluginResult) -> VMResult<Value> {
    if pr.flags & PLUGIN_RESULT_HOSTVALUE != 0 {
        Ok(Value::from_raw(pr.value))
    } else {
        match catnip_core::nanbox::from_raw_scalar(pr.value) {
            Some(bits) => Ok(Value::from_raw(bits)),
            None => Err(VMError::RuntimeError(
                "plugin returned a non-scalar value without the host-builder flag".into(),
            )),
        }
    }
}

/// Admit a static plugin attribute (ABI v5 boundary lock).
///
/// Mirror of `admit_plugin_scalar` for the static-attr boundary: a host-built
/// pointer (`PLUGIN_ATTR_HOSTVALUE`) lives in the host heap and is trusted as-is;
/// anything else must be an inline scalar (`from_raw_scalar`, proven in
/// `CatnipBoundaryProof`). Rejects a raw pointer presented without the flag,
/// rather than dereferencing it (the value is `clone_refcount`-ed right after).
fn admit_plugin_attr(attr: &PluginAttr, name: &str, plugin: &str) -> VMResult<Value> {
    if attr.flags & PLUGIN_ATTR_HOSTVALUE != 0 {
        Ok(Value::from_raw(attr.value))
    } else {
        match catnip_core::nanbox::from_raw_scalar(attr.value) {
            Some(bits) => Ok(Value::from_raw(bits)),
            None => Err(VMError::RuntimeError(format!(
                "plugin '{}': attr '{}' is a non-scalar value without the host-builder flag",
                plugin, name
            ))),
        }
    }
}

/// Interpret a `PluginResult`, wrapping object handles with `obj_callbacks`.
pub fn interpret_plugin_result(pr: PluginResult, obj_callbacks: &PluginObjectCallbacks) -> VMResult<Value> {
    if pr.error_code != 0 {
        if pr.error_code == 0x45584954 {
            return Err(VMError::Exit(pr.value as i32));
        }
        let msg = if pr.error_message.is_null() {
            format!("plugin error (code {})", pr.error_code)
        } else {
            // SAFETY: `pr.error_message` is non-null (just checked) and points to
            // a NUL-terminated C string owned by the plugin; copied into an owned
            // String via lossy conversion before returning.
            unsafe { CStr::from_ptr(pr.error_message) }
                .to_string_lossy()
                .into_owned()
        };
        return Err(VMError::RuntimeError(msg));
    }
    if pr.flags & PLUGIN_RESULT_OBJECT != 0 {
        Ok(Value::from_plugin_object(pr.value, obj_callbacks.clone()))
    } else {
        admit_plugin_scalar(&pr)
    }
}

/// Invoke a module-level plugin function by its call pointer.
pub fn call_plugin_fn(
    call_fn: PluginCallFn,
    short_name: &str,
    args: &[Value],
    obj_callbacks: &PluginObjectCallbacks,
) -> VMResult<Value> {
    let c_name = std::ffi::CString::new(short_name)
        .map_err(|_| VMError::RuntimeError("plugin function name contains null byte".into()))?;
    let raw_args: Vec<u64> = args.iter().map(|v| v.bits()).collect();
    // SAFETY: `call_fn` is a plugin call pointer from a validated descriptor;
    // `c_name` is a valid NUL-terminated string and `raw_args` a live slice, both
    // outliving the call; catch_unwind contains any plugin-side panic.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        call_fn(c_name.as_ptr(), raw_args.as_ptr(), raw_args.len())
    }));
    match result {
        Ok(pr) => interpret_plugin_result(pr, obj_callbacks),
        Err(_) => Err(VMError::RuntimeError(format!(
            "plugin function '{}' panicked",
            short_name
        ))),
    }
}

/// Invoke a method on a plugin object handle.
pub fn call_plugin_method(
    handle: u64,
    method_fn: PluginMethodFn,
    method: &str,
    args: &[Value],
    callbacks: &PluginObjectCallbacks,
) -> VMResult<Value> {
    let c_method =
        std::ffi::CString::new(method).map_err(|_| VMError::RuntimeError("method name contains null byte".into()))?;
    let raw_args: Vec<u64> = args.iter().map(|v| v.bits()).collect();
    // SAFETY: `method_fn` is a callback from a validated plugin descriptor;
    // `handle` is an opaque token issued by that plugin, and `c_method`/`raw_args`
    // stay live for the call; catch_unwind contains any plugin-side panic.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        method_fn(handle, c_method.as_ptr(), raw_args.as_ptr(), raw_args.len())
    }));
    match result {
        Ok(pr) => interpret_plugin_result(pr, callbacks),
        Err(_) => Err(VMError::RuntimeError(format!("plugin method '{}' panicked", method))),
    }
}

/// Invoke getattr on a plugin object handle.
pub fn call_plugin_getattr(
    handle: u64,
    getattr_fn: PluginGetAttrFn,
    attr: &str,
    callbacks: &PluginObjectCallbacks,
) -> VMResult<Value> {
    let c_attr =
        std::ffi::CString::new(attr).map_err(|_| VMError::RuntimeError("attribute name contains null byte".into()))?;
    // SAFETY: `getattr_fn` is a callback from a validated plugin descriptor;
    // `handle` is an opaque token issued by that plugin, and `c_attr` stays live
    // for the call; catch_unwind contains any plugin-side panic.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        getattr_fn(handle, c_attr.as_ptr())
    }));
    match result {
        Ok(pr) => interpret_plugin_result(pr, callbacks),
        Err(_) => Err(VMError::RuntimeError(format!("plugin getattr '{}' panicked", attr))),
    }
}

/// Probe whether `name` is a member (attribute or method) of a plugin object.
///
/// Returns `false` on a null byte in `name` or a callback panic, so a failed
/// probe degrades to "not a member" (the caller raises AttributeError) rather
/// than fabricating a phantom bound method.
pub fn call_plugin_has_member(handle: u64, has_member_fn: PluginHasMemberFn, name: &str) -> bool {
    let Ok(c_name) = std::ffi::CString::new(name) else {
        return false;
    };
    // SAFETY: `has_member_fn` is a callback from a validated plugin descriptor;
    // `handle` is an opaque token issued by that plugin, and `c_name` stays live
    // for the call; per the ABI the probe must not mutate or block, and
    // catch_unwind contains any plugin-side panic.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        has_member_fn(handle, c_name.as_ptr())
    }));
    matches!(result, Ok(n) if n != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // host_make_dict receives plugin-owned key/value tokens (ownership transfer).
    // to_key takes an independent ref for pointer keys, so the key token must be
    // released -- otherwise every string-keyed dict a plugin builds leaks a ref.
    #[test]
    fn host_make_dict_releases_key_tokens() {
        use crate::collections::ValueKey;
        use std::sync::Arc;
        let key = Value::from_string("dictkey".to_string()); // NativeString strong=1 (the token)
        let witness = match key.to_key().unwrap() {
            ValueKey::Str(a) => a, // strong=2 (token + witness)
            _ => unreachable!(),
        };
        assert_eq!(Arc::strong_count(&witness), 2);
        let kbits = [key.bits()];
        let vbits = [Value::from_int(1).bits()];
        // host_make_dict consumes the key/value tokens.
        let dict_bits = unsafe { host_make_dict(kbits.as_ptr(), vbits.as_ptr(), 1) };
        Value::from_raw(dict_bits).decref(); // drop the dict -> releases its ValueKey
        assert_eq!(Arc::strong_count(&witness), 1, "host_make_dict leaked the key token");
    }

    /// Build catnip_hello and return the path to the .so/.dylib.
    fn build_hello_plugin() -> PathBuf {
        crate::test_support::hello_plugin()
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
                error_message: c"unknown method".as_ptr(),
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
                error_message: c"unknown attr".as_ptr(),
            },
        }
    }

    unsafe extern "C" fn mock_drop(handle: u64) {
        MOCK_DROPPED.fetch_add(handle, Ordering::Relaxed);
    }

    /// Members: attribute `id` and methods `value`/`child`.
    unsafe extern "C" fn mock_has_member(_handle: u64, name: *const c_char) -> u8 {
        let name = unsafe { CStr::from_ptr(name) }.to_bytes();
        u8::from(matches!(name, b"id" | b"value" | b"child"))
    }

    fn mock_callbacks() -> PluginObjectCallbacks {
        PluginObjectCallbacks {
            method: Some(mock_method),
            getattr: Some(mock_getattr),
            drop: Some(mock_drop),
            has_member: Some(mock_has_member),
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
        assert_eq!(PLUGIN_ABI_VERSION, 5);
    }

    #[test]
    fn admit_attr_scalar_without_flag_is_accepted() {
        // An inline scalar (no HOSTVALUE flag) passes from_raw_scalar untouched.
        let attr = PluginAttr::scalar(c"n".as_ptr(), Value::from_int(42).bits());
        let v = admit_plugin_attr(&attr, "n", "test").expect("scalar attr admitted");
        assert_eq!(v.as_int(), Some(42));
    }

    #[test]
    fn admit_attr_pointer_without_flag_is_rejected() {
        // A pointer-tagged value presented without HOSTVALUE must be refused, not
        // dereferenced. A host string is a real heap pointer; drop it afterwards.
        let s = Value::from_string("argv".to_string());
        let attr = PluginAttr::scalar(c"argv".as_ptr(), s.bits());
        let err = admit_plugin_attr(&attr, "argv", "test").unwrap_err();
        assert!(format!("{err:?}").contains("without the host-builder flag"));
        s.decref();
    }

    #[test]
    fn admit_attr_pointer_with_flag_is_trusted() {
        // The same pointer, declared host-built, is admitted as-is.
        let s = Value::from_string("argv".to_string());
        let attr = PluginAttr::host_value(c"argv".as_ptr(), s.bits());
        let v = admit_plugin_attr(&attr, "argv", "test").expect("host-built attr admitted");
        assert!(v.is_native_str());
        v.decref();
    }

    #[test]
    fn test_plugin_has_member_probe() {
        // Known members (attr + methods) report present; unknown names absent.
        assert!(call_plugin_has_member(7, mock_has_member, "id"));
        assert!(call_plugin_has_member(7, mock_has_member, "value"));
        assert!(call_plugin_has_member(7, mock_has_member, "child"));
        assert!(!call_plugin_has_member(7, mock_has_member, "typo"));
        // Null byte in the name degrades to "not a member".
        assert!(!call_plugin_has_member(7, mock_has_member, "bad\0name"));
    }

    // -- IO plugin integration tests --

    fn build_io_plugin() -> PathBuf {
        crate::test_support::io_plugin()
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
        crate::test_support::sys_plugin()
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
