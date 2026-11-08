//! HTTP/1 framing checks (`http/request-parsing`, change `e0-reject-ambiguous-http1-framing`): inputs that two parsers
//! can read differently are answered with `400` and end the connection, as Node's `http` module does, and nothing sent
//! after them is processed. Raw sockets on 127.0.0.1.
//!
//! `fixtures/http1_framing/node-24.13.0.json` holds what Node 24 (llhttp 9.3.0) answered to the same requests
//! (`openspec/changes/e0-reject-ambiguous-http1-framing/node-probe/probe.mjs --json --path /t/echo`). Node is not needed
//! to run this file, only to regenerate the fixture.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use corvis::ServerOptions;
use corvis::prelude::*;
use corvis::testing::request;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

static COUNTS: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);

fn count_of(key: &str) -> usize {
    COUNTS.lock().unwrap().as_ref().and_then(|m| m.get(key).copied()).unwrap_or(0)
}

fn bump(key: &str) {
    *COUNTS.lock().unwrap().get_or_insert_with(HashMap::new).entry(key.to_string()).or_default() += 1;
}

#[controller("t")]
pub struct TController;

#[routes]
impl TController {
    #[get("echo")]
    fn echo_get(&self, #[req] req: Request) -> String {
        String::from_utf8_lossy(req.raw_body()).into_owned()
    }

    #[post("echo")]
    #[http_code(200)]
    fn echo_post(&self, #[req] req: Request) -> String {
        String::from_utf8_lossy(req.raw_body()).into_owned()
    }

    /// Counts its calls, whatever the method.
    #[get("count/:key")]
    fn count_get(&self, #[param("key")] key: String) -> String {
        bump(&key);
        "counted".into()
    }

    #[post("count/:key")]
    #[http_code(200)]
    fn count_post(&self, #[param("key")] key: String) -> String {
        bump(&key);
        "counted".into()
    }

    #[post("json")]
    #[http_code(200)]
    fn json(&self, #[body] body: Value) -> Value {
        body
    }

    /// A response body that arrives in several chunks, over about 400 ms.
    #[get("stream")]
    fn stream(&self) -> Response {
        let chunks = futures::stream::unfold(0u8, |i| async move {
            if i == 5 {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(80)).await;
            Some((Ok(bytes::Bytes::from(format!("part{i};"))), i + 1))
        });
        Response::stream(Box::pin(chunks), "text/plain")
    }
}

#[module(controllers = [TController])]
pub struct AppModule;

async fn serve(options: Option<ServerOptions>) -> SocketAddr {
    let module = corvis::testing::Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    if let Some(o) = options {
        app.set_server_options(o);
    }
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    std::mem::forget(app);
    addr
}

fn insecure() -> ServerOptions {
    ServerOptions::new().insecure_http_parser(true)
}

/// Writes `bytes`, reads until the server closes the connection or 1.5 s pass, and returns what arrived.
async fn exchange(addr: SocketAddr, bytes: &[u8]) -> Vec<u8> {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let _ = s.write_all(bytes).await;
    read_all(&mut s, Duration::from_millis(1500)).await
}

async fn read_all(s: &mut TcpStream, wait: Duration) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match tokio::time::timeout_at(deadline, s.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
            Ok(Ok(n)) => out.extend_from_slice(&buf[..n]),
        }
    }
    out
}

/// Every `HTTP/1.x NNN` in what arrived, as the probe collects them.
fn statuses_of(out: &[u8]) -> Vec<u16> {
    let mut found = Vec::new();
    let mut i = 0;
    while i + 12 <= out.len() {
        if out[i..].starts_with(b"HTTP/1.")
            && matches!(out[i + 7], b'0' | b'1')
            && out[i + 8] == b' '
            && let Some(n) = std::str::from_utf8(&out[i + 9..i + 12]).ok().and_then(|s| s.parse::<u16>().ok())
        {
            found.push(n);
            i += 12;
            continue;
        }
        i += 1;
    }
    found
}

