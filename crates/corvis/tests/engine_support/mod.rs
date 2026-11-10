//! Shared helpers of the Engine.IO tests: an in-process application with an Engine.IO server attached, a raw HTTP
//! client for the polling transport and small decoders. Servers bind to 127.0.0.1.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::time::Duration;

use corvis::prelude::*;
use corvis::socket_io::engine::{EngineServer, EngineSocket};
use corvis::socket_io::EngineOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

pub mod harness_app;

#[module]
pub struct AppModule;

/// An application with one Engine.IO server attached; connected sockets arrive on `sockets`.
pub struct Fixture {
    pub app: CorvisApplication,
    pub addr: SocketAddr,
    pub engine: EngineServer,
    pub sockets: mpsc::UnboundedReceiver<EngineSocket>,
}

pub async fn start(options: EngineOptions) -> Fixture {
    start_with(options, |_| {}).await
}

pub async fn start_with(options: EngineOptions, configure: impl FnOnce(&mut CorvisApplication)) -> Fixture {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    configure(&mut app);
    let engine = EngineServer::new(options).unwrap();
    let (tx, sockets) = mpsc::unbounded_channel();
    engine.on_connection(move |socket| {
        let _ = tx.send(socket);
    });
    engine.attach(&app.get_http_server()).unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    Fixture { app, addr, engine, sockets }
}

impl Fixture {
    /// The next session the server opened.
    pub async fn socket(&mut self) -> EngineSocket {
        tokio::time::timeout(Duration::from_secs(10), self.sockets.recv()).await.expect("a session within 10 s").expect("open channel")
    }
}

pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", self.text()))
    }
}

/// One HTTP/1.1 request on its own connection, read until the server closes it.
pub async fn http(addr: SocketAddr, method: &str, target: &str, headers: &[(&str, &str)], body: &[u8]) -> Resp {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut head = format!("{method} {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    let mut has_length = false;
    for (k, v) in headers {
        has_length |= k.eq_ignore_ascii_case("content-length");
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if !has_length && (!body.is_empty() || method == "POST") {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut raw = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), stream.read_to_end(&mut raw)).await.expect("the response within 20 s").ok();
    parse_response(&raw)
}

pub fn parse_response(raw: &[u8]) -> Resp {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or_else(|| panic!("no response head in {:?}", String::from_utf8_lossy(raw)));
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status: u16 = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect();
    let mut body = raw[split + 4..].to_vec();
    if headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked")) {
        body = dechunk(&body);
    }
    Resp { status, headers, body }
}

fn dechunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(eol) = data.windows(2).position(|w| w == b"\r\n") else { break };
        let size = usize::from_str_radix(String::from_utf8_lossy(&data[..eol]).trim(), 16).unwrap_or(0);
        data = &data[eol + 2..];
        if size == 0 || data.len() < size {
            break;
        }
        out.extend_from_slice(&data[..size]);
        data = &data[(size + 2).min(data.len())..];
    }
    out
}

pub const POLLING: &str = "/socket.io/?EIO=4&transport=polling";

pub fn poll_url(sid: &str) -> String {
    format!("{POLLING}&sid={sid}")
}

/// The packets of a polling payload as `(type digit, data)`.
pub fn packets(payload: &str) -> Vec<String> {
    payload.split('\x1e').map(str::to_string).collect()
}

/// Opens a polling session and returns its id and the open packet's JSON.
pub async fn open_polling(addr: SocketAddr) -> (String, serde_json::Value) {
    let resp = http(addr, "GET", POLLING, &[], b"").await;
    assert_eq!(resp.status, 200, "{}", resp.text());
    let text = resp.text();
    let open = text.strip_prefix('0').unwrap_or_else(|| panic!("not an open packet: {text:?}"));
    let json: serde_json::Value = serde_json::from_str(open).unwrap();
    (json["sid"].as_str().unwrap().to_string(), json)
}

pub async fn poll(addr: SocketAddr, sid: &str) -> Resp {
    http(addr, "GET", &poll_url(sid), &[], b"").await
}

pub async fn post(addr: SocketAddr, sid: &str, body: &str) -> Resp {
    http(addr, "POST", &poll_url(sid), &[("Content-Type", "text/plain;charset=UTF-8")], body.as_bytes()).await
}

pub type WsClient = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

/// A WebSocket client: the Engine.IO WebSocket transport, optionally for an existing session.
pub async fn ws_connect(addr: SocketAddr, sid: Option<&str>) -> WsClient {
    let url = match sid {
        Some(sid) => format!("ws://{addr}/socket.io/?EIO=4&transport=websocket&sid={sid}"),
        None => format!("ws://{addr}/socket.io/?EIO=4&transport=websocket"),
    };
    tokio_tungstenite::connect_async(url).await.expect("WebSocket handshake").0
}

/// Reads the next text frame, within 10 s.
pub async fn next_text(ws: &mut WsClient) -> String {
    use futures::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    loop {
        let next = tokio::time::timeout(Duration::from_secs(10), ws.next()).await.expect("a frame within 10 s");
        match next {
            Some(Ok(Message::Text(t))) => return t,
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            other => panic!("expected a text frame, got {other:?}"),
        }
    }
}

pub async fn send_text(ws: &mut WsClient, text: &str) {
    use futures::SinkExt;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(text.to_string())).await.unwrap();
}

/// Runs the probe sequence on a new WebSocket: `2probe` -> `3probe`.
pub async fn probe(ws: &mut WsClient) {
    send_text(ws, "2probe").await;
    assert_eq!(next_text(ws).await, "3probe");
}
