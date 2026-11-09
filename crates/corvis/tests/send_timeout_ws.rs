//! `send_timeout` also applies to upgraded connections, to their writes only: a WebSocket peer whose socket accepts
//! nothing for the timeout loses its connection (no close frame, `1006` locally), while idle connections and slow but
//! live readers keep theirs.
#![cfg(feature = "websockets")]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use corvis::prelude::*;
use corvis::ws::{WebSocketServer, WebSocketServerOptions};
use corvis::{HttpAdapterHost, OnGatewayConnection, OnGatewayDisconnect, ServerOptions, WsClient, WsServer, websocket_gateway};
use futures::{SinkExt, StreamExt};
use tokio::net::{TcpSocket, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const MESSAGES: usize = 24;
const PAYLOAD: usize = 128 * 1024;

/// Ids of the `/feed` clients that connected, and `(id, clients left in the server)` of each `handle_disconnect`.
/// Connections of earlier tests close after their test ended, so a test only looks at the id of its own client.
static FEED_CONNECTED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static FEED_DISCONNECTS: Mutex<Vec<(String, usize)>> = Mutex::new(Vec::new());
static RAW_MESSAGE_LEN: AtomicUsize = AtomicUsize::new(0);
static SPLIT_WRITER_DONE: AtomicUsize = AtomicUsize::new(0);
static RAW_CLOSES: AtomicUsize = AtomicUsize::new(0);
static RAW_CLOSE: Mutex<Option<u16>> = Mutex::new(None);
static RAW_ACK: Mutex<Option<bool>> = Mutex::new(None);
/// The tests share the counters above: they run one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 3 MiB at once: more than the kernel buffers of a client that does not read take, less than `max_buffered_amount`.
fn emit_burst(client: &WsClient) {
    for i in 0..MESSAGES {
        client.emit("chunk", json!({ "i": i, "pad": "x".repeat(PAYLOAD) }));
    }
    client.emit("done", json!({}));
}

#[websocket_gateway(path = "/feed")]
pub struct FeedGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl FeedGateway {}

#[async_trait]
impl OnGatewayConnection for FeedGateway {
    async fn handle_connection(&self, client: WsClient) {
        FEED_CONNECTED.lock().unwrap().push(client.id().to_string());
        emit_burst(&client);
    }
}

#[async_trait]
impl OnGatewayDisconnect for FeedGateway {
    async fn handle_disconnect(&self, client: WsClient) {
        FEED_DISCONNECTS.lock().unwrap().push((client.id().to_string(), self.server.clients().len()));
    }
}

/// Sends the burst and starts the closing handshake at once.
#[websocket_gateway(path = "/closing")]
pub struct ClosingGateway;

#[handlers]
impl ClosingGateway {}

#[async_trait]
impl OnGatewayConnection for ClosingGateway {
    async fn handle_connection(&self, client: WsClient) {
        emit_burst(&client);
        client.disconnect();
    }
}

/// Never sends anything.
#[websocket_gateway(path = "/quiet")]
pub struct QuietGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl QuietGateway {}

#[injectable]
pub struct RawService {
    adapter_host: Arc<HttpAdapterHost>,
}

#[async_trait]
impl OnApplicationBootstrap for RawService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        let server = self.adapter_host.http_adapter().get_http_server();
        let wss = WebSocketServer::new(WebSocketServerOptions::default());
        server.on_upgrade(move |req| {
            if req.pathname() == "/rawclose" {
                wss.handle_upgrade(&req, |ws| {
                    ws.on_close(|_, _| {
                        RAW_CLOSES.fetch_add(1, Ordering::SeqCst);
                    });
                    ws.close(Some(1000), "").unwrap();
                });
                return;
            }
            if req.pathname() == "/split" {
                // A custom protocol with a reader task and a writer task on the same upgraded socket.
                let mut headers = http::HeaderMap::new();
                headers.insert("upgrade", http::HeaderValue::from_static("chat"));
                headers.insert("connection", http::HeaderValue::from_static("Upgrade"));
                let Ok(upgraded) = req.switch_protocols(headers) else { return };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let Ok(io) = upgraded.await else { return };
                    let (mut read, mut write) = tokio::io::split(io);
                    tokio::spawn(async move {
                        let chunk = vec![b'x'; 64 * 1024];
                        while write.write_all(&chunk).await.is_ok() {}
                        SPLIT_WRITER_DONE.store(1, Ordering::SeqCst);
                    });
                    tokio::spawn(async move {
                        let mut buf = [0u8; 1024];
                        while matches!(read.read(&mut buf).await, Ok(n) if n > 0) {}
                    });
                });
                return;
            }
            if req.pathname() != "/raw" {
                return;
            }
            wss.handle_upgrade(&req, |ws| {
                ws.on_close(|code, _| *RAW_CLOSE.lock().unwrap() = Some(code));
                ws.on_message(|m| RAW_MESSAGE_LEN.store(m.to_text().len(), Ordering::SeqCst));
                for _ in 0..3 {
                    ws.send(vec![b'x'; 1024 * 1024]).unwrap();
                }
                tokio::spawn(async move {
                    let ok = ws.send_and_wait("after").await.is_ok();
                    *RAW_ACK.lock().unwrap() = Some(ok);
                });
            });
        });
        Ok(())
    }
}