async fn statuses(addr: SocketAddr, bytes: &[u8]) -> Vec<u16> {
    statuses_of(&exchange(addr, bytes).await)
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

fn get(path: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
}

/// A `400`, or the connection closed without a response.
fn rejected(statuses: &[u16]) -> bool {
    statuses.is_empty() || statuses == [400]
}

// ---------------------------------------------------------------------------
// Smoke
// ---------------------------------------------------------------------------

#[corvis::test]
async fn valid_get_is_answered() {
    let addr = serve(None).await;
    assert_eq!(statuses(addr, get("/t/echo").as_bytes()).await, [200]);
}

// ---------------------------------------------------------------------------
// Differential against Node
// ---------------------------------------------------------------------------

/// Statuses Node answers that Corvis does not give back, with the reason (design.md, decision 7).
fn expected_difference(id: &str, corvis: &[u16]) -> Option<&'static str> {
    match id {
        // Node parses the whole read before the first response is written and destroys the socket on the error, so the
        // valid first request gets no response. hyper answers one request at a time: `200`, then `400` or the end.
        "pipelined-valid-then-bare-lf" => {
            (corvis == [200] || corvis == [200, 400]).then_some("the first request is answered before the second is rejected")
        }
        // hyper answers `400` to `HTTP/1.0` with `Transfer-Encoding`; Node accepts it.
        "http10-te" => rejected(corvis).then_some("hyper is stricter"),
        // One possible reading each, so not ambiguous framing: Corvis accepts them, Node answers 400 (open question 2).
        "dup-cl-identical" | "te-chunked-twice" => (corvis == [200, 200]).then_some("accepted, a single reading (documented difference)"),
        _ => None,
    }
}

