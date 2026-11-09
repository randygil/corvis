//! WebSockets over TLS (`wss://`): the same gateways, the send timeout on upgraded connections, and the read pause
//! below TLS: a peer that never reads must not get the server to read, and queue replies for, an unbounded amount of
//! ciphertext, whether the replies are pongs or TLS's own (`KeyUpdate`).
#![cfg(all(feature = "tls", feature = "websockets"))]

mod tls_common;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use corvis::prelude::*;
use corvis::tls::TlsOptions;
use corvis::{OnGatewayConnection, OnGatewayDisconnect, ServerOptions, WsClient, WsServer, websocket_gateway};
use futures::{SinkExt, StreamExt};
use tls_common::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

const MESSAGES: usize = 24;
const PAYLOAD: usize = 128 * 1024;

static DISCONNECTS: AtomicUsize = AtomicUsize::new(0);
/// The tests share the counter above: they run one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[websocket_gateway(path = "/chat")]
pub struct ChatGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl ChatGateway {
    #[subscribe_message("echo")]
    async fn echo(&self, #[message_body] data: Value) -> Value {
        data
    }
}

#[async_trait]
impl OnGatewayConnection for ChatGateway {
    async fn handle_connection(&self, client: WsClient) {
        client.emit("welcome", json!({ "id": client.id() }));
    }
}

/// Sends 3 MiB at once on connection, more than the buffers of a client that does not read take.
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
        for i in 0..MESSAGES {
            client.emit("chunk", json!({ "i": i, "pad": "x".repeat(PAYLOAD) }));
        }
        client.emit("done", json!({}));
    }
}

#[async_trait]
impl OnGatewayDisconnect for FeedGateway {
    async fn handle_disconnect(&self, _client: WsClient) {
        DISCONNECTS.fetch_add(1, Ordering::SeqCst);
    }
}

#[module(providers = [ChatGateway, FeedGateway])]
pub struct AppModule;

async fn serve(tls: TlsOptions, options: ServerOptions) -> SocketAddr {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.set_server_options(options);
    let addr = app.listen_tls_in_background("127.0.0.1:0", tls).await.unwrap();
    std::mem::forget(app);
    addr
}

type Ws = WebSocketStream<TlsStream<TcpStream>>;

async fn wss(tls: TlsStream<TcpStream>, addr: SocketAddr, path: &str) -> Ws {
    let (ws, _) = tokio_tungstenite::client_async(format!("wss://localhost:{}{path}", addr.port()), tls).await.unwrap();
    ws
}