#[module(providers = [FeedGateway, ClosingGateway, QuietGateway, RawService])]
pub struct AppModule;

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn start(options: ServerOptions) -> (CorvisApplication, SocketAddr) {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.set_server_options(options);
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr)
}

fn timeout_1s() -> ServerOptions {
    ServerOptions::new().send_timeout(Duration::from_secs(1))
}

/// A WebSocket client with a 4 KiB receive buffer.
async fn connect(addr: SocketAddr, path: &str) -> Client {
    let socket = TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let stream = socket.connect(addr).await.unwrap();
    let (ws, _) = tokio_tungstenite::client_async(format!("ws://{addr}{path}"), MaybeTlsStream::Plain(stream)).await.unwrap();
    ws
}

/// Reads until `done`, or until the stream ends or errors. Returns the chunks seen and whether `done` arrived.
async fn read_to_end(ws: &mut Client, limit: Duration) -> (usize, bool) {
    let mut chunks = 0usize;
    let end = Instant::now() + limit;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        let msg = tokio::time::timeout(left, ws.next()).await.expect("the connection ended in time");
        match msg {
            Some(Ok(Message::Text(t))) => {
                let v: Value = serde_json::from_str(&t).unwrap();
                match v["event"].as_str() {
                    Some("chunk") => chunks += 1,
                    Some("done") => return (chunks, true),
                    _ => {}
                }
            }
            Some(Ok(_)) => {}
            Some(Err(_)) | None => return (chunks, false),
        }
    }
}

