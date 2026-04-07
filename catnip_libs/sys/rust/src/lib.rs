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
    PLUGIN_ABI_MAGIC, PLUGIN_ABI_VERSION, PluginAttr, PluginCallFn, PluginDescriptor, PluginResult,
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

// ---------------------------------------------------------------------------
// Build attrs at init time (argv, environ, platform, etc.)
// ---------------------------------------------------------------------------

fn build_attrs() -> Vec<PluginAttr> {
    // Static attr names (must live for the life of the plugin)
    static PROTOCOL_NAME: &[u8] = b"PROTOCOL\0";
    static ARGV_NAME: &[u8] = b"argv\0";
    static ENVIRON_NAME: &[u8] = b"environ\0";
    static PLATFORM_NAME: &[u8] = b"platform\0";
    static EXECUTABLE_NAME: &[u8] = b"executable\0";
    static VERSION_NAME: &[u8] = b"version\0";
    static CPU_COUNT_NAME: &[u8] = b"cpu_count\0";

    let mut attrs = Vec::new();

    // PROTOCOL
    attrs.push(PluginAttr {
        name: PROTOCOL_NAME.as_ptr() as *const c_char,
        value: Value::from_str("rust").bits(),
    });

    // argv as a list
    let argv_items: Vec<Value> = std::env::args().map(|a| Value::from_string(a)).collect();
    attrs.push(PluginAttr {
        name: ARGV_NAME.as_ptr() as *const c_char,
        value: Value::from_list(argv_items).bits(),
    });

    // environ as a dict
    {
        let mut map = indexmap::IndexMap::new();
        for (k, v) in std::env::vars() {
            let key =
                catnip_vm::collections::ValueKey::Str(std::sync::Arc::new(catnip_vm::value::NativeString::new(k)));
            map.insert(key, Value::from_string(v));
        }
        attrs.push(PluginAttr {
            name: ENVIRON_NAME.as_ptr() as *const c_char,
            value: Value::from_dict(map).bits(),
        });
    }

    // platform
    let platform = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        other => other,
    };
    attrs.push(PluginAttr {
        name: PLATFORM_NAME.as_ptr() as *const c_char,
        value: Value::from_str(platform).bits(),
    });

    // executable
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    attrs.push(PluginAttr {
        name: EXECUTABLE_NAME.as_ptr() as *const c_char,
        value: Value::from_string(exe).bits(),
    });

    // version
    attrs.push(PluginAttr {
        name: VERSION_NAME.as_ptr() as *const c_char,
        value: Value::from_str(env!("CARGO_PKG_VERSION")).bits(),
    });

    // cpu_count
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    attrs.push(PluginAttr {
        name: CPU_COUNT_NAME.as_ptr() as *const c_char,
        value: Value::from_int(cpus as i64).bits(),
    });

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
            error_message: b"unknown function\0".as_ptr() as *const c_char,
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

#[unsafe(no_mangle)]
pub extern "C" fn catnip_plugin_init() -> *const PluginDescriptor {
    let sd = DESCRIPTOR.get_or_init(|| {
        let attrs = build_attrs();
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

        let argv = argv.unwrap_or_else(|| std::env::args().collect());
        m.add("argv", argv)?;

        let environ = PyDict::new(py);
        for (k, v) in std::env::vars() {
            environ.set_item(&k, &v)?;
        }
        m.add("environ", environ)?;

        let executable = executable.unwrap_or_else(|| {
            std::env::current_exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        m.add("executable", executable)?;

        m.add("version", env!("CARGO_PKG_VERSION"))?;
        m.add("platform", std::env::consts::OS)?;

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
