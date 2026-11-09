//! `ServerOptions::max_pending_response_bytes`: the response memory one connection holds for a client that does not
//! read is bounded (admission before the handler, reset above the cap), while clients that read keep working. Memory is
//! asserted through behavior (which streams get HEADERS, which are reset with `ENHANCE_YOUR_CALM`, how many handlers
//! ran), with small caps so every scenario runs in seconds.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use corvis::ServerOptions;
use corvis::prelude::*;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpSocket, TcpStream};

const MIB: usize = 1024 * 1024;
const ENHANCE_YOUR_CALM: u32 = 0xb;
const CANCEL: u32 = 0x8;
const REFUSED_STREAM: u32 = 0x7;

static COUNTED: AtomicUsize = AtomicUsize::new(0);
static COUNTED_REFUSED: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_H1: AtomicUsize = AtomicUsize::new(0);
static MAX_ACTIVE_H1: AtomicUsize = AtomicUsize::new(0);
static COUNTED_WAIT: AtomicUsize = AtomicUsize::new(0);

#[controller("t")]
pub struct TController;

#[routes]
impl TController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }

    /// 256 KiB with a length.
    #[get("quarter")]
    fn quarter(&self) -> String {
        "a".repeat(256 * 1024)
    }

    /// 1 MiB with a length.
    #[get("big")]
    fn big(&self) -> String {
        "a".repeat(MIB)
    }

    /// 256 KiB after half a second: a burst of these runs its handlers all at once and they return together.
    #[get("slow-quarter")]
    async fn slow_quarter(&self) -> String {
        tokio::time::sleep(Duration::from_millis(500)).await;
        "a".repeat(256 * 1024)
    }

    /// 1 MiB after half a second.
    #[get("slow-big")]
    async fn slow_big(&self) -> String {
        tokio::time::sleep(Duration::from_millis(500)).await;
        "a".repeat(MIB)
    }

    /// 8 MiB with a length.
    #[get("huge")]
    fn huge(&self) -> String {
        "a".repeat(8 * MIB)
    }

    /// 64 KiB with a length.
    #[get("small")]
    fn small(&self) -> String {
        "a".repeat(64 * 1024)
    }

    /// 256 KiB; counts its runs.
    #[get("counted")]
    fn counted(&self) -> String {
        COUNTED.fetch_add(1, Ordering::SeqCst);
        "a".repeat(256 * 1024)
    }

    /// 256 KiB; counts its runs (the reset-while-waiting test).
    #[get("counted-wait")]
    fn counted_wait(&self) -> String {
        COUNTED_WAIT.fetch_add(1, Ordering::SeqCst);
        "a".repeat(256 * 1024)
    }

    /// 256 KiB; counts its runs (the refused-after-waiting test).
    #[get("counted-refused")]
    fn counted_refused(&self) -> String {
        COUNTED_REFUSED.fetch_add(1, Ordering::SeqCst);
        "a".repeat(256 * 1024)
    }

    /// A stream longer than its declared length: hyper writes ten bytes and drops the rest.
    #[get("badlen")]
    fn badlen(&self) -> StreamableFile {
        let s = futures::stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(vec![120u8; 64 * 1024]))]);
        StreamableFile::from_stream(Box::pin(s), StreamableFileOptions::new().length(10))
    }

    /// Counts the handlers of one connection that run at the same time (HTTP/1 pipelining).
    #[get("overlap")]
    async fn overlap(&self) -> String {
        let now = ACTIVE_H1.fetch_add(1, Ordering::SeqCst) + 1;
        MAX_ACTIVE_H1.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(150)).await;
        ACTIVE_H1.fetch_sub(1, Ordering::SeqCst);
        "a".repeat(64 * 1024)
    }
}

#[module(controllers = [TController])]
pub struct AppModule;

