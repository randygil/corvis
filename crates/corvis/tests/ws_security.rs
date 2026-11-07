//! Regression tests for the WebSocket hardening (gateways and `corvis::ws`): close deadline
//! after `disconnect()`, bounded send buffers, in-flight handler limit, handshake validation,
//! `max_payload`, panics in hooks and listeners, and `messages()` backpressure.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvis::prelude::*;
use corvis::websockets::GatewayOptions;
use corvis::ws::{WebSocket, WebSocketServer, WebSocketServerOptions};
use corvis::{OnGatewayConnection, OnGatewayInit, WsClient, WsServer, websocket_gateway};
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

// ---------------------------------------------------------------- gateways

/// Rejects every connection with `4401`, like an authentication check in `handle_connection`.
#[websocket_gateway(path = "/kick")]
pub struct KickGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl KickGateway {}

#[async_trait]
impl OnGatewayInit for KickGateway {
    async fn after_init(&self, server: WsServer) {
        server.set_options(GatewayOptions::default().close_timeout(Duration::from_millis(300)));
    }
}

#[async_trait]
impl OnGatewayConnection for KickGateway {
    async fn handle_connection(&self, client: WsClient) {
        client.emit("exception", json!({ "message": "Unauthorized" }));
        client.disconnect_with(4401, "Unauthorized");
    }
}

static FLOOD_OK: AtomicUsize = AtomicUsize::new(0);
static RUNNING: AtomicUsize = AtomicUsize::new(0);
static MAX_RUNNING: AtomicUsize = AtomicUsize::new(0);

#[websocket_gateway(path = "/gw")]
pub struct MainGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl MainGateway {
    /// Sends 400 frames of 64 KiB to the caller (25 MiB), which does not read.
    #[subscribe_message("flood")]
    fn flood(&self, #[connected_socket] client: WsClient) {
        let chunk = "x".repeat(64 * 1024);
        let ok = (0..400).filter(|_| client.send_text(chunk.clone())).count();
        FLOOD_OK.store(ok, Ordering::SeqCst);
    }

    #[subscribe_message("slow")]
    async fn slow(&self) {
        let now = RUNNING.fetch_add(1, Ordering::SeqCst) + 1;
        MAX_RUNNING.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(100)).await;
        RUNNING.fetch_sub(1, Ordering::SeqCst);
    }

    #[subscribe_message("echo")]
    fn echo(&self, #[message_body] data: Value) -> Value {
        data
    }
}

#[async_trait]
impl OnGatewayInit for MainGateway {
    async fn after_init(&self, server: WsServer) {
        server.set_options(
            GatewayOptions::default()
                .max_buffered_amount(1024 * 1024)
                .max_inflight(4)
                .close_timeout(Duration::from_millis(300)),
        );
    }
}

#[async_trait]
impl OnGatewayConnection for MainGateway {
    async fn handle_connection(&self, client: WsClient) {
        if client.handshake().query_param("boom").is_some() {
            panic!("handle_connection panic");
        }
    }
}

#[websocket_gateway(path = "/strict")]
pub struct StrictGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl StrictGateway {
    #[subscribe_message("echo")]
    fn echo(&self, #[message_body] data: Value) -> Value {
        data
    }
}

#[async_trait]
impl OnGatewayInit for StrictGateway {
    async fn after_init(&self, server: WsServer) {
        server.set_options(
            GatewayOptions::default()
                .allowed_origins(["https://chat.example"])
                .verify_client(|req| req.query_param("deny").is_none()),
        );
    }
}

/// Default options (`max_payload` 1 MiB).
#[websocket_gateway(path = "/plain")]
pub struct PlainGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl PlainGateway {
    #[subscribe_message("echo")]
    fn echo(&self, #[message_body] data: Value) -> Value {
        data
    }
}

// ---------------------------------------------------------------- corvis::ws

#[derive(Default)]
pub struct RawState {
    wss: Mutex<Option<WebSocketServer>>,
    sockets: Mutex<Vec<WebSocket>>,
}

