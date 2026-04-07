// FILE: catnip_libs/io/rust/src/lib.rs
//! Catnip `io` stdlib plugin (native ABI v2 + optional PyO3 backend).
//!
//! Exports: PROTOCOL, VERSION, print, write, writeln, eprint, input, open.
//! open() returns a PluginObject file handle with methods: read, readline, write, close.
//!
//! The PyO3 backend (feature `pyo3`) provides `build_module()` and `PyInit_catnip_io`
//! for the Python pipeline. The native ABI is always available.

#[cfg(feature = "pyo3")]
pub use pymodule::build_module;

use std::cell::RefCell;
use std::ffi::{CStr, c_char};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::sync::OnceLock;

use catnip_vm::Value;
use catnip_vm::plugin::{
    PLUGIN_ABI_MAGIC, PLUGIN_ABI_VERSION, PLUGIN_RESULT_OBJECT, PluginAttr, PluginCallFn, PluginDescriptor,
    PluginDropFn, PluginGetAttrFn, PluginMethodFn, PluginResult,
};

// ---------------------------------------------------------------------------
// Static data
// ---------------------------------------------------------------------------

static MODULE_NAME: &[u8] = b"io\0";
static MODULE_VERSION: &[u8] = b"0.1.0\0";
static VERSION_ATTR_NAME: &[u8] = b"VERSION\0";
static PROTOCOL_ATTR_NAME: &[u8] = b"PROTOCOL\0";

static FN_NAMES: &[&[u8]] = &[b"print\0", b"write\0", b"writeln\0", b"eprint\0", b"input\0", b"open\0"];

// ---------------------------------------------------------------------------
// File handle storage (thread-local for single-threaded PureVM)
// ---------------------------------------------------------------------------

enum FileState {
    Reader(BufReader<std::fs::File>),
    Writer(BufWriter<std::fs::File>),
    Closed,
}

struct FileHandle {
    path: String,
    mode: String,
    state: FileState,
}

thread_local! {
    static FILES: RefCell<Vec<Option<FileHandle>>> = RefCell::new(Vec::new());
}

fn alloc_file(handle: FileHandle) -> u64 {
    FILES.with(|files| {
        let mut files = files.borrow_mut();
        // Reuse a free slot
        for (i, slot) in files.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(handle);
                return i as u64;
            }
        }
        let idx = files.len();
        files.push(Some(handle));
        idx as u64
    })
}

fn with_file<T>(handle: u64, f: impl FnOnce(&mut FileHandle) -> T) -> Option<T> {
    FILES.with(|files| {
        let mut files = files.borrow_mut();
        let slot = files.get_mut(handle as usize)?;
        slot.as_mut().map(f)
    })
}

