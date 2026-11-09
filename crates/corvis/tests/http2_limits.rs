//! HTTP/2 limits follow `ServerOptions` (h2c): header list size, concurrent streams, PING keep-alive,
//! `max_connections` counting sockets, and the per-stream and idle timeouts.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use corvis::ServerOptions;
use corvis::prelude::*;
use futures::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, StreamBody};
use hyper::body::Frame;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;

type B = BoxBody<Bytes, std::io::Error>;
type Sender = hyper::client::conn::http2::SendRequest<B>;

/// A handler that counts how many calls run at once and waits until the test releases it.
struct Hold {
    active: AtomicUsize,
    max: AtomicUsize,
    gate: Semaphore,
}

impl Hold {
    const fn new() -> Self {
        Hold { active: AtomicUsize::new(0), max: AtomicUsize::new(0), gate: Semaphore::const_new(0) }
    }
    async fn run(&self) {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(now, Ordering::SeqCst);
        self.gate.acquire().await.unwrap().forget();
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

static CANCEL_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static CANCEL_PEAK: AtomicUsize = AtomicUsize::new(0);

/// Counts a running handler until its future is dropped.
struct Running;

impl Running {
    fn new() -> Self {
        let n = CANCEL_ACTIVE.fetch_add(1, Ordering::SeqCst) + 1;
        CANCEL_PEAK.fetch_max(n, Ordering::SeqCst);
        Running
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        CANCEL_ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

static HOLD_STREAMS: Hold = Hold::new();
static HOLD_CONNS: Hold = Hold::new();

#[controller("t")]
pub struct TController;

#[routes]
impl TController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }

    #[get("slow")]
    async fn slow(&self) -> Value {
        tokio::time::sleep(Duration::from_secs(2)).await;
        json!({ "ok": true })
    }

    #[post("json")]
    fn json(&self, #[body] body: Value) -> Value {
        body
    }

    #[get("hold-streams")]
    async fn hold_streams(&self) -> Value {
        HOLD_STREAMS.run().await;
        json!({ "ok": true })
    }

    #[get("hold-cancel")]
    async fn hold_cancel(&self) -> Value {
        let _running = Running::new();
        tokio::time::sleep(Duration::from_secs(30)).await;
        json!({ "ok": true })
    }

    #[get("hold-conns")]
    async fn hold_conns(&self) -> Value {
        HOLD_CONNS.run().await;
        json!({ "ok": true })
    }
}

#[module(controllers = [TController])]
pub struct AppModule;

async fn serve(options: ServerOptions) -> std::net::SocketAddr {
    let module = corvis::testing::Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    app.set_server_options(options);
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    std::mem::forget(app);
    addr
}

fn empty() -> B {
    Empty::<Bytes>::new().map_err(|e| match e {}).boxed()
}

async fn connect(addr: std::net::SocketAddr) -> (Sender, tokio::task::JoinHandle<()>) {
    let io = TokioIo::new(TcpStream::connect(addr).await.unwrap());
    let (sender, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), io).await.unwrap();
    (
        sender,
        tokio::spawn(async move {
            let _ = conn.await;
        }),
    )
}

fn get(addr: std::net::SocketAddr, path: &str) -> hyper::Request<B> {
    hyper::Request::get(format!("http://{addr}{path}")).body(empty()).unwrap()
}

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const EMPTY_SETTINGS: &[u8] = &[0, 0, 0, 4, 0, 0, 0, 0, 0];
const FRAME_SETTINGS: u8 = 4;
const FRAME_PING: u8 = 6;

/// A client that speaks the preface and an empty SETTINGS frame and then reads frames.
struct RawClient(TcpStream);

struct RawFrame {
    kind: u8,
    flags: u8,
    payload: Vec<u8>,
}

impl RawClient {
    async fn connect(addr: std::net::SocketAddr) -> Self {
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(PREFACE).await.unwrap();
        s.write_all(EMPTY_SETTINGS).await.unwrap();
        RawClient(s)
    }