#[injectable]
pub struct RawService {
    adapter_host: Arc<HttpAdapterHost>,
    #[default]
    state: Arc<RawState>,
}

#[async_trait]
impl OnApplicationBootstrap for RawService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        let server = self.adapter_host.http_adapter().get_http_server();
        let wss = WebSocketServer::new(
            WebSocketServerOptions::default().close_timeout(Duration::from_millis(300)).max_unread_bytes(2 * 1024 * 1024),
        );
        *self.state.wss.lock().unwrap() = Some(wss.clone());
        let state = self.state.clone();
        server.on_upgrade(move |req| {
            let path = req.pathname().to_string();
            let state = state.clone();
            wss.handle_upgrade(&req, move |ws| {
                state.sockets.lock().unwrap().push(ws.clone());
                match path.as_str() {
                    "/raw/panic" => {
                        ws.on_message(|m| {
                            if m.to_text() == "boom" {
                                panic!("listener panic");
                            }
                        });
                    }
                    "/raw/stream" => {
                        // Never polled: the connection must stop reading instead of buffering.
                        let stream = ws.messages();
                        std::mem::forget(stream);
                    }
                    _ => {}
                }
            });
        });
        Ok(())
    }
}

#[module(providers = [KickGateway, MainGateway, StrictGateway, PlainGateway, RawService])]
pub struct AppModule;

// ---------------------------------------------------------------- helpers

async fn start() -> (CorvisApplication, SocketAddr) {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr)
}

/// A WebSocket client on a raw socket that only reads when asked to.
async fn raw_connect(addr: SocketAddr, path: &str, extra: &str, recv_buffer: Option<u32>) -> (TcpStream, String) {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    if let Some(size) = recv_buffer {
        socket.set_recv_buffer_size(size).unwrap();
    }
    let mut s = socket.connect(addr).await.unwrap();
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n{extra}\r\n"
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if s.read(&mut b).await.unwrap() == 0 {
            break;
        }
        head.push(b[0]);
    }
    (s, String::from_utf8_lossy(&head).into_owned())
}

/// Sends a raw HTTP request and returns the status line.
async fn status_line(addr: SocketAddr, request: String) -> String {
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        let mut buf = [0u8; 1024];
        while !out.windows(2).any(|w| w == b"\r\n") {
            match s.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
    })
    .await;
    String::from_utf8_lossy(&out).lines().next().unwrap_or("").to_string()
}

