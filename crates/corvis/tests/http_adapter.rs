//! `HttpAdapterHost` + raw `upgrade` listeners + `corvis::ws` (the `ws` package).

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use corvis::prelude::*;
use corvis::ws::{ReadyState, WebSocket, WebSocketServer, WebSocketServerOptions, WsMessage};
use corvis::{OnGatewayConnection, WsClient, WsServer, websocket_gateway};
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

const BRIDGE_PATH: &str = "/ws/bridge";

#[derive(Default)]
pub struct BridgeState {
    clients: Mutex<HashSet<WebSocket>>,
    acks: Mutex<Vec<Value>>,
    closes: Mutex<Vec<(u16, String)>>,
    server: Mutex<Option<WebSocketServer>>,
}

/// A service that injects `HttpAdapterHost` and attaches a `ws`
/// server (`noServer: true`) on one path of the HTTP server.
#[injectable]
pub struct BridgeService {
    adapter_host: Arc<HttpAdapterHost>,
    #[default]
    state: Arc<BridgeState>,
}

impl BridgeService {
    fn register(state: &Arc<BridgeState>, ws: WebSocket) {
        state.clients.lock().unwrap().insert(ws.clone());
        let s = state.clone();
        let me = ws.clone();
        ws.on_close(move |code, reason| {
            s.clients.lock().unwrap().remove(&me);
            s.closes.lock().unwrap().push((code, reason.to_string()));
        });
        let s = state.clone();
        let me = ws.clone();
        ws.on_message(move |data| {
            let Ok(json) = serde_json::from_str::<Value>(&data.to_string()) else { return };
            if json["kind"] == "PING" {
                let _ = me.send(json!({ "kind": "PONG", "binary": data.is_binary() }));
                return;
            }
            s.acks.lock().unwrap().push(json);
        });
    }

    fn open_client(&self) -> Option<WebSocket> {
        self.state.clients.lock().unwrap().iter().find(|ws| ws.ready_state() == ReadyState::Open).cloned()
    }

    pub async fn emit(&self, payload: Value) -> Result<(), String> {
        let client = self.open_client().ok_or("no-bridge")?;
        client.send_and_wait(json!({ "kind": "EMIT", "payload": payload })).await.map_err(|e| e.to_string())
    }
}

#[async_trait]
impl OnApplicationBootstrap for BridgeService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        let server = self.adapter_host.http_adapter().get_http_server();
        let wss = WebSocketServer::new(WebSocketServerOptions::default());
        *self.state.server.lock().unwrap() = Some(wss.clone());
        let state = self.state.clone();
        server.on_upgrade(move |req| {
            if req.pathname() != BRIDGE_PATH {
                req.destroy();
                return;
            }
            let state = state.clone();
            wss.handle_upgrade(&req, move |ws| BridgeService::register(&state, ws));
        });
        Ok(())
    }
}

#[async_trait]
impl OnModuleDestroy for BridgeService {
    async fn on_module_destroy(&self) -> corvis::Result<()> {
        for c in self.state.clients.lock().unwrap().drain() {
            c.terminate();
        }
        if let Some(wss) = self.state.server.lock().unwrap().take() {
            wss.close();
        }
        Ok(())
    }
}

#[controller]
pub struct PingController;

#[routes]
impl PingController {
    #[get("ping")]
    fn ping(&self) -> Value {
        json!({ "ok": true })
    }
}

#[websocket_gateway(path = "/chat")]
pub struct ChatGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl ChatGateway {
    #[subscribe_message("echo")]
    async fn echo(&self, #[message_body] data: Value) -> Value {
        let _ = self.server.count();
        data
    }
}

#[async_trait]
impl OnGatewayConnection for ChatGateway {
    async fn handle_connection(&self, client: WsClient) {
        client.emit("welcome", json!({}));
    }
}

#[module(controllers = [PingController], providers = [BridgeService, ChatGateway])]
pub struct AppModule;

async fn next_text<S>(ws: &mut S) -> String
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("timeout").unwrap().unwrap();
        if let Message::Text(t) = msg {
            return t;
        }
    }
}

async fn wait_for(mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached");
}

/// Sends raw bytes and returns everything the server answers until it closes.
async fn raw(addr: std::net::SocketAddr, request: &str) -> String {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await.expect("server kept the socket open");
    String::from_utf8_lossy(&out).into_owned()
}

