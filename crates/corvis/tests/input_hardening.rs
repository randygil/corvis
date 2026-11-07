//! Request-input hardening through the real HTTP server: JSON depth, `req.ip()` with the default
//! `trust proxy`, cookies, route params, query strings, `req.hostname()` and `Content-Encoding`.

use std::net::SocketAddr;
use std::time::Duration;

use corvis::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[controller]
pub struct InputController;

#[routes]
impl InputController {
    #[post("echo")]
    fn echo(&self, #[body] body: Value) -> Value {
        json!({ "type": match body { Value::Object(_) => "object", Value::Array(_) => "array", _ => "other" } })
    }

    #[post("noop")]
    fn noop(&self) -> String {
        "ok".into()
    }

    #[get("ip")]
    fn ip(&self, #[ip] ip: String) -> Value {
        json!({ "ip": ip })
    }

    #[get("q")]
    fn q(&self, #[req] req: Request) -> Value {
        Value::Object(req.query().clone())
    }

    #[get("users/:id")]
    fn user(&self, #[param("id")] id: String) -> Value {
        json!({ "id": id })
    }

    #[get("cookie")]
    fn cookie(&self, #[req] req: Request) -> Value {
        json!(req.cookies())
    }

    #[get("host")]
    fn host(&self, #[req] req: Request) -> Value {
        json!({ "hostname": req.hostname() })
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
    imports = [ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 3)])],
    controllers = [InputController, LimitedController],
)]
pub struct InputModule;

async fn start(configure: impl FnOnce(&mut CorvisApplication)) -> (CorvisApplication, SocketAddr) {
    let mut app = CorvisFactory::create::<InputModule>().await.unwrap();
    configure(&mut app);
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr)
}

/// Sends a raw HTTP/1.1 request (with `Connection: close`) and returns `(status, body)`;
/// `None` when the connection is dropped without a response.
async fn send(addr: SocketAddr, head: &[u8], body: &[u8]) -> Option<(u16, String)> {
    let mut s = tokio::net::TcpStream::connect(addr).await.ok()?;
    let mut req = head.to_vec();
    req.extend_from_slice(format!("Host: test\r\nConnection: close\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes());
    req.extend_from_slice(body);
    s.write_all(&req).await.ok()?;
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), s.read_to_end(&mut out)).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text.get(9..12)?.parse().ok()?;
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    Some((status, body))
}

async fn get(addr: SocketAddr, path: &str, headers: &[&str]) -> (u16, String) {
    let head = format!("GET {path} HTTP/1.1\r\n{}", headers.iter().map(|h| format!("{h}\r\n")).collect::<String>());
    send(addr, head.as_bytes(), b"").await.expect("response")
}

fn nested(depth: usize) -> Vec<u8> {
    let mut v = vec![b'['; depth];
    v.extend(std::iter::repeat_n(b']', depth));
    v
}

/// http-input #1 / validation-supply #1: 100 000 nested arrays used to abort the process
/// (stack overflow while dropping / cloning the parsed `Value`).
#[corvis::test]
async fn deeply_nested_json_is_rejected_and_the_server_survives() {
    let (_app, addr) = start(|app| {
        app.use_body_parser("json", 1 << 20);
    })
    .await;
    let json_head = |path: &str| format!("POST {path} HTTP/1.1\r\nContent-Type: application/json\r\n").into_bytes();
    for path in ["/echo", "/noop", "/does-not-exist"] {
        for depth in [3_000, 50_000, 100_000] {
            let (status, body) = send(addr, &json_head(path), &nested(depth)).await.expect("server crashed");
            assert_eq!(status, 400, "{path} {depth}: {body}");
            assert!(body.contains("Maximum nesting depth of 256 exceeded in JSON"), "{body}");
        }
    }
    let deep_object = format!("{}1{}", "{\"a\":".repeat(100_000), "}".repeat(100_000));
    assert_eq!(send(addr, &json_head("/echo"), deep_object.as_bytes()).await.expect("server crashed").0, 400);
    // Within the limit the body is accepted as before.
    assert_eq!(send(addr, &json_head("/echo"), &nested(256)).await.unwrap(), (201, r#"{"type":"array"}"#.into()));
    assert_eq!(get(addr, "/ip", &[]).await.0, 200);
}

/// http-input #4 / auth-ws #1: `X-Forwarded-For` is ignored unless `trust proxy` is set, so
/// rotating it neither spoofs `#[ip]` nor bypasses the throttler.
#[corvis::test]
async fn forwarded_for_is_ignored_by_default() {
    let (_app, addr) = start(|_| {}).await;
    let (_, body) = get(addr, "/ip", &["X-Forwarded-For: 1.2.3.4, 127.0.0.1"]).await;
    assert_eq!(body, r#"{"ip":"127.0.0.1"}"#);
    let mut statuses = Vec::new();
    for i in 1..=5 {
        statuses.push(get(addr, "/limited", &[&format!("X-Forwarded-For: 10.0.0.{i}")]).await.0);
    }
    assert_eq!(statuses, [200, 200, 200, 429, 429]);
}

/// http-input #6 / auth-ws #11: `cookie.parse` semantics.
#[corvis::test]
async fn cookies_first_occurrence_wins() {
    let (_app, addr) = start(|_| {}).await;
    let (_, body) = get(addr, "/cookie", &["Cookie: session=legit; other=1; session=attacker"]).await;
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!({ "session": "legit", "other": "1" }));
    let (_, body) = get(addr, "/cookie", &["Cookie: a=\"quoted%20v\"; b=%E0%A4%A; c=%+41; d = x y ; e"]).await;
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!({ "a": "quoted v", "b": "%E0%A4%A", "c": "%+41", "d": "x y" }));
    // Several Cookie headers are joined like Node does; the first one still wins.
    let (_, body) = get(addr, "/cookie", &["Cookie: s=1", "Cookie: s=2; t=3"]).await;
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!({ "s": "1", "t": "3" }));
}

