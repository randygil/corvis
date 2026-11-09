//! `app.set('trust proxy', ...)` (`CorvisApplication::set_trust_proxy`): `req.ip()`, `req.ips()`
//! and `req.hostname()` with `proxy-addr` semantics, through the real HTTP server.

use std::net::SocketAddr;

use corvis::TrustProxy;
use corvis::prelude::*;
use corvis::websockets::GatewayOptions;
use corvis::{OnGatewayInit, WsClient, WsServer, websocket_gateway};
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[controller]
pub struct ProxyController;

#[routes]
impl ProxyController {
    #[get("who")]
    fn who(&self, #[ip] ip: String, #[req] req: Request) -> Value {
        json!({ "ip": ip, "ips": req.ips(), "hostname": req.hostname() })
    }
}

#[controller("limited")]
#[use_guards(ThrottlerGuard)]
pub struct LimitedController;

#[routes]
impl LimitedController {
    #[get]
    fn hit(&self) -> String {
        "ok".into()
    }
}

#[module(
    imports = [ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 2)])],
    controllers = [ProxyController, LimitedController],
)]
pub struct ProxyModule;

async fn start(trust: impl Into<TrustProxy>) -> (CorvisApplication, SocketAddr) {
    let mut app = CorvisFactory::create::<ProxyModule>().await.unwrap();
    app.set_trust_proxy(trust);
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr)
}

async fn get(addr: SocketAddr, path: &str, headers: &[&str]) -> (u16, String) {
    let head = format!(
        "GET {path} HTTP/1.1\r\nHost: app.local:8080\r\nConnection: close\r\n{}\r\n",
        headers.iter().map(|h| format!("{h}\r\n")).collect::<String>()
    );
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(head.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    (out[9..12].parse().unwrap(), out.split_once("\r\n\r\n").unwrap().1.to_string())
}

async fn who(addr: SocketAddr, headers: &[&str]) -> Value {
    serde_json::from_str(&get(addr, "/who", headers).await.1).unwrap()
}

const XFF: &str = "X-Forwarded-For: 6.6.6.6, 1.2.3.4, 10.0.0.2";

#[corvis::test]
async fn default_trusts_no_proxy() {
    let (_app, addr) = start(TrustProxy::default()).await;
    let v = who(addr, &[XFF, "X-Forwarded-Host: evil.example"]).await;
    assert_eq!(v, json!({ "ip": "127.0.0.1", "ips": [], "hostname": "app.local" }));
}

#[corvis::test]
async fn trust_all_uses_the_left_most_entry() {
    let (_app, addr) = start(true).await;
    let v = who(addr, &[XFF, "X-Forwarded-Host: front.example:443, other"]).await;
    assert_eq!(v, json!({ "ip": "6.6.6.6", "ips": ["6.6.6.6", "1.2.3.4", "10.0.0.2"], "hostname": "front.example" }));
}

#[corvis::test]
async fn trust_hop_count() {
    let (_app, addr) = start(2).await;
    let v = who(addr, &[XFF]).await;
    assert_eq!(v, json!({ "ip": "1.2.3.4", "ips": ["1.2.3.4", "10.0.0.2"], "hostname": "app.local" }));
}

#[corvis::test]
async fn trust_subnets_stop_at_the_first_untrusted_hop() {
    let (_app, addr) = start(TrustProxy::parse("loopback, 10.0.0.0/8").unwrap()).await;
    let v = who(addr, &[XFF, "X-Forwarded-Host: front.example"]).await;
    assert_eq!(v, json!({ "ip": "1.2.3.4", "ips": ["1.2.3.4", "10.0.0.2"], "hostname": "front.example" }));
    // The throttler keys on the real client: prepending forged entries does not reset the count.
    let mut statuses = Vec::new();
    for i in 1..=4 {
        statuses.push(get(addr, "/limited", &[&format!("X-Forwarded-For: 9.9.9.{i}, 1.2.3.4")]).await.0);
    }
    assert_eq!(statuses, [200, 200, 429, 429]);
}

#[corvis::test]
async fn untrusted_socket_ignores_forwarded_headers() {
    let (_app, addr) = start(TrustProxy::parse("10.0.0.0/8").unwrap()).await;
    let v = who(addr, &[XFF, "X-Forwarded-Host: evil.example"]).await;
    assert_eq!(v, json!({ "ip": "127.0.0.1", "ips": [], "hostname": "app.local" }));
}

#[test]
fn invalid_trust_lists_are_errors() {
    assert_eq!(TrustProxy::parse("loopback, 300.0.0.1").unwrap_err().to_string(), "invalid IP address: 300.0.0.1");
    assert_eq!(TrustProxy::parse("10.0.0.0/40").unwrap_err().to_string(), "invalid range on address: 10.0.0.0/40");
}

// ---- WebSocket handshakes ----

#[websocket_gateway(path = "/who")]
pub struct WhoGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl WhoGateway {
    #[subscribe_message("who")]
    fn who(&self, #[connected_socket] client: WsClient) -> Value {
        let h = client.handshake();
        json!({ "ip": h.ip(), "ips": h.ips(), "hostname": h.hostname(), "remote": h.remote_addr().map(|a| a.ip().to_string()) })
    }
}

/// Accepts only the handshakes whose resolved address is `203.0.113.7`.
#[websocket_gateway(path = "/strict")]
pub struct StrictWsGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl StrictWsGateway {}

#[async_trait]
impl OnGatewayInit for StrictWsGateway {
    async fn after_init(&self, server: WsServer) {
        server.set_options(GatewayOptions::default().verify_client(|req| req.ip() == "203.0.113.7"));
    }
}

#[websocket_gateway(path = "/limited-ws")]
pub struct LimitedGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl LimitedGateway {
    #[subscribe_message("hit")]
    fn hit(&self) -> Value {
        json!("ok")
    }
}

#[module(providers = [WhoGateway, StrictWsGateway])]
pub struct WsProxyModule;

#[module(
    imports = [ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 2)])],
    providers = [LimitedGateway, Provider::app_guard::<ThrottlerGuard>()],
)]
pub struct WsThrottledModule;

