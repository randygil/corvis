//! Security regressions of the HTTP transport and of file uploads (multipart):
//!
//! - multipart bodies are read only by file interceptors, after the guards, streaming, with the
//!   multer limits and a finite body limit enforced during the read;
//! - `append-field` cannot allocate dense arrays from a tiny body;
//! - Node-like timeouts (`headersTimeout`, `requestTimeout`, `keepAliveTimeout`) and head size;
//! - disk storage: no alternate data streams / device names, no escape from the destination,
//!   no orphan files after a failing pipe.

use std::time::{Duration, Instant};

use corvis::multer::{DiskStorageOptions, Limits, MulterOptions, disk_storage};
use corvis::prelude::*;
use corvis::testing::request;
use corvis::{FileInterceptor, MaxFileSizeValidator, NoFilesInterceptor, ParseFilePipe, ServerOptions, UploadedFile};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn upload_dir(tag: &str) -> String {
    let d = std::env::temp_dir().join(format!("corvis-transport-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

fn files_in(dir: &str) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|r| r.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default()
}

#[injectable]
pub struct DenyGuard;

#[async_trait]
impl CanActivate for DenyGuard {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Ok(false)
    }
}

#[controller("t")]
pub struct TController;

#[routes]
impl TController {
    #[post("json")]
    fn json(&self, #[body] body: Value) -> Value {
        body
    }

    #[get("slow")]
    async fn slow(&self) -> Value {
        tokio::time::sleep(Duration::from_millis(800)).await;
        json!({ "ok": true })
    }

    #[post("limited")]
    #[use_interceptors(FileInterceptor::new("file").limits(Limits::new().file_size(1024)))]
    fn limited(&self, #[uploaded_file] file: Option<UploadedFile>) -> Value {
        json!({ "size": file.map(|f| f.size) })
    }

    #[post("upload")]
    #[use_interceptors(FileInterceptor::new("file"))]
    fn upload(&self, #[uploaded_file] file: Option<UploadedFile>, #[body] body: Value) -> Value {
        json!({ "body": body, "size": file.as_ref().map(|f| f.size), "content": file.and_then(|f| f.buffer).map(|b| String::from_utf8_lossy(&b).into_owned()) })
    }

    #[post("small-body")]
    #[use_interceptors(FileInterceptor::new("file").options(MulterOptions::new().body_limit(64 * 1024)))]
    fn small_body(&self, #[uploaded_file] file: Option<UploadedFile>) -> Value {
        json!({ "size": file.map(|f| f.size) })
    }

    #[post("guarded")]
    #[use_guards(DenyGuard)]
    #[use_interceptors(FileInterceptor::new("file"))]
    fn guarded(&self) -> Value {
        json!({ "ok": true })
    }

    #[post("none")]
    #[use_interceptors(NoFilesInterceptor::new())]
    fn none(&self, #[body] body: Value) -> Value {
        body
    }

    #[post("raw")]
    async fn raw(&self, #[req] req: Request) -> HttpResult<Value> {
        let body = corvis::multer::read_body(&req, 1024).await?;
        Ok(json!({ "len": body.len() }))
    }

    #[post("disk-validated")]
    #[use_interceptors(FileInterceptor::new("file").options(MulterOptions::new().dest(upload_dir("validated"))))]
    fn disk_validated(&self, #[uploaded_file(ParseFilePipe::new().validator(MaxFileSizeValidator::new(10)))] file: UploadedFile) -> Value {
        json!({ "path": file.path })
    }

    #[post("disk-handler-fails")]
    #[use_interceptors(FileInterceptor::new("file").options(MulterOptions::new().dest(upload_dir("handler"))))]
    fn disk_handler_fails(&self, #[uploaded_file] _file: UploadedFile) -> HttpResult<Value> {
        Err(BadRequestException::new("nope"))
    }

    #[post("disk-keep")]
    #[use_interceptors(FileInterceptor::new("file").options(MulterOptions::new().dest(upload_dir("keep")).remove_files_on_error(false)))]
    fn disk_keep(&self, #[uploaded_file] _file: UploadedFile) -> HttpResult<Value> {
        Err(BadRequestException::new("nope"))
    }

    #[post("disk-raw")]
    #[use_interceptors(FileInterceptor::new("file").storage(disk_storage(
        DiskStorageOptions::new().destination(upload_dir("raw")).filename(|_req, file| Ok(format!("123-{}", file.originalname))),
    )))]
    fn disk_raw(&self, #[uploaded_file] file: UploadedFile) -> Value {
        json!({ "path": file.path })
    }

    #[post("disk-preserve")]
    #[use_interceptors(FileInterceptor::new("file").options(MulterOptions::new().preserve_path(true).storage(disk_storage(
        DiskStorageOptions::new().destination(upload_dir("preserve")).filename(|_req, file| Ok(file.originalname.clone())),
    ))))]
    fn disk_preserve(&self, #[uploaded_file] file: UploadedFile) -> Value {
        json!({ "path": file.path })
    }
}