    /// The next frame: `Ok(None)` on end of stream or error, `Err(())` when `wait` elapses first.
    async fn frame(&mut self, wait: Duration) -> Result<Option<RawFrame>, ()> {
        let read = async {
            let mut head = [0u8; 9];
            if self.0.read_exact(&mut head).await.is_err() {
                return None;
            }
            let len = u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize;
            let mut payload = vec![0u8; len];
            if self.0.read_exact(&mut payload).await.is_err() {
                return None;
            }
            Some(RawFrame { kind: head[3], flags: head[4], payload })
        };
        tokio::time::timeout(wait, read).await.map_err(|_| ())
    }
}

async fn status_of(sender: &mut Sender, req: hyper::Request<B>) -> u16 {
    let res = sender.send_request(req).await.unwrap();
    let status = res.status().as_u16();
    let _ = res.into_body().collect().await;
    status
}

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

/// Rapid reset with accepted streams: each new stream is preceded by the RST_STREAM of the one opened a few
/// streams earlier, so the peer's own stream count never exceeds the cap while handlers still run.
#[corvis::test]
async fn rapid_reset_never_exceeds_the_stream_cap_and_closes_the_connection() {
    const CAP: u32 = 2;
    let addr = serve(ServerOptions::new().h2c(true).http2_max_concurrent_streams(CAP)).await;
    let mut c = RawClient::connect(addr).await;
    // Wait for the server's SETTINGS and acknowledge them.
    let first = c.frame(Duration::from_secs(5)).await.unwrap().unwrap();
    assert_eq!(first.kind, FRAME_SETTINGS);
    let (mut rd, mut wr) = c.0.into_split();
    wr.write_all(&frame_bytes(FRAME_SETTINGS, 1, 0, &[])).await.unwrap();
    let closed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = closed.clone();
    let reader = tokio::spawn(async move {
        let mut goaway = None;
        loop {
            let mut head = [0u8; 9];
            if rd.read_exact(&mut head).await.is_err() {
                break;
            }
            let len = u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize;
            let mut payload = vec![0u8; len];
            if rd.read_exact(&mut payload).await.is_err() {
                break;
            }
            if head[3] == 7 {
                goaway = Some(u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]));
            }
        }
        flag.store(true, Ordering::SeqCst);
        goaway
    });
    let block = request_block("/t/hold-cancel");
    let mut id: u32 = 1;
    let started = Instant::now();
    let mut sent = 0u32;
    while started.elapsed() < Duration::from_secs(5) && !closed.load(Ordering::SeqCst) {
        let mut batch = Vec::new();
        for _ in 0..5 {
            if id > 2 * CAP {
                batch.extend(frame_bytes(3, 0, id - 2 * CAP, &8u32.to_be_bytes()));
            }
            batch.extend(frame_bytes(1, 0x5, id, &block));
            id += 2;
            sent += 1;
        }
        if wr.write_all(&batch).await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let goaway = tokio::time::timeout(Duration::from_secs(5), reader).await.expect("the server closed the connection").unwrap();
    assert!(sent > 500, "the flood went past the threshold ({sent} streams); GOAWAY code {goaway:?}");
    let peak = CANCEL_PEAK.load(Ordering::SeqCst);
    eprintln!("rapid reset: {sent} streams sent in {:.2?}, peak handlers {peak}, GOAWAY {goaway:?}", started.elapsed());
    assert!(peak <= CAP as usize, "at most {CAP} handlers at once, saw {peak}");
}

#[corvis::test]
async fn zero_keep_alive_interval_means_no_pings() {
    let addr = serve(ServerOptions::new().h2c(true).http2_keep_alive_interval(Duration::ZERO)).await;
    let mut c = RawClient::connect(addr).await;
    let end = Instant::now() + Duration::from_millis(1200);
    while let Ok(Some(f)) = c.frame(end.saturating_duration_since(Instant::now())).await {
        assert_ne!(f.kind, FRAME_PING, "a zero interval sends no PING");
    }
}