#[corvis::test]
async fn statuses_match_node() {
    let addr = serve(None).await;
    let fixture: Value = serde_json::from_str(include_str!("fixtures/http1_framing/node-24.13.0.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 27, "the fixture lists every case of the probe");
    let mut failures = Vec::new();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let bytes: Vec<u8> = case["request"].as_str().unwrap().chars().map(|c| c as u32 as u8).collect();
        let node: Vec<u16> = case["statuses"].as_array().unwrap().iter().map(|s| s.as_u64().unwrap() as u16).collect();
        let ours = statuses(addr, &bytes).await;
        let same = if let Some(_reason) = expected_difference(id, &ours) {
            true
        } else if node == [400] {
            rejected(&ours)
        } else {
            node == ours
        };
        if !same {
            failures.push(format!("{id}: node {node:?}, corvis {ours:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---------------------------------------------------------------------------
// Request head lines end with CRLF
// ---------------------------------------------------------------------------

async fn assert_head_rejected(head: &str) {
    let addr = serve(None).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(head.as_bytes()).await.unwrap();
    let out = read_all(&mut s, Duration::from_secs(3)).await;
    assert_eq!(statuses_of(&out), [400], "{:?}", String::from_utf8_lossy(&out));
    assert!(closed_within(&mut s, Duration::from_secs(2)).await);
}

#[corvis::test]
async fn bare_lf_in_the_request_line_and_headers_is_rejected() {
    assert_head_rejected("GET /t/echo HTTP/1.1\nHost: x\n\n").await;
}

#[corvis::test]
async fn bare_lf_after_the_request_line_only_is_rejected() {
    assert_head_rejected("GET /t/echo HTTP/1.1\nHost: x\r\n\r\n").await;
}

#[corvis::test]
async fn head_ended_by_crlf_lf_is_rejected() {
    assert_head_rejected("GET /t/echo HTTP/1.1\r\nHost: x\r\n\n").await;
}

#[corvis::test]
async fn last_header_ended_by_a_bare_lf_is_rejected() {
    assert_head_rejected("GET /t/echo HTTP/1.1\r\nHost: x\n\r\n").await;
}

#[corvis::test]
async fn empty_lines_before_the_request_line_are_ignored() {
    let addr = serve(None).await;
    assert_eq!(statuses(addr, format!("\r\n{}", get("/t/echo")).as_bytes()).await, [200]);
    assert_eq!(statuses(addr, format!("\n{}", get("/t/echo")).as_bytes()).await, [200]);
    assert_eq!(statuses(addr, format!("\r\n\n\r\n{}", get("/t/echo")).as_bytes()).await, [200]);
}

#[corvis::test]
async fn bare_cr_in_a_header_is_still_rejected() {
    let addr = serve(None).await;
    assert!(rejected(&statuses(addr, b"GET /t/echo HTTP/1.1\r\nHost: x\rX: y\r\n\r\n").await));
}

// ---------------------------------------------------------------------------
// Chunk size line
// ---------------------------------------------------------------------------

const CHUNKED_JSON: &str =
    "POST /t/json HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";

async fn chunked_json(body: &str) -> Vec<u8> {
    let addr = serve(None).await;
    exchange(addr, format!("{CHUNKED_JSON}{body}").as_bytes()).await
}

async fn chunked_sink(key: &str, body: &str) -> Vec<u16> {
    let addr = serve(None).await;
    let req = format!("POST /t/count/{key} HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{body}");
    statuses(addr, req.as_bytes()).await
}

#[corvis::test]
async fn whitespace_after_the_chunk_size_is_rejected() {
    for (key, body) in
        [("cs-space", "2 \r\n{}\r\n0\r\n\r\n"), ("cs-tab", "2\t\r\n{}\r\n0\r\n\r\n"), ("cs-space-ext", "2 ;a=b\r\n{}\r\n0\r\n\r\n")]
    {
        assert!(rejected(&chunked_sink(key, body).await), "{key}");
        assert_eq!(count_of(key), 0, "{key}");
    }
    let out = chunked_json("2 \r\n{}\r\n0\r\n\r\n").await;
    assert!(rejected(&statuses_of(&out)), "{:?}", String::from_utf8_lossy(&out));
}

#[corvis::test]
async fn chunk_extension_without_whitespace_is_accepted() {
    let out = chunked_json("2;a=b\r\n{}\r\n0\r\n\r\n").await;
    let text = String::from_utf8_lossy(&out);
    assert_eq!(statuses_of(&out), [200], "{text:?}");
    assert!(text.ends_with("{}"), "{text:?}");
}

#[corvis::test]
async fn leading_zeros_in_a_chunk_size_are_accepted() {
    let out = chunked_json("00000000000000000000000002\r\n{}\r\n0\r\n\r\n").await;
    assert_eq!(statuses_of(&out), [200], "{:?}", String::from_utf8_lossy(&out));
}

#[corvis::test]
async fn chunk_errors_that_were_already_rejected_stay_rejected() {
    for (name, body) in [
        ("overflow", "10000000000000002\r\n{}\r\n0\r\n\r\n"),
        ("bare-lf", "2\n{}\n0\n\n"),
        ("lf-in-extension", "2;a=b\nc\r\n{}\r\n0\r\n\r\n"),
        ("no-size", "\r\n{}\r\n0\r\n\r\n"),
        ("bad-digit", "zz\r\n{}\r\n0\r\n\r\n"),
        ("no-cr-after-data", "2\r\n{}XX0\r\n\r\n"),
    ] {
        let out = chunked_json(body).await;
        assert!(rejected(&statuses_of(&out)), "{name}: {:?}", String::from_utf8_lossy(&out));
    }
}

// ---------------------------------------------------------------------------
// Content-Length with Transfer-Encoding
// ---------------------------------------------------------------------------

#[corvis::test]
async fn content_length_with_transfer_encoding_is_rejected_in_both_orders() {
    let addr = serve(None).await;
    for (key, order) in [
        ("both-cl-te", "Content-Length: 4\r\nTransfer-Encoding: chunked\r\n"),
        ("both-te-cl", "Transfer-Encoding: chunked\r\nContent-Length: 4\r\n"),
    ] {
        let req = format!("POST /t/count/{key} HTTP/1.1\r\nHost: x\r\n{order}\r\n2\r\nhi\r\n0\r\n\r\n");
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(req.as_bytes()).await.unwrap();
        let out = read_all(&mut s, Duration::from_secs(3)).await;
        assert_eq!(statuses_of(&out), [400], "{key}: {:?}", String::from_utf8_lossy(&out));
        assert!(closed_within(&mut s, Duration::from_secs(2)).await, "{key}");
        assert_eq!(count_of(key), 0, "{key}");
    }
}

// ---------------------------------------------------------------------------
// Trailer section
// ---------------------------------------------------------------------------

#[corvis::test]
async fn bare_lf_between_trailer_fields_is_rejected() {
    let out = chunked_json("2\r\n{}\r\n0\r\nX-A: b\nX-C: d\r\n\r\n").await;
    assert!(rejected(&statuses_of(&out)), "{:?}", String::from_utf8_lossy(&out));
}

#[corvis::test]
async fn bare_lf_lf_in_the_trailer_does_not_serve_the_pipelined_request() {
    let addr = serve(None).await;
    let req = format!(
        "POST /t/count/trailer-lf-lf HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n0\r\nX-A: b\n\nGARBAGE\r\n\r\n{}",
        get("/t/count/trailer-lf-lf-next")
    );
    let got = statuses(addr, req.as_bytes()).await;
    assert!(rejected(&got), "{got:?}");
    assert_eq!(count_of("trailer-lf-lf"), 0);
    assert_eq!(count_of("trailer-lf-lf-next"), 0);
}

#[corvis::test]
async fn valid_trailer_field_is_accepted() {
    let out = chunked_json("2\r\n{}\r\n0\r\nX-A: b\r\n\r\n").await;
    let text = String::from_utf8_lossy(&out);
    assert_eq!(statuses_of(&out), [200], "{text:?}");
    assert!(text.ends_with("{}"), "{text:?}");
    let out = chunked_json("2\r\n{}\r\n0\r\nX-A: b\r\nX-B: c\r\n\r\n").await;
    assert_eq!(statuses_of(&out), [200], "{:?}", String::from_utf8_lossy(&out));
}

// ---------------------------------------------------------------------------
// A framing rejection ends the connection
// ---------------------------------------------------------------------------

#[corvis::test]
async fn rejected_head_followed_by_a_valid_request_is_not_processed() {
    let addr = serve(None).await;
    let req = format!("GET /t/count/rej-head HTTP/1.1\nHost: x\n\n{}", get("/t/count/rej-head"));
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(req.as_bytes()).await.unwrap();
    let out = read_all(&mut s, Duration::from_secs(3)).await;
    assert_eq!(statuses_of(&out), [400], "{:?}", String::from_utf8_lossy(&out));
    assert_eq!(count_of("rej-head"), 0);
}

#[corvis::test]
async fn valid_request_followed_by_a_rejected_one_gets_its_response() {
    let addr = serve(None).await;
    // The first response streams in several chunks: the rejection of the second request waits for it.
    let req = "GET /t/stream HTTP/1.1\r\nHost: x\r\n\r\nGET /t/count/after-stream HTTP/1.1\nHost: x\n\n";
    let out = exchange(addr, req.as_bytes()).await;
    let text = String::from_utf8_lossy(&out).into_owned();
    let got = statuses_of(&out);
    assert!(got == [200, 400] || got == [200], "{text:?}");
    for i in 0..5 {
        assert!(text.contains(&format!("part{i};")), "{text:?}");
    }
    assert_eq!(count_of("after-stream"), 0);
}

#[corvis::test]
async fn valid_request_followed_by_a_rejected_one_with_a_counted_route() {
    let addr = serve(None).await;
    let req = format!("{}GET /t/count/second HTTP/1.1\nHost: x\n\n", "GET /t/count/first HTTP/1.1\r\nHost: x\r\n\r\n");
    let got = statuses(addr, req.as_bytes()).await;
    assert!(got == [200, 400] || got == [200], "{got:?}");
    assert_eq!(count_of("first"), 1);
    assert_eq!(count_of("second"), 0);
}

#[corvis::test]
async fn rejected_chunked_body_followed_by_a_valid_request_is_not_processed() {
    let addr = serve(None).await;
    let req = format!(
        "POST /t/count/rej-chunk HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n2 \r\nhi\r\n0\r\n\r\n{}",
        get("/t/count/rej-chunk-next")
    );
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(req.as_bytes()).await.unwrap();
    let out = read_all(&mut s, Duration::from_secs(3)).await;
    assert!(rejected(&statuses_of(&out)), "{:?}", String::from_utf8_lossy(&out));
    assert_eq!(count_of("rej-chunk"), 0);
    assert_eq!(count_of("rej-chunk-next"), 0);
    assert!(closed_within(&mut s, Duration::from_secs(2)).await);
}

#[corvis::test]
async fn upgrade_request_answered_without_switching_is_followed_by_validation() {
    let addr = serve(None).await;
    let req = "GET /t/count/up-first HTTP/1.1\r\nHost: x\r\nConnection: upgrade\r\nUpgrade: foo\r\n\r\nGET /t/count/up-second HTTP/1.1\nHost: x\n\n";
    let got = statuses(addr, req.as_bytes()).await;
    assert!(got == [200, 400] || got == [200], "{got:?}");
    assert_eq!(count_of("up-first"), 1);
    assert_eq!(count_of("up-second"), 0);
}

#[corvis::test]
async fn valid_requests_after_an_upgrade_request_are_served() {
    let addr = serve(None).await;
    let req = format!("GET /t/count/up-ok HTTP/1.1\r\nHost: x\r\nConnection: upgrade\r\nUpgrade: foo\r\n\r\n{}", get("/t/count/up-ok"));
    assert_eq!(statuses(addr, req.as_bytes()).await, [200, 200]);
    assert_eq!(count_of("up-ok"), 2);
}

/// A pipelined valid stream written one byte at a time is served like one written at once.
#[corvis::test]
async fn requests_split_across_reads_are_served() {
    let addr = serve(None).await;
    let body = "{\"a\":1}";
    let mut stream = Vec::new();
    stream.extend_from_slice(
        format!("POST /t/json HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len())
            .as_bytes(),
    );
    stream.extend_from_slice(b"POST /t/json HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n7;e=1\r\n{\"a\":1}\r\n0\r\nX-T: v\r\n\r\n");
    stream.extend_from_slice(get("/t/echo").as_bytes());
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.set_nodelay(true).unwrap();
    for b in &stream {
        s.write_all(std::slice::from_ref(b)).await.unwrap();
        tokio::task::yield_now().await;
    }
    let out = read_all(&mut s, Duration::from_secs(5)).await;
    assert_eq!(statuses_of(&out), [200, 200, 200], "{:?}", String::from_utf8_lossy(&out));
}

/// Writes `parts` one after the other with a pause between them, so that the server reads them separately.
async fn exchange_in_parts(addr: SocketAddr, parts: &[&[u8]]) -> Vec<u8> {
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.set_nodelay(true).unwrap();
    for part in parts {
        let _ = s.write_all(part).await;
        let _ = s.flush().await;
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    read_all(&mut s, Duration::from_millis(1500)).await
}

const BAD_HEADS: [&str; 4] = [
    "GET /t/echo HTTP/1.1\nHost: x\n\n",
    "GET /t/echo HTTP/1.1\r\nHost: x\r\n\n",
    "GET /t/echo HTTP/1.1\r\nHost: x\rX: y\r\n\r\n",
    "POST /t/echo HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n",
];

/// Empty lines in front of a rejected head must not hide the `400` (hyper keeps a lone `\r\n` buffered as the start
/// of a head).
#[corvis::test]
async fn a_rejection_after_empty_lines_is_still_answered_with_400() {
    let addr = serve(None).await;
    let valid = "GET /t/echo HTTP/1.1\r\nHost: x\r\n\r\n";
    for bad in BAD_HEADS {
        for empty in ["\r\n", "\n", "\r\n\n\r\n"] {
            let one_write = format!("{empty}{bad}");
            assert_eq!(statuses(addr, one_write.as_bytes()).await, [400], "{one_write:?}");
            let got = statuses_of(&exchange_in_parts(addr, &[empty.as_bytes(), bad.as_bytes()]).await);
            assert_eq!(got, [400], "two writes: {empty:?} then {bad:?}");
            // A keep-alive request, then the empty line, then the rejected head.
            let keep_alive = format!("{valid}{empty}{bad}");
            assert_eq!(statuses(addr, keep_alive.as_bytes()).await, [200, 400], "{keep_alive:?}");
            let got = statuses_of(&exchange_in_parts(addr, &[valid.as_bytes(), empty.as_bytes(), bad.as_bytes()]).await);
            assert_eq!(got, [200, 400], "three writes: {empty:?} then {bad:?}");
        }
    }
}

/// hyper's server reads `Content-Length` with `from_digits`: a list is a `400` there, and the check hands it over.
#[corvis::test]
async fn content_length_lists_are_answered_by_hyper_with_400() {
    let addr = serve(None).await;
    for value in ["3, 3", "3,3", "3, 4", "3,"] {
        let req = format!("POST /t/count/cl-list HTTP/1.1\r\nHost: x\r\nContent-Length: {value}\r\nConnection: close\r\n\r\nabc");
        assert_eq!(statuses(addr, req.as_bytes()).await, [400], "{value}");
    }
    assert_eq!(count_of("cl-list"), 0);
    // Two identical header lines are one reading for hyper too.
    let req = "POST /t/echo HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc";
    let out = exchange(addr, req.as_bytes()).await;
    assert_eq!(statuses_of(&out), [200]);
    assert!(String::from_utf8_lossy(&out).ends_with("abc"));
}

/// The real path (`TimedIo` and hyper's `auto::Builder`) with pipelined input cut at every byte.
#[corvis::test]
async fn pipelined_streams_cut_at_every_byte_behave_the_same() {
    let addr = serve(None).await;
    let body = "{\"a\":1}";
    let mut valid = Vec::new();
    valid.extend_from_slice(b"\r\n");
    valid.extend_from_slice(
        format!("POST /t/json HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len())
            .as_bytes(),
    );
    valid.extend_from_slice(b"POST /t/json HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n7;e=1\r\n{\"a\":1}\r\n0\r\nX-T: v\r\n\r\n");
    valid.extend_from_slice(get("/t/echo").as_bytes());
    let mut rejected = valid[..valid.len() - get("/t/echo").len()].to_vec();
    rejected.extend_from_slice(b"\r\nGET /t/echo HTTP/1.1\nHost: x\n\n");
    rejected.extend_from_slice(get("/t/count/every-byte").as_bytes());
    for at in 1..valid.len() {
        let got = statuses_of(&exchange_in_parts(addr, &[&valid[..at], &valid[at..]]).await);
        assert_eq!(got, [200, 200, 200], "valid stream cut at {at}");
    }
    for at in 1..rejected.len() {
        let got = statuses_of(&exchange_in_parts(addr, &[&rejected[..at], &rejected[at..]]).await);
        assert_eq!(got, [200, 200, 400], "rejected stream cut at {at}");
    }
    assert_eq!(count_of("every-byte"), 0);
}

#[corvis::test]
async fn in_memory_requests_are_unaffected() {
    let module = corvis::testing::Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    let r = request(&app).get("/t/echo").send().await;
    assert_eq!(r.status(), 200);
}

// ---------------------------------------------------------------------------
// Opt-out
// ---------------------------------------------------------------------------

#[corvis::test]
async fn bare_lf_is_accepted_with_the_option_and_the_next_request_is_served() {
    let addr = serve(Some(insecure())).await;
    let req = format!("GET /t/count/opt-lf HTTP/1.1\nHost: x\n\n{}", get("/t/count/opt-lf"));
    assert_eq!(statuses(addr, req.as_bytes()).await, [200, 200]);
    assert_eq!(count_of("opt-lf"), 2);
}

#[corvis::test]
async fn ambiguous_chunked_framing_is_accepted_with_the_option() {
    let addr = serve(Some(insecure())).await;
    for (name, body) in [
        ("space", "2 \r\n{}\r\n0\r\n\r\n"),
        ("tab", "2\t\r\n{}\r\n0\r\n\r\n"),
        ("space-ext", "2 ;a=b\r\n{}\r\n0\r\n\r\n"),
        ("trailer-lf-lf", "2\r\n{}\r\n0\r\nX-A: b\n\nGARBAGE\r\n\r\n"),
    ] {
        let out = exchange(addr, format!("{CHUNKED_JSON}{body}").as_bytes()).await;
        let text = String::from_utf8_lossy(&out);
        assert_eq!(statuses_of(&out)[..1], [200], "{name}: {text:?}");
        assert!(text.contains("{}"), "{name}: {text:?}");
    }
}

#[corvis::test]
async fn both_length_headers_are_decoded_as_chunked_with_the_option() {
    let addr = serve(Some(insecure())).await;
    let req = "POST /t/json HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n";
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(req.as_bytes()).await.unwrap();
    let out = read_all(&mut s, Duration::from_secs(3)).await;
    let text = String::from_utf8_lossy(&out);
    assert_eq!(statuses_of(&out), [200], "{text:?}");
    assert!(text.ends_with("{}"), "{text:?}");
    assert!(closed_within(&mut s, Duration::from_secs(2)).await);
}

#[corvis::test]
async fn errors_that_hyper_rejects_stay_rejected_with_the_option() {
    let addr = serve(Some(insecure())).await;
    assert!(rejected(&statuses(addr, b"GET /t/echo HTTP/1.1\r\nHost: x\rX: y\r\n\r\n").await));
    for body in ["10000000000000002\r\n{}\r\n0\r\n\r\n", "2\n{}\n0\n\n", "2;a=b\nc\r\n{}\r\n0\r\n\r\n"] {
        let out = exchange(addr, format!("{CHUNKED_JSON}{body}").as_bytes()).await;
        assert!(rejected(&statuses_of(&out)), "{body:?}: {:?}", String::from_utf8_lossy(&out));
    }
}
