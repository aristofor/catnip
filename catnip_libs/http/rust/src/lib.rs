// FILE: catnip_libs/http/rust/src/lib.rs
//! Catnip `http` stdlib plugin (native ABI v2).
//!
//! Exports: PROTOCOL, VERSION, Server (constructor), Request (type marker), serve.
//! Server() returns a PluginObject handle. Server.recv() returns a Request handle.

use std::cell::RefCell;
use std::ffi::{CStr, c_char};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use tiny_http::{Response, Server as TinyServer};

use catnip_vm::Value;
use catnip_vm::plugin::{
    PLUGIN_ABI_MAGIC, PLUGIN_ABI_VERSION, PLUGIN_RESULT_OBJECT, PluginAttr, PluginCallFn, PluginDescriptor,
    PluginDropFn, PluginGetAttrFn, PluginMethodFn, PluginResult,
};

// ---------------------------------------------------------------------------
// Static data
// ---------------------------------------------------------------------------

static MODULE_NAME: &[u8] = b"http\0";
static MODULE_VERSION: &[u8] = b"0.1.0\0";

static FN_NAMES: &[&[u8]] = &[b"Server\0", b"serve\0"];

// ---------------------------------------------------------------------------
// Content-type detection
// ---------------------------------------------------------------------------

fn detect_content_type(body: &str) -> &'static str {
    let trimmed = body.trim_start();
    if trimmed.starts_with("<!") || trimmed.starts_with("<html") || trimmed.starts_with("<HTML") {
        "text/html"
    } else if trimmed.starts_with("<svg") {
        "image/svg+xml"
    } else {
        "text/plain"
    }
}

// ---------------------------------------------------------------------------
// Object storage (thread-local)
// ---------------------------------------------------------------------------

enum HttpObject {
    Server(Arc<TinyServer>),
    Request(RefCell<Option<tiny_http::Request>>),
}

thread_local! {
    static OBJECTS: RefCell<Vec<Option<HttpObject>>> = RefCell::new(Vec::new());
}

fn alloc_object(obj: HttpObject) -> u64 {
    OBJECTS.with(|objects| {
        let mut objects = objects.borrow_mut();
        for (i, slot) in objects.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(obj);
                return i as u64;
            }
        }
        let idx = objects.len();
        objects.push(Some(obj));
        idx as u64
    })
}

fn with_object<T>(handle: u64, f: impl FnOnce(&HttpObject) -> T) -> Option<T> {
    OBJECTS.with(|objects| {
        let objects = objects.borrow();
        objects.get(handle as usize)?.as_ref().map(f)
    })
}

fn free_object(handle: u64) {
    OBJECTS.with(|objects| {
        let mut objects = objects.borrow_mut();
        if let Some(slot) = objects.get_mut(handle as usize) {
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
        b"Server" => {
            let addr = match args_slice.first().and_then(|&r| extract_str(r)) {
                Some(a) => a,
                None => return err(b"Server() requires an address string\0"),
            };
            match TinyServer::http(&addr) {
                Ok(server) => ok_object(alloc_object(HttpObject::Server(Arc::new(server)))),
                Err(e) => {
                    let msg = format!("http.Server: {}\0", e);
                    let ptr = msg.as_ptr() as *const c_char;
                    std::mem::forget(msg);
                    PluginResult {
                        value: 0,
                        error_code: 1,
                        flags: 0,
                        error_message: ptr,
                    }
                }
            }
        }
        b"serve" => {
            let body = match args_slice.first().and_then(|&r| extract_str(r)) {
                Some(b) => b,
                None => return err(b"serve() requires a body string\0"),
            };
            let port = args_slice
                .get(1)
                .and_then(|&r| Value::from_raw(r).as_int())
                .unwrap_or(0) as u16;
            let content_type = args_slice.get(2).and_then(|&r| extract_str(r));
            let open_browser = args_slice
                .get(3)
                .map(|&r| Value::from_raw(r).is_truthy())
                .unwrap_or(true);

            let addr = format!("127.0.0.1:{}", port);
            let server = match TinyServer::http(&addr) {
                Ok(s) => s,
                Err(e) => {
                    let msg = format!("http.serve: {}\0", e);
                    let ptr = msg.as_ptr() as *const c_char;
                    std::mem::forget(msg);
                    return PluginResult {
                        value: 0,
                        error_code: 1,
                        flags: 0,
                        error_message: ptr,
                    };
                }
            };

            let actual_addr = server.server_addr().to_string();
            if open_browser {
                let _ = open::that(format!("http://{}", actual_addr));
            }

            let ct = content_type.as_deref().unwrap_or_else(|| detect_content_type(&body));
            match server.recv() {
                Ok(req) => {
                    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], ct.as_bytes()).unwrap();
                    let resp = Response::from_string(&body).with_status_code(200).with_header(header);
                    let _ = req.respond(resp);
                }
                Err(_) => {}
            }
            ok_nil()
        }
        _ => err(b"unknown function\0"),
    }
}