#[corvis::test]
async fn zero_keep_alive_timeout_is_clamped_to_one_second() {
    let options =
        ServerOptions::new().h2c(true).http2_keep_alive_interval(Duration::from_millis(200)).http2_keep_alive_timeout(Duration::ZERO);
    let addr = serve(options).await;
    let (mut sender, conn) = connect(addr).await;
    assert_eq!(status_of(&mut sender, get(addr, "/t/ok")).await, 200);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!conn.is_finished(), "an acknowledged PING keeps the connection open");
    assert_eq!(status_of(&mut sender, get(addr, "/t/ok")).await, 200);
}

#[corvis::test]
async fn http1_is_unaffected_by_http2_options() {
    let addr = serve(ServerOptions::new().h2c(true).http2_max_concurrent_streams(1)).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    for _ in 0..2 {
        s.write_all(b"GET /t/ok HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = tokio::time::timeout(Duration::from_secs(5), s.read(&mut buf)).await.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200"));
    }
}

#[corvis::test]
async fn oversized_header_list_is_431_and_the_connection_survives() {
    let addr = serve(ServerOptions::new().h2c(true).max_header_size(8 * 1024)).await;
    let (mut sender, _conn) = connect(addr).await;
    let big = hyper::Request::get(format!("http://{addr}/t/ok")).header("x-big", "a".repeat(12 * 1024)).body(empty()).unwrap();
    assert_eq!(status_of(&mut sender, big).await, 431);
    assert_eq!(status_of(&mut sender, get(addr, "/t/ok")).await, 200);
}

#[corvis::test]
async fn default_header_list_allows_24_kib() {
    let addr = serve(ServerOptions::new().h2c(true)).await;
    let (mut sender, _conn) = connect(addr).await;
    let big = hyper::Request::get(format!("http://{addr}/t/ok")).header("x-big", "a".repeat(24 * 1024)).body(empty()).unwrap();
    assert_eq!(status_of(&mut sender, big).await, 200);
}