fn free_file(handle: u64) {
    FILES.with(|files| {
        let mut files = files.borrow_mut();
        if let Some(slot) = files.get_mut(handle as usize) {
            *slot = None;
        }
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ok_nil() -> PluginResult {
    PluginResult {
        value: Value::NIL.bits(),
        error_code: 0,
        flags: 0,
        error_message: std::ptr::null(),
    }
}

fn ok_val(v: Value) -> PluginResult {
    PluginResult {
        value: v.bits(),
        error_code: 0,
        flags: 0,
        error_message: std::ptr::null(),
    }
}

fn ok_object(handle: u64) -> PluginResult {
    PluginResult {
        value: handle,
        error_code: 0,
        flags: PLUGIN_RESULT_OBJECT,
        error_message: std::ptr::null(),
    }
}

fn err(msg: &'static [u8]) -> PluginResult {
    PluginResult {
        value: 0,
        error_code: 1,
        flags: 0,
        error_message: msg.as_ptr() as *const c_char,
    }
}

fn extract_str(raw: u64) -> Option<String> {
    let v = Value::from_raw(raw);
    if v.is_native_str() {
        Some(unsafe { v.as_native_str_ref().unwrap() }.to_string())
    } else {
        None
    }
}

fn display_val(raw: u64) -> String {
    Value::from_raw(raw).display_string()
}

// ---------------------------------------------------------------------------
// Module-level function dispatch
// ---------------------------------------------------------------------------

unsafe extern "C" fn plugin_call(function_name: *const c_char, args: *const u64, argc: usize) -> PluginResult {
    let name = unsafe { CStr::from_ptr(function_name) }.to_bytes();
    let args_slice = if argc > 0 {
        unsafe { std::slice::from_raw_parts(args, argc) }
    } else {
        &[]
    };

    match name {
        b"print" => {
            let mut out = std::io::stdout().lock();
            for (i, &raw) in args_slice.iter().enumerate() {
                if i > 0 {
                    let _ = out.write_all(b" ");
                }
                let _ = out.write_all(display_val(raw).as_bytes());
            }
            let _ = out.write_all(b"\n");
            ok_nil()
        }
        b"write" => {
            let mut out = std::io::stdout().lock();
            for &raw in args_slice {
                let _ = out.write_all(display_val(raw).as_bytes());
            }
            let _ = out.flush();
            ok_nil()
        }
        b"writeln" => {
            let mut out = std::io::stdout().lock();
            for &raw in args_slice {
                let _ = out.write_all(display_val(raw).as_bytes());
            }
            let _ = out.write_all(b"\n");
            ok_nil()
        }
        b"eprint" => {
            let mut out = std::io::stderr().lock();
            for (i, &raw) in args_slice.iter().enumerate() {
                if i > 0 {
                    let _ = out.write_all(b" ");
                }
                let _ = out.write_all(display_val(raw).as_bytes());
            }
            let _ = out.write_all(b"\n");
            ok_nil()
        }
        b"input" => {
            if let Some(&prompt_raw) = args_slice.first() {
                let prompt = display_val(prompt_raw);
                print!("{}", prompt);
                let _ = std::io::stdout().flush();
            }
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                Ok(0) => err(b"EOFError: end of input\0"),
                Ok(_) => {
                    if line.ends_with('\n') {
                        line.pop();
                    }
                    if line.ends_with('\r') {
                        line.pop();
                    }
                    ok_val(Value::from_string(line))
                }
                Err(_) => err(b"input error\0"),
            }
        }
        b"open" => {
            let path = match args_slice.first().and_then(|&r| extract_str(r)) {
                Some(p) => p,
                None => return err(b"open() requires a string path\0"),
            };
            let mode = if let Some(&raw) = args_slice.get(1) {
                let v = Value::from_raw(raw);
                if v.is_nil() {
                    "r".to_string()
                } else if v.is_native_str() {
                    unsafe { v.as_native_str_ref().unwrap() }.to_string()
                } else {
                    return err(b"open() mode must be a string\0");
                }
            } else {
                "r".to_string()
            };

            let handle = match mode.as_str() {
                "r" => {
                    match std::fs::File::open(&path) {
                        Ok(f) => FileHandle {
                            path,
                            mode,
                            state: FileState::Reader(BufReader::new(f)),
                        },
                        Err(e) => {
                            // Leak a static-ish error message via Box
                            let msg = format!("FileNotFoundError: {}: '{}'\0", e, path);
                            let ptr = msg.as_ptr() as *const c_char;
                            std::mem::forget(msg);
                            return PluginResult {
                                value: 0,
                                error_code: 1,
                                flags: 0,
                                error_message: ptr,
                            };
                        }
                    }
                }
                "w" => match std::fs::File::create(&path) {
                    Ok(f) => FileHandle {
                        path,
                        mode,
                        state: FileState::Writer(BufWriter::new(f)),
                    },
                    Err(e) => {
                        let msg = format!("IOError: {}: '{}'\0", e, path);
                        let ptr = msg.as_ptr() as *const c_char;
                        std::mem::forget(msg);
                        return PluginResult {
                            value: 0,
                            error_code: 1,
                            flags: 0,
                            error_message: ptr,
                        };
                    }
                },
                "a" => match std::fs::OpenOptions::new().append(true).create(true).open(&path) {
                    Ok(f) => FileHandle {
                        path,
                        mode,
                        state: FileState::Writer(BufWriter::new(f)),
                    },
                    Err(e) => {
                        let msg = format!("IOError: {}: '{}'\0", e, path);
                        let ptr = msg.as_ptr() as *const c_char;
                        std::mem::forget(msg);
                        return PluginResult {
                            value: 0,
                            error_code: 1,
                            flags: 0,
                            error_message: ptr,
                        };
                    }
                },
                _ => return err(b"open() mode must be 'r', 'w', or 'a'\0"),
            };

            ok_object(alloc_file(handle))
        }
        _ => err(b"unknown function\0"),
    }
}

// ---------------------------------------------------------------------------
// Object method dispatch (file handles)
// ---------------------------------------------------------------------------

unsafe extern "C" fn plugin_method(handle: u64, method: *const c_char, args: *const u64, argc: usize) -> PluginResult {
    let method = unsafe { CStr::from_ptr(method) }.to_bytes();

    match method {
        b"read" => {
            match with_file(handle, |fh| {
                if let FileState::Reader(ref mut r) = fh.state {
                    let mut buf = String::new();
                    r.read_to_string(&mut buf).map(|_| buf)
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "file not opened for reading",
                    ))
                }
            }) {
                Some(Ok(s)) => ok_val(Value::from_string(s)),
                Some(Err(e)) => {
                    let msg = format!("read error: {}\0", e);
                    let ptr = msg.as_ptr() as *const c_char;
                    std::mem::forget(msg);
                    PluginResult {
                        value: 0,
                        error_code: 1,
                        flags: 0,
                        error_message: ptr,
                    }
                }
                None => err(b"invalid file handle\0"),
            }
        }
        b"readline" => {
            match with_file(handle, |fh| {
                if let FileState::Reader(ref mut r) = fh.state {
                    let mut line = String::new();
                    r.read_line(&mut line).map(|_| line)
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "file not opened for reading",
                    ))
                }
            }) {
                Some(Ok(s)) => ok_val(Value::from_string(s)),
                Some(Err(_)) => err(b"readline error\0"),
                None => err(b"invalid file handle\0"),
            }
        }
        b"write" => {
            let data = if argc > 0 {
                let raw = unsafe { *args };
                extract_str(raw).unwrap_or_else(|| display_val(raw))
            } else {
                return err(b"write() requires 1 argument\0");
            };
            match with_file(handle, |fh| {
                if let FileState::Writer(ref mut w) = fh.state {
                    w.write_all(data.as_bytes()).map(|_| data.chars().count())
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "file not opened for writing",
                    ))
                }
            }) {
                Some(Ok(n)) => ok_val(Value::from_int(n as i64)),
                Some(Err(_)) => err(b"write error\0"),
                None => err(b"invalid file handle\0"),
            }
        }
        b"close" => {
            with_file(handle, |fh| {
                if let FileState::Writer(ref mut w) = fh.state {
                    let _ = w.flush();
                }
                fh.state = FileState::Closed;
            });
            ok_nil()
        }
        _ => err(b"unknown method\0"),
    }
}