/// http-input #7: `%+X` is not a valid escape (`decodeURIComponent` throws, Express answers 400).
#[corvis::test]
async fn route_params_need_hex_escapes() {
    let (_app, addr) = start(|_| {}).await;
    for path in ["/users/%+1", "/users/%+A", "/users/%-1", "/users/%ZZ"] {
        assert_eq!(get(addr, path, &[]).await.0, 400, "{path}");
    }
    assert_eq!(get(addr, "/users/%41%0a", &[]).await, (200, r#"{"id":"A\n"}"#.into()));
}

/// http-input #8: `node:querystring` stops after 1000 pairs (`maxKeys`), duplicates included.
#[corvis::test]
async fn simple_query_parser_caps_pairs() {
    let (_app, addr) = start(|_| {}).await;
    let q = vec!["a=1"; 1500].join("&");
    let (status, body) = get(addr, &format!("/q?{q}"), &[]).await;
    assert_eq!(status, 200);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["a"].as_array().map(Vec::len), Some(1000));
    // Empty pairs count too, like Node.
    let q = format!("{}b=1", "&".repeat(1000));
    assert_eq!(get(addr, &format!("/q?{q}"), &[]).await.1, "{}");
}

/// http-input #9: IPv6 literals in `Host`.
#[corvis::test]
async fn hostname_keeps_ipv6_literals() {
    let (_app, addr) = start(|_| {}).await;
    let host = |h: &'static str| async move {
        let head = format!("GET /host HTTP/1.1\r\nHost: {h}\r\nConnection: close\r\n\r\n");
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(head.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out.split_once("\r\n\r\n").unwrap().1.to_string()
    };
    assert_eq!(host("[::1]:5210").await, r#"{"hostname":"[::1]"}"#);
    assert_eq!(host("[::1]").await, r#"{"hostname":"[::1]"}"#);
    assert_eq!(host("example.com:80").await, r#"{"hostname":"example.com"}"#);
    // X-Forwarded-Host is ignored without `trust proxy`.
    let (_, body) = get(addr, "/host", &["X-Forwarded-Host: evil.example"]).await;
    assert_eq!(body, r#"{"hostname":"test"}"#);
}

/// http-input #5: a non-ASCII `Content-Encoding` is not treated as absent.
#[corvis::test]
async fn non_ascii_content_encoding_is_unsupported() {
    let (_app, addr) = start(|_| {}).await;
    for ce in [&b"gzip\xff"[..], b"z\xc2\x9b2J", b"\xe9"] {
        let mut head = b"POST /echo HTTP/1.1\r\nContent-Type: application/json\r\nContent-Encoding: ".to_vec();
        head.extend_from_slice(ce);
        head.extend_from_slice(b"\r\n");
        let (status, body) = send(addr, &head, b"{}").await.unwrap();
        assert_eq!(status, 415, "{ce:?}: {body}");
    }
}
