//! `ServerOptions::send_timeout`: a client that stops accepting response bytes loses its connection (HTTP/1 and
//! h2c), while slow but live readers, idle Server-Sent Events streams and slow handlers keep theirs. The stalled
//! clients are raw sockets with a tiny receive buffer, so the server's writes block after a few KiB.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use corvis::ServerOptions;
use corvis::http::sse::{MessageEvent, Sse};
use corvis::prelude::*;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpSocket, TcpStream};

const SEND_TIMEOUT: Duration = Duration::from_secs(1);
const HUGE: usize = 8 * 1024 * 1024;
const FOX: &str = "The quick brown fox jumps over the lazy dog. ";

#[controller("t")]
pub struct TController;

#[routes]
impl TController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }

    /// 1 MiB with a length.
    #[get("big")]
    fn big(&self) -> String {
        "a".repeat(1024 * 1024)
    }

    /// 8 MiB with a length.
    #[get("huge")]
    fn huge(&self) -> String {
        "a".repeat(HUGE)
    }

    /// 8 MiB in 64 KiB chunks of bytes that do not compress.
    #[get("rand")]
    fn rand(&self) -> Response {
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        let items: Vec<Result<Bytes, std::io::Error>> = (0..HUGE / 65536)
            .map(|_| {
                let chunk: Vec<u8> = (0..65536)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        // Printable, so the type stays plausible; still about 6 bits of entropy per byte.
                        b' ' + (x >> 32) as u8 % 95
                    })
                    .collect();
                Ok(Bytes::from(chunk))
            })
            .collect();
        Response::stream(Box::pin(futures::stream::iter(items)), "text/plain")
    }

    /// Never ends: 64 KiB chunks as fast as they are taken. The kernel buffers of a loopback connection hold several
    /// MiB, so a response of a fixed size is not enough to block the server's writes.
    #[get("forever")]
    fn forever(&self) -> Response {
        let chunk = Bytes::from(vec![b'x'; 65536]);
        Response::stream(Box::pin(futures::stream::repeat_with(move || Ok::<_, std::io::Error>(chunk.clone()))), "application/octet-stream")
    }

    /// 4 KiB of text that compresses.
    #[get("text")]
    fn text(&self) -> String {
        FOX.repeat(100)
    }

    /// One event, three seconds of nothing, another event.
    #[sse("sse")]
    fn sse(&self) -> Sse {
        Sse::new(futures::stream::unfold(0u32, |i| async move {
            if i == 1 {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            (i < 2).then(|| (MessageEvent::new(format!("event {i}")), i + 1))
        }))
    }

    #[get("slow")]
    async fn slow(&self) -> Value {
        tokio::time::sleep(Duration::from_secs(3)).await;
        json!({ "slow": true })
    }
}

#[module(controllers = [TController])]
pub struct AppModule;

async fn serve(options: ServerOptions) -> SocketAddr {
    let module = corvis::testing::Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.silence_route_logs();
    app.init().await.unwrap();
    app.set_server_options(options);
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    std::mem::forget(app);
    addr
}

/// Options where only `send_timeout` can end a connection that is not misbehaving on the read side, and where a
/// stalled connection holds the only slot, so a second client being served proves it was closed.
fn one_slot(send_timeout: impl Into<Option<Duration>>) -> ServerOptions {
    ServerOptions::new()
        .h2c(true)
        .max_connections(1)
        .send_timeout(send_timeout)
        .keep_alive_timeout(None)
        .headers_timeout(None)
        .request_timeout(None)
}

/// Options for the clients that must not be cut.
fn live(send_timeout: Duration) -> ServerOptions {
    ServerOptions::new().h2c(true).send_timeout(send_timeout).keep_alive_timeout(None).headers_timeout(None).request_timeout(None)
}

/// A connection whose receive buffer holds a few KiB: what the client does not read blocks the server's writes.
async fn tiny_client(addr: SocketAddr) -> TcpStream {
    let socket = TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    socket.connect(addr).await.unwrap()
}