// ---------------------------------------------------------------------------
// Object attribute access
// ---------------------------------------------------------------------------

unsafe extern "C" fn plugin_getattr(handle: u64, attr: *const c_char) -> PluginResult {
    let attr = unsafe { CStr::from_ptr(attr) }.to_bytes();

    match attr {
        b"name" => match with_file(handle, |fh| fh.path.clone()) {
            Some(p) => ok_val(Value::from_string(p)),
            None => err(b"invalid file handle\0"),
        },
        b"mode" => match with_file(handle, |fh| fh.mode.clone()) {
            Some(m) => ok_val(Value::from_string(m)),
            None => err(b"invalid file handle\0"),
        },
        b"closed" => match with_file(handle, |fh| matches!(fh.state, FileState::Closed)) {
            Some(c) => ok_val(Value::from_bool(c)),
            None => err(b"invalid file handle\0"),
        },
        _ => err(b"unknown attribute\0"),
    }
}

// ---------------------------------------------------------------------------
// Object drop
// ---------------------------------------------------------------------------

unsafe extern "C" fn plugin_drop(handle: u64) {
    // Flush writer before freeing
    with_file(handle, |fh| {
        if let FileState::Writer(ref mut w) = fh.state {
            let _ = w.flush();
        }
    });
    free_file(handle);
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
        let attrs = vec![
            PluginAttr {
                name: PROTOCOL_ATTR_NAME.as_ptr() as *const c_char,
                value: Value::from_str("rust").bits(),
            },
            PluginAttr {
                name: VERSION_ATTR_NAME.as_ptr() as *const c_char,
                value: Value::from_str("0.1.0").bits(),
            },
        ];

        let fn_ptrs: Vec<*const c_char> = FN_NAMES.iter().map(|n| n.as_ptr() as *const c_char).collect();

        let desc = PluginDescriptor {
            abi_magic: PLUGIN_ABI_MAGIC,
            abi_version: PLUGIN_ABI_VERSION,
            module_name: MODULE_NAME.as_ptr() as *const c_char,
            module_version: MODULE_VERSION.as_ptr() as *const c_char,
            num_attrs: 2,
            attrs: attrs.as_ptr(),
            num_functions: FN_NAMES.len() as u32,
            functions: fn_ptrs.as_ptr(),
            call: plugin_call as PluginCallFn,
            method: Some(plugin_method as PluginMethodFn),
            getattr: Some(plugin_getattr as PluginGetAttrFn),
            drop: Some(plugin_drop as PluginDropFn),
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
    use pyo3::types::PyTuple;

    fn get_output<'py>(py: Python<'py>, file: Option<&Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
        match file {
            Some(f) => Ok(f.clone()),
            None => Ok(py.import("sys")?.getattr("stdout")?.into_any()),
        }
    }

    fn do_print(
        py: Python<'_>,
        values: &Bound<'_, PyTuple>,
        sep: &str,
        end: &str,
        file: Option<&Bound<'_, PyAny>>,
        flush: bool,
    ) -> PyResult<()> {
        let out = get_output(py, file)?;
        let parts: Vec<String> = values
            .iter()
            .map(|v| {
                v.str()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| format!("{:?}", v))
            })
            .collect();
        out.call_method1("write", (format!("{}{}", parts.join(sep), end),))?;
        if flush {
            out.call_method0("flush")?;
        }
        Ok(())
    }

    #[pyfunction]
    #[pyo3(signature = (*values, sep=" ", end="\n", file=None, flush=false))]
    fn print(
        py: Python<'_>,
        values: &Bound<'_, PyTuple>,
        sep: &str,
        end: &str,
        file: Option<&Bound<'_, PyAny>>,
        flush: bool,
    ) -> PyResult<()> {
        do_print(py, values, sep, end, file, flush)
    }

    #[pyfunction]
    #[pyo3(signature = (*values, file=None, flush=true))]
    fn write(
        py: Python<'_>,
        values: &Bound<'_, PyTuple>,
        file: Option<&Bound<'_, PyAny>>,
        flush: bool,
    ) -> PyResult<()> {
        do_print(py, values, "", "", file, flush)
    }

    #[pyfunction]
    #[pyo3(signature = (*values, file=None, flush=true))]
    fn writeln(
        py: Python<'_>,
        values: &Bound<'_, PyTuple>,
        file: Option<&Bound<'_, PyAny>>,
        flush: bool,
    ) -> PyResult<()> {
        do_print(py, values, "", "\n", file, flush)
    }

    #[pyfunction]
    #[pyo3(signature = (*values, sep=" ", end="\n", flush=true))]
    fn eprint(py: Python<'_>, values: &Bound<'_, PyTuple>, sep: &str, end: &str, flush: bool) -> PyResult<()> {
        let stderr = py.import("sys")?.getattr("stderr")?;
        do_print(py, values, sep, end, Some(&stderr), flush)
    }

    #[pyfunction]
    #[pyo3(signature = (prompt=""))]
    fn input(py: Python<'_>, prompt: &str) -> PyResult<String> {
        let sys = py.import("sys")?;
        if !prompt.is_empty() {
            let stdout = sys.getattr("stdout")?;
            stdout.call_method1("write", (prompt,))?;
            stdout.call_method0("flush")?;
        }
        let line: String = sys.getattr("stdin")?.call_method0("readline")?.extract()?;
        if line.is_empty() {
            return Err(pyo3::exceptions::PyEOFError::new_err("end of input"));
        }
        Ok(line.trim_end_matches('\n').to_string())
    }

    #[pyfunction]
    #[pyo3(signature = (file, mode="r", buffering=-1, encoding=None, errors=None, newline=None, closefd=true, opener=None))]
    fn open<'py>(
        py: Python<'py>,
        file: &Bound<'py, PyAny>,
        mode: &str,
        buffering: i32,
        encoding: Option<&str>,
        errors: Option<&str>,
        newline: Option<&str>,
        closefd: bool,
        opener: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let kwargs = pyo3::types::PyDict::new(py);
        kwargs.set_item("mode", mode)?;
        kwargs.set_item("buffering", buffering)?;
        kwargs.set_item("closefd", closefd)?;
        if let Some(v) = encoding {
            kwargs.set_item("encoding", v)?;
        }
        if let Some(v) = errors {
            kwargs.set_item("errors", v)?;
        }
        if let Some(v) = newline {
            kwargs.set_item("newline", v)?;
        }
        if let Some(v) = opener {
            kwargs.set_item("opener", v)?;
        }
        py.import("builtins")?.getattr("open")?.call((file,), Some(&kwargs))
    }

    fn register_items(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add("PROTOCOL", "rust")?;
        m.add("VERSION", "0.1.0")?;
        m.add_function(pyo3::wrap_pyfunction!(print, m)?)?;
        m.add_function(pyo3::wrap_pyfunction!(write, m)?)?;
        m.add_function(pyo3::wrap_pyfunction!(writeln, m)?)?;
        m.add_function(pyo3::wrap_pyfunction!(eprint, m)?)?;
        m.add_function(pyo3::wrap_pyfunction!(input, m)?)?;
        m.add_function(pyo3::wrap_pyfunction!(open, m)?)?;
        Ok(())
    }

    /// Build and return the `io` module as a Python module object.
    /// Called by the embedded Python loader.
    pub fn build_module(py: Python<'_>) -> PyResult<Py<PyModule>> {
        let m = PyModule::new(py, "io")?;
        register_items(&m)?;
        Ok(m.unbind())
    }

    #[pymodule]
    fn catnip_io(m: &Bound<'_, PyModule>) -> PyResult<()> {
        register_items(m)
    }
}