// ---------------------------------------------------------------------------
// Object method dispatch
// ---------------------------------------------------------------------------

unsafe extern "C" fn plugin_method(handle: u64, method: *const c_char, args: *const u64, argc: usize) -> PluginResult {
    let method = unsafe { CStr::from_ptr(method) }.to_bytes();

    let is_server = with_object(handle, |obj| matches!(obj, HttpObject::Server(_))).unwrap_or(false);

    if is_server {
        return match method {
            b"recv" => {
                let result = with_object(handle, |obj| {
                    if let HttpObject::Server(s) = obj {
                        s.recv().ok()
                    } else {
                        None
                    }
                })
                .flatten();
                match result {
                    Some(req) => ok_object(alloc_object(HttpObject::Request(RefCell::new(Some(req))))),
                    None => ok_nil(),
                }
            }
            b"try_recv" => {
                let result = with_object(handle, |obj| {
                    if let HttpObject::Server(s) = obj {
                        s.try_recv().ok().flatten()
                    } else {
                        None
                    }
                })
                .flatten();
                match result {
                    Some(req) => ok_object(alloc_object(HttpObject::Request(RefCell::new(Some(req))))),
                    None => ok_nil(),
                }
            }
            b"recv_timeout" => {
                let secs = if argc > 0 {
                    let v = Value::from_raw(unsafe { *args });
                    v.as_float().or_else(|| v.as_int().map(|i| i as f64)).unwrap_or(1.0)
                } else {
                    1.0
                };
                let result = with_object(handle, |obj| {
                    if let HttpObject::Server(s) = obj {
                        s.recv_timeout(Duration::from_secs_f64(secs.max(0.0))).ok().flatten()
                    } else {
                        None
                    }
                })
                .flatten();
                match result {
                    Some(req) => ok_object(alloc_object(HttpObject::Request(RefCell::new(Some(req))))),
                    None => ok_nil(),
                }
            }
            b"close" => {
                with_object(handle, |obj| {
                    if let HttpObject::Server(s) = obj {
                        s.unblock();
                    }
                });
                ok_nil()
            }
            _ => err(b"unknown Server method\0"),
        };
    }

    // Request methods
    match method {
        b"body" => {
            let result = OBJECTS.with(|objects| {
                let objects = objects.borrow();
                let obj = objects.get(handle as usize)?.as_ref()?;
                if let HttpObject::Request(cell) = obj {
                    let mut borrow = cell.borrow_mut();
                    let req = borrow.as_mut()?;
                    let mut buf = String::new();
                    req.as_reader().read_to_string(&mut buf).ok()?;
                    Some(buf)
                } else {
                    None
                }
            });
            match result {
                Some(s) => ok_val(Value::from_string(s)),
                None => err(b"request already consumed or invalid handle\0"),
            }
        }
        b"respond" => {
            let args_slice = if argc > 0 {
                unsafe { std::slice::from_raw_parts(args, argc) }
            } else {
                return err(b"respond() requires a body argument\0");
            };
            let body = match extract_str(args_slice[0]) {
                Some(b) => b,
                None => return err(b"respond() body must be a string\0"),
            };
            let status = args_slice
                .get(1)
                .and_then(|&r| Value::from_raw(r).as_int())
                .unwrap_or(200) as u16;
            let ct = args_slice
                .get(2)
                .and_then(|&r| extract_str(r))
                .unwrap_or_else(|| "text/plain".to_string());

            let result = OBJECTS.with(|objects| {
                let objects = objects.borrow();
                let obj = objects.get(handle as usize)?.as_ref()?;
                if let HttpObject::Request(cell) = obj {
                    let req = cell.borrow_mut().take()?;
                    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], ct.as_bytes()).ok()?;
                    let resp = Response::from_string(&body)
                        .with_status_code(status)
                        .with_header(header);
                    req.respond(resp).ok()
                } else {
                    None
                }
            });
            match result {
                Some(()) => ok_nil(),
                None => err(b"request already consumed or invalid handle\0"),
            }
        }
        _ => err(b"unknown Request method\0"),
    }
}

