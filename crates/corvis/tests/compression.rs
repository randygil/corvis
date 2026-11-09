//! Response compression (`compression` feature): the scenarios of the `http/compression` spec through
//! the test client and a listening server, and the replay of the requests recorded against the
//! original stack (`fixtures/compression/compression.json`, see `generate.mjs`).
#![cfg(feature = "compression")]

use std::collections::HashMap;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bytes::Bytes;
use corvis::http::compression::{CompressionOptions, should_compress};
use corvis::prelude::*;
use corvis::testing::{Test, TestResponse, request};
use corvis::{Res, serve_static};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// ---- Content shared with fixtures/compression/generate.mjs ----

const FOX: &str = "The quick brown fox jumps over the lazy dog. ";

fn text(n: usize) -> String {
    FOX.repeat(n.div_ceil(FOX.len())).chars().take(n).collect()
}

fn bin(n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * 31 + 7) % 251) as u8).collect()
}

fn gzip(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn chunks(sizes: &[usize]) -> corvis::http::ByteStream {
    let items: Vec<Result<Bytes, std::io::Error>> = sizes.iter().map(|n| Ok(Bytes::from(text(*n)))).collect();
    Box::pin(futures::stream::iter(items))
}

// ---- The routes of the replay ----

#[controller]
pub struct Api;

#[routes]
impl Api {
    #[get("json-small")]
    fn json_small(&self) -> Value {
        json!({ "d": text(92) })
    }
    #[get("json-1023")]
    fn json_1023(&self) -> Value {
        json!({ "d": text(1015) })
    }
    #[get("json-1024")]
    fn json_1024(&self) -> Value {
        json!({ "d": text(1016) })
    }
    #[get("json-large")]
    fn json_large(&self) -> Value {
        json!({ "d": text(10232) })
    }
    #[get("html-large")]
    fn html_large(&self) -> String {
        text(10240)
    }
    #[get("text-large")]
    fn text_large(&self, #[res] res: Res) {
        res.type_("text/plain").send(text(10240));
    }
    #[get("png")]
    fn png(&self, #[res] res: Res) {
        res.type_("image/png").send(Bytes::from(bin(10240)));
    }
    #[get("octet")]
    fn octet(&self, #[res] res: Res) {
        res.type_("application/octet-stream").send(Bytes::from(bin(10240)));
    }
    #[get("no-transform")]
    fn no_transform(&self) -> Response {
        Response::json(json!({ "d": text(10232) })).with_header("cache-control", "private, no-transform")
    }
    #[get("encoded")]
    fn encoded(&self, #[res] res: Res) {
        res.set("Content-Encoding", "gzip").type_("json").send(Bytes::from(gzip(text(10240).as_bytes())));
    }
    #[get("vary-origin")]
    fn vary_origin(&self) -> Response {
        Response::json(json!({ "d": text(10232) })).with_header("vary", "Origin")
    }
    #[get("vary-star")]
    fn vary_star(&self) -> Response {
        Response::json(json!({ "d": text(10232) })).with_header("vary", "*")
    }
    #[get("stream-small")]
    fn stream_small(&self) -> Response {
        Response::stream(chunks(&[40, 30, 30]), "text/plain; charset=utf-8")
    }
    #[get("stream-large")]
    fn stream_large(&self) -> Response {
        Response::stream(chunks(&[4096, 4096, 4096]), "text/plain; charset=utf-8")
    }
    #[get("no-content")]
    #[http_code(204)]
    fn no_content(&self) {}
    #[sse("sse")]
    fn sse(&self) -> Sse {
        Sse::new(futures::stream::iter((0..3).map(|i| MessageEvent::new(format!("event {i}")))))
    }
    /// Sends the first event at once and the second one second later.
    #[sse("sse-slow")]
    fn sse_slow(&self) -> Sse {
        Sse::new(futures::stream::unfold(0u32, |i| async move {
            if i == 1 {
                tokio::time::sleep(Duration::from_millis(1500)).await;
            }
            (i < 2).then(|| (MessageEvent::new(format!("event {i}")), i + 1))
        }))
    }
    /// Two 2 KiB chunks, one second apart.
    #[get("slow")]
    fn slow(&self) -> Response {
        let s = futures::stream::unfold(0u32, |i| async move {
            if i == 1 {
                tokio::time::sleep(Duration::from_millis(1500)).await;
            }
            (i < 2).then(|| (Ok::<_, std::io::Error>(Bytes::from(text(2048))), i + 1))
        });
        Response::stream(Box::pin(s), "text/plain")
    }
}

#[module(controllers = [Api])]
pub struct ApiModule;

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// A folder served at `/static`: `big.txt` (100 KiB), `small.txt`, `big.json` (50 KiB),
/// `huge.json` (5 MiB), `photo.png` (100 KiB).
struct Files {
    root: PathBuf,
}

impl Files {
    fn new() -> Files {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("corvis-compression-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("big.txt"), text(102_400)).unwrap();
        std::fs::write(root.join("small.txt"), text(100)).unwrap();
        std::fs::write(root.join("big.json"), serde_json::to_vec(&json!({ "d": text(51_192) })).unwrap()).unwrap();
        std::fs::write(root.join("huge.json"), huge_json()).unwrap();
        std::fs::write(root.join("photo.png"), bin(102_400)).unwrap();
        Files { root }
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn huge_json() -> Vec<u8> {
    let mut s = String::from("{\"rows\":[");
    let mut i = 0;
    while s.len() < 5 * 1024 * 1024 - 40 {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("{{\"id\":{i},\"name\":\"row {i}\",\"note\":\"{}\"}}", FOX.trim()));
        i += 1;
    }
    s.push_str("]}");
    s.into_bytes()
}

async fn app(files: &Path, options: Option<CompressionOptions>, cors: bool) -> CorvisApplication {
    let module = Test::create_testing_module().controller::<Api>().with_logger(false).compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.silence_route_logs();
    if let Some(o) = options {
        app.enable_compression(o);
    }
    if cors {
        app.enable_cors(CorsOptions::new().reflect_origin());
    }
    app.use_middleware(serve_static(files).mount("/static"));
    app.init().await.unwrap();
    app
}

// ---- Decoding ----

fn decode(encoding: Option<&str>, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    match encoding {
        Some("gzip") => flate2::read::GzDecoder::new(data).read_to_end(&mut out).unwrap(),
        Some("deflate") => flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).unwrap(),
        Some("br") => brotli::Decompressor::new(data, 4096).read_to_end(&mut out).unwrap(),
        None => {
            out.extend_from_slice(data);
            data.len()
        }
        Some(other) => panic!("unexpected encoding {other}"),
    };
    out
}

fn decoded(r: &TestResponse) -> Vec<u8> {
    decode(r.header("content-encoding"), &r.body)
}

async fn get(app: &CorvisApplication, path: &str, accept: Option<&str>) -> TestResponse {
    let c = request(app);
    let mut req = c.get(path);
    if let Some(a) = accept {
        req = req.header("accept-encoding", a);
    }
    req.send().await
}

// ---- Scenarios through the test client ----

#[corvis::test]
async fn not_enabled_leaves_responses_alone() {
    let files = Files::new();
    let app = app(&files.root, None, false).await;
    let r = get(&app, "/json-large", Some("gzip")).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.header("content-encoding"), None);
    assert_eq!(r.header("vary"), None);
    assert_eq!(r.header("content-length"), Some("10240"));
}

#[corvis::test]
async fn compresses_json_with_etag() {
    let files = Files::new();
    let on = app(&files.root, Some(CompressionOptions::new()), false).await;
    let off = app(&files.root, None, false).await;
    let plain = get(&off, "/json-large", Some("gzip")).await;
    let r = get(&on, "/json-large", Some("gzip")).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.header("content-encoding"), Some("gzip"));
    assert_eq!(r.header("content-length"), None);
    assert_eq!(r.header("vary"), Some("Accept-Encoding"));
    assert!(r.header("etag").is_some_and(|e| e.starts_with("W/\"2800-")));
    assert_eq!(r.header("etag"), plain.header("etag"));
    assert_eq!(r.header("content-type"), plain.header("content-type"));
    assert_eq!(decoded(&r), plain.body.to_vec());
    assert!(r.body.len() < 1000);
}