async fn wait_for(what: &str, limit: Duration, mut cond: impl FnMut() -> bool) {
    let end = Instant::now() + limit;
    while !cond() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[corvis::test]
async fn a_gateway_client_that_never_reads_is_dropped() {
    let _g = SERIAL.lock().await;
    FEED_CONNECTED.lock().unwrap().clear();
    let (_app, addr) = start(timeout_1s()).await;
    let mut ws = connect(addr, "/feed").await;
    wait_for("handle_connection", Duration::from_secs(8), || FEED_CONNECTED.lock().unwrap().len() == 1).await;
    let id = FEED_CONNECTED.lock().unwrap()[0].clone();
    let mine = || FEED_DISCONNECTS.lock().unwrap().iter().filter(|(i, _)| *i == id).cloned().collect::<Vec<_>>();

    wait_for("handle_disconnect", Duration::from_secs(8), || mine().len() == 1).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(mine().len(), 1, "handle_disconnect runs once");
    assert_eq!(mine()[0].1, 0, "server.clients() is empty");
    let (_, done) = read_to_end(&mut ws, Duration::from_secs(5)).await;
    assert!(!done, "the connection ended before the burst was delivered");
}

#[corvis::test]
async fn the_dropped_client_leaves_its_slot() {
    let _g = SERIAL.lock().await;
    let (_app, addr) = start(timeout_1s().max_connections(1)).await;
    let _stalled = connect(addr, "/feed").await;
    let started = Instant::now();
    let second = tokio::time::timeout(Duration::from_secs(8), async {
        let stream = TcpStream::connect(addr).await.unwrap();
        tokio_tungstenite::client_async(format!("ws://{addr}/quiet"), MaybeTlsStream::Plain(stream)).await
    })
    .await
    .expect("the second client is served once the first is dropped");
    assert!(second.is_ok());
    assert!(started.elapsed() >= Duration::from_millis(500), "the slot was taken at first");
}

#[corvis::test]
async fn a_client_that_pings_but_never_reads_is_dropped() {
    let _g = SERIAL.lock().await;
    let (_app, addr) = start(timeout_1s()).await;
    let mut ws = connect(addr, "/feed").await;

    let end = Instant::now() + Duration::from_secs(8);
    let mut dropped = false;
    while Instant::now() < end {
        // A closed connection makes the write fail.
        if ws.send(Message::Ping(b"hi".to_vec())).await.is_err() {
            dropped = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(dropped, "pings kept a stalled connection alive");
}

#[corvis::test]
async fn an_idle_connection_is_not_dropped() {
    let _g = SERIAL.lock().await;
    let (_app, addr) = start(timeout_1s()).await;
    let mut ws = connect(addr, "/quiet").await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    // Still usable: a ping is answered with a pong.
    ws.send(Message::Ping(b"p".to_vec())).await.unwrap();
    let msg = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("a pong in time");
    assert!(matches!(msg, Some(Ok(Message::Pong(_)))), "got {msg:?}");
}

#[corvis::test]
async fn a_slow_reader_keeps_its_connection() {
    let _g = SERIAL.lock().await;
    let (_app, addr) = start(timeout_1s()).await;
    let mut ws = connect(addr, "/feed").await;
    let mut chunks = 0usize;
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next()).await.expect("a message in time");
        let Some(Ok(msg)) = msg else { panic!("the connection ended after {chunks} chunks") };
        if let Message::Text(t) = msg {
            let v: Value = serde_json::from_str(&t).unwrap();
            match v["event"].as_str() {
                Some("chunk") => {
                    chunks += 1;
                    tokio::time::sleep(Duration::from_millis(400)).await;
                }
                Some("done") => break,
                _ => {}
            }
        }
    }
    assert_eq!(chunks, MESSAGES);
}

#[corvis::test]
async fn a_websocket_client_that_stops_reading_keeps_its_connection_without_a_send_timeout() {
    let _g = SERIAL.lock().await;
    let (_app, addr) = start(ServerOptions::new().send_timeout(None)).await;
    let mut ws = connect(addr, "/feed").await;

    // Not reading for three seconds.
    tokio::time::sleep(Duration::from_secs(3)).await;

    let (chunks, done) = read_to_end(&mut ws, Duration::from_secs(10)).await;
    assert!(done, "the connection ended after {chunks} chunks");
    assert_eq!(chunks, MESSAGES);
}

#[corvis::test]
async fn the_deadline_ends_a_closing_handshake_early() {
    let _g = SERIAL.lock().await;
    let (_app, addr) = start(timeout_1s()).await;
    let mut ws = connect(addr, "/closing").await;
    let started = Instant::now();
    // Not reading: the close frame cannot be delivered; the connection must end after about send_timeout, not after
    // the 30 s close_timeout. Probe the socket by reading once the server has had time to drop it.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let (_, done) = read_to_end(&mut ws, Duration::from_secs(5)).await;
    assert!(!done);
    assert!(started.elapsed() < Duration::from_secs(15));
}

#[corvis::test]
async fn a_plain_websocket_that_never_reads_is_closed_with_1006() {
    let _g = SERIAL.lock().await;
    *RAW_CLOSE.lock().unwrap() = None;
    *RAW_ACK.lock().unwrap() = None;
    let (_app, addr) = start(timeout_1s()).await;
    let _ws = connect(addr, "/raw").await;

    wait_for("on_close", Duration::from_secs(8), || RAW_CLOSE.lock().unwrap().is_some()).await;
    assert_eq!(*RAW_CLOSE.lock().unwrap(), Some(1006));
    wait_for("send_and_wait", Duration::from_secs(2), || RAW_ACK.lock().unwrap().is_some()).await;
    assert_eq!(*RAW_ACK.lock().unwrap(), Some(false));
}

/// A client on a raw socket (4 KiB receive buffer) that completed the handshake and reads nothing else.
async fn raw_client(addr: SocketAddr, path: &str) -> TcpStream {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let socket = TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let mut stream = socket.connect(addr).await.unwrap();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 101"), "{}", String::from_utf8_lossy(&head));
    stream
}

/// Sends masked empty pings until the socket stops accepting bytes for half a second. Returns the bytes accepted.
async fn ping_until_blocked(stream: &TcpStream, cap: usize) -> usize {
    const PING: [u8; 6] = [0x89, 0x80, 0, 0, 0, 0];
    let batch: Vec<u8> = PING.iter().copied().cycle().take(PING.len() * 10_000).collect();
    let mut sent = 0usize;
    while sent < cap {
        match tokio::time::timeout(Duration::from_millis(500), stream.writable()).await {
            Err(_) => break,
            Ok(r) => r.unwrap(),
        }
        match stream.try_write(&batch) {
            Ok(n) => sent += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => break,
        }
    }
    sent
}

/// Bytes of pings a peer that never reads gets the server to take before the server stops reading.
const PING_BUDGET: usize = 24 * 1024 * 1024;

#[corvis::test]
async fn a_ping_flood_without_reading_is_bounded_on_a_gateway() {
    let _g = SERIAL.lock().await;
    let (_app, addr) = start(ServerOptions::new().send_timeout(Duration::from_secs(30))).await;
    let stream = raw_client(addr, "/feed").await;
    let sent = ping_until_blocked(&stream, 4 * PING_BUDGET).await;
    assert!(sent < PING_BUDGET, "the server kept reading pings: {sent} bytes accepted");
}

#[corvis::test]
async fn a_ping_flood_without_reading_is_bounded_on_a_plain_websocket() {
    let _g = SERIAL.lock().await;
    let (_app, addr) = start(ServerOptions::new().send_timeout(Duration::from_secs(30))).await;
    let stream = raw_client(addr, "/raw").await;
    let sent = ping_until_blocked(&stream, 4 * PING_BUDGET).await;
    assert!(sent < PING_BUDGET, "the server kept reading pings: {sent} bytes accepted");
}

/// The server starts the closing handshake; the peer answers with an invalid frame instead of a close reply. The
/// close event must run exactly once (the writer has already finished: it must not be polled again).
#[corvis::test]
async fn an_invalid_frame_during_the_closing_handshake_closes_the_plain_socket_once() {
    use tokio::io::AsyncWriteExt;
    let _g = SERIAL.lock().await;
    RAW_CLOSES.store(0, Ordering::SeqCst);
    let (_app, addr) = start(timeout_1s()).await;
    let mut stream = raw_client(addr, "/rawclose").await;
    // Let the close frame go out and the writer finish before the invalid frame arrives.
    tokio::time::sleep(Duration::from_millis(300)).await;
    stream.write_all(&[0x81, 0x82, 0, 0, 0, 0, 0xff, 0xfe]).await.unwrap();
    wait_for("on_close", Duration::from_secs(5), || RAW_CLOSES.load(Ordering::SeqCst) >= 1).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(RAW_CLOSES.load(Ordering::SeqCst), 1);
}

/// A peer that floods pings and reads continuously in small chunks: what the server accepts stays tied to what it
/// writes back, so no stock of pongs builds up and the connection is not stalled either.
#[corvis::test]
async fn a_ping_flood_with_a_trickle_reader_stays_proportional_to_what_is_read() {
    use tokio::io::AsyncReadExt;
    let _g = SERIAL.lock().await;
    let (_app, addr) = start(ServerOptions::new().send_timeout(Duration::from_secs(10))).await;
    let stream = raw_client(addr, "/feed").await;
    let (mut read_half, write_half) = stream.into_split();
    let read = Arc::new(AtomicUsize::new(0));
    let counter = read.clone();
    let reader = tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        while let Ok(n) = read_half.read(&mut buf).await {
            if n == 0 {
                break;
            }
            counter.fetch_add(n, Ordering::SeqCst);
        }
    });
    // Masked pings with a 125-byte payload: each is answered with a 127-byte pong.
    let mut ping = vec![0x89, 0x80 | 125, 0, 0, 0, 0];
    ping.extend([b'p'; 125]);
    let batch: Vec<u8> = ping.iter().copied().cycle().take(ping.len() * 100).collect();
    let mut sent = 0usize;
    let end = Instant::now() + Duration::from_secs(4);
    while Instant::now() < end {
        match tokio::time::timeout(Duration::from_millis(200), write_half.writable()).await {
            Ok(Ok(())) => {}
            _ => continue,
        }
        if let Ok(n) = write_half.try_write(&batch) {
            sent += n;
        }
    }
    reader.abort();
    let received = read.load(Ordering::SeqCst);
    // The reader also drains the 3 MiB the gateway queued at connect.
    let pongs_read = received.saturating_sub(3 * 1024 * 1024 + 64 * 1024);
    assert!(sent < 2 * pongs_read + 16 * 1024 * 1024, "sent {sent} bytes of pings, the client read {received} bytes");
}