fn masked_text(payload: &str) -> Vec<u8> {
    let p = payload.as_bytes();
    let mut f = vec![0x81u8];
    match p.len() {
        n if n < 126 => f.push(0x80 | n as u8),
        n if n < 65536 => {
            f.push(0x80 | 126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            f.push(0x80 | 127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    f.extend_from_slice(&[0, 0, 0, 0]); // zero mask
    f.extend_from_slice(p);
    f
}

/// Reads until EOF (or error); `true` if the server closed the TCP connection in time.
async fn closed_within(s: &mut TcpStream, timeout: Duration) -> bool {
    tokio::time::timeout(timeout, async {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match s.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(_) => continue,
            }
        }
    })
    .await
    .is_ok()
}

async fn eventually(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < end {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    f()
}

async fn close_code<S>(ws: &mut S) -> Option<u16>
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    while let Ok(Some(m)) = tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
        match m {
            Ok(Message::Close(f)) => return Some(f.map(|f| u16::from(f.code)).unwrap_or(1005)),
            Ok(_) => continue,
            Err(_) => return None,
        }
    }
    None
}

// ---------------------------------------------------------------- tests

/// auth-ws #5: a client that ignores the close frame leaves `clients()` at once and its TCP
/// connection is destroyed after `close_timeout`.
#[corvis::test]
async fn gateway_disconnect_removes_client_and_closes_tcp() {
    let (mut app, addr) = start().await;
    let gw = app.get::<KickGateway>().unwrap();
    let mut socks = Vec::new();
    for _ in 0..5 {
        let (s, head) = raw_connect(addr, "/kick", "", None).await;
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        socks.push(s);
    }
    assert!(eventually(Duration::from_millis(500), || gw.server.count() == 0).await, "count = {}", gw.server.count());
    for s in &mut socks {
        assert!(closed_within(s, Duration::from_secs(3)).await, "the server kept the TCP connection open");
    }
    app.close().await.unwrap();
}

/// auth-ws #6: the send buffer of a client that does not read is bounded; when it overflows
/// the client is disconnected.
#[corvis::test]
async fn gateway_send_buffer_is_bounded() {
    let (mut app, addr) = start().await;
    let gw = app.get::<MainGateway>().unwrap();
    let (mut s, head) = raw_connect(addr, "/gw", "", Some(4096)).await;
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    s.write_all(&masked_text(&json!({ "event": "flood" }).to_string())).await.unwrap();
    assert!(eventually(Duration::from_secs(5), || FLOOD_OK.load(Ordering::SeqCst) > 0).await);
    let ok = FLOOD_OK.load(Ordering::SeqCst);
    assert!(ok < 400, "all {ok} frames (25 MiB) were queued for a client that does not read");
    assert!(eventually(Duration::from_secs(1), || gw.server.count() == 0).await);
    assert!(closed_within(&mut s, Duration::from_secs(5)).await);
    app.close().await.unwrap();
}

/// auth-ws #6: at most `max_inflight` handlers of one connection run at once.
#[corvis::test]
async fn gateway_inflight_handlers_are_bounded() {
    let (mut app, addr) = start().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/gw")).await.unwrap();
    for _ in 0..40 {
        ws.send(Message::Text(json!({ "event": "slow" }).to_string())).await.unwrap();
    }
    ws.send(Message::Text(json!({ "event": "echo", "data": "done" }).to_string())).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(10), ws.next()).await.unwrap().unwrap().unwrap();
    assert_eq!(reply.into_text().unwrap(), json!({ "event": "echo", "data": "done" }).to_string());
    let max = MAX_RUNNING.load(Ordering::SeqCst);
    assert!(max <= 4, "{max} handlers ran at once");
    app.close().await.unwrap();
}

/// auth-ws #12: handshake checks of `ws` (method, key, version), `allowed_origins`,
/// `verify_client` and `max_payload`.
#[corvis::test]
async fn gateway_handshake_is_validated() {
    let (mut app, addr) = start().await;
    let req = |method: &str, path: &str, key: &str, extra: &str| {
        format!("{method} {path} HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n{key}{extra}\r\n")
    };
    let good_key = "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n";
    let v13 = "Sec-WebSocket-Version: 13\r\n";
    assert!(status_line(addr, req("POST", "/gw", good_key, v13)).await.starts_with("HTTP/1.1 405"));
    assert!(status_line(addr, req("GET", "/gw", "Sec-WebSocket-Key: no-es-base64\r\n", v13)).await.starts_with("HTTP/1.1 400"));
    assert!(status_line(addr, req("GET", "/gw", good_key, "")).await.starts_with("HTTP/1.1 400"));
    assert!(status_line(addr, req("GET", "/gw", good_key, v13)).await.starts_with("HTTP/1.1 101"));
    let evil = format!("{v13}Origin: https://evil.example\r\n");
    assert!(status_line(addr, req("GET", "/strict", good_key, &evil)).await.starts_with("HTTP/1.1 403"));
    let ok = format!("{v13}Origin: https://CHAT.example\r\n");
    assert!(status_line(addr, req("GET", "/strict", good_key, &ok)).await.starts_with("HTTP/1.1 101"));
    assert!(status_line(addr, req("GET", "/strict?deny=1", good_key, v13)).await.starts_with("HTTP/1.1 401"));

    // Default `max_payload` (1 MiB): a 2 MiB message is not handled; the connection is closed
    // with 1009 (the client may see a reset instead, as with `ws`, which destroys the socket).
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/plain")).await.unwrap();
    let big = json!({ "event": "echo", "data": "y".repeat(2 * 1024 * 1024) }).to_string();
    let _ = ws.send(Message::Text(big)).await;
    let next = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("the oversized message was not rejected");
    match next {
        Some(Ok(Message::Close(f))) => assert_eq!(f.map(|f| u16::from(f.code)), Some(1009)),
        Some(Ok(other)) => panic!("unexpected reply to an oversized message: {:?}", other.into_text().map(|t| t.len())),
        None | Some(Err(_)) => {}
    }
    app.close().await.unwrap();
}