async fn serve_with(options: ServerOptions, compression: bool) -> SocketAddr {
    let module = corvis::testing::Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.silence_route_logs();
    #[cfg(feature = "compression")]
    if compression {
        app.enable_compression(corvis::http::compression::CompressionOptions::new().threshold(0));
    }
    #[cfg(not(feature = "compression"))]
    let _ = compression;
    app.init().await.unwrap();
    app.set_server_options(options);
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    std::mem::forget(app);
    addr
}

async fn serve(options: ServerOptions) -> SocketAddr {
    serve_with(options, false).await
}

/// No timeout may end a connection that is not misbehaving on the read side.
fn quiet(options: ServerOptions) -> ServerOptions {
    options.h2c(true).send_timeout(None).keep_alive_timeout(None).headers_timeout(None).request_timeout(None)
}

/// Small numbers: 20 streams share a 1 MiB cap, so a 256 KiB response is five times its share.
fn small_cap() -> ServerOptions {
    quiet(ServerOptions::new().http2_max_concurrent_streams(20).max_pending_response_bytes(MIB))
}

// ---- Raw HTTP/2 (copied from tests/send_timeout.rs, with the payload of RST_STREAM) ----

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const SETTINGS_WINDOW_0: &[u8] = &[0, 4, 0, 0, 0, 0];

fn frame_bytes(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let mut v = vec![(len >> 16) as u8, (len >> 8) as u8, len as u8, kind, flags];
    v.extend_from_slice(&stream.to_be_bytes());
    v.extend_from_slice(payload);
    v
}

/// HPACK block of `GET <path>` over http, authority "x" (static table indexes, no Huffman), with `extra` appended.
fn request_block(path: &str, extra: &[u8]) -> Vec<u8> {
    let mut b = vec![0x82, 0x86, 0x04, path.len() as u8];
    b.extend_from_slice(path.as_bytes());
    b.extend_from_slice(&[0x01, 0x01, 0x78]);
    b.extend_from_slice(extra);
    b
}

/// `accept-encoding: gzip, deflate` (static table index 16).
const ACCEPT_GZIP: &[u8] = &[0x90];

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

#[derive(Clone, Copy, PartialEq, Debug)]
enum Seen {
    Headers(u32),
    Data(u32, usize),
    Reset(u32, u32),
}

/// A raw h2c client that reads every frame (answering PINGs and SETTINGS) and never opens a window by itself.
struct H2Raw {
    wr: Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
    /// The server closed the connection.
    closed: Arc<AtomicBool>,
    seen: Arc<std::sync::Mutex<Vec<Seen>>>,
    next_stream: std::sync::Mutex<u32>,
}

impl H2Raw {
    async fn connect(addr: SocketAddr, settings: &[u8]) -> H2Raw {
        let (mut rd, mut wr) = TcpSocket::new_v4().unwrap().connect(addr).await.unwrap().into_split();
        wr.write_all(PREFACE).await.unwrap();
        wr.write_all(&frame_bytes(4, 0, 0, settings)).await.unwrap();
        let wr = Arc::new(tokio::sync::Mutex::new(wr));
        let closed = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (w, c, s) = (wr.clone(), closed.clone(), seen.clone());
        tokio::spawn(async move {
            while let Some(f) = read_frame(&mut rd).await {
                match f.kind {
                    0 => s.lock().unwrap().push(Seen::Data(f.stream, f.payload.len())),
                    1 => s.lock().unwrap().push(Seen::Headers(f.stream)),
                    3 => {
                        let code = u32::from_be_bytes(f.payload[..4].try_into().unwrap());
                        s.lock().unwrap().push(Seen::Reset(f.stream, code));
                    }
                    _ => {}
                }
                if f.kind == 6 && f.flags & 1 == 0 && w.lock().await.write_all(&frame_bytes(6, 1, 0, &f.payload)).await.is_err() {
                    break;
                }
                if f.kind == 4 && f.flags & 1 == 0 && w.lock().await.write_all(&frame_bytes(4, 1, 0, &[])).await.is_err() {
                    break;
                }
            }
            c.store(true, Ordering::SeqCst);
        });
        H2Raw { wr, closed, seen, next_stream: std::sync::Mutex::new(1) }
    }