/// A custom protocol with separate reader and writer tasks: the client sends a byte now and then but never reads.
/// The deadline fires on the reader's poll as well; the writer must be woken and fail too.
#[corvis::test]
async fn a_split_writer_ends_at_the_deadline_while_the_peer_sends_bytes() {
    use tokio::io::AsyncWriteExt;
    let _g = SERIAL.lock().await;
    SPLIT_WRITER_DONE.store(0, Ordering::SeqCst);
    let (_app, addr) = start(timeout_1s()).await;
    let mut stream = raw_client(addr, "/split").await;
    let end = Instant::now() + Duration::from_secs(8);
    while SPLIT_WRITER_DONE.load(Ordering::SeqCst) == 0 && Instant::now() < end {
        let _ = stream.write_all(b"k").await;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(SPLIT_WRITER_DONE.load(Ordering::SeqCst), 1, "the writer task hung");
}

/// A half-duplex client writes a moderately large message before it reads, while the server streams to it: the read
/// budget lets the message through.
#[corvis::test]
async fn a_message_written_before_reading_is_processed_while_the_server_streams() {
    use tokio::io::AsyncWriteExt;
    let _g = SERIAL.lock().await;
    RAW_MESSAGE_LEN.store(0, Ordering::SeqCst);
    let (_app, addr) = start(ServerOptions::new().send_timeout(Duration::from_secs(30))).await;
    let mut stream = raw_client(addr, "/raw").await;
    let len = 32 * 1024usize;
    let mut frame = vec![0x81, 0xFE, (len >> 8) as u8, len as u8, 0, 0, 0, 0];
    frame.extend(std::iter::repeat_n(b'a', len));
    tokio::time::timeout(Duration::from_secs(5), stream.write_all(&frame)).await.expect("the write completes").unwrap();
    wait_for("the message", Duration::from_secs(5), || RAW_MESSAGE_LEN.load(Ordering::SeqCst) == len).await;
}
