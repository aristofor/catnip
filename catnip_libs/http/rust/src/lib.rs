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
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use tiny_http::{Response, Server as TinyServer};

use catnip_vm::Value;
use catnip_vm::collections::{NativeDict, ValueKey};
use catnip_vm::plugin::{
    PLUGIN_ABI_MAGIC, PLUGIN_ABI_VERSION, PLUGIN_RESULT_OBJECT, PluginAttr, PluginCallFn, PluginDescriptor,
    PluginDropFn, PluginGetAttrFn, PluginMethodFn, PluginResult,
};
use catnip_vm::value::NativeString;

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
                value: Value::from_str("0.2.0").bits(),
            },
            PluginAttr {
                name: REQUEST_ATTR_NAME.as_ptr() as *const c_char,
                value: Value::from_str("http.Request").bits(),
            },
            PluginAttr {
                name: RESPONSE_ATTR_NAME.as_ptr() as *const c_char,
                value: Value::from_str("http.Response").bits(),
            },
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
mod tests {
    use super::*;
    use std::thread;

    fn spawn_echo_server() -> (String, thread::JoinHandle<()>) {
        let server = TinyServer::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap().to_string();
        let handle = thread::spawn(move || {
            while let Ok(mut req) = server.recv() {
                let url = req.url().to_string();
                let method = req.method().to_string();
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                let reply = format!("{} {} {}", method, url, body);
                let header = tiny_http::Header::from_bytes(&b"X-Echo"[..], b"yes").unwrap();
                let resp = tiny_http::Response::from_string(reply)
                    .with_status_code(200)
                    .with_header(header);
                let _ = req.respond(resp);
            }
        });
        (addr, handle)
    }

    #[test]
    fn test_http_get() {
        let (addr, _h) = spawn_echo_server();
        let url = format!("http://{}/hello", addr);
        let resp = http_request("GET", &url, None).expect("get failed");
        assert_eq!(resp.status, 200);
        assert!(resp.body.starts_with("GET /hello"));
        assert!(resp.headers.iter().any(|(k, v)| k == "x-echo" && v == "yes"));
    }

