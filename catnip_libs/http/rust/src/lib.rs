// FILE: catnip_libs/http/rust/src/lib.rs
//! Catnip `http` stdlib plugin (native ABI v2).
//!
//! Exports: PROTOCOL, VERSION, Server, Request, Response, serve, get, post, put, delete.
//! Server() returns a PluginObject handle. Server.recv() returns a Request handle.
//! get/post/put/delete return a Response handle.

use std::cell::RefCell;
use std::ffi::{CStr, c_char};
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use tiny_http::{Response, Server as TinyServer};

use catnip_vm::Value;
use catnip_vm::collections::{NativeDict, ValueKey};
use catnip_vm::plugin::{
    PLUGIN_ABI_MAGIC, PLUGIN_ABI_VERSION, PLUGIN_RESULT_HOSTVALUE, PLUGIN_RESULT_OBJECT, PluginAttr, PluginCallFn,
    PluginDescriptor, PluginDropFn, PluginGetAttrFn, PluginHasMemberFn, PluginHostApi, PluginMethodFn, PluginResult,
};
use catnip_vm::value::NativeString;

// ABI v4: host value-builder API, stored at init.
static HOST_API: AtomicPtr<PluginHostApi> = AtomicPtr::new(std::ptr::null_mut());

#[inline]
fn host() -> &'static PluginHostApi {
    let p = HOST_API.load(Ordering::Acquire);
    if p.is_null() {
        // Not loaded by a host (unit tests calling functions directly): the
        // plugin is its own host, so build in this crate's catnip_vm heap.
        &catnip_vm::plugin::PLUGIN_HOST_API
    } else {
        // SAFETY: set by catnip_plugin_init before any call; host-owned, 'static.
        unsafe { &*p }
    }
}

// ---------------------------------------------------------------------------
// Static data
// ---------------------------------------------------------------------------

static MODULE_NAME: &[u8] = b"http\0";
static MODULE_VERSION: &[u8] = b"0.2.0\0";

static FN_NAMES: &[&[u8]] = &[
    b"Server\0",
    b"serve\0",
    b"get\0",
    b"post\0",
    b"put\0",
    b"delete\0",
    b"request\0",
    b"basic_auth\0",
    b"bearer\0",
];

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