// ---------------------------------------------------------------------------
// Object attribute access
// ---------------------------------------------------------------------------

unsafe extern "C" fn plugin_getattr(handle: u64, attr: *const c_char) -> PluginResult {
    let attr = unsafe { CStr::from_ptr(attr) }.to_bytes();

    let is_server = with_object(handle, |obj| matches!(obj, HttpObject::Server(_))).unwrap_or(false);

    if is_server {
        return match attr {
            b"addr" => {
                match with_object(handle, |obj| {
                    if let HttpObject::Server(s) = obj {
                        Some(s.server_addr().to_string())
                    } else {
                        None
                    }
                })
                .flatten()
                {
                    Some(a) => ok_val(Value::from_string(a)),
                    None => err(b"invalid handle\0"),
                }
            }
            _ => err(b"unknown Server attribute\0"),
        };
    }

    // Request attributes
    match attr {
        b"url" => {
            match with_object(handle, |obj| {
                if let HttpObject::Request(cell) = obj {
                    cell.borrow().as_ref().map(|r| r.url().to_string())
                } else {
                    None
                }
            })
            .flatten()
            {
                Some(u) => ok_val(Value::from_string(u)),
                None => err(b"request consumed or invalid\0"),
            }
        }
        b"method" => {
            match with_object(handle, |obj| {
                if let HttpObject::Request(cell) = obj {
                    cell.borrow().as_ref().map(|r| r.method().to_string())
                } else {
                    None
                }
            })
            .flatten()
            {
                Some(m) => ok_val(Value::from_string(m)),
                None => err(b"request consumed or invalid\0"),
            }
        }
        b"headers" => {
            match with_object(handle, |obj| {
                if let HttpObject::Request(cell) = obj {
                    cell.borrow().as_ref().map(|r| {
                        r.headers()
                            .iter()
                            .map(|h| (h.field.to_string(), h.value.to_string()))
                            .collect::<Vec<_>>()
                    })
                } else {
                    None
                }
            })
            .flatten()
            {
                Some(hdrs) => {
                    let mut map = indexmap::IndexMap::new();
                    for (k, v) in hdrs {
                        map.insert(
                            catnip_vm::collections::ValueKey::Str(std::sync::Arc::new(
                                catnip_vm::value::NativeString::new(k),
                            )),
                            Value::from_string(v),
                        );
                    }
                    ok_val(Value::from_dict(map))
                }
                None => err(b"request consumed or invalid\0"),
            }
        }
        _ => err(b"unknown Request attribute\0"),
    }
}

// ---------------------------------------------------------------------------
// Object drop
// ---------------------------------------------------------------------------

unsafe extern "C" fn plugin_drop(handle: u64) {
    free_object(handle);
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

static PROTOCOL_ATTR_NAME: &[u8] = b"PROTOCOL\0";
static VERSION_ATTR_NAME: &[u8] = b"VERSION\0";
static REQUEST_ATTR_NAME: &[u8] = b"Request\0";

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
            PluginAttr {
                name: REQUEST_ATTR_NAME.as_ptr() as *const c_char,
                value: Value::from_str("http.Request").bits(),
            },
        ];

        let fn_ptrs: Vec<*const c_char> = FN_NAMES.iter().map(|n| n.as_ptr() as *const c_char).collect();

        let desc = PluginDescriptor {
            abi_magic: PLUGIN_ABI_MAGIC,
            abi_version: PLUGIN_ABI_VERSION,
            module_name: MODULE_NAME.as_ptr() as *const c_char,
            module_version: MODULE_VERSION.as_ptr() as *const c_char,
            num_attrs: 3,
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
