//! Tests for the http stdlib plugin.

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
    let resp = http_request_full("POST", &url, &[], Some("hello payload"), None, MAX_BODY_SIZE).expect("post failed");
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
            let resp =
                tiny_http::Response::from_string(r#"{"name":"cat","age":3,"tags":["a","b"]}"#).with_status_code(200);
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