#[corvis::test]
async fn raw_upgrade_listener_bridge() {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let host = app.get::<HttpAdapterHost>().unwrap();
    assert!(host.listening());
    assert_eq!(host.http_adapter().get_http_server().address(), Some(addr));
    assert_eq!(app.get_http_server().upgrade_listener_count(), 1);
    let bridge = app.get::<BridgeService>().unwrap();
    assert_eq!(bridge.emit(json!(1)).await, Err("no-bridge".to_string()));

    // A plain RFC 6455 client on the bridge path.
    let (mut ws, resp) = tokio_tungstenite::connect_async(format!("ws://{addr}{BRIDGE_PATH}?token=abc")).await.unwrap();
    assert_eq!(resp.status(), 101);
    assert!(resp.headers().get("x-powered-by").is_none());
    wait_for(|| bridge.open_client().is_some()).await;
    let client = bridge.open_client().unwrap();
    assert_eq!(client.upgrade_request().query_param("token"), Some("abc"));
    assert_eq!(client.protocol(), "");

    // Server -> client EMIT frame, client -> server ACK frame.
    let emit = tokio::spawn({
        let bridge = bridge.clone();
        async move { bridge.emit(json!({ "doc": 7 })).await }
    });
    let frame: Value = serde_json::from_str(&next_text(&mut ws).await).unwrap();
    assert_eq!(frame, json!({ "kind": "EMIT", "payload": { "doc": 7 } }));
    emit.await.unwrap().unwrap();
    ws.send(Message::Text(json!({ "kind": "ACK", "ok": true }).to_string())).await.unwrap();
    wait_for(|| bridge.state.acks.lock().unwrap().len() == 1).await;
    assert_eq!(bridge.state.acks.lock().unwrap()[0], json!({ "kind": "ACK", "ok": true }));

    // Binary frames are delivered as binary messages.
    ws.send(Message::Binary(br#"{"kind":"PING"}"#.to_vec())).await.unwrap();
    assert_eq!(serde_json::from_str::<Value>(&next_text(&mut ws).await).unwrap(), json!({ "kind": "PONG", "binary": true }));

    // Ping frames are answered automatically.
    ws.send(Message::Ping(b"hb".to_vec())).await.unwrap();
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap().unwrap().unwrap() {
            Message::Pong(p) => {
                assert_eq!(p, b"hb");
                break;
            }
            _ => continue,
        }
    }

    // A second client is tracked separately.
    let (mut ws2, _) = tokio_tungstenite::connect_async(format!("ws://{addr}{BRIDGE_PATH}")).await.unwrap();
    wait_for(|| bridge.state.clients.lock().unwrap().len() == 2).await;

    // Client-initiated close: code and reason reach `on_close`, the client is removed.
    ws.close(Some(CloseFrame { code: CloseCode::Library(4000), reason: "bye".into() })).await.unwrap();
    wait_for(|| bridge.state.clients.lock().unwrap().len() == 1).await;
    assert_eq!(bridge.state.closes.lock().unwrap()[0], (4000, "bye".to_string()));
    assert_eq!(client.ready_state(), ReadyState::Closed);
    assert!(client.send("late").unwrap_err().to_string().starts_with("WebSocket is not open: readyState 3 (CLOSED)"));
    assert_eq!(client.closed().await, (4000, "bye".to_string()));

    // Server-initiated close handshake.
    let other = bridge.open_client().unwrap();
    other.close(Some(1000), "done").unwrap();
    assert_eq!(other.ready_state(), ReadyState::Closing);
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws2.next()).await.unwrap() {
            Some(Ok(Message::Close(Some(f)))) => {
                assert_eq!(u16::from(f.code), 1000);
                assert_eq!(f.reason, "done");
                break;
            }
            Some(Ok(_)) => continue,
            other => panic!("unexpected {other:?}"),
        }
    }
    // Keep polling so the client flushes its close reply, then the server closes the TCP.
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_secs(5), ws2.next()).await {}
    let closed = tokio::time::timeout(Duration::from_secs(5), other.closed()).await.expect("close handshake");
    assert_eq!(closed, (1000, "done".to_string()));
    assert!(other.close(Some(999), "").is_err());

    // Gateways keep working next to raw listeners.
    let (mut chat, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/chat")).await.unwrap();
    assert_eq!(serde_json::from_str::<Value>(&next_text(&mut chat).await).unwrap()["event"], "welcome");

    // Plain HTTP requests are routed normally.
    let r = raw(addr, "GET /ping HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await;
    assert!(r.starts_with("HTTP/1.1 200"), "{r}");

    app.close().await.unwrap();
}

#[corvis::test]
async fn unknown_upgrade_paths_are_destroyed() {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();

    // The listener destroys the socket: no response at all.
    assert!(tokio_tungstenite::connect_async(format!("ws://{addr}/ws/other")).await.is_err());
    let r = raw(
        addr,
        "GET /ws/other HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
    )
    .await;
    assert_eq!(r, "");
    // Any upgrade request goes to the listeners (Node semantics), even for an HTTP route.
    let r = raw(addr, "GET /ping HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: h2c\r\n\r\n").await;
    assert_eq!(r, "");

    // Handshake validation of `ws`' handleUpgrade.
    let r =
        raw(addr, "GET /ws/bridge HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\r\n")
            .await;
    assert!(r.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{r}");
    assert!(r.to_ascii_lowercase().contains("connection: close"), "{r}");
    assert!(r.to_ascii_lowercase().contains("content-type: text/html"), "{r}");
    assert!(r.ends_with("\r\n\r\nMissing or invalid Sec-WebSocket-Key header"), "{r}");
    let r = raw(
        addr,
        "GET /ws/bridge HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 7\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
    )
    .await;
    assert!(r.starts_with("HTTP/1.1 400"), "{r}");
    assert!(r.to_ascii_lowercase().contains("sec-websocket-version: 13, 8"), "{r}");
    let r = raw(
        addr,
        "POST /ws/bridge HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
    )
    .await;
    assert!(r.starts_with("HTTP/1.1 405") && r.ends_with("Invalid HTTP method"), "{r}");

    app.close().await.unwrap();
}

#[module(providers = [ProtoService])]
pub struct ProtoModule;

#[injectable]
pub struct ProtoService {
    adapter_host: Arc<HttpAdapterHost>,
}

#[async_trait]
impl OnApplicationBootstrap for ProtoService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        let wss = WebSocketServer::new(WebSocketServerOptions::default().max_payload(16));
        self.adapter_host.http_adapter().get_http_server().on_upgrade(move |req| {
            wss.handle_upgrade(&req, |ws| {
                let me = ws.clone();
                let mut messages = ws.messages();
                tokio::spawn(async move {
                    while let Some(m) = messages.next().await {
                        let reply = match m {
                            WsMessage::Text(t) if t == "kill" => {
                                me.terminate();
                                continue;
                            }
                            WsMessage::Text(t) => format!("{}:{t}", me.protocol()),
                            WsMessage::Binary(b) => format!("bin:{}", b.len()),
                        };
                        let _ = me.send(reply);
                    }
                });
                let errors = ws.clone();
                ws.on_error(move |e| {
                    let _ = e;
                    let _ = errors.ready_state();
                });
            });
        });
        Ok(())
    }
}