    async fn send(&self, bytes: &[u8]) {
        let _ = self.wr.lock().await.write_all(bytes).await;
    }

    /// Opens `n` streams to `path`; their ids.
    async fn open(&self, n: usize, path: &str, extra: &[u8]) -> Vec<u32> {
        let mut ids = Vec::new();
        let mut buf = Vec::new();
        {
            let mut next = self.next_stream.lock().unwrap();
            for _ in 0..n {
                buf.extend(frame_bytes(1, 0x5, *next, &request_block(path, extra)));
                ids.push(*next);
                *next += 2;
            }
        }
        self.send(&buf).await;
        ids
    }

    async fn reset(&self, ids: &[u32]) {
        let buf: Vec<u8> = ids.iter().flat_map(|id| frame_bytes(3, 0, *id, &CANCEL.to_be_bytes())).collect();
        self.send(&buf).await;
    }

    /// Grants `bytes` of window to one stream and to the connection.
    async fn grant(&self, stream: u32, bytes: u32) {
        let mut buf = frame_bytes(8, 0, 0, &bytes.to_be_bytes());
        buf.extend(frame_bytes(8, 0, stream, &bytes.to_be_bytes()));
        self.send(&buf).await;
    }

    fn headers(&self, ids: &[u32]) -> Vec<u32> {
        let seen = self.seen.lock().unwrap();
        ids.iter().copied().filter(|id| seen.contains(&Seen::Headers(*id))).collect()
    }

    fn resets(&self, ids: &[u32], code: u32) -> usize {
        let seen = self.seen.lock().unwrap();
        ids.iter().filter(|id| seen.contains(&Seen::Reset(**id, code))).count()
    }

