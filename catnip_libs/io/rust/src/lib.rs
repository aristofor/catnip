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
use std::sync::atomic::{AtomicPtr, Ordering};

use catnip_vm::Value;
use catnip_vm::plugin::{
    PLUGIN_ABI_MAGIC, PLUGIN_ABI_VERSION, PLUGIN_RESULT_HOSTVALUE, PLUGIN_RESULT_OBJECT, PluginAttr, PluginCallFn,
    PluginDescriptor, PluginDropFn, PluginGetAttrFn, PluginHasMemberFn, PluginHostApi, PluginMethodFn, PluginResult,
};

// ABI v4: host value-builder API, stored at init so structured returns are
// built in the host heap.
static HOST_API: AtomicPtr<PluginHostApi> = AtomicPtr::new(std::ptr::null_mut());

#[inline]
fn host() -> &'static PluginHostApi {
    // SAFETY: set by catnip_plugin_init before any call; host-owned, 'static.
    unsafe { &*HOST_API.load(Ordering::Acquire) }
}

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
    static FILES: RefCell<Vec<Option<FileHandle>>> = const { RefCell::new(Vec::new()) };
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

// ABI v4: return a string built in the host heap (never a plugin-owned Arc).
fn ok_host_string(s: &str) -> PluginResult {
    let value = unsafe { (host().make_string)(s.as_ptr(), s.len()) };
    PluginResult {
        value,
        error_code: 0,
        flags: PLUGIN_RESULT_HOSTVALUE,
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

/// Build a plugin error result from an I/O failure. Leaks the message string;
/// the host copies it immediately (same pattern as the read/open error paths).
fn io_err(op: &str, e: &std::io::Error) -> PluginResult {
    let msg = format!("IOError: {} failed: {}\0", op, e);
    let ptr = msg.as_ptr() as *const c_char;
    std::mem::forget(msg);
    PluginResult {
        value: 0,
        error_code: 1,
        flags: 0,
        error_message: ptr,
    }
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
            let res = (|| -> std::io::Result<()> {
                for (i, &raw) in args_slice.iter().enumerate() {
                    if i > 0 {
                        out.write_all(b" ")?;
                    }
                    out.write_all(display_val(raw).as_bytes())?;
                }
                out.write_all(b"\n")
            })();
            match res {
                Ok(()) => ok_nil(),
                Err(e) => io_err("print", &e),
            }
        }
        b"write" => {
            let mut out = std::io::stdout().lock();
            let res = (|| -> std::io::Result<()> {
                for &raw in args_slice {
                    out.write_all(display_val(raw).as_bytes())?;
                }
                out.flush()
            })();
            match res {
                Ok(()) => ok_nil(),
                Err(e) => io_err("write", &e),
            }
        }
        b"writeln" => {
            let mut out = std::io::stdout().lock();
            let res = (|| -> std::io::Result<()> {
                for &raw in args_slice {
                    out.write_all(display_val(raw).as_bytes())?;
                }
                out.write_all(b"\n")
            })();
            match res {
                Ok(()) => ok_nil(),
                Err(e) => io_err("writeln", &e),
            }
        }
        b"eprint" => {
            let mut out = std::io::stderr().lock();
            let res = (|| -> std::io::Result<()> {
                for (i, &raw) in args_slice.iter().enumerate() {
                    if i > 0 {
                        out.write_all(b" ")?;
                    }
                    out.write_all(display_val(raw).as_bytes())?;
                }
                out.write_all(b"\n")
            })();
            match res {
                Ok(()) => ok_nil(),
                Err(e) => io_err("eprint", &e),
            }
        }
        b"input" => {
            if let Some(&prompt_raw) = args_slice.first() {
                // Best-effort prompt: write directly to the locked stdout instead of
                // print!, which panics on write errors -- a panic across extern "C" is UB.
                let prompt = display_val(prompt_raw);
                let mut out = std::io::stdout().lock();
                let _ = out.write_all(prompt.as_bytes());
                let _ = out.flush();
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
                    ok_host_string(&line)
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
                    Err(std::io::Error::other("file not opened for reading"))
                }
            }) {
                Some(Ok(s)) => ok_host_string(&s),
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
                    Err(std::io::Error::other("file not opened for reading"))
                }
            }) {
                Some(Ok(s)) => ok_host_string(&s),
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
                    Err(std::io::Error::other("file not opened for writing"))
                }
            }) {
                Some(Ok(n)) => ok_val(Value::from_int(n as i64)),
                Some(Err(_)) => err(b"write error\0"),
                None => err(b"invalid file handle\0"),
            }
        }
        b"close" => {
            // Flush before closing so a write error (e.g. disk full) surfaces
            // instead of being silently lost; the handle is closed either way,
            // matching CPython, which still releases the fd when close() raises.
            let res = with_file(handle, |fh| {
                let flushed = match fh.state {
                    FileState::Writer(ref mut w) => w.flush(),
                    _ => Ok(()),
                };
                fh.state = FileState::Closed;
                flushed
            });
            match res {
                Some(Ok(())) => ok_nil(),
                Some(Err(e)) => io_err("close", &e),
                None => err(b"invalid file handle\0"),
            }
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
            Some(p) => ok_host_string(&p),
            None => err(b"invalid file handle\0"),
        },
        b"mode" => match with_file(handle, |fh| fh.mode.clone()) {
            Some(m) => ok_host_string(&m),
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
// Object membership probe (static: a File's members don't depend on its state)
// ---------------------------------------------------------------------------

unsafe extern "C" fn plugin_has_member(_handle: u64, name: *const c_char) -> u8 {
    let name = unsafe { CStr::from_ptr(name) }.to_bytes();
    u8::from(matches!(
        name,
        b"name" | b"mode" | b"closed" | b"read" | b"readline" | b"write" | b"close"
    ))
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

/// Plugin ABI entry point: builds and returns the module descriptor.
///
/// # Safety
/// `host_api` must point to a valid `PluginHostApi` for the duration of the call.
/// The catnip_vm loader upholds this contract when initializing the plugin.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn catnip_plugin_init(host_api: *const PluginHostApi) -> *const PluginDescriptor {
    HOST_API.store(host_api as *mut PluginHostApi, Ordering::Release);
    let sd = DESCRIPTOR.get_or_init(|| {
        let mk = |s: &str| unsafe { ((*host_api).make_string)(s.as_ptr(), s.len()) };
        let attrs = vec![
            PluginAttr::host_value(PROTOCOL_ATTR_NAME.as_ptr() as *const c_char, mk("rust")),
            PluginAttr::host_value(VERSION_ATTR_NAME.as_ptr() as *const c_char, mk("0.1.0")),
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
            has_member: Some(plugin_has_member as PluginHasMemberFn),
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
        let mut line: String = sys.getattr("stdin")?.call_method0("readline")?.extract()?;
        if line.is_empty() {
            return Err(pyo3::exceptions::PyEOFError::new_err("end of input"));
        }
        // Strip the trailing line terminator, matching the native ABI backend
        // (a single \n then \r, so \n and \r\n both yield the bare line).
        if line.ends_with('\n') {
            line.pop();
        }
        if line.ends_with('\r') {
            line.pop();
        }
        Ok(line)
    }

    // Miroir de la signature du builtin Python open() : 8 paramètres irréductibles.
    #[allow(clippy::too_many_arguments)]
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

// ---------------------------------------------------------------------------
// Tests for the native file-handle path
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // The native open/write/close path has no .cat coverage (the integration
    // tests only exercise module-level print/write). close() must flush the
    // BufWriter so buffered bytes reach disk, and report a flush error instead
    // of swallowing it.
    #[test]
    fn close_flushes_buffered_writes() {
        let path = std::env::temp_dir().join(format!("catnip_io_close_{}.txt", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        let handle = alloc_file(FileHandle {
            path: path.to_string_lossy().into_owned(),
            mode: "w".to_string(),
            state: FileState::Writer(BufWriter::new(file)),
        });

        // Buffer bytes without flushing -- BufWriter holds them until flush.
        with_file(handle, |fh| {
            if let FileState::Writer(ref mut w) = fh.state {
                w.write_all(b"buffered").unwrap();
            }
        });

        let res = unsafe { plugin_method(handle, c"close".as_ptr(), std::ptr::null(), 0) };
        assert_eq!(res.error_code, 0, "close on a healthy writer succeeds");

        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "buffered", "close flushed the buffered bytes");

        let _ = std::fs::remove_file(&path);
    }

    // A second close on an already-closed handle stays a no-op success.
    #[test]
    fn double_close_is_noop_success() {
        let path = std::env::temp_dir().join(format!("catnip_io_dbl_{}.txt", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        let handle = alloc_file(FileHandle {
            path: path.to_string_lossy().into_owned(),
            mode: "w".to_string(),
            state: FileState::Writer(BufWriter::new(file)),
        });

        let first = unsafe { plugin_method(handle, c"close".as_ptr(), std::ptr::null(), 0) };
        let second = unsafe { plugin_method(handle, c"close".as_ptr(), std::ptr::null(), 0) };
        assert_eq!(first.error_code, 0);
        assert_eq!(second.error_code, 0, "double close stays a success");

        let _ = std::fs::remove_file(&path);
    }

    // The fix: a flush failure at close() must surface, not be swallowed.
    // Force a real flush error by wrapping a read-only file as a Writer -- the
    // byte buffers fine, but the deferred flush write() hits EBADF on the
    // read-only fd. With the old `let _ = w.flush()` this returned nil.
    #[test]
    fn close_reports_flush_error() {
        let path = std::env::temp_dir().join(format!("catnip_io_ro_{}.txt", std::process::id()));
        std::fs::write(&path, b"").unwrap();
        let read_only = std::fs::File::open(&path).unwrap(); // O_RDONLY
        let handle = alloc_file(FileHandle {
            path: path.to_string_lossy().into_owned(),
            mode: "w".to_string(),
            state: FileState::Writer(BufWriter::new(read_only)),
        });

        // Buffered; the write to the read-only fd is deferred until flush.
        with_file(handle, |fh| {
            if let FileState::Writer(ref mut w) = fh.state {
                let _ = w.write_all(b"x");
            }
        });

        let res = unsafe { plugin_method(handle, c"close".as_ptr(), std::ptr::null(), 0) };
        assert_ne!(res.error_code, 0, "a flush error at close must surface");

        let _ = std::fs::remove_file(&path);
    }
}