#[module(controllers = [TController], providers = [DenyGuard])]
pub struct AppModule;

async fn app() -> CorvisApplication {
    let module = corvis::testing::Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    app
}

async fn serve(options: Option<ServerOptions>) -> std::net::SocketAddr {
    let mut app = app().await;
    if let Some(o) = options {
        app.set_server_options(o);
    }
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    std::mem::forget(app);
    addr
}

/// Reads until the end of the response head (or EOF / timeout) and returns what arrived.
async fn read_head(s: &mut TcpStream, wait: Duration) -> String {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match tokio::time::timeout_at(deadline, s.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
            Ok(Ok(n)) => {
                out.extend_from_slice(&buf[..n]);
                if let Some(i) = out.windows(4).position(|w| w == b"\r\n\r\n") {
                    // Also take the body when it is announced and short.
                    let head = String::from_utf8_lossy(&out[..i]).to_ascii_lowercase();
                    let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok());
                    if len.is_none_or(|n| out.len() >= i + 4 + n) {
                        break;
                    }
                }
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `true` if the server closed the connection within `wait`.
async fn closed_within(s: &mut TcpStream, wait: Duration) -> bool {
    let mut buf = [0u8; 1024];
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match tokio::time::timeout_at(deadline, s.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) => return true,
            Ok(Ok(_)) => continue,
            Err(_) => return false,
        }
    }
}

const CT: &str = "multipart/form-data; boundary=BB";

fn file_part_head(name: &str, filename: &str) -> String {
    format!(
        "--BB\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
}

/// A multipart POST whose head announces `length` bytes, of which only `sent` is written.
async fn partial_multipart(addr: std::net::SocketAddr, path: &str, length: u64, sent: &[u8]) -> TcpStream {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let head = format!("POST {path} HTTP/1.1\r\nHost: x\r\nContent-Type: {CT}\r\nContent-Length: {length}\r\n\r\n");
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(sent).await.unwrap();
    s
}

// ---------------------------------------------------------------------------
// Multipart is read only by file interceptors, after the guards, with limits
// ---------------------------------------------------------------------------

/// Before: any `multipart/*` request was buffered whole (no limit) before routing, so a 404 was
/// sent only after the whole body arrived.
#[corvis::test]
async fn multipart_to_unknown_route_is_answered_without_reading_the_body() {
    let addr = serve(None).await;
    let mut s = partial_multipart(addr, "/does-not-exist", 500 * 1024 * 1024, &[b'x'; 4096]).await;
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 404"), "{r:?}");
    // The same for a route without file interceptor and for a guarded upload route.
    let mut s = partial_multipart(addr, "/t/json", 500 * 1024 * 1024, &[b'x'; 4096]).await;
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 201"), "{r:?}");
    let mut s = partial_multipart(addr, "/t/guarded", 500 * 1024 * 1024, &[b'x'; 4096]).await;
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 403"), "{r:?}");
}

/// A body nobody reads (guard rejects) is discarded up to the transport limit after the
/// response: the client that already sent it gets the response and the connection is reused.
#[corvis::test]
async fn unread_body_is_drained_after_the_response() {
    let addr = serve(None).await;
    let body = vec![b'x'; 512 * 1024];
    let mut s = TcpStream::connect(addr).await.unwrap();
    let head = format!("POST /t/guarded HTTP/1.1\r\nHost: x\r\nContent-Type: {CT}\r\nContent-Length: {}\r\n\r\n", body.len());
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(&body).await.unwrap();
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 403"), "{r:?}");
    s.write_all(b"GET /t/slow HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 200"), "{r:?}");
}