#[corvis::test]
async fn untouched_responses() {
    let files = Files::new();
    let app = app(&files.root, Some(CompressionOptions::new()), false).await;
    for path in ["/png", "/no-transform", "/sse"] {
        let r = get(&app, path, Some("gzip")).await;
        assert_eq!(r.header("content-encoding"), None, "{path}");
        assert_eq!(r.header("vary"), None, "{path}");
    }
    let r = get(&app, "/png", Some("gzip")).await;
    assert_eq!(r.body.to_vec(), bin(10240));
    let r = get(&app, "/octet", Some("gzip")).await;
    assert_eq!(r.header("content-encoding"), Some("gzip"));
    assert_eq!(decoded(&r), bin(10240));
    assert_eq!(r.header("vary"), Some("Accept-Encoding"));
}

#[corvis::test]
async fn vary_goes_before_the_other_checks() {
    let files = Files::new();
    let app = app(&files.root, Some(CompressionOptions::new()), true).await;
    let r = get(&app, "/json-small", None).await;
    assert_eq!(r.header("content-encoding"), None);
    assert!(r.header("vary").unwrap().ends_with("Accept-Encoding"));
    let r = get(&app, "/json-large", None).await;
    assert_eq!(r.header("content-encoding"), None);
    assert!(r.header("vary").unwrap().contains("Accept-Encoding"));
    // CORS sets `Vary: Origin` on a request with an Origin header
    let c = request(&app);
    let r = c.get("/json-large").header("origin", "https://app.example").header("accept-encoding", "gzip").send().await;
    assert_eq!(r.header("content-encoding"), Some("gzip"));
    assert_eq!(r.header("vary"), Some("Origin, Accept-Encoding"));
    // Vary: * stays
    let r = get(&app, "/vary-star", Some("gzip")).await;
    assert_eq!(r.header("vary"), Some("*"));
    assert_eq!(r.header("content-encoding"), Some("gzip"));
    let r = get(&app, "/vary-origin", Some("gzip")).await;
    assert_eq!(r.header("vary"), Some("Origin, Accept-Encoding"));
}