#[corvis::test]
async fn stream_cap_limits_concurrent_handlers() {
    let addr = serve(ServerOptions::new().h2c(true).http2_max_concurrent_streams(2)).await;
    let (sender, _conn) = connect(addr).await;
    // A request first, so the client has the server's SETTINGS (a stream opened before them is refused).
    assert_eq!(status_of(&mut sender.clone(), get(addr, "/t/ok")).await, 200);
    let tasks: Vec<_> = (0..3)
        .map(|_| {
            let mut s = sender.clone();
            tokio::spawn(async move { status_of(&mut s, get(addr, "/t/hold-streams")).await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(HOLD_STREAMS.max.load(Ordering::SeqCst), 2);
    HOLD_STREAMS.gate.add_permits(3);
    for t in tasks {
        assert_eq!(t.await.unwrap(), 200);
    }
    assert_eq!(HOLD_STREAMS.max.load(Ordering::SeqCst), 2);
}

#[corvis::test]
async fn default_stream_cap_is_100() {
    let addr = serve(ServerOptions::new().h2c(true)).await;
    let mut c = RawClient::connect(addr).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let f = c.frame(left).await.expect("SETTINGS arrives").expect("connection open");
        if f.kind == FRAME_SETTINGS && f.flags & 1 == 0 {
            let cap =
                f.payload.chunks(6).find(|e| u16::from_be_bytes([e[0], e[1]]) == 3).map(|e| u32::from_be_bytes([e[2], e[3], e[4], e[5]]));
            assert_eq!(cap, Some(100));
            return;
        }
    }
}

#[corvis::test]
async fn max_connections_counts_sockets_not_streams() {
    let addr = serve(ServerOptions::new().h2c(true).max_connections(1).http2_max_concurrent_streams(4)).await;
    let (sender, conn) = connect(addr).await;
    assert_eq!(status_of(&mut sender.clone(), get(addr, "/t/ok")).await, 200);
    let tasks: Vec<_> = (0..4)
        .map(|_| {
            let mut s = sender.clone();
            tokio::spawn(async move { status_of(&mut s, get(addr, "/t/hold-conns")).await })
        })
        .collect();
    let deadline = Instant::now() + Duration::from_secs(5);
    while HOLD_CONNS.active.load(Ordering::SeqCst) < 4 {
        assert!(Instant::now() < deadline, "the four streams run concurrently");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // A second connection waits in the accept queue: it gets no SETTINGS.
    let mut second = RawClient::connect(addr).await;
    assert!(second.frame(Duration::from_millis(800)).await.is_err());
    HOLD_CONNS.gate.add_permits(4);
    for t in tasks {
        assert_eq!(t.await.unwrap(), 200);
    }
    drop(sender);
    conn.abort();
    let f = second.frame(Duration::from_secs(5)).await.expect("served once the first closes");
    assert!(f.is_some_and(|f| f.kind == FRAME_SETTINGS));
}

fn ping_options() -> ServerOptions {
    ServerOptions::new()
        .h2c(true)
        .headers_timeout(None)
        .keep_alive_timeout(None)
        .http2_keep_alive_interval(Duration::from_millis(200))
        .http2_keep_alive_timeout(Duration::from_millis(300))
}

#[corvis::test]
async fn unanswered_ping_closes_the_connection() {
    let addr = serve(ping_options()).await;
    let mut c = RawClient::connect(addr).await;
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut saw_ping = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match c.frame(left).await {
            Err(()) => panic!("connection still open after 8 s (ping seen: {saw_ping})"),
            Ok(None) => break,
            Ok(Some(f)) => saw_ping |= f.kind == FRAME_PING && f.flags & 1 == 0,
        }
    }
    assert!(saw_ping, "a PING frame arrived before the close");
}

#[corvis::test]
async fn answered_ping_keeps_the_connection_open() {
    let addr = serve(ping_options()).await;
    let (mut sender, conn) = connect(addr).await;
    assert_eq!(status_of(&mut sender, get(addr, "/t/slow")).await, 200);
    assert!(!conn.is_finished());
}

#[corvis::test]
async fn no_ping_by_default() {
    let addr = serve(ServerOptions::new().h2c(true)).await;
    let mut c = RawClient::connect(addr).await;
    let end = Instant::now() + Duration::from_secs(1);
    while let Ok(Some(f)) = c.frame(end.saturating_duration_since(Instant::now())).await {
        assert_ne!(f.kind, FRAME_PING, "no PING without an interval");
    }
}

#[corvis::test]
async fn request_timeout_applies_per_stream() {
    let addr = serve(ServerOptions::new().h2c(true).request_timeout(Duration::from_secs(1))).await;
    let (sender, _conn) = connect(addr).await;
    let body: B = BodyExt::boxed(StreamBody::new(
        futures::stream::once(async { Ok::<_, std::io::Error>(Frame::data(Bytes::from_static(b"{"))) }).chain(futures::stream::pending()),
    ));
    let post = hyper::Request::post(format!("http://{addr}/t/json")).header("content-type", "application/json").body(body).unwrap();
    let started = Instant::now();
    let mut s1 = sender.clone();
    let slow = tokio::spawn(async move { status_of(&mut s1, post).await });
    let mut s2 = sender.clone();
    assert_eq!(status_of(&mut s2, get(addr, "/t/ok")).await, 200);
    assert_eq!(slow.await.unwrap(), 408);
    assert!(started.elapsed() >= Duration::from_millis(900));
}

#[corvis::test]
async fn idle_http2_connection_is_closed() {
    let addr = serve(ServerOptions::new().h2c(true).keep_alive_timeout(Duration::from_secs(1))).await;
    let (mut sender, conn) = connect(addr).await;
    assert_eq!(status_of(&mut sender, get(addr, "/t/ok")).await, 200);
    let started = Instant::now();
    tokio::time::timeout(Duration::from_secs(5), conn).await.expect("closed by the server").unwrap();
    assert!(started.elapsed() >= Duration::from_millis(800));
}