/// Before: `limits.fileSize` was applied after buffering the whole body.
#[corvis::test]
async fn file_size_limit_aborts_the_read() {
    let addr = serve(None).await;
    let mut sent = file_part_head("file", "a.bin").into_bytes();
    sent.extend_from_slice(&[b'a'; 64 * 1024]);
    let mut s = partial_multipart(addr, "/t/limited", 9 * 1024 * 1024, &sent).await;
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 413") && r.contains("File too large"), "{r:?}");
}

/// Before: no limit at all. Now a declared length over the body limit is a 413 before reading.
#[corvis::test]
async fn multipart_body_limit_rejects_a_declared_length() {
    let addr = serve(None).await;
    let mut s = partial_multipart(addr, "/t/upload", 11 * 1024 * 1024, b"").await;
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 413") && r.contains("request entity too large"), "{r:?}");
}

/// A chunked body (no length) is cut as soon as it crosses the limit.
#[corvis::test]
async fn multipart_body_limit_applies_while_streaming() {
    let addr = serve(None).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    let head = format!("POST /t/small-body HTTP/1.1\r\nHost: x\r\nContent-Type: {CT}\r\nTransfer-Encoding: chunked\r\n\r\n");
    s.write_all(head.as_bytes()).await.unwrap();
    let first = file_part_head("file", "a.bin");
    s.write_all(format!("{:x}\r\n{first}\r\n", first.len()).as_bytes()).await.unwrap();
    let chunk = vec![b'z'; 16 * 1024];
    for _ in 0..8 {
        // 128 KiB > 64 KiB: the server answers before the body ends (never terminated here).
        if s.write_all(format!("{:x}\r\n", chunk.len()).as_bytes()).await.is_err() {
            break;
        }
        let _ = s.write_all(&chunk).await;
        let _ = s.write_all(b"\r\n").await;
    }
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 413"), "{r:?}");
}