#[corvis::test]
async fn threshold_and_streams() {
    let files = Files::new();
    let app = app(&files.root, Some(CompressionOptions::new()), false).await;
    let r = get(&app, "/json-1023", Some("gzip")).await;
    assert_eq!(r.header("content-encoding"), None);
    assert_eq!(r.header("vary"), Some("Accept-Encoding"));
    let r = get(&app, "/json-1024", Some("gzip")).await;
    assert_eq!(r.header("content-encoding"), Some("gzip"));
    // A stream without length is compressed whatever its size.
    let r = get(&app, "/stream-small", Some("gzip")).await;
    assert_eq!(r.header("content-encoding"), Some("gzip"));
    assert_eq!(String::from_utf8(decoded(&r)).unwrap(), format!("{}{}{}", text(40), text(30), text(30)));
    // a custom threshold
    let big = self::app(&files.root, Some(CompressionOptions::new().threshold(20_000)), false).await;
    assert_eq!(get(&big, "/json-large", Some("gzip")).await.header("content-encoding"), None);
    let none = self::app(&files.root, Some(CompressionOptions::new().threshold(0)), false).await;
    assert_eq!(get(&none, "/json-small", Some("gzip")).await.header("content-encoding"), Some("gzip"));
}

#[corvis::test]
async fn head_range_and_already_encoded() {
    let files = Files::new();
    let app = app(&files.root, Some(CompressionOptions::new()), false).await;
    let c = request(&app);
    let r = c.head("/json-large").header("accept-encoding", "gzip").send().await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.header("content-encoding"), None);
    assert_eq!(r.header("vary"), Some("Accept-Encoding"));
    assert_eq!(r.header("content-length"), Some("10240"));
    assert!(r.body.is_empty());

    let r = c.get("/static/big.txt").header("range", "bytes=0-49999").header("accept-encoding", "gzip").send().await;
    assert_eq!(r.status(), 206);
    assert_eq!(r.header("content-range"), Some("bytes 0-49999/102400"));
    assert_eq!(r.header("content-encoding"), None);
    assert_eq!(r.body.len(), 50_000);
    let r = c.get("/static/big.txt").header("range", "bytes=999999-").header("accept-encoding", "gzip").send().await;
    assert_eq!(r.status(), 416);
    assert_eq!(r.header("content-encoding"), None);

    let r = get(&app, "/encoded", Some("gzip")).await;
    assert_eq!(r.header("content-encoding"), Some("gzip"));
    assert_eq!(r.header("vary"), Some("Accept-Encoding"));
    assert_eq!(decode(Some("gzip"), &r.body), text(10240).as_bytes());
}