/// A second HTTP/1 client; the time until it is answered, or `None` when `wait` elapses first.
async fn probe(addr: SocketAddr, wait: Duration) -> Option<Duration> {
    let t = Instant::now();
    let served = tokio::time::timeout(wait, async {
        let mut s = TcpStream::connect(addr).await.ok()?;
        s.write_all(b"GET /t/ok HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.ok()?;
        let mut buf = [0u8; 64];
        let n = s.read(&mut buf).await.ok()?;
        (n > 0).then_some(())
    })
    .await;
    matches!(served, Ok(Some(()))).then(|| t.elapsed())
}

/// The second client must be served: the stalled connection was closed by the send timeout.
async fn assert_closed_by_timeout(addr: SocketAddr) {
    let served = probe(addr, Duration::from_secs(15)).await;
    let served = served.expect("the stalled connection kept its slot: a second client was never served");
    assert!(served < Duration::from_secs(12), "{served:?}");
}

// ---- Raw HTTP/2 ----

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const SETTINGS_WINDOW_0: &[u8] = &[0, 4, 0, 0, 0, 0];
const SETTINGS_WINDOW_1: &[u8] = &[0, 4, 0, 0, 0, 1];

fn frame_bytes(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let mut v = vec![(len >> 16) as u8, (len >> 8) as u8, len as u8, kind, flags];
    v.extend_from_slice(&stream.to_be_bytes());
    v.extend_from_slice(payload);
    v
}

/// HPACK block of `GET <path>` over http, authority "x" (static table indexes, no Huffman).
fn request_block(path: &str) -> Vec<u8> {
    let mut b = vec![0x82, 0x86, 0x04, path.len() as u8];
    b.extend_from_slice(path.as_bytes());
    b.extend_from_slice(&[0x01, 0x01, 0x78]);
    b
}

struct RawFrame {
    kind: u8,
    flags: u8,
    stream: u32,
    payload: Vec<u8>,
}

async fn read_frame(rd: &mut OwnedReadHalf) -> Option<RawFrame> {
    let mut head = [0u8; 9];
    rd.read_exact(&mut head).await.ok()?;
    let len = u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize;
    let mut payload = vec![0u8; len];
    rd.read_exact(&mut payload).await.ok()?;
    Some(RawFrame { kind: head[3], flags: head[4], stream: u32::from_be_bytes([head[5] & 0x7f, head[6], head[7], head[8]]), payload })
}

/// A raw h2c client: preface and a SETTINGS frame with `settings` as parameters. With `read`, a task reads every frame
/// (answering PINGs, never opening a window); without it nothing is read at all.
struct H2Raw {
    wr: Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
    /// The server closed the connection (the reader hit the end of the stream or an error).
    closed: Arc<AtomicBool>,
    /// Frames the reader saw, as (kind, flags, stream) in order.
    seen: Arc<std::sync::Mutex<Vec<(u8, u8, u32)>>>,
}

impl H2Raw {
    async fn connect(addr: SocketAddr, settings: &[u8], read: bool) -> H2Raw {
        let (mut rd, mut wr) = tiny_client(addr).await.into_split();
        wr.write_all(PREFACE).await.unwrap();
        wr.write_all(&frame_bytes(4, 0, 0, settings)).await.unwrap();
        let wr = Arc::new(tokio::sync::Mutex::new(wr));
        let closed = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        if read {
            let (w, c, s) = (wr.clone(), closed.clone(), seen.clone());
            tokio::spawn(async move {
                while let Some(f) = read_frame(&mut rd).await {
                    s.lock().unwrap().push((f.kind, f.flags, f.stream));
                    // PING without ACK: answer it, as a live client does.
                    if f.kind == 6 && f.flags & 1 == 0 && w.lock().await.write_all(&frame_bytes(6, 1, 0, &f.payload)).await.is_err() {
                        break;
                    }
                    // The server's SETTINGS: acknowledge.
                    if f.kind == 4 && f.flags & 1 == 0 && w.lock().await.write_all(&frame_bytes(4, 1, 0, &[])).await.is_err() {
                        break;
                    }
                }
                c.store(true, Ordering::SeqCst);
            });
        }
        H2Raw { wr, closed, seen }
    }

    async fn send(&self, bytes: &[u8]) -> std::io::Result<()> {
        self.wr.lock().await.write_all(bytes).await
    }

    async fn get(&self, stream: u32, path: &str) {
        self.send(&frame_bytes(1, 0x5, stream, &request_block(path))).await.unwrap();
    }
}

// ---- hyper clients ----

type B = BoxBody<Bytes, std::io::Error>;

fn empty() -> B {
    Empty::<Bytes>::new().map_err(|e| match e {}).boxed()
}

async fn h2_client(addr: SocketAddr) -> hyper::client::conn::http2::SendRequest<B> {
    let io = TokioIo::new(TcpStream::connect(addr).await.unwrap());
    let (sender, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), io).await.unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    sender
}

fn h2_get(addr: SocketAddr, path: &str) -> hyper::Request<B> {
    hyper::Request::get(format!("http://{addr}{path}")).body(empty()).unwrap()
}

// ---- Raw HTTP/1 ----