    fn data_of(&self, stream: u32) -> usize {
        self.seen.lock().unwrap().iter().map(|s| if let Seen::Data(id, n) = s { if *id == stream { *n } else { 0 } } else { 0 }).sum()
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

async fn until(what: &str, secs: u64, mut done: impl FnMut() -> bool) {
    let t = Instant::now();
    while !done() {
        assert!(t.elapsed() < Duration::from_secs(secs), "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
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

/// GETs `path`; the status and the body length.
async fn download(sender: &mut hyper::client::conn::http2::SendRequest<B>, addr: SocketAddr, path: &str) -> Result<(u16, usize), String> {
    let req = hyper::Request::builder().uri(format!("http://{addr}{path}")).body(empty()).unwrap();
    let resp = sender.send_request(req).await.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let body = resp.into_body().collect().await.map_err(|e| e.to_string())?.to_bytes();
    Ok((status, body.len()))
}

// ---- Scenarios ----

/// Window 0: 20 streams of 256 KiB against a 1 MiB cap, all handlers running at once. Four responses are kept, the other 16 are dropped and reset
/// with ENHANCE_YOUR_CALM, and the connection survives.
#[tokio::test]
async fn a_burst_to_a_client_that_does_not_read_keeps_four_and_resets_the_rest() {
    let addr = serve(small_cap()).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0).await;
    let ids = c.open(20, "/t/slow-quarter", &[]).await;
    until("every stream was answered or reset", 10, || c.headers(&ids).len() + c.resets(&ids, ENHANCE_YOUR_CALM) >= 20).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(c.headers(&ids).len(), 4, "only what fits the cap is kept");
    assert_eq!(c.resets(&ids, ENHANCE_YOUR_CALM), 16);
    assert!(!c.is_closed());
}

#[tokio::test]
async fn the_limit_can_be_turned_off() {
    let addr = serve(small_cap().max_pending_response_bytes(None)).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0).await;
    let ids = c.open(20, "/t/quarter", &[]).await;
    until("every stream got its headers", 10, || c.headers(&ids).len() == 20).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(c.resets(&ids, ENHANCE_YOUR_CALM), 0);
}

/// One response larger than the cap is always sent.
#[tokio::test]
async fn one_response_larger_than_the_cap_is_sent() {
    let addr = serve(small_cap()).await;
    let mut client = h2_client(addr).await;
    let (status, len) = download(&mut client, addr, "/t/huge").await.unwrap();
    assert_eq!((status, len), (200, 8 * MIB));
}

/// 100 concurrent 64 KiB requests from a client that reads normally, with the default options: every one is answered.
#[tokio::test]
async fn a_hundred_concurrent_small_responses_are_all_answered() {
    let addr = serve(quiet(ServerOptions::new())).await;
    let client = h2_client(addr).await;
    let mut tasks = Vec::new();
    for _ in 0..100 {
        let mut c = client.clone();
        tasks.push(tokio::spawn(async move { download(&mut c, addr, "/t/small").await }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap(), Ok((200, 64 * 1024)));
    }
}

/// Four held responses fill the cap: a fifth stream waits without reaching its handler until the client reads one.
#[tokio::test]
async fn new_streams_wait_while_a_stalled_client_holds_the_budget() {
    let addr = serve(small_cap()).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0).await;
    let held = c.open(4, "/t/counted", &[]).await;
    until("four handlers ran", 10, || COUNTED.load(Ordering::SeqCst) >= 4).await;
    until("four responses are held", 10, || c.headers(&held).len() == 4).await;
    let waiting = c.open(1, "/t/counted", &[]).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(COUNTED.load(Ordering::SeqCst), 4, "the handler of the waiting stream must not run");
    assert!(c.headers(&waiting).is_empty());
    // The client reads one response: room for the waiting stream.
    c.grant(held[0], 256 * 1024).await;
    until("the waiting stream's handler ran", 10, || COUNTED.load(Ordering::SeqCst) == 5).await;
    assert_eq!(c.data_of(held[0]), 256 * 1024);
}

/// Pipelined HTTP/1 requests run one handler at a time (the share of an HTTP/1 handler is the whole cap), and all of
/// them are answered. (A loopback socket takes tens of MiB without the client reading, so the wait for a response that
/// is still being written is covered by the unit tests of the budget.) `max_pending_response_wait` does not apply over
/// HTTP/1: even with a wait of one millisecond no pipelined request is refused or cut, each one is served once the
/// previous response is out.
#[tokio::test]
async fn pipelined_http1_requests_run_one_handler_at_a_time_and_are_never_cut_by_the_wait() {
    let addr = serve(quiet(ServerOptions::new().max_pending_response_wait(Duration::from_millis(1)))).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut requests = Vec::new();
    for i in 0..3 {
        let close = if i == 2 { "Connection: close\r\n" } else { "" };
        requests.extend_from_slice(format!("GET /t/overlap HTTP/1.1\r\nHost: x\r\n{close}\r\n").as_bytes());
    }
    s.write_all(&requests).await.unwrap();
    let mut all = Vec::new();
    s.read_to_end(&mut all).await.unwrap();
    assert_eq!(String::from_utf8_lossy(&all).matches("HTTP/1.1 200").count(), 3);
    assert_eq!(MAX_ACTIVE_H1.load(Ordering::SeqCst), 1);
}

/// A stream reset while it waits for room never reaches its handler, and counts as a cancelled request: with a ceiling
/// of three, four such resets close the connection.
#[tokio::test]
async fn a_waiting_request_reset_by_the_client_never_runs_and_counts_as_cancelled() {
    let addr = serve(small_cap().http2_max_cancelled_streams(3)).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0).await;
    let held = c.open(4, "/t/counted-wait", &[]).await;
    until("four responses are held", 10, || c.headers(&held).len() == 4).await;
    let waiting = c.open(4, "/t/counted-wait", &[]).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!c.is_closed());
    c.reset(&waiting).await;
    until("the connection is told to go away", 10, || c.is_closed()).await;
    assert_eq!(COUNTED_WAIT.load(Ordering::SeqCst), 4, "none of the waiting handlers ran");
}

/// 100 streams of 1 MiB against the default 8 MiB: eight are kept; the connection lives while a kept stream moves.
#[tokio::test]
async fn one_progressing_stream_next_to_stalled_ones() {
    let addr = serve(quiet(ServerOptions::new())).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0).await;
    let ids = c.open(100, "/t/slow-big", &[]).await;
    until("every stream was answered or reset", 15, || c.headers(&ids).len() + c.resets(&ids, ENHANCE_YOUR_CALM) >= 100).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let kept = c.headers(&ids);
    assert_eq!(kept.len(), 8, "at most eight responses are kept");
    assert_eq!(c.resets(&ids, ENHANCE_YOUR_CALM), 92);
    for _ in 0..4 {
        c.grant(kept[0], 16 * 1024).await;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(!c.is_closed());
    assert!(c.data_of(kept[0]) >= 4 * 16 * 1024 - 16, "the progressing stream moved: {}", c.data_of(kept[0]));
}

/// 16 KiB to every stream each period keeps nothing more than eight responses, and new streams wait.
#[tokio::test]
async fn trickled_window_updates_keep_at_most_eight_responses_and_new_streams_wait() {
    let addr = serve(quiet(ServerOptions::new())).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0).await;
    let ids = c.open(100, "/t/slow-big", &[]).await;
    until("every stream was answered or reset", 15, || c.headers(&ids).len() + c.resets(&ids, ENHANCE_YOUR_CALM) >= 100).await;
    let kept = c.headers(&ids);
    assert_eq!(kept.len(), 8);
    for _ in 0..3 {
        let mut buf = frame_bytes(8, 0, 0, &(16u32 * 1024 * 100).to_be_bytes());
        for id in &ids {
            buf.extend(frame_bytes(8, 0, *id, &(16u32 * 1024).to_be_bytes()));
        }
        c.send(&buf).await;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let later = c.open(20, "/t/big", &[]).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(c.headers(&later).is_empty(), "the pending bytes did not drop: new streams wait");
    assert_eq!(c.resets(&later, ENHANCE_YOUR_CALM), 0);
    assert!(!c.is_closed());
}

/// Open 100 streams, never read, reset them, open 100 more: every round keeps at most eight responses. The pace follows
/// the server (a round ends when its eight kept responses are out), not the clock.
#[tokio::test]
async fn an_rst_cycle_keeps_eight_responses_per_round() {
    let addr = serve(quiet(ServerOptions::new())).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0).await;
    for round in 0..3 {
        let ids = c.open(100, "/t/slow-big", &[]).await;
        until("the eight responses of the round are out", 60, || c.is_closed() || c.headers(&ids).len() >= 8).await;
        // Give a ninth response the chance to show up before resetting.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(c.headers(&ids).len() <= 8, "round {round} kept {}", c.headers(&ids).len());
        c.reset(&ids).await;
    }
}