#[corvis::test]
async fn negotiation() {
    let files = Files::new();
    let app = app(&files.root, Some(CompressionOptions::new()), false).await;
    let plain = text(10232);
    for (accept, expected) in [
        (None, None),
        (Some("identity"), None),
        (Some("*"), Some("br")),
        (Some("gzip, deflate, br"), Some("br")),
        (Some("br;q=0.5, gzip"), Some("gzip")),
        (Some("deflate"), Some("deflate")),
        (Some("gzip;q=0, deflate"), Some("deflate")),
        (Some("identity;q=0, gzip;q=0"), None),
        (Some("compress"), None),
    ] {
        let r = get(&app, "/json-large", accept).await;
        assert_eq!(r.status(), 200, "{accept:?}");
        assert_eq!(r.header("content-encoding"), expected, "{accept:?}");
        let body = String::from_utf8(decoded(&r)).unwrap();
        assert!(body.contains(&plain), "{accept:?}");
    }
    // deflate is the zlib format (RFC 1950): CMF 0x78 and a valid header checksum
    let r = get(&app, "/json-large", Some("deflate")).await;
    assert_eq!(r.body[0], 0x78);
    assert_eq!((u16::from(r.body[0]) * 256 + u16::from(r.body[1])) % 31, 0);
}

#[corvis::test]
async fn custom_filter_and_options() {
    let files = Files::new();
    let filter = CompressionOptions::new().filter(|ctx| !ctx.request_headers.contains_key("x-no-compression") && should_compress(ctx));
    let app = app(&files.root, Some(filter), false).await;
    let c = request(&app);
    let r = c.get("/json-large").header("accept-encoding", "gzip").header("x-no-compression", "1").send().await;
    assert_eq!(r.header("content-encoding"), None);
    let r = c.get("/json-large").header("accept-encoding", "gzip").send().await;
    assert_eq!(r.header("content-encoding"), Some("gzip"));
    // the filter sees the method, path and response status
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    let spy = CompressionOptions::new().filter(move |ctx| {
        log.lock().unwrap().push(format!("{} {} {}", ctx.method, ctx.path, ctx.status.as_u16()));
        should_compress(ctx)
    });
    let app = self::app(&files.root, Some(spy), false).await;
    get(&app, "/json-large?x=1", Some("gzip")).await;
    assert_eq!(seen.lock().unwrap().as_slice(), ["GET /json-large 200"]);

    // levels and qualities out of range are clamped and still produce valid streams
    let app = self::app(&files.root, Some(CompressionOptions::new().level(20).brotli_quality(50)), false).await;
    for accept in ["gzip", "br", "deflate"] {
        let r = get(&app, "/json-large", Some(accept)).await;
        assert_eq!(r.header("content-encoding"), Some(accept));
        assert!(decoded(&r).starts_with(b"{\"d\":\"The quick"));
    }
    let o = CompressionOptions::new().level(20).brotli_quality(50);
    assert_eq!((o.gzip_level(), o.quality()), (9, 11));
}