/// The streamed path parses exactly like before (fields, files, epilogue), byte by byte.
#[corvis::test]
async fn streamed_upload_over_the_socket() {
    let addr = serve(None).await;
    let body = format!(
        "--BB\r\nContent-Disposition: form-data; name=\"tags[]\"\r\n\r\nx\r\n{}hola mundo\r\n--BB--\r\n",
        file_part_head("file", "C:\\dir\\a.txt")
    );
    let mut s = TcpStream::connect(addr).await.unwrap();
    let head = format!("POST /t/upload HTTP/1.1\r\nHost: x\r\nContent-Type: {CT}\r\nContent-Length: {}\r\n\r\n", body.len());
    s.write_all(head.as_bytes()).await.unwrap();
    for b in body.as_bytes().chunks(7) {
        s.write_all(b).await.unwrap();
        s.flush().await.unwrap();
    }
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 201"), "{r:?}");
    assert!(r.ends_with(r#"{"body":{"tags":["x"]},"size":10,"content":"hola mundo"}"#), "{r:?}");
    // The connection stays usable (keep-alive) after a fully read upload.
    s.write_all(b"GET /t/slow HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 200"), "{r:?}");
}

/// `corvis::multer::read_body` gives handlers the multipart body the server left unread.
#[corvis::test]
async fn read_body_reads_the_pending_multipart_body() {
    let addr = serve(None).await;
    let body = b"--BB\r\n\r\n--BB--";
    let mut s = partial_multipart(addr, "/t/raw", body.len() as u64, body).await;
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.ends_with(r#"{"len":14}"#), "{r:?}");
    let mut s = partial_multipart(addr, "/t/raw", 4096, &[b'x'; 4096]).await;
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 413"), "{r:?}");
}

/// `x[1000000]=v` (71 bytes) used to allocate ~200 MiB: now a far index becomes an object key.
#[corvis::test]
async fn sparse_index_in_field_name_does_not_allocate() {
    let app = app().await;
    let r = request(&app).post("/t/none").field("x[1000000]", "v").field("y[1]", "b").send().await;
    r.expect_status(201).expect_json(json!({ "x": { "1000000": "v" }, "y": [null, "b"] }));
}

/// A field name nested 5000 levels deep (the most a 16 KiB part header allows) used to build an
/// equally deep `req.body` (stack overflow, the process aborts). Past 32 levels (default) the name is a literal key.
#[corvis::test]
async fn deeply_nested_field_name_does_not_overflow() {
    let app = app().await;
    let name = format!("a{}", "[b]".repeat(5000));
    let r = request(&app).post("/t/none").field(&name, "v").send().await;
    r.expect_status(201);
    assert_eq!(r.json_value()[name.as_str()], json!("v"));
}

// ---------------------------------------------------------------------------
// Non-multipart bodies
// ---------------------------------------------------------------------------

/// Before: the body was read up to the 1 MiB transport limit before the 100 kB JSON limit
/// rejected it; a declared length over the JSON limit now gets a 413 without reading.
#[corvis::test]
async fn json_over_the_parser_limit_is_rejected_before_reading() {
    let addr = serve(None).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(b"POST /t/json HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 200000\r\n\r\n[").await.unwrap();
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 413"), "{r:?}");
}

/// Before: any read error (malformed chunked encoding, aborted client) was a 413.
#[corvis::test]
async fn malformed_chunked_body_is_not_reported_as_too_large() {
    let addr = serve(None).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(
        b"POST /t/json HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n{}\r\n0\r\n\r\n",
    )
    .await
    .unwrap();
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(!r.contains(" 413 "), "{r:?}");
    assert!(r.is_empty() || r.starts_with("HTTP/1.1 400"), "{r:?}");
}

// Chunked framing kept on purpose (requirements of `http/request-parsing`, see `docs/security/http-parsing.md`).
async fn chunked_json(body: &[u8]) -> String {
    let addr = serve(None).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut req =
        b"POST /t/json HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
            .to_vec();
    req.extend_from_slice(body);
    s.write_all(&req).await.unwrap();
    read_head(&mut s, Duration::from_secs(5)).await
}

#[corvis::test]
async fn chunk_size_with_leading_zeros_is_accepted() {
    let r = chunked_json(b"00000000000000000000000002\r\n{}\r\n0\r\n\r\n").await;
    assert!(r.starts_with("HTTP/1.1 201"), "{r:?}");
    assert!(r.ends_with("{}"), "{r:?}");
}

#[corvis::test]
async fn chunk_size_over_64_bits_is_rejected() {
    let r = chunked_json(b"10000000000000002\r\n{}\r\n0\r\n\r\n").await;
    assert!(r.is_empty() || r.starts_with("HTTP/1.1 400"), "{r:?}");
}

#[corvis::test]
async fn bare_lf_in_chunked_framing_is_rejected() {
    let r = chunked_json(b"2\n{}\n0\n\n").await;
    assert!(r.is_empty() || r.starts_with("HTTP/1.1 400"), "{r:?}");
}

/// Before: hyper accepted ~400 KiB of headers (Node: 16 KiB).
#[corvis::test]
async fn oversized_head_is_rejected() {
    let addr = serve(None).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    let big = "a".repeat(40 * 1024);
    s.write_all(format!("GET /t/slow HTTP/1.1\r\nHost: x\r\nX-Big: {big}\r\n\r\n").as_bytes()).await.unwrap();
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 431"), "{r:?}");
}

/// Before: the server closed the socket while the rest of the oversized request was still
/// arriving, which Windows turns into a reset that discards the 431 the client had not read yet.
/// The client keeps sending past the head, so unread input is guaranteed when the server answers.
#[corvis::test]
async fn early_error_response_survives_unread_input() {
    let addr = serve(None).await;
    for _ in 0..5 {
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(&oversized_head()).await.unwrap();
        // Input that arrives after the server has answered; errors only mean it already closed.
        let _ = s.write_all(&vec![b'b'; 256 * 1024]).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = s.write_all(&vec![b'b'; 64 * 1024]).await;
        let r = read_head(&mut s, Duration::from_secs(5)).await;
        assert!(r.starts_with("HTTP/1.1 431"), "{r:?}");
    }
}

const GET_SLOW: &[u8] = b"GET /t/slow HTTP/1.1\r\nHost: x\r\n\r\n";

/// Keeps writing `chunk` every `pause` (as fast as the socket takes it when `pause` is zero) until the
/// server closes or `limit` passes. Returns how long the writes lasted.
async fn send_until_closed(s: &mut TcpStream, chunk: &[u8], pause: Duration, limit: Duration) -> Duration {
    let started = Instant::now();
    while started.elapsed() < limit {
        match tokio::time::timeout(Duration::from_secs(3), s.write_all(chunk)).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => break,
            Err(_) => panic!("write blocked: the server stopped reading but kept the connection"),
        }
        if !pause.is_zero() {
            tokio::time::sleep(pause).await;
        }
    }
    started.elapsed()
}

fn oversized_head() -> Vec<u8> {
    let big = "a".repeat(40 * 1024);
    format!("GET /t/slow HTTP/1.1\r\nHost: x\r\nX-Big: {big}\r\n\r\n").into_bytes()
}

/// Time until a client queued behind a full `max_connections(1)` server is served, counted from `since`.
async fn queued_client_served_after(addr: std::net::SocketAddr, since: Instant) -> Duration {
    let mut next = TcpStream::connect(addr).await.unwrap();
    // An unknown route answers at once, so the time measured is the wait for the permit.
    next.write_all(b"GET /t/none-such HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let r = read_head(&mut next, Duration::from_secs(8)).await;
    assert!(r.starts_with("HTTP/1.1 404"), "{r:?}");
    since.elapsed()
}

/// The drain after an early error response is bounded in bytes: a peer that keeps sending as fast
/// as it can is cut off within a few seconds.
#[corvis::test]
async fn drain_after_an_early_error_response_is_bounded_in_bytes() {
    let addr = serve(None).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(&oversized_head()).await.unwrap();
    let took = send_until_closed(&mut s, &[b'b'; 64 * 1024], Duration::ZERO, Duration::from_secs(8)).await;
    assert!(took < Duration::from_secs(5), "server kept draining an endless sender for {took:?}");
}

/// ...and in time: a slow trickle after the 431 does not keep the connection (and its permit) past
/// the drain time cap.
#[corvis::test]
async fn drain_after_an_early_error_response_is_bounded_in_time() {
    let addr = serve(Some(ServerOptions::new().max_connections(1))).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(&oversized_head()).await.unwrap();
    let started = Instant::now();
    let trickle = tokio::spawn(async move {
        let _ = send_until_closed(&mut s, b"b", Duration::from_millis(50), Duration::from_secs(8)).await;
        s
    });
    let served = queued_client_served_after(addr, started).await;
    assert!(served >= Duration::from_millis(1500), "drained for only {served:?}");
    assert!(served < Duration::from_millis(3500), "permit held for {served:?}");
    drop(trickle.await.unwrap());
}

/// hyper's own header read timeout closes a connection that stays quiet after its first response:
/// no drain, so the permit is free right after `headers_timeout`.
#[corvis::test]
async fn idle_keep_alive_connection_released_without_drain() {
    let addr = serve(Some(
        ServerOptions::new().max_connections(1).headers_timeout(Duration::from_secs(1)).keep_alive_timeout(Duration::from_secs(10)),
    ))
    .await;
    let mut first = TcpStream::connect(addr).await.unwrap();
    first.write_all(GET_SLOW).await.unwrap();
    assert!(read_head(&mut first, Duration::from_secs(5)).await.starts_with("HTTP/1.1 200"));
    let served = queued_client_served_after(addr, Instant::now()).await;
    assert!(served < Duration::from_millis(1500), "permit released after {served:?}");
    drop(first);
}

/// A slowloris on the second request of a keep-alive connection is closed at `headers_timeout`
/// without a drain.
#[corvis::test]
async fn slowloris_on_keep_alive_connection_released_without_drain() {
    let addr = serve(Some(
        ServerOptions::new().max_connections(1).headers_timeout(Duration::from_secs(1)).keep_alive_timeout(Duration::from_secs(10)),
    ))
    .await;
    let mut first = TcpStream::connect(addr).await.unwrap();
    first.write_all(GET_SLOW).await.unwrap();
    assert!(read_head(&mut first, Duration::from_secs(5)).await.starts_with("HTTP/1.1 200"));
    let started = Instant::now();
    let drip = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        for b in b"GET /t/slow HTTP/1.1\r\nHost: x\r\nX-Slow: " {
            if first.write_all(&[*b]).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        first
    });
    let served = queued_client_served_after(addr, started).await;
    assert!(served < Duration::from_millis(1500), "permit released after {served:?}");
    drop(drip.await.unwrap());
}

/// An upgrade listener that destroys the socket leaves nothing to protect: the connection closes at once.
#[corvis::test]
async fn destroyed_upgrade_socket_is_closed_without_drain() {
    let mut app = app().await;
    app.get_http_server().on_upgrade(|u| u.destroy());
    app.set_server_options(ServerOptions::new().max_connections(1));
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    std::mem::forget(app);
    let mut first = TcpStream::connect(addr).await.unwrap();
    first.write_all(b"GET /x HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: foo\r\n\r\n").await.unwrap();
    // The client stays quiet; the server must close and free the permit on its own.
    let served = queued_client_served_after(addr, Instant::now()).await;
    assert!(served < Duration::from_millis(1000), "permit released after {served:?}");
    drop(first);
}

// ---------------------------------------------------------------------------
// Timeouts (slowloris)
// ---------------------------------------------------------------------------

fn short_timeouts() -> ServerOptions {
    ServerOptions::new()
        .headers_timeout(Duration::from_millis(600))
        .request_timeout(Duration::from_millis(600))
        .keep_alive_timeout(Duration::from_millis(300))
}

#[corvis::test]
async fn silent_and_slow_header_connections_are_closed() {
    let addr = serve(Some(short_timeouts())).await;
    // A connection that sends nothing.
    let mut silent = TcpStream::connect(addr).await.unwrap();
    // Headers dripped byte by byte.
    let mut drip = TcpStream::connect(addr).await.unwrap();
    let started = Instant::now();
    for b in b"GET /t/slow HTTP/1.1\r\nHost: x\r\n".iter().take(6) {
        let _ = drip.write_all(&[*b]).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    assert!(closed_within(&mut silent, Duration::from_secs(3)).await, "silent connection kept open");
    assert!(closed_within(&mut drip, Duration::from_secs(3)).await, "dripping connection kept open");
    assert!(started.elapsed() < Duration::from_secs(4));
    // An HTTP/2 prior-knowledge connection that stalls after the preface is closed too.
    let mut h2 = TcpStream::connect(addr).await.unwrap();
    h2.write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n").await.unwrap();
    assert!(closed_within(&mut h2, Duration::from_secs(4)).await, "idle h2c connection kept open");
}

#[corvis::test]
async fn idle_keep_alive_connection_is_closed_but_slow_handlers_are_not() {
    let addr = serve(Some(short_timeouts())).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    // The handler takes 800 ms, longer than every timeout: a request in flight is not idle.
    s.write_all(b"GET /t/slow HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 200"), "{r:?}");
    assert!(closed_within(&mut s, Duration::from_secs(2)).await, "idle keep-alive connection kept open");
}

#[corvis::test]
async fn incomplete_body_gets_408() {
    let addr = serve(Some(short_timeouts())).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(b"POST /t/json HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{\"a\":").await.unwrap();
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 408"), "{r:?}");
    // The same for a multipart upload read by the interceptor.
    let mut s = partial_multipart(addr, "/t/upload", 1000, file_part_head("file", "a").as_bytes()).await;
    let r = read_head(&mut s, Duration::from_secs(5)).await;
    assert!(r.starts_with("HTTP/1.1 408"), "{r:?}");
}

#[corvis::test]
async fn h2c_can_be_disabled() {
    let addr = serve(Some(ServerOptions::new().h2c(false))).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n\x00\x00\x00\x04\x00\x00\x00\x00\x00").await.unwrap();
    let mut buf = [0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(3), s.read(&mut buf)).await.unwrap().unwrap_or(0);
    // No HTTP/2 SETTINGS frame comes back: the connection is closed (or answered as HTTP/1).
    assert!(n == 0 || buf.starts_with(b"HTTP/1.1"), "{:?}", &buf[..n]);
}

#[corvis::test]
async fn max_connections_queues_extra_connections() {
    let addr = serve(Some(ServerOptions::new().max_connections(1))).await;
    let mut first = TcpStream::connect(addr).await.unwrap();
    first.write_all(b"GET /t/slow HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    assert!(read_head(&mut first, Duration::from_secs(5)).await.starts_with("HTTP/1.1 200"));
    let mut second = TcpStream::connect(addr).await.unwrap();
    second.write_all(b"GET /t/slow HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    // Not served while the first connection is open...
    assert_eq!(read_head(&mut second, Duration::from_millis(1500)).await, "");
    drop(first);
    // ...and served once it closes.
    assert!(read_head(&mut second, Duration::from_secs(5)).await.starts_with("HTTP/1.1 200"));
}

// ---------------------------------------------------------------------------
// Disk storage
// ---------------------------------------------------------------------------

/// Before: the file stayed on disk although `ParseFilePipe` rejected the upload.
#[corvis::test]
async fn files_are_removed_when_a_pipe_or_the_handler_fails() {
    let app = app().await;
    let r = request(&app).post("/t/disk-validated").attach("file", vec![b'x'; 50], "a.bin").send().await;
    r.expect_status(400);
    assert!(files_in(&upload_dir("validated")).is_empty(), "{:?}", files_in(&upload_dir("validated")));
    let r = request(&app).post("/t/disk-handler-fails").attach("file", "x", "a.bin").send().await;
    r.expect_status(400);
    assert!(files_in(&upload_dir("handler")).is_empty());
    // Opt-out: `remove_files_on_error(false)` keeps them (multer behavior).
    let r = request(&app).post("/t/disk-keep").attach("file", "x", "a.bin").send().await;
    r.expect_status(400);
    assert_eq!(files_in(&upload_dir("keep")).len(), 1);
    let _ = std::fs::remove_dir_all(upload_dir("keep"));
}

/// Before: `filename="../../x"` with `preserve_path(true)` was written outside the destination.
#[corvis::test]
async fn disk_storage_stays_inside_the_destination() {
    let app = app().await;
    let dir = upload_dir("preserve");
    let marker = format!("escape-{}.txt", std::process::id());
    let r = request(&app).post("/t/disk-preserve").attach("file", "PWNED", &format!("../{marker}")).send().await;
    r.expect_status(400).expect_json(json!({ "message": "Invalid file name", "error": "Bad Request", "statusCode": 400 }));
    let outside = std::path::Path::new(&dir).parent().unwrap().join(&marker);
    assert!(!outside.exists());
    let r = request(&app).post("/t/disk-preserve").attach("file", "ok", "sub-ok.txt").send().await;
    r.expect_status(201);
    let _ = std::fs::remove_dir_all(dir);
}

/// Before (Windows): `normal.txt:evil` wrote the content into a hidden alternate data stream.
#[cfg(windows)]
#[corvis::test]
async fn disk_storage_rejects_streams_and_device_names_on_windows() {
    let app = app().await;
    let dir = upload_dir("raw");
    for name in ["normal.txt:evil", "CON", "nul.txt"] {
        let r = request(&app).post("/t/disk-raw").attach("file", "PWNED", name).send().await;
        if name.contains(':') {
            r.expect_status(400);
        } else {
            // `123-CON` is a regular name: only a bare device name is refused.
            r.expect_status(201);
        }
    }
    assert!(!std::path::Path::new(&dir).join("123-normal.txt").exists());
    let _ = std::fs::remove_dir_all(dir);
}