    #[test]
    fn test_http_post_with_body() {
        let (addr, _h) = spawn_echo_server();
        let url = format!("http://{}/items", addr);
        let resp = http_request("POST", &url, Some("payload=42")).expect("post failed");
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("POST /items payload=42"));
    }

    #[test]
    fn test_http_delete() {
        let (addr, _h) = spawn_echo_server();
        let url = format!("http://{}/x", addr);
        let resp = http_request("DELETE", &url, None).expect("delete failed");
        assert_eq!(resp.status, 200);
        assert!(resp.body.starts_with("DELETE /x"));
    }

    #[test]
    fn test_http_put_with_body() {
        let (addr, _h) = spawn_echo_server();
        let url = format!("http://{}/r", addr);
        let resp = http_request("PUT", &url, Some("data")).expect("put failed");
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("PUT /r data"));
    }

    #[test]
    fn test_http_get_invalid_url() {
        let result = http_request("GET", "not-a-url", None);
        assert!(result.is_err());
    }

    fn spawn_header_echo_server() -> (String, thread::JoinHandle<()>) {
        let server = TinyServer::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap().to_string();
        let handle = thread::spawn(move || {
            while let Ok(mut req) = server.recv() {
                let mut hdr_lines = Vec::new();
                for h in req.headers() {
                    hdr_lines.push(format!("{}: {}", h.field, h.value));
                }
                hdr_lines.sort();
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                let reply = format!("{}\n--\n{}", hdr_lines.join("\n"), body);
                let resp = tiny_http::Response::from_string(reply).with_status_code(200);
                let _ = req.respond(resp);
            }
        });
        (addr, handle)
    }

    #[test]
    fn test_http_request_with_headers() {
        let (addr, _h) = spawn_header_echo_server();
        let url = format!("http://{}/x", addr);
        let resp = http_request_full(
            "GET",
            &url,
            &[
                ("X-Foo".to_string(), "bar".to_string()),
                ("X-Baz".to_string(), "qux".to_string()),
            ],
            None,
            None,
            MAX_BODY_SIZE,
        )
        .expect("request failed");
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("x-foo: bar"));
        assert!(resp.body.contains("x-baz: qux"));
    }

    #[test]
    fn test_http_request_with_body() {
        let (addr, _h) = spawn_header_echo_server();
        let url = format!("http://{}/x", addr);
        let resp =
            http_request_full("POST", &url, &[], Some("hello payload"), None, MAX_BODY_SIZE).expect("post failed");
        assert!(resp.body.contains("hello payload"));
    }

    #[test]
    fn test_http_request_with_timeout() {
        // Server that accepts but never replies
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        thread::spawn(move || {
            // Accept and hold connection open
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::Read;
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                thread::sleep(Duration::from_secs(5));
            }
        });
        let url = format!("http://{}/", addr);
        let result = http_request_full("GET", &url, &[], None, Some(0.2), MAX_BODY_SIZE);
        assert!(result.is_err(), "timeout should produce an error");
    }

    #[test]
    fn test_http_request_max_body_under_limit() {
        let (addr, _h) = spawn_echo_server();
        let url = format!("http://{}/x", addr);
        // Set max_body absurdly low; echo reply is "GET /x " (7 bytes)
        let result = http_request_full("GET", &url, &[], None, None, 3);
        assert!(result.is_err(), "body should exceed 3-byte limit");
        let err = result.unwrap_err();
        assert!(err.contains("body"), "expected body-related error, got: {}", err);
    }

    #[test]
    fn test_http_response_json() {
        let server = TinyServer::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap().to_string();
        thread::spawn(move || {
            if let Ok(req) = server.recv() {
                let resp = tiny_http::Response::from_string(r#"{"name":"cat","age":3,"tags":["a","b"]}"#)
                    .with_status_code(200);
                let _ = req.respond(resp);
            }
        });
        let url = format!("http://{}/j", addr);
        let data = http_request("GET", &url, None).expect("request failed");

        // Parse body as JSON manually (mirrors what Response.json() does internally)
        let json: serde_json::Value = serde_json::from_str(&data.body).expect("invalid JSON");
        let val = json_to_value(&json);
        assert!(val.is_native_dict());
        let dict = unsafe { val.as_native_dict_ref().unwrap() };
        let name = dict.get_default(&str_key("name"), Value::NIL);
        assert!(name.is_native_str());
        let name_str = unsafe { name.as_native_str_ref().unwrap() };
        assert_eq!(name_str, "cat");
        let age = dict.get_default(&str_key("age"), Value::NIL);
        assert_eq!(age.as_int(), Some(3));
    }

    #[test]
    fn test_http_response_json_invalid() {
        let server = TinyServer::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap().to_string();
        thread::spawn(move || {
            if let Ok(req) = server.recv() {
                let resp = tiny_http::Response::from_string("not json").with_status_code(200);
                let _ = req.respond(resp);
            }
        });
        let url = format!("http://{}/j", addr);
        let data = http_request("GET", &url, None).expect("request failed");
        let result: Result<serde_json::Value, _> = serde_json::from_str(&data.body);
        assert!(result.is_err(), "should fail to parse 'not json'");
    }

    #[test]
    fn test_json_to_value_large_int_promotes_to_bigint() {
        // 2^53 -- larger than SMALLINT_MAX (2^46 - 1) but fits in i64.
        let json: serde_json::Value = serde_json::from_str("9007199254740992").unwrap();
        let v = json_to_value(&json);
        assert!(v.is_bigint(), "expected BigInt for value > SMALLINT_MAX");
        let n = unsafe { v.as_bigint_ref().unwrap() };
        assert_eq!(n, &rug::Integer::from(9007199254740992i64));
        v.decref();
    }

    #[test]
    fn test_json_to_value_u64_above_i64_max_preserves_precision() {
        // u64::MAX -- doesn't fit in i64, must round-trip as BigInt.
        let json: serde_json::Value = serde_json::from_str("18446744073709551615").unwrap();
        let v = json_to_value(&json);
        assert!(v.is_bigint(), "u64 > i64::MAX should produce BigInt");
        let n = unsafe { v.as_bigint_ref().unwrap() };
        assert_eq!(n, &rug::Integer::from(u64::MAX));
        v.decref();
    }

    #[test]
    fn test_opts_get_str_does_not_leak() {
        // Smoke test: build a dict with a string value, read it many times,
        // and make sure the value is still accessible (no premature drop).
        // A leaking implementation would also pass this; the precise leak
        // signature requires inspecting Arc::strong_count which is not
        // exposed publicly. We rely on the decref() calls in the helpers
        // and on miri/valgrind for stronger guarantees.
        let dict = NativeDict::empty();
        let val = Value::from_string("bar".to_string());
        dict.set_item(str_key("foo"), val);
        val.decref(); // dict holds the only strong ref now

        for _ in 0..100 {
            assert_eq!(opts_get_str(&dict, "foo").as_deref(), Some("bar"));
            assert!(opts_get_str(&dict, "missing").is_none());
        }
    }

    fn make_async_server() -> (ServerObject, String) {
        let inner = TinyServer::http("127.0.0.1:0").unwrap();
        let addr = inner.server_addr().to_ip().unwrap().to_string();
        let server = ServerObject {
            server: Arc::new(inner),
            async_state: RefCell::new(None),
        };
        (server, addr)
    }

    fn shutdown_async(server: &ServerObject) {
        server.shutdown();
    }

    #[test]
    fn test_async_server_start_idempotent() {
        let (server, _addr) = make_async_server();
        start_async_accept(&server).unwrap();
        start_async_accept(&server).unwrap(); // no-op, must not spawn a second thread
        assert!(server.async_state.borrow().is_some());
        shutdown_async(&server);
    }

    #[test]
    fn test_async_server_recv_after_start() {
        use std::time::Duration as StdDuration;

        let (server, addr) = make_async_server();
        start_async_accept(&server).unwrap();

        // Client request runs in another thread (will block until we respond).
        let url = format!("http://{}/hello", addr);
        let client = thread::spawn(move || http_request("GET", &url, None));

        // Poll the async channel up to ~1s for the inbound request.
        let mut req = None;
        for _ in 0..50 {
            thread::sleep(StdDuration::from_millis(20));
            let state = server.async_state.borrow();
            if let Some(st) = state.as_ref() {
                if let Ok(r) = st.receiver.try_recv() {
                    req = Some(r);
                    break;
                }
            }
        }
        let req = req.expect("recv_async should have surfaced the request");
        assert_eq!(req.method().as_str(), "GET");
        assert_eq!(req.url(), "/hello");

        let resp = tiny_http::Response::from_string("hi").with_status_code(200);
        req.respond(resp).unwrap();

        let client_resp = client.join().unwrap().expect("client failed");
        assert_eq!(client_resp.status, 200);
        assert_eq!(client_resp.body, "hi");

        shutdown_async(&server);
    }

    #[test]
    fn test_async_server_drop_releases_inner_server() {
        // Hold an external Arc to observe the strong_count before/after the
        // ServerObject is dropped without explicit close().
        let inner = Arc::new(TinyServer::http("127.0.0.1:0").unwrap());

        {
            let server = ServerObject {
                server: Arc::clone(&inner),
                async_state: RefCell::new(None),
            };
            // Before start(): test handle + ServerObject = 2.
            assert_eq!(Arc::strong_count(&inner), 2);

            start_async_accept(&server).unwrap();
            // Give the accept thread time to clone the Arc.
            thread::sleep(Duration::from_millis(50));
            assert_eq!(Arc::strong_count(&inner), 3, "accept thread should own a clone");
            // server is dropped here -- no explicit close() called.
        }

        // Drop must have unblocked recv() and joined the accept thread,
        // releasing its Arc<TinyServer>.
        assert_eq!(
            Arc::strong_count(&inner),
            1,
            "Drop should join the accept thread and release its Arc"
        );
    }

    #[test]
    fn test_async_server_close_joins_thread() {
        let (server, _addr) = make_async_server();
        start_async_accept(&server).unwrap();
        // Close should unblock recv() and let the accept thread exit cleanly.
        shutdown_async(&server);
        // After shutdown, the async_state is gone.
        assert!(server.async_state.borrow().is_none());
    }

    fn spawn_chunked_server<F>(handler: F) -> String
    where
        F: FnOnce(tiny_http::Request) + Send + 'static,
    {
        let server = TinyServer::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap().to_string();
        thread::spawn(move || {
            if let Ok(req) = server.recv() {
                handler(req);
            }
        });
        addr
    }

    #[test]
    fn test_extract_boundary() {
        assert_eq!(
            extract_boundary("multipart/form-data; boundary=xyz"),
            Some("xyz".to_string())
        );
        assert_eq!(
            extract_boundary("multipart/form-data; boundary=\"with spaces\""),
            Some("with spaces".to_string())
        );
        assert_eq!(extract_boundary("text/plain"), None);
    }

    #[test]
    fn test_parse_multipart_basic() {
        let boundary = "abc123";
        let body = format!(
            "--{b}\r\n\
             Content-Disposition: form-data; name=\"field1\"\r\n\
             \r\n\
             hello\r\n\
             --{b}\r\n\
             Content-Disposition: form-data; name=\"field2\"\r\n\
             \r\n\
             world\r\n\
             --{b}--\r\n",
            b = boundary
        );
        let parts = parse_multipart(body.as_bytes(), boundary);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name, "field1");
        assert_eq!(parts[0].data, b"hello");
        assert!(parts[0].filename.is_none());
        assert_eq!(parts[1].name, "field2");
        assert_eq!(parts[1].data, b"world");
    }

    #[test]
    fn test_parse_multipart_with_file() {
        let boundary = "B";
        let header = b"--B\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"x.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n";
        let payload: &[u8] = &[0x00, 0x01, 0x02, 0xff];
        let trailer = b"\r\n--B--\r\n";
        let mut body = Vec::new();
        body.extend_from_slice(header);
        body.extend_from_slice(payload);
        body.extend_from_slice(trailer);

        let parts = parse_multipart(&body, boundary);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].name, "upload");
        assert_eq!(parts[0].filename.as_deref(), Some("x.bin"));
        assert_eq!(parts[0].content_type.as_deref(), Some("application/octet-stream"));
        assert_eq!(parts[0].data, vec![0x00, 0x01, 0x02, 0xff]);
    }

    #[test]
    fn test_parse_multipart_anchored_against_inner_bytes() {
        // The payload contains "--xyz" but never preceded by \r\n + followed
        // by \r\n or --, so it must not be interpreted as a boundary line.
        let boundary = "xyz";
        let header = b"--xyz\r\nContent-Disposition: form-data; name=\"f\"\r\n\r\n";
        let payload = b"prefix--xyzsuffix";
        let trailer = b"\r\n--xyz--\r\n";
        let mut body = Vec::new();
        body.extend_from_slice(header);
        body.extend_from_slice(payload);
        body.extend_from_slice(trailer);

        let parts = parse_multipart(&body, boundary);
        assert_eq!(parts.len(), 1, "inner --xyz must not be treated as a delimiter");
        assert_eq!(parts[0].name, "f");
        assert_eq!(parts[0].data, payload);
    }

    #[test]
    fn test_parse_multipart_case_insensitive_headers() {
        let boundary = "B";
        let body = "--B\r\n\
            content-disposition: form-data; NAME=\"f\"; FileName=\"x.txt\"\r\n\
            CONTENT-TYPE: text/plain\r\n\
            \r\n\
            hello\r\n\
            --B--\r\n";
        let parts = parse_multipart(body.as_bytes(), boundary);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].name, "f");
        assert_eq!(parts[0].filename.as_deref(), Some("x.txt"));
        assert_eq!(parts[0].content_type.as_deref(), Some("text/plain"));
        assert_eq!(parts[0].data, b"hello");
    }

    #[test]
    fn test_basic_auth_format() {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode("user:pass");
        assert_eq!(encoded, "dXNlcjpwYXNz");
    }

    #[test]
    fn test_status_allows_body() {
        for s in [100u16, 101, 150, 199, 204, 304] {
            assert!(!status_allows_body(s), "status {} must reject body", s);
        }
        for s in [200u16, 201, 206, 301, 400, 404, 500] {
            assert!(status_allows_body(s), "status {} should allow body", s);
        }
    }

    #[test]
    fn test_chunked_rejects_head_request() {
        use std::io::Write;
        use std::net::TcpStream;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let rejected = Arc::new(AtomicBool::new(false));
        let observed_method = Arc::new(std::sync::Mutex::new(String::new()));
        let rejected_clone = rejected.clone();
        let method_clone = observed_method.clone();
        let addr = spawn_chunked_server(move |req| {
            *method_clone.lock().unwrap() = req.method().as_str().to_string();
            if ChunkedWriter::try_from_request(req, 200, "text/plain".to_string()).is_err() {
                rejected_clone.store(true, Ordering::SeqCst);
            }
        });

        let mut stream = TcpStream::connect(&addr).unwrap();
        stream
            .write_all(b"HEAD / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        // Wait for the handler to run.
        for _ in 0..50 {
            thread::sleep(Duration::from_millis(20));
            if !observed_method.lock().unwrap().is_empty() {
                break;
            }
        }

        assert_eq!(
            *observed_method.lock().unwrap(),
            "HEAD",
            "server should have seen a HEAD request"
        );
        assert!(
            rejected.load(Ordering::SeqCst),
            "HEAD must be rejected by try_from_request"
        );
    }

    #[test]
    fn test_chunked_rejects_http10_request() {
        use std::io::Write;
        use std::net::TcpStream;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let rejected = Arc::new(AtomicBool::new(false));
        let observed = Arc::new(AtomicBool::new(false));
        let rejected_clone = rejected.clone();
        let observed_clone = observed.clone();
        let addr = spawn_chunked_server(move |req| {
            observed_clone.store(true, Ordering::SeqCst);
            if ChunkedWriter::try_from_request(req, 200, "text/plain".to_string()).is_err() {
                rejected_clone.store(true, Ordering::SeqCst);
            }
        });

        let mut stream = TcpStream::connect(&addr).unwrap();
        stream
            .write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        for _ in 0..50 {
            thread::sleep(Duration::from_millis(20));
            if observed.load(Ordering::SeqCst) {
                break;
            }
        }

        assert!(observed.load(Ordering::SeqCst), "server should have seen the request");
        assert!(
            rejected.load(Ordering::SeqCst),
            "HTTP/1.0 must be rejected by try_from_request"
        );
    }

    #[test]
    fn test_chunked_response_basic() {
        let addr = spawn_chunked_server(|req| {
            let mut w = ChunkedWriter::try_from_request(req, 200, "text/plain".to_string()).unwrap();
            w.send_chunk(b"Hello ").unwrap();
            w.send_chunk(b"World").unwrap();
            w.end().unwrap();
        });
        let url = format!("http://{}/", addr);
        let resp = http_request("GET", &url, None).expect("client failed");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, "Hello World");
    }

    #[test]
    fn test_chunked_drop_sends_terminator() {
        let addr = spawn_chunked_server(|req| {
            let mut w = ChunkedWriter::try_from_request(req, 200, "text/plain".to_string()).unwrap();
            w.send_chunk(b"hi").unwrap();
            // No explicit end(); Drop must send the terminating chunk.
        });
        let url = format!("http://{}/", addr);
        let resp = http_request("GET", &url, None).expect("client should not hang");
        assert_eq!(resp.body, "hi");
    }

    #[test]
    fn test_chunked_skips_empty_chunks() {
        let addr = spawn_chunked_server(|req| {
            let mut w = ChunkedWriter::try_from_request(req, 200, "text/plain".to_string()).unwrap();
            w.send_chunk(b"a").unwrap();
            w.send_chunk(b"").unwrap(); // empty must NOT terminate
            w.send_chunk(b"b").unwrap();
            w.end().unwrap();
        });
        let url = format!("http://{}/", addr);
        let resp = http_request("GET", &url, None).expect("client failed");
        assert_eq!(resp.body, "ab");
    }

    #[test]
    fn test_sse_event_format() {
        let addr = spawn_chunked_server(|req| {
            let mut w = ChunkedWriter::try_from_request(req, 200, "text/event-stream".to_string()).unwrap();
            // Replicates the wire format produced by send_event(data, type?).
            w.send_chunk(b"data: hello\n\n").unwrap();
            w.send_chunk(b"event: update\ndata: payload\n\n").unwrap();
            w.end().unwrap();
        });
        let url = format!("http://{}/", addr);
        let resp = http_request("GET", &url, None).expect("client failed");
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("data: hello\n\n"));
        assert!(resp.body.contains("event: update\ndata: payload\n\n"));
    }

    #[test]
    fn test_sse_multiline_data_is_split() {
        // send_event() in plugin_method splits multi-line data on '\n' to one
        // 'data:' line per piece, matching the SSE spec. This test exercises
        // the same formatting logic via a manual chunk.
        let addr = spawn_chunked_server(|req| {
            let mut w = ChunkedWriter::try_from_request(req, 200, "text/event-stream".to_string()).unwrap();
            let mut payload = String::new();
            for line in "line1\nline2\nline3".split('\n') {
                payload.push_str("data: ");
                payload.push_str(line);
                payload.push('\n');
            }
            payload.push('\n');
            w.send_chunk(payload.as_bytes()).unwrap();
            w.end().unwrap();
        });
        let url = format!("http://{}/", addr);
        let resp = http_request("GET", &url, None).expect("client failed");
        assert!(resp.body.contains("data: line1\ndata: line2\ndata: line3\n\n"));
    }

    #[test]
    fn test_http_body_read_failure_propagates() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        // Raw TCP server that advertises a longer Content-Length than it sends
        // and closes the connection mid-transfer.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort");
                // Drop the stream -- client must surface the truncated body.
            }
        });

        let url = format!("http://{}/x", addr);
        let result = http_request("GET", &url, None);
        assert!(
            result.is_err(),
            "truncated response should propagate as error, got {:?}",
            result.as_ref().map(|r| (r.status, r.body.len()))
        );
        let err = result.unwrap_err();
        assert!(err.contains("body"), "expected body-related error, got: {}", err);
    }
}