#[corvis::test]
async fn quality_changes_the_size_not_the_content() {
    let files = Files::new();
    let sizes: Vec<usize> = {
        let mut v = Vec::new();
        for q in [0, 11] {
            let app = app(&files.root, Some(CompressionOptions::new().brotli_quality(q)), false).await;
            let r = get(&app, "/static/big.json", Some("br")).await;
            assert_eq!(decoded(&r), std::fs::read(files.root.join("big.json")).unwrap());
            v.push(r.body.len());
        }
        v
    };
    assert!(sizes[1] < sizes[0], "{sizes:?}");
}

#[corvis::test]
async fn large_static_file_is_compressed_and_complete() {
    let files = Files::new();
    let app = app(&files.root, Some(CompressionOptions::new()), false).await;
    let original = std::fs::read(files.root.join("huge.json")).unwrap();
    assert!(original.len() >= 5 * 1024 * 1024 - 100);
    for accept in ["gzip", "br", "deflate"] {
        let r = get(&app, "/static/huge.json", Some(accept)).await;
        assert_eq!(r.status(), 200, "{accept}");
        assert_eq!(r.header("content-encoding"), Some(accept));
        assert_eq!(r.header("content-length"), None);
        assert_eq!(r.header("accept-ranges"), Some("bytes"));
        assert!(r.header("etag").is_some());
        assert!(decoded(&r) == original, "{accept}");
        assert!(r.body.len() < original.len() / 4);
    }
    let r = get(&app, "/static/photo.png", Some("gzip")).await;
    assert_eq!(r.header("content-encoding"), None);
    assert_eq!(r.header("vary"), None);
}

// ---- A listening server ----

struct RawResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl RawResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

type Head = (usize, u16, Vec<(String, String)>);

fn split_head(buf: &[u8]) -> Option<Head> {
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split(' ').nth(1)?.parse().ok()?;
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
    Some((end + 4, status, headers))
}

fn dechunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(line_end) = data.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(std::str::from_utf8(&data[..line_end]).unwrap().split(';').next().unwrap().trim(), 16).unwrap();
        data = &data[line_end + 2..];
        if size == 0 {
            break;
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
    out
}

/// One request on its own connection, answered until the connection closes.
async fn raw(addr: SocketAddr, method: &str, path: &str, headers: &[(&str, &str)]) -> RawResponse {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), s.read_to_end(&mut buf)).await.expect("response in time").unwrap();
    let (body_at, status, headers) = split_head(&buf).expect("a response head");
    let raw_body = &buf[body_at..];
    let chunked = headers.iter().any(|(k, v)| k == "transfer-encoding" && v.contains("chunked"));
    let body = if chunked { dechunk(raw_body) } else { raw_body.to_vec() };
    RawResponse { status, headers, body }
}

#[corvis::test]
async fn test_client_and_server_agree() {
    let files = Files::new();
    let mut app = app(&files.root, Some(CompressionOptions::new()), false).await;
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    for path in ["/json-large", "/html-large", "/json-small", "/png", "/stream-large", "/static/big.txt"] {
        let t = get(&app, path, Some("gzip")).await;
        let s = raw(addr, "GET", path, &[("Accept-Encoding", "gzip")]).await;
        assert_eq!(s.status, t.status(), "{path}");
        assert_eq!(s.header("content-encoding"), t.header("content-encoding"), "{path}");
        assert_eq!(s.header("vary"), t.header("vary"), "{path}");
        assert!(decode(s.header("content-encoding"), &s.body) == decoded(&t), "{path}");
    }
    app.close().await.unwrap();
}

