//! `app.set('trust proxy', ...)` (`CorvisApplication::set_trust_proxy`): `req.ip()`, `req.ips()`
//! and `req.hostname()` with `proxy-addr` semantics, through the real HTTP server.

use std::net::SocketAddr;

use corvis::TrustProxy;
use corvis::prelude::*;
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