/// auth-ws #9: a panic in `handle_connection` closes the connection (1011) and leaves no
/// client behind.
#[corvis::test]
async fn gateway_hook_panic_cleans_up() {
    let (mut app, addr) = start().await;
    let gw = app.get::<MainGateway>().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/gw?boom=1")).await.unwrap();
    assert_eq!(close_code(&mut ws).await, Some(1011));
    assert!(eventually(Duration::from_secs(1), || gw.server.count() == 0).await, "count = {}", gw.server.count());
    app.close().await.unwrap();
}

/// auth-ws #9: a panicking `corvis::ws` listener closes its socket (1011) and the socket
/// leaves `wss.clients()`.
#[corvis::test]
async fn ws_listener_panic_cleans_up() {
    let (mut app, addr) = start().await;
    let raw = app.get::<RawService>().unwrap();
    let wss = raw.state.wss.lock().unwrap().clone().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/raw/panic")).await.unwrap();
    ws.send(Message::Text("boom".into())).await.unwrap();
    assert_eq!(close_code(&mut ws).await, Some(1011));
    assert!(eventually(Duration::from_secs(1), || wss.clients().is_empty()).await);
    app.close().await.unwrap();
}

/// auth-ws #6: `ws.send` to a peer that does not read fails once `max_buffered_amount` is
/// reached, and the connection is destroyed after `close_timeout`.
#[corvis::test]
async fn ws_send_buffer_is_bounded() {
    let (mut app, addr) = start().await;
    let raw = app.get::<RawService>().unwrap();
    let (mut s, head) = raw_connect(addr, "/raw/quiet", "", Some(4096)).await;
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(eventually(Duration::from_secs(1), || !raw.state.sockets.lock().unwrap().is_empty()).await);
    let ws = raw.state.sockets.lock().unwrap().last().cloned().unwrap();
    let chunk = vec![7u8; 64 * 1024];
    let ok = (0..400).take_while(|_| ws.send(chunk.clone()).is_ok()).count();
    assert!(ok < 400, "all {ok} frames (25 MiB) were queued for a peer that does not read");
    assert!(ws.buffered_amount() <= 8 * 1024 * 1024 + 64 * 1024 + 64, "{}", ws.buffered_amount());
    let closed = tokio::time::timeout(Duration::from_secs(3), ws.closed()).await;
    assert!(closed.is_ok(), "the connection was not destroyed");
    assert!(closed_within(&mut s, Duration::from_secs(3)).await);
    app.close().await.unwrap();
}

/// auth-ws #6: when nobody reads `messages()`, the connection stops reading from the socket
/// instead of buffering without limit.
#[corvis::test]
async fn ws_messages_stream_applies_backpressure() {
    let (mut app, addr) = start().await;
    let (mut s, head) = raw_connect(addr, "/raw/stream", "", None).await;
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    let frame = masked_text(&"z".repeat(1024 * 1024));
    let sent = tokio::time::timeout(Duration::from_secs(3), async {
        for _ in 0..96 {
            s.write_all(&frame).await.unwrap();
        }
    })
    .await;
    assert!(sent.is_err(), "96 MiB were accepted while the stream was never read");
    app.close().await.unwrap();
}