#[derive(Debug)]
struct ResponseData {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

/// Background accept loop state attached to a Server after `start()`.
struct AsyncServerState {
    receiver: mpsc::Receiver<tiny_http::Request>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

struct ServerObject {
    server: Arc<TinyServer>,
    /// `None` until `start()`, then `Some` until the server is closed.
    async_state: RefCell<Option<AsyncServerState>>,
}

impl ServerObject {
    /// Stop the async accept loop (if any) and unblock pending recv() calls.
    /// Idempotent: safe to call from both `close()` and `Drop`.
    fn shutdown(&self) {
        self.server.unblock();
        if let Some(mut st) = self.async_state.borrow_mut().take() {
            if let Some(handle) = st.thread_handle.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for ServerObject {
    fn drop(&mut self) {
        // Ensure the accept thread (and its Arc<TinyServer> clone) is gone
        // even if Catnip code dropped the Server without calling close().
        self.shutdown();
    }
}

enum HttpObject {
    Server(ServerObject),
    Request(RefCell<Option<tiny_http::Request>>),
    Response(ResponseData),
    Chunked(RefCell<ChunkedWriter>),
}

/// Raw chunked-encoding writer attached to a Request. Writes the HTTP/1.1
/// status line + headers lazily on first chunk, then framed `<hex>\r\n<data>\r\n`
/// chunks until `end()` (or Drop) emits the terminating `0\r\n\r\n`.
struct ChunkedWriter {
    writer: Option<Box<dyn Write + Send>>,
    headers_sent: bool,
    status: u16,
    content_type: String,
}

impl ChunkedWriter {
    /// Try to start a chunked response from a Request. Refuses combinations
    /// that would produce a protocol-invalid wire format (HTTP/1.0 has no
    /// chunked encoding, HEAD must not carry a body, 1xx/204/304 must not
    /// carry a body either). For those cases the caller should fall back to
    /// `Request.respond()`.
    fn try_from_request(req: tiny_http::Request, status: u16, content_type: String) -> Result<Self, String> {
        if !status_allows_body(status) {
            return Err(format!(
                "status {} must not have a body (use respond() instead)",
                status
            ));
        }
        let version = req.http_version().clone();
        if (version.0, version.1) < (1, 1) {
            return Err(format!(
                "chunked encoding requires HTTP/1.1+, got HTTP/{}.{}",
                version.0, version.1
            ));
        }
        if req.method().as_str().eq_ignore_ascii_case("HEAD") {
            return Err("HEAD requests must not receive a body; use respond() with an empty body".to_string());
        }
        Ok(Self {
            writer: Some(req.into_writer()),
            headers_sent: false,
            status,
            content_type,
        })
    }

    fn write_headers(w: &mut dyn Write, status: u16, content_type: &str) -> io::Result<()> {
        let reason = match status {
            200 => "OK",
            201 => "Created",
            202 => "Accepted",
            204 => "No Content",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            500 => "Internal Server Error",
            _ => "",
        };
        write!(w, "HTTP/1.1 {} {}\r\n", status, reason)?;
        write!(w, "Content-Type: {}\r\n", content_type)?;
        write!(w, "Transfer-Encoding: chunked\r\n")?;
        write!(w, "Cache-Control: no-cache\r\n")?;
        write!(w, "\r\n")?;
        Ok(())
    }

    fn send_chunk(&mut self, data: &[u8]) -> io::Result<()> {
        let w = self
            .writer
            .as_mut()
            .ok_or_else(|| io::Error::other("chunked stream closed"))?;
        if !self.headers_sent {
            Self::write_headers(w.as_mut(), self.status, &self.content_type)?;
            self.headers_sent = true;
        }
        if data.is_empty() {
            // Skip empty chunks: a `0\r\n\r\n` chunk would prematurely terminate.
            return Ok(());
        }
        write!(w, "{:x}\r\n", data.len())?;
        w.write_all(data)?;
        write!(w, "\r\n")?;
        w.flush()
    }

    fn end(&mut self) -> io::Result<()> {
        if let Some(mut w) = self.writer.take() {
            if !self.headers_sent {
                Self::write_headers(w.as_mut(), self.status, &self.content_type)?;
            }
            write!(w, "0\r\n\r\n")?;
            w.flush()?;
        }
        Ok(())
    }
}

impl Drop for ChunkedWriter {
    fn drop(&mut self) {
        // Ensure the terminating chunk is sent even if user code forgot end().
        let _ = self.end();
    }
}

/// Whether a response with this status is allowed to carry a body per RFC 7230 §3.3.
/// 1xx, 204 and 304 must never have a body and therefore cannot be chunked.
fn status_allows_body(status: u16) -> bool {
    !matches!(status, 100..=199 | 204 | 304)
}

// ---------------------------------------------------------------------------
// Multipart parser (server-side: parse incoming multipart/form-data bodies)
// ---------------------------------------------------------------------------

struct MultipartPart {
    name: String,
    filename: Option<String>,
    content_type: Option<String>,
    data: Vec<u8>,
}

fn find_subseq(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Extract the boundary token from a `Content-Type: multipart/form-data; boundary=...` header.
fn extract_boundary(content_type: &str) -> Option<String> {
    for token in content_type.split(';') {
        let token = token.trim();
        if let Some(rest) = token.strip_prefix("boundary=") {
            return Some(rest.trim_matches('"').to_string());
        }
    }
    None
}

/// Find the start of a delimiter line `--<boundary>` (either at body start or
/// after a `\r\n`) that is then closed by either `\r\n` (regular part) or `--`
/// (final terminator). Returns the byte position of the leading `--`.
fn find_delimiter(body: &[u8], dash_boundary: &[u8], from: usize) -> Option<usize> {
    let mut cursor = from;
    while cursor + dash_boundary.len() <= body.len() {
        let p = find_subseq(&body[cursor..], dash_boundary)?;
        let abs = cursor + p;
        // Must start at body[0] or be preceded by \r\n.
        let line_anchored = abs == 0 || body.get(abs - 2..abs) == Some(&b"\r\n"[..]);
        // Must be followed by \r\n (regular) or -- (final terminator).
        let after = abs + dash_boundary.len();
        let suffix = body.get(after..after + 2);
        let suffix_ok = suffix == Some(&b"\r\n"[..]) || suffix == Some(&b"--"[..]);
        if line_anchored && suffix_ok {
            return Some(abs);
        }
        cursor = abs + dash_boundary.len();
    }
    None
}

/// Lower-case ASCII prefix match: returns the suffix after a `<name>:` header,
/// trimmed, when `name` matches case-insensitively. Header names are restricted
/// to ASCII per RFC 7230, so byte-level lowering is sound.
fn header_value<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let colon = line.find(':')?;
    if line[..colon].trim().eq_ignore_ascii_case(name) {
        Some(line[colon + 1..].trim_start())
    } else {
        None
    }
}

fn parse_multipart(body: &[u8], boundary: &str) -> Vec<MultipartPart> {
    let mut parts = Vec::new();
    let dash_boundary = format!("--{}", boundary);
    let dash_bytes = dash_boundary.as_bytes();

    // Collect anchored delimiter positions until we hit the final terminator.
    let mut positions = Vec::new();
    let mut from = 0;
    while let Some(pos) = find_delimiter(body, dash_bytes, from) {
        positions.push(pos);
        let after = pos + dash_bytes.len();
        if body.get(after..after + 2) == Some(&b"--"[..]) {
            break; // final boundary; subsequent bytes are epilogue
        }
        from = after + 2; // skip past \r\n
    }

    for w in positions.windows(2) {
        // Body of a part starts after "--<boundary>\r\n" and ends right before
        // the "\r\n--<boundary>" that introduces the next delimiter.
        let part_start = w[0] + dash_bytes.len() + 2; // skip \r\n after delimiter
        let part_end = w[1].saturating_sub(2); // strip the \r\n preceding next delimiter
        if part_start >= part_end {
            continue;
        }
        let part_bytes = &body[part_start..part_end];

        let Some(header_end) = find_subseq(part_bytes, b"\r\n\r\n") else {
            continue;
        };
        let headers_raw = &part_bytes[..header_end];
        let data = &part_bytes[header_end + 4..];

        let mut name = None;
        let mut filename = None;
        let mut content_type = None;
        for line_raw in headers_raw.split(|&b| b == b'\n') {
            let Ok(line) = std::str::from_utf8(line_raw) else {
                continue;
            };
            let line = line.trim();
            if let Some(rest) = header_value(line, "Content-Disposition") {
                for token in rest.split(';') {
                    let token = token.trim();
                    let Some(eq) = token.find('=') else { continue };
                    let param = token[..eq].trim();
                    let value = token[eq + 1..].trim().trim_matches('"');
                    if param.eq_ignore_ascii_case("name") {
                        name = Some(value.to_string());
                    } else if param.eq_ignore_ascii_case("filename") {
                        filename = Some(value.to_string());
                    }
                }
            } else if let Some(rest) = header_value(line, "Content-Type") {
                content_type = Some(rest.to_string());
            }
        }

        if let Some(name) = name {
            parts.push(MultipartPart {
                name,
                filename,
                content_type,
                data: data.to_vec(),
            });
        }
    }
    parts
}

thread_local! {
    static OBJECTS: RefCell<Vec<Option<HttpObject>>> = const { RefCell::new(Vec::new()) };
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

// ABI v4: rebuild a plugin-side Value into the host heap, so the host never
// dereferences a plugin-owned Arc. The plugin reads its own Arcs (same process,
// just built) and hands back host tokens. Cloned sub-items are decref'd, exactly
// like Value::display_string, so the plugin-side tree stays refcount-balanced.
fn to_host_token(v: &Value) -> u64 {
    let h = host();
    if v.is_native_str() {
        let s = unsafe { v.as_native_str_ref().unwrap() };
        unsafe { (h.make_string)(s.as_ptr(), s.len()) }
    } else if v.is_native_bytes() {
        let b = unsafe { v.as_native_bytes_ref().unwrap() }.as_bytes();
        unsafe { (h.make_bytes)(b.as_ptr(), b.len()) }
    } else if v.is_native_list() {
        let items = unsafe { v.as_native_list_ref().unwrap() }.as_slice_cloned();
        let tokens: Vec<u64> = items
            .iter()
            .map(|it| {
                let t = to_host_token(it);
                it.decref();
                t
            })
            .collect();
        unsafe { (h.make_list)(tokens.as_ptr(), tokens.len()) }
    } else if v.is_native_dict() {
        let dict = unsafe { v.as_native_dict_ref().unwrap() };
        let keys = dict.keys_cloned();
        let mut ktoks = Vec::with_capacity(keys.len());
        let mut vtoks = Vec::with_capacity(keys.len());
        for k in &keys {
            let kv = k.to_value();
            let vv = dict.get_item(k).unwrap_or(Value::NIL);
            ktoks.push(to_host_token(&kv));
            vtoks.push(to_host_token(&vv));
            kv.decref();
            vv.decref();
        }
        unsafe { (h.make_dict)(ktoks.as_ptr(), vtoks.as_ptr(), ktoks.len()) }
    } else if v.is_bigint() {
        let s = unsafe { v.as_bigint_ref().unwrap() }.to_string();
        unsafe { (h.make_bigint)(s.as_ptr(), s.len()) }
    } else {
        // Scalar (int/float/bool/nil/symbol) -- crosses directly.
        v.bits()
    }
}

fn ok_val(v: Value) -> PluginResult {
    let value = to_host_token(&v);
    v.decref();
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

fn err_owned(msg: String) -> PluginResult {
    let cstr = format!("{}\0", msg);
    let ptr = cstr.as_ptr() as *const c_char;
    std::mem::forget(cstr);
    PluginResult {
        value: 0,
        error_code: 1,
        flags: 0,
        error_message: ptr,
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
// Async Server (channel-based accept loop)
// ---------------------------------------------------------------------------

/// Start a background accept loop that pushes incoming requests into a channel.
/// Idempotent: calling `start()` twice is a no-op.
fn start_async_accept(s: &ServerObject) -> Result<(), String> {
    let mut state = s.async_state.borrow_mut();
    if state.is_some() {
        return Ok(()); // already started
    }
    let (sender, receiver) = mpsc::channel::<tiny_http::Request>();
    let server = Arc::clone(&s.server);
    let handle = thread::spawn(move || {
        // server.recv() blocks until a connection arrives or unblock() is called.
        // unblock() makes recv() return Err -> loop terminates -> thread exits.
        while let Ok(req) = server.recv() {
            if sender.send(req).is_err() {
                break; // receiver dropped (server object freed)
            }
        }
    });
    *state = Some(AsyncServerState {
        receiver,
        thread_handle: Some(handle),
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// HTTP client
// ---------------------------------------------------------------------------

static CLIENT: OnceLock<ureq::Agent> = OnceLock::new();

/// Hard cap on response body size when buffering into `String`.
/// Configurable per-request via `http.request(method, url, { max_body })`
/// (étape 2). For now, applied uniformly to all client verbs.
const MAX_BODY_SIZE: u64 = 32 * 1024 * 1024;

fn client() -> &'static ureq::Agent {
    CLIENT.get_or_init(|| ureq::Agent::config_builder().http_status_as_error(false).build().into())
}

fn http_request(method: &str, url: &str, body: Option<&str>) -> Result<ResponseData, String> {
    http_request_full(method, url, &[], body, None, MAX_BODY_SIZE)
}

fn http_request_full(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&str>,
    timeout_secs: Option<f64>,
    max_body: u64,
) -> Result<ResponseData, String> {
    let agent = client();
    let method_upper = method.to_ascii_uppercase();

    let mut builder = ureq::http::Request::builder().method(method_upper.as_str()).uri(url);
    for (k, v) in headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    let body_bytes: Vec<u8> = body.map(|s| s.as_bytes().to_vec()).unwrap_or_default();
    let mut request = builder
        .body(body_bytes)
        .map_err(|e| format!("http.request: invalid request: {}", e))?;

    if let Some(t) = timeout_secs {
        request = agent
            .configure_request(request)
            .timeout_global(Some(Duration::from_secs_f64(t.max(0.0))))
            .build();
    }

    let mut resp = agent
        .run(request)
        .map_err(|e| format!("http.{}: {}", method.to_lowercase(), e))?;
    let status = resp.status().as_u16();
    let resp_headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = resp
        .body_mut()
        .with_config()
        .limit(max_body)
        .lossy_utf8(true)
        .read_to_string()
        .map_err(|e| format!("http.{}: failed to read body: {}", method.to_lowercase(), e))?;

    Ok(ResponseData {
        status,
        headers: resp_headers,
        body,
    })
}

// ---------------------------------------------------------------------------
// Options dict helpers
// ---------------------------------------------------------------------------

fn str_key(name: &str) -> ValueKey {
    ValueKey::Str(Arc::new(NativeString::new(name.to_string())))
}

fn opts_get_str(dict: &NativeDict, key: &str) -> Option<String> {
    // NativeDict::get_default() increments refcount; release it after we copy the data out.
    let v = dict.get_default(&str_key(key), Value::NIL);
    let result = if v.is_native_str() {
        Some(unsafe { v.as_native_str_ref().unwrap() }.to_string())
    } else {
        None
    };
    v.decref();
    result
}

fn opts_get_float(dict: &NativeDict, key: &str) -> Option<f64> {
    let v = dict.get_default(&str_key(key), Value::NIL);
    let result = if v.is_nil() {
        None
    } else {
        v.as_float().or_else(|| v.as_int().map(|i| i as f64))
    };
    v.decref();
    result
}

fn opts_get_int(dict: &NativeDict, key: &str) -> Option<i64> {
    let v = dict.get_default(&str_key(key), Value::NIL);
    let result = v.as_int();
    v.decref();
    result
}

fn opts_get_headers(dict: &NativeDict, key: &str) -> Vec<(String, String)> {
    let v = dict.get_default(&str_key(key), Value::NIL);
    if !v.is_native_dict() {
        v.decref();
        return Vec::new();
    }
    let headers = unsafe { v.as_native_dict_ref().unwrap() };
    let mut out = Vec::new();
    for vk in headers.keys_cloned() {
        if let ValueKey::Str(k) = &vk {
            if let Ok(val) = headers.get_item(&vk) {
                if val.is_native_str() {
                    let s = unsafe { val.as_native_str_ref().unwrap() };
                    out.push((k.as_str().to_string(), s.to_string()));
                }
                val.decref();
            }
        }
    }
    v.decref();
    out
}

// ---------------------------------------------------------------------------
// JSON conversion (serde_json::Value -> catnip Value)
// ---------------------------------------------------------------------------

fn json_to_value(j: &serde_json::Value) -> Value {
    match j {
        serde_json::Value::Null => Value::NIL,
        serde_json::Value::Bool(b) => Value::from_bool(*b),
        serde_json::Value::Number(n) => {
            // i64 path: promotes to BigInt automatically if outside SmallInt range.
            if let Some(i) = n.as_i64() {
                Value::from_i64(i)
            } else if let Some(u) = n.as_u64() {
                // u64 > i64::MAX: must be stored as BigInt to preserve precision.
                Value::from_bigint(rug::Integer::from(u))
            } else if let Some(f) = n.as_f64() {
                Value::from_float(f)
            } else {
                Value::NIL
            }
        }
        serde_json::Value::String(s) => Value::from_string(s.clone()),
        serde_json::Value::Array(arr) => {
            let items: Vec<Value> = arr.iter().map(json_to_value).collect();
            Value::from_list(items)
        }
        serde_json::Value::Object(obj) => {
            let mut map = indexmap::IndexMap::new();
            for (k, v) in obj {
                map.insert(ValueKey::Str(Arc::new(NativeString::new(k.clone()))), json_to_value(v));
            }
            Value::from_dict(map)
        }
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
                Ok(server) => ok_object(alloc_object(HttpObject::Server(ServerObject {
                    server: Arc::new(server),
                    async_state: RefCell::new(None),
                }))),
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
            if let Ok(req) = server.recv() {
                let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], ct.as_bytes()).unwrap();
                let resp = Response::from_string(&body).with_status_code(200).with_header(header);
                let _ = req.respond(resp);
            }
            ok_nil()
        }
        b"get" | b"delete" | b"post" | b"put" => {
            let url = match args_slice.first().and_then(|&r| extract_str(r)) {
                Some(u) => u,
                None => return err(b"http verb requires a url string\0"),
            };
            let method = match name {
                b"get" => "GET",
                b"delete" => "DELETE",
                b"post" => "POST",
                b"put" => "PUT",
                _ => unreachable!(),
            };
            let body = args_slice.get(1).and_then(|&r| extract_str(r));
            match http_request(method, &url, body.as_deref()) {
                Ok(data) => ok_object(alloc_object(HttpObject::Response(data))),
                Err(msg) => err_owned(msg),
            }
        }
        b"request" => {
            let method = match args_slice.first().and_then(|&r| extract_str(r)) {
                Some(m) => m,
                None => return err(b"request() requires a method string\0"),
            };
            let url = match args_slice.get(1).and_then(|&r| extract_str(r)) {
                Some(u) => u,
                None => return err(b"request() requires a url string\0"),
            };
            let (headers, body, timeout, max_body) = match args_slice.get(2) {
                Some(&raw) => {
                    let v = Value::from_raw(raw);
                    if v.is_native_dict() {
                        let opts = unsafe { v.as_native_dict_ref().unwrap() };
                        (
                            opts_get_headers(opts, "headers"),
                            opts_get_str(opts, "body"),
                            opts_get_float(opts, "timeout"),
                            opts_get_int(opts, "max_body")
                                .filter(|&n| n > 0)
                                .map(|n| n as u64)
                                .unwrap_or(MAX_BODY_SIZE),
                        )
                    } else if v.is_nil() {
                        (Vec::new(), None, None, MAX_BODY_SIZE)
                    } else {
                        return err(b"request() opts must be a dict or nil\0");
                    }
                }
                None => (Vec::new(), None, None, MAX_BODY_SIZE),
            };
            match http_request_full(&method, &url, &headers, body.as_deref(), timeout, max_body) {
                Ok(data) => ok_object(alloc_object(HttpObject::Response(data))),
                Err(msg) => err_owned(msg),
            }
        }
        b"basic_auth" => {
            let user = match args_slice.first().and_then(|&r| extract_str(r)) {
                Some(u) => u,
                None => return err(b"basic_auth() requires user string\0"),
            };
            let pass = match args_slice.get(1).and_then(|&r| extract_str(r)) {
                Some(p) => p,
                None => return err(b"basic_auth() requires password string\0"),
            };
            use base64::Engine;
            let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", user, pass));
            ok_val(Value::from_string(format!("Basic {}", encoded)))
        }
        b"bearer" => {
            let token = match args_slice.first().and_then(|&r| extract_str(r)) {
                Some(t) => t,
                None => return err(b"bearer() requires a token string\0"),
            };
            ok_val(Value::from_string(format!("Bearer {}", token)))
        }
        _ => err(b"unknown function\0"),
    }
}

// ---------------------------------------------------------------------------
// Object method dispatch
// ---------------------------------------------------------------------------

unsafe extern "C" fn plugin_method(handle: u64, method: *const c_char, args: *const u64, argc: usize) -> PluginResult {
    let method = unsafe { CStr::from_ptr(method) }.to_bytes();

    let kind = with_object(handle, |obj| match obj {
        HttpObject::Server(_) => 0u8,
        HttpObject::Request(_) => 1u8,
        HttpObject::Response(_) => 2u8,
        HttpObject::Chunked(_) => 3u8,
    });

    let is_server = matches!(kind, Some(0));
    let is_response = matches!(kind, Some(2));
    let is_chunked = matches!(kind, Some(3));

    if is_chunked {
        let args_slice = if argc > 0 {
            unsafe { std::slice::from_raw_parts(args, argc) }
        } else {
            &[]
        };
        return match method {
            b"send_chunk" => {
                let data = match args_slice.first().and_then(|&r| extract_str(r)) {
                    Some(d) => d,
                    None => return err(b"send_chunk() requires a string\0"),
                };
                let result = OBJECTS.with(|objects| {
                    let objects = objects.borrow();
                    let obj = objects.get(handle as usize)?.as_ref()?;
                    if let HttpObject::Chunked(cell) = obj {
                        Some(cell.borrow_mut().send_chunk(data.as_bytes()))
                    } else {
                        None
                    }
                });
                match result {
                    Some(Ok(())) => ok_nil(),
                    Some(Err(e)) => err_owned(format!("send_chunk: {}", e)),
                    None => err(b"invalid chunked handle\0"),
                }
            }
            b"send_event" => {
                // SSE event: optional event_type + required data.
                // send_event(data) -> "data: <data>\n\n"
                // send_event(data, event_type) -> "event: <type>\ndata: <data>\n\n"
                let data = match args_slice.first().and_then(|&r| extract_str(r)) {
                    Some(d) => d,
                    None => return err(b"send_event() requires data string\0"),
                };
                let event_type = args_slice.get(1).and_then(|&r| extract_str(r));
                let mut payload = String::new();
                if let Some(t) = event_type {
                    payload.push_str("event: ");
                    payload.push_str(&t);
                    payload.push('\n');
                }
                for line in data.split('\n') {
                    payload.push_str("data: ");
                    payload.push_str(line);
                    payload.push('\n');
                }
                payload.push('\n');
                let result = OBJECTS.with(|objects| {
                    let objects = objects.borrow();
                    let obj = objects.get(handle as usize)?.as_ref()?;
                    if let HttpObject::Chunked(cell) = obj {
                        Some(cell.borrow_mut().send_chunk(payload.as_bytes()))
                    } else {
                        None
                    }
                });
                match result {
                    Some(Ok(())) => ok_nil(),
                    Some(Err(e)) => err_owned(format!("send_event: {}", e)),
                    None => err(b"invalid chunked handle\0"),
                }
            }
            b"end" => {
                let result = OBJECTS.with(|objects| {
                    let objects = objects.borrow();
                    let obj = objects.get(handle as usize)?.as_ref()?;
                    if let HttpObject::Chunked(cell) = obj {
                        Some(cell.borrow_mut().end())
                    } else {
                        None
                    }
                });
                match result {
                    Some(Ok(())) => ok_nil(),
                    Some(Err(e)) => err_owned(format!("end: {}", e)),
                    None => err(b"invalid chunked handle\0"),
                }
            }
            _ => err(b"unknown Chunked method (send_chunk, send_event, end)\0"),
        };
    }

    if is_response {
        return match method {
            b"json" => {
                let body = with_object(handle, |obj| {
                    if let HttpObject::Response(data) = obj {
                        Some(data.body.clone())
                    } else {
                        None
                    }
                })
                .flatten();
                match body {
                    Some(s) => match serde_json::from_str::<serde_json::Value>(&s) {
                        Ok(json) => ok_val(json_to_value(&json)),
                        Err(e) => err_owned(format!("http.Response.json: {}", e)),
                    },
                    None => err(b"invalid handle\0"),
                }
            }
            _ => err(b"unknown Response method (json)\0"),
        };
    }

    if is_server {
        return match method {
            b"recv" => {
                let result = with_object(handle, |obj| {
                    if let HttpObject::Server(s) = obj {
                        s.server.recv().ok()
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
                        s.server.try_recv().ok().flatten()
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
                        s.server
                            .recv_timeout(Duration::from_secs_f64(secs.max(0.0)))
                            .ok()
                            .flatten()
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
            b"start" => {
                let res = with_object(handle, |obj| {
                    if let HttpObject::Server(s) = obj {
                        Some(start_async_accept(s))
                    } else {
                        None
                    }
                })
                .flatten();
                match res {
                    Some(Ok(())) => ok_nil(),
                    Some(Err(msg)) => err_owned(msg),
                    None => err(b"invalid handle\0"),
                }
            }
            b"recv_async" => {
                let res = with_object(handle, |obj| {
                    if let HttpObject::Server(s) = obj {
                        // None if start() not called yet, otherwise Result<Option<Request>>
                        let state = s.async_state.borrow();
                        state.as_ref().map(|st| st.receiver.try_recv().ok())
                    } else {
                        None
                    }
                });
                match res {
                    Some(Some(Some(req))) => ok_object(alloc_object(HttpObject::Request(RefCell::new(Some(req))))),
                    Some(Some(None)) => ok_nil(), // queue empty
                    Some(None) => err(b"recv_async() requires start() to be called first\0"),
                    None => err(b"invalid handle\0"),
                }
            }
            b"close" => {
                with_object(handle, |obj| {
                    if let HttpObject::Server(s) = obj {
                        s.shutdown();
                    }
                });
                ok_nil()
            }
            _ => err(b"unknown Server method\0"),
        };
    }

    // Request methods
    match method {
        b"multipart" => {
            let parsed = OBJECTS.with(|objects| {
                let objects = objects.borrow();
                let obj = objects.get(handle as usize)?.as_ref()?;
                if let HttpObject::Request(cell) = obj {
                    let mut borrow = cell.borrow_mut();
                    let req = borrow.as_mut()?;
                    let ct = req
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Content-Type"))?
                        .value
                        .to_string();
                    let boundary = extract_boundary(&ct)?;
                    let mut body = Vec::new();
                    std::io::Read::read_to_end(req.as_reader(), &mut body).ok()?;
                    Some(parse_multipart(&body, &boundary))
                } else {
                    None
                }
            });
            match parsed {
                Some(parts) => {
                    let items: Vec<Value> = parts
                        .into_iter()
                        .map(|p| {
                            let mut map = indexmap::IndexMap::new();
                            map.insert(str_key("name"), Value::from_string(p.name));
                            map.insert(
                                str_key("filename"),
                                p.filename.map(Value::from_string).unwrap_or(Value::NIL),
                            );
                            map.insert(
                                str_key("content_type"),
                                p.content_type.map(Value::from_string).unwrap_or(Value::NIL),
                            );
                            map.insert(str_key("data"), Value::from_bytes(p.data));
                            Value::from_dict(map)
                        })
                        .collect();
                    ok_val(Value::from_list(items))
                }
                None => err(b"request consumed, missing Content-Type, or invalid boundary\0"),
            }
        }
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
        b"start_chunked" | b"start_sse" => {
            let args_slice = if argc > 0 {
                unsafe { std::slice::from_raw_parts(args, argc) }
            } else {
                &[]
            };
            // start_chunked(status?, content_type?) ; start_sse() forces SSE defaults.
            let (status, content_type) = if method == b"start_sse" {
                (200u16, "text/event-stream".to_string())
            } else {
                let status = args_slice
                    .first()
                    .and_then(|&r| Value::from_raw(r).as_int())
                    .unwrap_or(200) as u16;
                let ct = args_slice
                    .get(1)
                    .and_then(|&r| extract_str(r))
                    .unwrap_or_else(|| "text/plain".to_string());
                (status, ct)
            };

            let taken = OBJECTS.with(|objects| {
                let objects = objects.borrow();
                let obj = objects.get(handle as usize)?.as_ref()?;
                if let HttpObject::Request(cell) = obj {
                    cell.borrow_mut().take()
                } else {
                    None
                }
            });
            match taken {
                Some(req) => match ChunkedWriter::try_from_request(req, status, content_type) {
                    Ok(writer) => ok_object(alloc_object(HttpObject::Chunked(RefCell::new(writer)))),
                    Err(msg) => err_owned(msg),
                },
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

    let kind = with_object(handle, |obj| match obj {
        HttpObject::Server(_) => 0u8,
        HttpObject::Request(_) => 1u8,
        HttpObject::Response(_) => 2u8,
        HttpObject::Chunked(_) => 3u8,
    });

    let is_server = matches!(kind, Some(0));
    let is_response = matches!(kind, Some(2));

    if is_response {
        return match attr {
            b"status" => {
                let s = with_object(handle, |obj| {
                    if let HttpObject::Response(data) = obj {
                        Some(data.status)
                    } else {
                        None
                    }
                })
                .flatten();
                match s {
                    Some(s) => ok_val(Value::from_int(s as i64)),
                    None => err(b"invalid handle\0"),
                }
            }
            b"body" => {
                let b = with_object(handle, |obj| {
                    if let HttpObject::Response(data) = obj {
                        Some(data.body.clone())
                    } else {
                        None
                    }
                })
                .flatten();
                match b {
                    Some(s) => ok_val(Value::from_string(s)),
                    None => err(b"invalid handle\0"),
                }
            }
            b"headers" => {
                let hdrs = with_object(handle, |obj| {
                    if let HttpObject::Response(data) = obj {
                        Some(data.headers.clone())
                    } else {
                        None
                    }
                })
                .flatten();
                match hdrs {
                    Some(list) => {
                        let mut map = indexmap::IndexMap::new();
                        for (k, v) in list {
                            map.insert(
                                catnip_vm::collections::ValueKey::Str(std::sync::Arc::new(
                                    catnip_vm::value::NativeString::new(k),
                                )),
                                Value::from_string(v),
                            );
                        }
                        ok_val(Value::from_dict(map))
                    }
                    None => err(b"invalid handle\0"),
                }
            }
            _ => err(b"unknown Response attribute (status, headers, body)\0"),
        };
    }

    if is_server {
        return match attr {
            b"addr" => {
                match with_object(handle, |obj| {
                    if let HttpObject::Server(s) = obj {
                        Some(s.server.server_addr().to_string())
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
        b"cookies" => {
            let result = with_object(handle, |obj| {
                if let HttpObject::Request(cell) = obj {
                    cell.borrow().as_ref().map(|r| {
                        r.headers()
                            .iter()
                            .filter(|h| h.field.equiv("Cookie"))
                            .map(|h| h.value.to_string())
                            .collect::<Vec<_>>()
                    })
                } else {
                    None
                }
            })
            .flatten();
            match result {
                Some(headers) => {
                    let mut map = indexmap::IndexMap::new();
                    for header in headers {
                        for pair in header.split(';') {
                            let pair = pair.trim();
                            if let Some((k, v)) = pair.split_once('=') {
                                map.insert(
                                    ValueKey::Str(Arc::new(NativeString::new(k.trim().to_string()))),
                                    Value::from_string(v.trim().to_string()),
                                );
                            }
                        }
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
// Object membership probe (per type: members differ across object kinds)
// ---------------------------------------------------------------------------

unsafe extern "C" fn plugin_has_member(handle: u64, name: *const c_char) -> u8 {
    let name = unsafe { CStr::from_ptr(name) }.to_bytes();

    let kind = with_object(handle, |obj| match obj {
        HttpObject::Server(_) => 0u8,
        HttpObject::Request(_) => 1u8,
        HttpObject::Response(_) => 2u8,
        HttpObject::Chunked(_) => 3u8,
    });

    let present = match kind {
        // Server: attr `addr` + methods.
        Some(0) => matches!(
            name,
            b"addr" | b"recv" | b"try_recv" | b"recv_timeout" | b"start" | b"recv_async" | b"close"
        ),
        // Request: attrs + methods.
        Some(1) => matches!(
            name,
            b"url"
                | b"method"
                | b"headers"
                | b"cookies"
                | b"multipart"
                | b"body"
                | b"respond"
                | b"start_chunked"
                | b"start_sse"
        ),
        // Response: attrs + method `json`.
        Some(2) => matches!(name, b"status" | b"body" | b"headers" | b"json"),
        // Chunked: methods only.
        Some(3) => matches!(name, b"send_chunk" | b"send_event" | b"end"),
        _ => false,
    };

    u8::from(present)
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
static RESPONSE_ATTR_NAME: &[u8] = b"Response\0";

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
            PluginAttr::host_value(VERSION_ATTR_NAME.as_ptr() as *const c_char, mk("0.2.0")),
            PluginAttr::host_value(REQUEST_ATTR_NAME.as_ptr() as *const c_char, mk("http.Request")),
            PluginAttr::host_value(RESPONSE_ATTR_NAME.as_ptr() as *const c_char, mk("http.Response")),
        ];

        let fn_ptrs: Vec<*const c_char> = FN_NAMES.iter().map(|n| n.as_ptr() as *const c_char).collect();

        let desc = PluginDescriptor {
            abi_magic: PLUGIN_ABI_MAGIC,
            abi_version: PLUGIN_ABI_VERSION,
            module_name: MODULE_NAME.as_ptr() as *const c_char,
            module_version: MODULE_VERSION.as_ptr() as *const c_char,
            num_attrs: 4,
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