/// Reads the head of a response (up to the blank line); returns it and the body bytes that came with it.
async fn read_head(s: &mut TcpStream) -> (String, Vec<u8>) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(10), s.read(&mut tmp)).await.expect("response head in time").unwrap();
        assert!(n > 0, "the connection ended before the head: {}", String::from_utf8_lossy(&buf));
        buf.extend_from_slice(&tmp[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..i]).into_owned();
            return (head, buf[i + 4..].to_vec());
        }
    }
}

/// GET with `Connection: close`, reading to the end.
async fn fetch(addr: SocketAddr, path: &str, extra: &str) -> (String, Vec<u8>) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{extra}\r\n").as_bytes()).await.unwrap();
    let (head, mut body) = read_head(&mut s).await;
    tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut body)).await.expect("body in time").unwrap();
    (head, body)
}

// ---- Stalled clients ----

/// A client that never reads, over HTTP/2, floods PINGs until its own writes block.
async fn ping_flood(addr: SocketAddr) -> H2Raw {
    let c = H2Raw::connect(addr, &[], false).await;
    let batch: Vec<u8> = (0..2000u64).flat_map(|i| frame_bytes(6, 0, 0, &i.to_be_bytes())).collect();
    let mut sent = 0usize;
    loop {
        match tokio::time::timeout(Duration::from_secs(1), c.send(&batch)).await {
            Ok(Ok(())) => sent += batch.len(),
            _ => break,
        }
        assert!(sent < 512 * 1024 * 1024, "the writes never blocked");
    }
    c
}

#[corvis::test]
async fn http1_pipelined_requests_that_are_never_read_are_cut() {
    let addr = serve(one_slot(SEND_TIMEOUT)).await;
    let mut s = tiny_client(addr).await;
    let batch = b"GET /t/ok HTTP/1.1\r\nHost: x\r\n\r\n".repeat(500);
    let mut sent = 0usize;
    loop {
        match tokio::time::timeout(Duration::from_secs(1), s.write_all(&batch)).await {
            Ok(Ok(())) => sent += batch.len(),
            _ => break,
        }
        assert!(sent < 512 * 1024 * 1024, "the writes never blocked");
    }
    assert_closed_by_timeout(addr).await;
    drop(s);
}

#[corvis::test]
async fn http2_ping_flood_that_is_never_read_is_cut() {
    let addr = serve(one_slot(SEND_TIMEOUT)).await;
    let c = ping_flood(addr).await;
    assert_closed_by_timeout(addr).await;
    drop(c);
}

#[corvis::test]
async fn the_ping_flood_keeps_its_slot_without_a_send_timeout() {
    let addr = serve(one_slot(None)).await;
    let c = ping_flood(addr).await;
    assert_eq!(probe(addr, Duration::from_secs(4)).await, None, "the slot was released although the timeout is off");
    drop(c);
}

#[corvis::test]
async fn http1_large_response_to_a_client_that_stops_reading_is_cut() {
    let addr = serve(one_slot(SEND_TIMEOUT)).await;
    let mut s = tiny_client(addr).await;
    s.write_all(b"GET /t/forever HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let (head, _) = read_head(&mut s).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert_closed_by_timeout(addr).await;
    drop(s);
}

#[corvis::test]
async fn the_large_response_keeps_its_slot_without_a_send_timeout() {
    let addr = serve(one_slot(None)).await;
    let mut s = tiny_client(addr).await;
    s.write_all(b"GET /t/forever HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let (head, _) = read_head(&mut s).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert_eq!(probe(addr, Duration::from_secs(4)).await, None, "the slot was released although the timeout is off");
    drop(s);
}

#[corvis::test]
async fn http2_large_response_to_a_client_that_stops_reading_is_cut() {
    let addr = serve(one_slot(SEND_TIMEOUT)).await;
    let mut sender = h2_client(addr).await;
    // The response head arrives; the body is never read, so no window is ever granted.
    let response = sender.send_request(h2_get(addr, "/t/huge")).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_closed_by_timeout(addr).await;
    drop(response);
}

#[corvis::test]
async fn an_http2_stream_with_a_zero_window_is_cut_although_the_client_answers_pings() {
    let options = one_slot(Duration::from_secs(2))
        .http2_keep_alive_interval(Duration::from_millis(500))
        .http2_keep_alive_timeout(Duration::from_secs(1));
    let addr = serve(options).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0, true).await;
    c.get(1, "/t/big").await;
    assert_closed_by_timeout(addr).await;
    // The connection that was stalled was closed by the server.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(c.closed.load(Ordering::SeqCst));
    let pings = c.seen.lock().unwrap().iter().filter(|f| f.0 == 6).count();
    assert!(pings > 0, "the keep-alive PINGs were exchanged while the stream was stalled");
}

#[corvis::test]
async fn an_http2_stream_with_a_window_of_one_byte_is_cut() {
    let addr = serve(one_slot(SEND_TIMEOUT)).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_1, true).await;
    c.get(1, "/t/big").await;
    assert_closed_by_timeout(addr).await;
    assert!(c.closed.load(Ordering::SeqCst) || probe(addr, Duration::from_secs(1)).await.is_some());
}

