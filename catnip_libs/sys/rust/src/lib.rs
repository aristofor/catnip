// FILE: catnip_libs/sys/rust/src/lib.rs
//! Catnip `sys` stdlib plugin (native ABI v2 + optional PyO3 backend).
//!
//! Exports: PROTOCOL, argv, environ, executable, version, platform, cpu_count, exit.
//! Pure Rust -- no Python sys/os imports. All data from std::env.

#[cfg(feature = "pyo3")]
pub use pymodule::build_module;

use std::ffi::{CStr, c_char};
use std::sync::OnceLock;

use catnip_vm::Value;
use catnip_vm::plugin::{
    PLUGIN_ABI_MAGIC, PLUGIN_ABI_VERSION, PluginAttr, PluginCallFn, PluginDescriptor, PluginHostApi, PluginResult,
};

// ---------------------------------------------------------------------------
// Static data
// ---------------------------------------------------------------------------

static MODULE_NAME: &[u8] = b"sys\0";
static MODULE_VERSION: &[u8] = b"0.1.0\0";

static FN_NAMES: &[&[u8]] = &[b"exit\0"];

/// Error code for exit -- the host converts this to VMError::Exit.
pub const EXIT_ERROR_CODE: u32 = 0x45584954; // "EXIT"

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Platform string following Python's `sys.platform` convention (darwin, win32),
/// shared by the native and PyO3 backends so both report the same value.
fn platform_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Build attrs at init time (argv, environ, platform, etc.)
// ---------------------------------------------------------------------------

fn build_attrs(host: &PluginHostApi) -> Vec<PluginAttr> {
    // Static attr names (must live for the life of the plugin)
    static PROTOCOL_NAME: &[u8] = b"PROTOCOL\0";
    static ARGV_NAME: &[u8] = b"argv\0";
    static ENVIRON_NAME: &[u8] = b"environ\0";
    static PLATFORM_NAME: &[u8] = b"platform\0";
    static EXECUTABLE_NAME: &[u8] = b"executable\0";
    static VERSION_NAME: &[u8] = b"version\0";
    static CPU_COUNT_NAME: &[u8] = b"cpu_count\0";

    // ABI v4: every structured value is built in the host heap via the host
    // builder callbacks, so no plugin-owned Arc ever crosses the boundary.
    let mk = |s: &str| unsafe { (host.make_string)(s.as_ptr(), s.len()) };
    let mut attrs = Vec::new();

    // PROTOCOL
    attrs.push(PluginAttr::host_value(
        PROTOCOL_NAME.as_ptr() as *const c_char,
        mk("rust"),
    ));

    // argv as a list of host strings (the string tokens are consumed by make_list)
    // args_os/vars_os : ne paniquent pas sur argv/env non-UTF-8 (contrairement à
    // args/vars). Un init de plugin qui panique traverserait `extern "C"` (UB).
    let argv_tokens: Vec<u64> = std::env::args_os().map(|a| mk(&a.to_string_lossy())).collect();
    let argv_val = unsafe { (host.make_list)(argv_tokens.as_ptr(), argv_tokens.len()) };
    attrs.push(PluginAttr::host_value(ARGV_NAME.as_ptr() as *const c_char, argv_val));

    // environ as a host dict of host strings
    let mut keys: Vec<u64> = Vec::new();
    let mut vals: Vec<u64> = Vec::new();
    for (k, v) in std::env::vars_os() {
        keys.push(mk(&k.to_string_lossy()));
        vals.push(mk(&v.to_string_lossy()));
    }
    let environ_val = unsafe { (host.make_dict)(keys.as_ptr(), vals.as_ptr(), keys.len()) };
    attrs.push(PluginAttr::host_value(
        ENVIRON_NAME.as_ptr() as *const c_char,
        environ_val,
    ));

    // platform
    attrs.push(PluginAttr::host_value(
        PLATFORM_NAME.as_ptr() as *const c_char,
        mk(platform_name()),
    ));

    // executable
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    attrs.push(PluginAttr::host_value(
        EXECUTABLE_NAME.as_ptr() as *const c_char,
        mk(&exe),
    ));

    // version -- the Catnip runtime version, not this plugin crate's version
    attrs.push(PluginAttr::host_value(
        VERSION_NAME.as_ptr() as *const c_char,
        mk(catnip_vm::CATNIP_VERSION),
    ));

    // cpu_count (scalar -- crosses directly)
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    attrs.push(PluginAttr::scalar(
        CPU_COUNT_NAME.as_ptr() as *const c_char,
        Value::from_int(cpus as i64).bits(),
    ));

    attrs
}

// ---------------------------------------------------------------------------
// Function dispatch
// ---------------------------------------------------------------------------