/// The streams an RST cycle opens and resets count as cancelled requests, whether the server dropped their response,
/// they were still waiting for room, or their handler was running: one round of 100 passes a ceiling of 30 and the
/// connection is told to go away. The rapid-reset rule is a rate (a default floor of 500 cancelled requests per 10 s),
/// so the ceiling is lowered to make the outcome depend on counts, not on how fast the machine runs the cycle.
#[tokio::test]
async fn an_rst_cycle_is_closed_by_the_rapid_reset_rule() {
    let addr = serve(quiet(ServerOptions::new()).http2_max_cancelled_streams(30)).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0).await;
    let ids = c.open(100, "/t/slow-big", &[]).await;
    until("eight responses are out", 60, || c.is_closed() || c.headers(&ids).len() >= 8).await;
    c.reset(&ids).await;
    until("the connection is told to go away", 60, || c.is_closed()).await;
}

/// A request that waits for room longer than `max_pending_response_wait` is refused (the handler never ran, the client
/// may retry), and refusals do not count against the rapid-reset rule: with a ceiling of three, six of them leave the
/// connection open. The streams that were held keep working.
#[tokio::test]
async fn a_request_that_waits_too_long_is_refused_and_not_counted_as_cancelled() {
    let addr = serve(small_cap().max_pending_response_wait(Duration::from_millis(700)).http2_max_cancelled_streams(3)).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0).await;
    let held = c.open(4, "/t/counted-refused", &[]).await;
    until("four responses are held", 10, || c.headers(&held).len() == 4).await;
    let waiting = c.open(6, "/t/counted-refused", &[]).await;
    until("every waiting stream was refused", 10, || c.resets(&waiting, REFUSED_STREAM) == 6).await;
    assert_eq!(COUNTED_REFUSED.load(Ordering::SeqCst), 4, "the handlers of the refused streams never ran");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!c.is_closed(), "refusals are not cancellations");
    // The held responses still flow once the client reads.
    c.grant(held[0], 256 * 1024).await;
    until("a held response is delivered", 10, || c.data_of(held[0]) == 256 * 1024).await;
}