async fn start_ws<M: corvis::di::Module>(trust: Option<TrustProxy>) -> (CorvisApplication, SocketAddr) {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    if let Some(trust) = trust {
        app.set_trust_proxy(trust);
    }
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr)
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn ws_connect(addr: SocketAddr, path: &str, headers: &[(&str, &str)]) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
    let mut req = format!("ws://{addr}{path}").into_client_request().unwrap();
    for (k, v) in headers {
        req.headers_mut().insert(HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
    }
    tokio_tungstenite::connect_async(req).await.map(|(ws, _)| ws)
}

async fn ws_call(ws: &mut Ws, event: &str) -> Value {
    ws.send(tokio_tungstenite::tungstenite::Message::Text(json!({ "event": event }).to_string())).await.unwrap();
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await.expect("timeout").unwrap().unwrap();
        if let tokio_tungstenite::tungstenite::Message::Text(t) = msg {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

async fn ws_who(addr: SocketAddr, headers: &[(&str, &str)]) -> Value {
    let mut ws = ws_connect(addr, "/who", headers).await.unwrap();
    let reply = ws_call(&mut ws, "who").await;
    assert_eq!(reply["event"], "who");
    reply["data"].clone()
}

fn hops(n: usize) -> Option<TrustProxy> {
    Some(TrustProxy::from(n))
}

#[corvis::test]
async fn gateway_behind_one_trusted_proxy() {
    let (_app, addr) = start_ws::<WsProxyModule>(hops(1)).await;
    let v = ws_who(addr, &[("X-Forwarded-For", "203.0.113.7")]).await;
    assert_eq!(v["ip"], "203.0.113.7");
    assert_eq!(v["ips"], json!(["203.0.113.7"]));
    assert_eq!(v["remote"], "127.0.0.1");
}

#[corvis::test]
async fn gateway_default_setting_ignores_forwarded_headers() {
    let (_app, addr) = start_ws::<WsProxyModule>(None).await;
    let v = ws_who(addr, &[("X-Forwarded-For", "203.0.113.7"), ("X-Forwarded-Host", "evil.example")]).await;
    assert_eq!(v["ip"], "127.0.0.1");
    assert_eq!(v["ips"], json!([]));
    assert_ne!(v["hostname"], "evil.example");
}

#[corvis::test]
async fn gateway_untrusted_peer_cannot_forge_forwarded_headers() {
    let (_app, addr) = start_ws::<WsProxyModule>(Some(TrustProxy::parse("10.0.0.0/8").unwrap())).await;
    let v = ws_who(addr, &[("X-Forwarded-For", "203.0.113.7"), ("X-Forwarded-Host", "evil.example")]).await;
    assert_eq!(v["ip"], "127.0.0.1");
    assert_eq!(v["ips"], json!([]));
    assert_ne!(v["hostname"], "evil.example");
}

#[corvis::test]
async fn gateway_one_trusted_hop_ignores_client_supplied_entries() {
    let (_app, addr) = start_ws::<WsProxyModule>(hops(1)).await;
    let v = ws_who(addr, &[("X-Forwarded-For", "6.6.6.6, 203.0.113.7")]).await;
    assert_eq!(v["ip"], "203.0.113.7");
}

#[corvis::test]
async fn gateway_forwarded_host() {
    let (_app, addr) = start_ws::<WsProxyModule>(Some(TrustProxy::parse("loopback").unwrap())).await;
    let v = ws_who(addr, &[("X-Forwarded-Host", "chat.example.com")]).await;
    assert_eq!(v["hostname"], "chat.example.com");
}

fn assert_unauthorized(result: Result<Ws, tokio_tungstenite::tungstenite::Error>) {
    match result {
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => assert_eq!(r.status(), 401),
        other => panic!("expected 401, got {:?}", other.map(|_| ())),
    }
}

#[corvis::test]
async fn verify_client_sees_the_forwarded_address() {
    let (_app, addr) = start_ws::<WsProxyModule>(hops(1)).await;
    assert!(ws_connect(addr, "/strict", &[("X-Forwarded-For", "203.0.113.7")]).await.is_ok());
    assert_unauthorized(ws_connect(addr, "/strict", &[]).await);
}

#[corvis::test]
async fn verify_client_cannot_be_forged_without_trust() {
    let (_app, addr) = start_ws::<WsProxyModule>(None).await;
    assert_unauthorized(ws_connect(addr, "/strict", &[("X-Forwarded-For", "203.0.113.7")]).await);
}

/// Three messages from one client, then one from another, through the same socket address.
async fn throttle_two_clients(addr: SocketAddr) -> (Vec<Value>, Value) {
    let mut a = ws_connect(addr, "/limited-ws", &[("X-Forwarded-For", "203.0.113.7")]).await.unwrap();
    let mut b = ws_connect(addr, "/limited-ws", &[("X-Forwarded-For", "203.0.113.8")]).await.unwrap();
    let mut first = Vec::new();
    for _ in 0..3 {
        first.push(ws_call(&mut a, "hit").await);
    }
    (first, ws_call(&mut b, "hit").await)
}

#[corvis::test]
async fn throttler_keys_gateway_messages_by_the_forwarded_client() {
    let (_app, addr) = start_ws::<WsThrottledModule>(hops(1)).await;
    let (first, second) = throttle_two_clients(addr).await;
    assert_eq!(first[0]["event"], "hit");
    assert_eq!(first[1]["event"], "hit");
    assert_eq!(first[2]["event"], "exception");
    assert_eq!(second["event"], "hit");
}

#[corvis::test]
async fn throttler_without_trust_shares_the_socket_counter() {
    let (_app, addr) = start_ws::<WsThrottledModule>(None).await;
    let (first, second) = throttle_two_clients(addr).await;
    assert_eq!(first[2]["event"], "exception");
    assert_eq!(second["event"], "exception");
}