#[corvis::test]
async fn server_sent_events_are_not_compressed_and_arrive_at_once() {
    let files = Files::new();
    let mut app = app(&files.root, Some(CompressionOptions::new()), false).await;
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(b"GET /sse-slow HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: gzip, br\r\n\r\n").await.unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let started = std::time::Instant::now();
    while !String::from_utf8_lossy(&buf).contains("event 0") {
        let n = tokio::time::timeout(Duration::from_secs(5), s.read(&mut tmp)).await.unwrap().unwrap();
        assert!(n > 0, "closed early");
        buf.extend_from_slice(&tmp[..n]);
    }
    assert!(started.elapsed() < Duration::from_millis(1200), "the first event waited for the second");
    let head = String::from_utf8_lossy(&buf).to_ascii_lowercase();
    assert!(head.starts_with("http/1.1 200"), "{head}");
    assert!(head.contains("content-type: text/event-stream"), "{head}");
    assert!(!head.contains("content-encoding"), "{head}");
    assert!(!head.contains("vary:"), "{head}");
    drop(s);
    app.close().await.unwrap();
}

#[corvis::test]
async fn a_slow_stream_is_decodable_before_it_ends() {
    let files = Files::new();
    let mut app = app(&files.root, Some(CompressionOptions::new()), false).await;
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    for accept in ["gzip", "br", "deflate"] {
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(format!("GET /slow HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: {accept}\r\n\r\n").as_bytes()).await.unwrap();
        let started = std::time::Instant::now();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        let first = text(2048);
        let payload = loop {
            let n = tokio::time::timeout(Duration::from_secs(5), s.read(&mut tmp)).await.unwrap().unwrap();
            assert!(n > 0, "closed before the first chunk");
            buf.extend_from_slice(&tmp[..n]);
            let Some((at, status, headers)) = split_head(&buf) else { continue };
            assert_eq!(status, 200);
            assert!(headers.iter().any(|(k, v)| k == "content-encoding" && v == accept), "{headers:?}");
            let payload = partial_dechunk(&buf[at..]);
            if partial_decode(accept, &payload).len() >= first.len() {
                break payload;
            }
        };
        assert!(started.elapsed() < Duration::from_millis(1200), "{accept}: the first chunk waited for the second");
        assert_eq!(partial_decode(accept, &payload), first.as_bytes(), "{accept}");
        // the rest still arrives
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut rest)).await.unwrap().unwrap();
        buf.extend_from_slice(&rest);
        let (at, _, _) = split_head(&buf).unwrap();
        let all = decode(Some(accept), &dechunk(&buf[at..]));
        assert_eq!(all, format!("{}{}", text(2048), text(2048)).as_bytes(), "{accept}");
    }
    app.close().await.unwrap();
}

/// The data of the complete chunks of an unfinished chunked body.
fn partial_dechunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(line_end) = data.windows(2).position(|w| w == b"\r\n") {
        let Ok(size) = usize::from_str_radix(std::str::from_utf8(&data[..line_end]).unwrap_or("x").trim(), 16) else { break };
        if size == 0 || data.len() < line_end + 2 + size + 2 {
            break;
        }
        out.extend_from_slice(&data[line_end + 2..line_end + 2 + size]);
        data = &data[line_end + 2 + size + 2..];
    }
    out
}