#[corvis::test]
async fn subprotocols_streams_terminate_and_limits() {
    let mut app = CorvisFactory::create::<ProtoModule>().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();

    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = format!("ws://{addr}/any").into_client_request().unwrap();
    req.headers_mut().insert("sec-websocket-protocol", "v2.bridge, v1.bridge".parse().unwrap());
    let (mut ws, resp) = tokio_tungstenite::connect_async(req).await.unwrap();
    assert_eq!(resp.headers().get("sec-websocket-protocol").unwrap(), "v2.bridge");

    ws.send(Message::Text("hi".into())).await.unwrap();
    assert_eq!(next_text(&mut ws).await, "v2.bridge:hi");
    ws.send(Message::Binary(vec![1, 2, 3])).await.unwrap();
    assert_eq!(next_text(&mut ws).await, "bin:3");

    // Messages above `max_payload` close the connection (1009).
    ws.send(Message::Text("x".repeat(64))).await.unwrap();
    let mut closed = false;
    while let Ok(Some(m)) = tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
        match m {
            Ok(Message::Close(f)) => {
                assert_eq!(f.map(|f| u16::from(f.code)), Some(1009));
                closed = true;
                break;
            }
            Ok(_) => continue,
            Err(_) => {
                closed = true;
                break;
            }
        }
    }
    assert!(closed);

    // `terminate()` drops the TCP connection without a closing handshake.
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/any")).await.unwrap();
    ws.send(Message::Text("kill".into())).await.unwrap();
    let next = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap();
    assert!(!matches!(next, Some(Ok(Message::Text(_)))), "{next:?}");

    app.close().await.unwrap();
}

#[module(controllers = [PingController], providers = [ChatGateway])]
pub struct GatewayOnlyModule;

#[corvis::test]
async fn gateways_destroy_unknown_upgrade_paths_like_ws_adapter() {
    let mut app = CorvisFactory::create::<GatewayOnlyModule>().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    assert_eq!(app.get_http_server().upgrade_listener_count(), 0);
    assert!(tokio_tungstenite::connect_async(format!("ws://{addr}/elsewhere")).await.is_err());
    let r = raw(addr, "GET /ping HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n").await;
    assert_eq!(r, "");
    let r = raw(addr, "GET /ping HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await;
    assert!(r.starts_with("HTTP/1.1 200"), "{r}");
    let (mut chat, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/chat")).await.unwrap();
    assert_eq!(serde_json::from_str::<Value>(&next_text(&mut chat).await).unwrap()["event"], "welcome");
    app.close().await.unwrap();
}
