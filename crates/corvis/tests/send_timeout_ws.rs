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

static DISCONNECTS: AtomicUsize = AtomicUsize::new(0);
static CLIENTS_AT_DISCONNECT: AtomicUsize = AtomicUsize::new(usize::MAX);
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
        emit_burst(&client);
    }
}

#[async_trait]
impl OnGatewayDisconnect for FeedGateway {
    async fn handle_disconnect(&self, _client: WsClient) {
        CLIENTS_AT_DISCONNECT.store(self.server.clients().len(), Ordering::SeqCst);
        DISCONNECTS.fetch_add(1, Ordering::SeqCst);
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
            if req.pathname() != "/raw" {
                return;
            }
            wss.handle_upgrade(&req, |ws| {
                ws.on_close(|code, _| *RAW_CLOSE.lock().unwrap() = Some(code));
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
    DISCONNECTS.store(0, Ordering::SeqCst);
    let (_app, addr) = start(timeout_1s()).await;
    let mut ws = connect(addr, "/feed").await;

    wait_for("handle_disconnect", Duration::from_secs(8), || DISCONNECTS.load(Ordering::SeqCst) == 1).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(DISCONNECTS.load(Ordering::SeqCst), 1, "handle_disconnect runs once");
    assert_eq!(CLIENTS_AT_DISCONNECT.load(Ordering::SeqCst), 0, "server.clients() is empty");
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