fn partial_decode(encoding: &str, data: &[u8]) -> Vec<u8> {
    let mut reader: Box<dyn Read + '_> = match encoding {
        "gzip" => Box::new(flate2::read::GzDecoder::new(data)),
        "deflate" => Box::new(flate2::read::ZlibDecoder::new(data)),
        _ => Box::new(brotli::Decompressor::new(data, 4096)),
    };
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = reader.read(&mut buf) {
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

// ---- Replay of the requests recorded against the original stack ----

fn sha(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

#[corvis::test]
async fn replay_of_the_recorded_requests() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/compression/compression.json")).unwrap();
    assert_eq!(fixture["versions"]["compression"], "1.8.2");
    let files = Files::new();
    let mut servers: HashMap<&str, (CorvisApplication, SocketAddr)> = HashMap::new();
    for (name, options) in [("default", CompressionOptions::new()), ("threshold0", CompressionOptions::new().threshold(0))] {
        let mut a = app(&files.root, Some(options), false).await;
        let addr = a.listen_in_background("127.0.0.1:0").await.unwrap();
        servers.insert(name, (a, addr));
    }
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() > 200);
    let mut compared = 0;
    for case in cases {
        let (_, addr) = servers[case["app"].as_str().unwrap()];
        let method = case["method"].as_str().unwrap();
        let url = case["url"].as_str().unwrap();
        let mut headers: Vec<(String, String)> =
            case["headers"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
        if let Some(ae) = case["ae"].as_str() {
            headers.push(("Accept-Encoding".into(), ae.into()));
        }
        let hs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let r = raw(addr, method, url, &hs).await;
        let label = format!("{} {} {} ae={} {:?}", case["app"], method, url, case["ae"], case["headers"]);
        let divergence = case["divergence"].as_str();
        assert_eq!(i64::from(r.status), case["status"].as_i64().unwrap(), "{label}: status");
        assert_eq!(r.header("vary"), case["vary"].as_str(), "{label}: vary");
        assert_eq!(r.header("content-range"), case["contentRange"].as_str(), "{label}: content-range");
        assert_eq!(r.header("accept-ranges"), case["acceptRanges"].as_str(), "{label}: accept-ranges");
        if case["etagExact"].as_bool().unwrap() {
            assert_eq!(r.header("etag"), case["etag"].as_str(), "{label}: etag");
        } else {
            assert_eq!(r.header("etag").is_some(), case["etag"].is_string(), "{label}: etag presence");
        }
        if divergence == Some("range") {
            // Documented difference: responses with Content-Range are not compressed.
            assert_eq!(r.header("content-encoding"), None, "{label}");
            assert_eq!(r.header("content-length").map(|v| v.parse::<usize>().unwrap()), Some(r.body.len()), "{label}");
            assert_eq!(r.body.len() as u64, case["bodyLength"].as_u64().unwrap(), "{label}: ranged body length");
            continue;
        }
        assert_eq!(r.header("content-encoding"), case["contentEncoding"].as_str(), "{label}: content-encoding");
        assert_eq!(
            r.header("content-length").is_some(),
            case["contentLengthPresent"].as_bool().unwrap(),
            "{label}: content-length presence"
        );
        // The gzip bytes of the `encoded` route come from each stack's own zlib, so their length differs.
        if let Some(len) = case["contentLength"].as_u64().filter(|_| !case["rawBody"].as_bool().unwrap()) {
            assert_eq!(r.header("content-length"), Some(len.to_string()).as_deref(), "{label}: content-length");
        }
        if case["headersOnly"].as_bool().unwrap() || method == "HEAD" || case["bodySha256"].is_null() {
            compared += 1;
            continue;
        }
        // `rawBody` routes carry a Content-Encoding of their own: the recorded body is the gunzipped one.
        let body = decode(r.header("content-encoding"), &r.body);
        assert_eq!(body.len() as u64, case["bodyLength"].as_u64().unwrap(), "{label}: decoded length");
        assert_eq!(sha(&body), case["bodySha256"].as_str().unwrap(), "{label}: decoded body");
        compared += 1;
    }
    assert!(compared > 200, "{compared}");
    for (_, (mut a, _)) in servers {
        a.close().await.unwrap();
    }
}