#[corvis::test]
async fn the_zero_window_keeps_its_slot_without_a_send_timeout() {
    let addr = serve(one_slot(None)).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0, true).await;
    c.get(1, "/t/big").await;
    assert_eq!(probe(addr, Duration::from_secs(4)).await, None, "the slot was released although the timeout is off");
    assert!(!c.closed.load(Ordering::SeqCst));
}

// ---- Live clients ----

/// A client that reads what its socket holds (up to 128 KiB) every 400 ms for five send timeouts keeps its (endless)
/// response, and can then read at full speed. The server sees progress when the operating system reports its socket
/// writable, which for a slower reader (4 KiB every 400 ms) takes longer than one second on a loopback connection;
/// the default of 60 s is meant for such readers, not this test's 1 s.
#[corvis::test]
async fn http1_slow_but_live_reader_is_not_cut() {
    let addr = serve(live(SEND_TIMEOUT)).await;
    let mut s = tiny_client(addr).await;
    s.write_all(b"GET /t/forever HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let (_head, first) = read_head(&mut s).await;
    let mut got = first.len();
    let started = Instant::now();
    let mut buf = vec![0u8; 128 * 1024];
    while started.elapsed() < Duration::from_secs(5) {
        let n = s.read(&mut buf).await.unwrap();
        assert!(n > 0, "the server ended the response after {got} bytes");
        got += n;
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    let fast = got + 64 * 1024 * 1024;
    let mut chunk = vec![0u8; 256 * 1024];
    while got < fast {
        let n = tokio::time::timeout(Duration::from_secs(10), s.read(&mut chunk)).await.expect("more data in time").unwrap();
        assert!(n > 0, "the server ended the response after {got} bytes");
        got += n;
    }
}

#[corvis::test]
async fn http2_slow_but_live_reader_is_not_cut() {
    let addr = serve(live(SEND_TIMEOUT)).await;
    let mut sender = h2_client(addr).await;
    let response = sender.send_request(h2_get(addr, "/t/huge")).await.unwrap();
    assert_eq!(response.status(), 200);
    let mut body = response.into_body();
    let mut got = 0usize;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        let frame = body.frame().await.expect("the server ended the response early").unwrap();
        got += frame.into_data().map_or(0, |d| d.len());
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    while let Some(frame) = tokio::time::timeout(Duration::from_secs(10), body.frame()).await.expect("the rest in time") {
        got += frame.unwrap().into_data().map_or(0, |d| d.len());
    }
    assert_eq!(got, HUGE);
}

#[corvis::test]
async fn an_idle_sse_stream_over_http1_is_not_cut() {
    let addr = serve(live(SEND_TIMEOUT)).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(b"GET /t/sse HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let mut seen = String::new();
    let mut buf = [0u8; 1024];
    let started = Instant::now();
    while !seen.contains("event 1") {
        let n = tokio::time::timeout(Duration::from_secs(8), s.read(&mut buf)).await.expect("the next event in time").unwrap();
        assert!(n > 0, "the stream ended after {:?}: {seen}", started.elapsed());
        seen.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
    assert!(seen.contains("event 0"));
    assert!(started.elapsed() >= Duration::from_secs(3), "the second event came before the pause");
}

#[corvis::test]
async fn an_idle_sse_stream_over_h2c_is_not_cut() {
    let addr = serve(live(SEND_TIMEOUT)).await;
    let mut sender = h2_client(addr).await;
    let response = sender.send_request(h2_get(addr, "/t/sse")).await.unwrap();
    assert_eq!(response.status(), 200);
    let mut body = response.into_body();
    let mut seen = String::new();
    let started = Instant::now();
    while !seen.contains("event 1") {
        let frame = tokio::time::timeout(Duration::from_secs(8), body.frame()).await.expect("the next event in time");
        let frame = frame.unwrap_or_else(|| panic!("the stream ended after {:?}: {seen}", started.elapsed())).unwrap();
        if let Ok(data) = frame.into_data() {
            seen.push_str(&String::from_utf8_lossy(&data));
        }
    }
    assert!(started.elapsed() >= Duration::from_secs(3), "the second event came before the pause");
}

#[corvis::test]
async fn a_slow_handler_is_not_cut_over_http1_and_h2c() {
    let addr = serve(live(SEND_TIMEOUT)).await;
    let (head, body) = fetch(addr, "/t/slow", "").await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(String::from_utf8_lossy(&body).contains("\"slow\":true"));
    let mut sender = h2_client(addr).await;
    let response = sender.send_request(h2_get(addr, "/t/slow")).await.unwrap();
    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("\"slow\":true"));
}

/// Siblings of a stalled stream are not what is measured, but a connection that carries a stalled stream is closed
/// only when that stream has waited a full send timeout: a reader that opens its window in time is never cut.
#[corvis::test]
async fn an_http2_client_that_grants_window_in_time_keeps_its_connection() {
    let addr = serve(live(SEND_TIMEOUT)).await;
    let mut sender = h2_client(addr).await;
    for _ in 0..3 {
        let response = sender.send_request(h2_get(addr, "/t/big")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body.len(), 1024 * 1024);
    }
}

// ---- Idle h2c connection, stray frames ----

/// A client that sends a frame that starts no request (WINDOW_UPDATE) after its last response must not turn the idle
/// connection from the keep-alive phase into the headers phase, which has a much longer deadline.
#[corvis::test]
async fn stray_http2_frames_do_not_end_the_keep_alive_phase() {
    let options = ServerOptions::new().h2c(true).keep_alive_timeout(Duration::from_secs(1)).headers_timeout(Duration::from_secs(60));
    let addr = serve(options).await;
    let (mut rd, mut wr) = tiny_client(addr).await.into_split();
    wr.write_all(PREFACE).await.unwrap();
    wr.write_all(&frame_bytes(4, 0, 0, &[])).await.unwrap();
    wr.write_all(&frame_bytes(1, 0x5, 1, &request_block("/t/ok"))).await.unwrap();
    // Read until the response of stream 1 ends.
    loop {
        let f = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut rd)).await.expect("the response in time").expect("open");
        if f.stream == 1 && f.flags & 1 != 0 && (f.kind == 0 || f.kind == 1) {
            break;
        }
    }
    let closed = Arc::new(AtomicBool::new(false));
    let flag = closed.clone();
    tokio::spawn(async move {
        while read_frame(&mut rd).await.is_some() {}
        flag.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();
    while !closed.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(8) {
        if wr.write_all(&frame_bytes(8, 0, 0, &1u32.to_be_bytes())).await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(closed.load(Ordering::SeqCst), "WINDOW_UPDATE frames kept an idle connection open past keep_alive_timeout");
    assert!(started.elapsed() < Duration::from_secs(6), "{:?}", started.elapsed());
}

// ---- Compression ----

/// A compression slot is held until the response body is dropped, so a stalled client holds one until its connection
/// closes; the send timeout closing the connection must release it.
#[cfg(feature = "compression")]
#[corvis::test]
async fn a_stalled_client_releases_its_compression_slot_when_it_is_cut() {
    use corvis::http::compression::CompressionOptions;
    let module = corvis::testing::Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.silence_route_logs();
    app.enable_compression(CompressionOptions::new().max_concurrent(1));
    app.init().await.unwrap();
    app.set_server_options(live(SEND_TIMEOUT));
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    std::mem::forget(app);

    let gzip = "accept-encoding: gzip\r\n";
    let (head, _) = fetch(addr, "/t/text", gzip).await;
    assert!(head.to_ascii_lowercase().contains("content-encoding: gzip"), "compression is on: {head}");

    let mut s = tiny_client(addr).await;
    s.write_all(format!("GET /t/rand HTTP/1.1\r\nHost: x\r\n{gzip}\r\n").as_bytes()).await.unwrap();
    let (head, _) = read_head(&mut s).await;
    assert!(head.to_ascii_lowercase().contains("content-encoding: gzip"), "{head}");
    // The stalled body holds the only slot.
    let (head, _) = fetch(addr, "/t/text", gzip).await;
    assert!(!head.to_ascii_lowercase().contains("content-encoding"), "the slot was free while the stalled body held it: {head}");
    // After the send timeout the connection is closed and the slot is free again.
    let mut released = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let (head, _) = fetch(addr, "/t/text", gzip).await;
        if head.to_ascii_lowercase().contains("content-encoding: gzip") {
            released = true;
            break;
        }
    }
    assert!(released, "the stalled client's compression slot was never released");
    drop(s);
}