unsafe extern "C" fn plugin_call(function_name: *const c_char, args: *const u64, argc: usize) -> PluginResult {
    let name = unsafe { CStr::from_ptr(function_name) }.to_bytes();

    match name {
        b"exit" => {
            let code = if argc > 0 {
                let v = Value::from_raw(unsafe { *args });
                v.as_int().unwrap_or(0) as i32
            } else {
                0
            };
            // Signal exit via special error code; host converts to VMError::Exit
            let msg = format!("exit({})\0", code);
            let ptr = msg.as_ptr() as *const c_char;
            std::mem::forget(msg);
            PluginResult {
                value: code as u64,
                error_code: EXIT_ERROR_CODE,
                flags: 0,
                error_message: ptr,
            }
        }
        _ => PluginResult {
            value: 0,
            error_code: 1,
            flags: 0,
            error_message: c"unknown function".as_ptr(),
        },
    }
}

// ---------------------------------------------------------------------------
// Plugin init
// ---------------------------------------------------------------------------

struct StaticDescriptor {
    _attrs: Vec<PluginAttr>,
    _fn_ptrs: Vec<*const c_char>,
    desc: PluginDescriptor,
}

unsafe impl Send for StaticDescriptor {}
unsafe impl Sync for StaticDescriptor {}

static DESCRIPTOR: OnceLock<StaticDescriptor> = OnceLock::new();

/// Plugin ABI entry point: builds and returns the module descriptor.
///
/// # Safety
/// `host_api` must point to a valid `PluginHostApi` for the duration of the call.
/// The catnip_vm loader upholds this contract when initializing the plugin.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn catnip_plugin_init(host_api: *const PluginHostApi) -> *const PluginDescriptor {
    let sd = DESCRIPTOR.get_or_init(|| {
        let attrs = build_attrs(unsafe { &*host_api });
        let fn_ptrs: Vec<*const c_char> = FN_NAMES.iter().map(|n| n.as_ptr() as *const c_char).collect();

        let desc = PluginDescriptor {
            abi_magic: PLUGIN_ABI_MAGIC,
            abi_version: PLUGIN_ABI_VERSION,
            module_name: MODULE_NAME.as_ptr() as *const c_char,
            module_version: MODULE_VERSION.as_ptr() as *const c_char,
            num_attrs: attrs.len() as u32,
            attrs: attrs.as_ptr(),
            num_functions: FN_NAMES.len() as u32,
            functions: fn_ptrs.as_ptr(),
            call: plugin_call as PluginCallFn,
            method: None,
            getattr: None,
            drop: None,
            has_member: None,
        };

        StaticDescriptor {
            _attrs: attrs,
            _fn_ptrs: fn_ptrs,
            desc,
        }
    });

    &sd.desc as *const PluginDescriptor
}

// ---------------------------------------------------------------------------
// PyO3 backend (for Python pipeline compatibility)
// ---------------------------------------------------------------------------

#[cfg(feature = "pyo3")]
pub mod pymodule {
    use pyo3::prelude::*;
    use pyo3::types::PyDict;

    pub fn build_module(
        py: Python<'_>,
        argv: Option<Vec<String>>,
        executable: Option<String>,
    ) -> PyResult<Py<PyModule>> {
        let m = PyModule::new(py, "sys")?;
        register_items(&m, argv, executable)?;
        Ok(m.unbind())
    }

    fn register_items(m: &Bound<'_, PyModule>, argv: Option<Vec<String>>, executable: Option<String>) -> PyResult<()> {
        let py = m.py();
        m.add("PROTOCOL", "rust")?;

        let argv = argv.unwrap_or_else(|| std::env::args_os().map(|a| a.to_string_lossy().into_owned()).collect());
        m.add("argv", argv)?;

        let environ = PyDict::new(py);
        for (k, v) in std::env::vars_os() {
            environ.set_item(k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned())?;
        }
        m.add("environ", environ)?;

        let executable = executable.unwrap_or_else(|| {
            std::env::current_exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        m.add("executable", executable)?;

        m.add("version", catnip_vm::CATNIP_VERSION)?;
        m.add("platform", super::platform_name())?;

        let cpu_count = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        m.add("cpu_count", cpu_count)?;

        let exit_fn = py.eval(
            pyo3::ffi::c_str!("lambda code=0: (_ for _ in ()).throw(SystemExit(code))"),
            None,
            None,
        )?;
        m.add("exit", exit_fn)?;

        Ok(())
    }

    #[pymodule]
    fn catnip_sys(m: &Bound<'_, PyModule>) -> PyResult<()> {
        register_items(m, None, None)
    }
}