async fn wait_for(what: &str, limit: Duration, mut cond: impl FnMut() -> bool) {
    let end = Instant::now() + limit;
    while !cond() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn next_json(ws: &mut Ws) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("timeout").unwrap().unwrap();
        if let Message::Text(t) = msg {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

#[corvis::test]
async fn a_gateway_answers_over_wss_like_over_ws() {
    let pki = pki();
    let addr = serve(options(&pki), ServerOptions::new()).await;
    let mut ws = wss(connect(addr, &pki, H1).await, addr, "/chat").await;
    assert_eq!(next_json(&mut ws).await["event"], "welcome");
    ws.send(Message::Text(json!({ "event": "echo", "data": { "a": 1 } }).to_string())).await.unwrap();
    let reply = next_json(&mut ws).await;
    assert_eq!(reply["event"], "echo");
    assert_eq!(reply["data"], json!({ "a": 1 }));
}

#[corvis::test]
async fn a_gateway_client_over_tls_that_never_reads_is_dropped() {
    let _g = SERIAL.lock().await;
    DISCONNECTS.store(0, Ordering::SeqCst);
    let pki = pki();
    let addr = serve(options(&pki), ServerOptions::new().send_timeout(Duration::from_secs(1))).await;
    let _ws = wss(connect_tiny(addr, &pki, H1).await, addr, "/feed").await;
    wait_for("handle_disconnect", Duration::from_secs(10), || DISCONNECTS.load(Ordering::SeqCst) == 1).await;
}

/// A raw WebSocket handshake over TLS; the stream after the `101`.
async fn raw_upgrade(addr: SocketAddr, pki: &Pki, path: &str) -> TlsStream<TcpStream> {
    let mut s = connect_tiny(addr, pki, H1).await;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    );
    send(&mut s, request.as_bytes()).await;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 101"), "{}", String::from_utf8_lossy(&head));
    s
}

/// Bytes of plaintext a peer that never reads gets the server to take before the server stops reading.
const PING_BUDGET: usize = 4 * 1024 * 1024;

/// Sends masked empty pings until the connection stops accepting bytes for half a second. Returns the bytes sent.
async fn ping_until_blocked(s: &mut TlsStream<TcpStream>, cap: usize) -> usize {
    const PING: [u8; 6] = [0x89, 0x80, 0, 0, 0, 0];
    let batch: Vec<u8> = PING.iter().copied().cycle().take(PING.len() * 10_000).collect();
    let mut sent = 0usize;
    while sent < cap {
        let write = async {
            s.write_all(&batch).await?;
            s.flush().await
        };
        match tokio::time::timeout(Duration::from_millis(500), write).await {
            Ok(Ok(())) => sent += batch.len(),
            _ => break,
        }
    }
    sent
}

#[corvis::test]
async fn a_ping_flood_over_tls_without_reading_is_bounded() {
    let _g = SERIAL.lock().await;
    let pki = pki();
    let addr = serve(options(&pki), ServerOptions::new().send_timeout(Duration::from_secs(30))).await;
    let mut s = raw_upgrade(addr, &pki, "/feed").await;
    let sent = ping_until_blocked(&mut s, 8 * PING_BUDGET).await;
    assert!(sent < PING_BUDGET, "the server kept reading pings: {sent} bytes accepted");
}

/// The peer interleaves `KeyUpdate` requests (the server must answer each with a `KeyUpdate` of its own) with a byte
/// of a ping, and never reads. The replies are produced by rustls while it reads ciphertext, so only a pause below
/// TLS bounds them.
#[corvis::test]
async fn a_key_update_flood_over_tls_without_reading_is_bounded() {
    let _g = SERIAL.lock().await;
    const CAP: usize = 300_000;
    const PING: [u8; 6] = [0x89, 0x80, 0, 0, 0, 0];
    let pki = pki();
    let addr = serve(options(&pki), ServerOptions::new().send_timeout(Duration::from_secs(30))).await;
    let mut s = raw_upgrade(addr, &pki, "/feed").await;
    let mut iterations = 0usize;
    while iterations < CAP {
        s.get_mut().1.refresh_traffic_keys().unwrap();
        let write = async {
            s.write_all(&PING[iterations % PING.len()..][..1]).await?;
            s.flush().await
        };
        match tokio::time::timeout(Duration::from_millis(500), write).await {
            Ok(Ok(())) => iterations += 1,
            _ => break,
        }
    }
    assert!(iterations < CAP, "the server kept reading after {iterations} key updates");
}

/// After the pause, the connection goes on: what the peer sent while it was paused is processed, and the replies
/// arrive once it reads.
#[corvis::test]
async fn reading_resumes_after_the_pause() {
    let _g = SERIAL.lock().await;
    let pki = pki();
    let addr = serve(options(&pki), ServerOptions::new().send_timeout(Duration::from_secs(30))).await;
    let mut ws = wss(connect_tiny(addr, &pki, H1).await, addr, "/feed").await;
    // The burst fills the buffers while nothing is read; pings go out meanwhile.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let pings = 200usize;
    for _ in 0..pings {
        tokio::time::timeout(Duration::from_secs(5), ws.send(Message::Ping(b"hi".to_vec()))).await.expect("pings are accepted").unwrap();
    }
    let (mut chunks, mut done, mut pongs) = (0usize, false, 0usize);
    let end = Instant::now() + Duration::from_secs(20);
    while (!done || pongs < pings) && Instant::now() < end {
        let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(5), ws.next()).await else { break };
        match msg {
            Message::Text(t) => {
                let v: Value = serde_json::from_str(&t).unwrap();
                match v["event"].as_str() {
                    Some("chunk") => chunks += 1,
                    Some("done") => done = true,
                    _ => {}
                }
            }
            Message::Pong(_) => pongs += 1,
            _ => {}
        }
    }
    assert!(done && chunks == MESSAGES, "the burst arrived: done={done}, {chunks} chunks");
    assert_eq!(pongs, pings, "every ping was answered");
}