/// One big response read at a trickle must not block the other streams of the connection for ever.
#[tokio::test]
async fn a_slowly_read_big_response_does_not_block_other_streams_for_ever() {
    let addr = serve(small_cap().max_pending_response_wait(Duration::from_millis(800))).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0).await;
    c.send(&frame_bytes(8, 0, 0, &(1u32 << 30).to_be_bytes())).await;
    let big = c.open(1, "/t/huge", &[]).await;
    until("the big response is held", 10, || c.headers(&big).len() == 1).await;
    let small = c.open(5, "/t/ok", &[]).await;
    for _ in 0..8 {
        c.grant(big[0], 16 * 1024).await;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(c.resets(&small, REFUSED_STREAM), 5, "the waiting requests were refused, not left waiting");
    assert!(!c.is_closed());
}

/// Without a limit the wait has no end of its own.
#[tokio::test]
async fn the_wait_can_be_unlimited() {
    let addr = serve(small_cap().max_pending_response_wait(None)).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0).await;
    let held = c.open(4, "/t/quarter", &[]).await;
    until("four responses are held", 10, || c.headers(&held).len() == 4).await;
    let waiting = c.open(1, "/t/ok", &[]).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(c.headers(&waiting).is_empty());
    assert_eq!(c.resets(&waiting, REFUSED_STREAM), 0);
}

/// Bytes hyper drops (a stream longer than its declared length) must not leave the connection waiting for ever for
/// room that never comes back.
#[tokio::test]
async fn an_http1_response_hyper_truncates_does_not_wedge_the_connection() {
    let addr = serve(quiet(ServerOptions::new())).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut buf = vec![0u8; 65536];
    for _ in 0..3 {
        s.write_all(b"GET /t/badlen HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
        let _ = tokio::time::timeout(Duration::from_millis(500), s.read(&mut buf)).await;
    }
    s.write_all(b"GET /t/ok HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let mut got = Vec::new();
    let answered = tokio::time::timeout(Duration::from_secs(3), async {
        while !String::from_utf8_lossy(&got).contains("\"ok\":true") {
            let n = s.read(&mut buf).await.unwrap_or(0);
            if n == 0 {
                return false;
            }
            got.extend_from_slice(&buf[..n]);
        }
        true
    })
    .await;
    assert_eq!(answered, Ok(true), "the next request on the connection was never answered");
}

#[cfg(feature = "compression")]
#[tokio::test]
async fn compressed_responses_count_their_uncompressed_size() {
    let addr = serve_with(small_cap(), true).await;
    let c = H2Raw::connect(addr, SETTINGS_WINDOW_0).await;
    let ids = c.open(20, "/t/slow-quarter", ACCEPT_GZIP).await;
    until("every stream was answered or reset", 10, || c.headers(&ids).len() + c.resets(&ids, ENHANCE_YOUR_CALM) >= 20).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(c.headers(&ids).len(), 4, "compressed or not, each response counts 256 KiB");
    assert_eq!(c.resets(&ids, ENHANCE_YOUR_CALM), 16);
}

#[cfg(not(feature = "compression"))]
#[test]
fn compression_is_not_built() {
    let _ = ACCEPT_GZIP;
}
