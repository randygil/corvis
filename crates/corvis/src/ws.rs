//! Plain RFC 6455 WebSockets on the application's HTTP server, modelled after the
//! [`ws`](https://github.com/websockets/ws) package: `new WebSocketServer({ noServer: true })`
//! + `wss.handleUpgrade(req, socket, head, cb)` from an `upgrade` listener
//! (see [`crate::http_adapter`]).
//!
//! Frames are raw text/binary messages (no `{ event, data }` gateway protocol).
//!
//! ```ignore
//! let wss = WebSocketServer::new(WebSocketServerOptions::default());
//! server.on_upgrade(move |req| {
//!     if req.pathname() != "/ws/printer" {
//!         req.destroy();
//!         return;
//!     }
//!     wss.handle_upgrade(&req, |ws| {
//!         let peer = ws.clone();
//!         ws.on_message(move |msg| {
//!             let _ = peer.send(format!("echo: {msg}"));
//!         });
//!         ws.on_close(|code, reason| println!("closed {code} {reason}"));
//!     });
//! });
//! ```

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures::{SinkExt, Stream, StreamExt};
use http::{HeaderMap, HeaderValue, StatusCode};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};

use crate::http::{Request, Response};
use crate::http_adapter::{UpgradeRequest, UpgradedIo};
use crate::logger::Logger;

/// A message received or sent on a [`WebSocket`] (`data` + `isBinary` in `ws`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsMessage {
    Text(String),
    Binary(Bytes),
}

impl WsMessage {
    pub fn is_binary(&self) -> bool {
        matches!(self, WsMessage::Binary(_))
    }
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            WsMessage::Text(t) => t.as_bytes(),
            WsMessage::Binary(b) => b,
        }
    }
    /// The payload decoded as UTF-8 (lossy), like `String(data)` in Node.
    pub fn to_text(&self) -> Cow<'_, str> {
        match self {
            WsMessage::Text(t) => Cow::Borrowed(t),
            WsMessage::Binary(b) => String::from_utf8_lossy(b),
        }
    }
    pub fn into_bytes(self) -> Bytes {
        match self {
            WsMessage::Text(t) => Bytes::from(t),
            WsMessage::Binary(b) => b,
        }
    }
    fn into_tungstenite(self) -> Message {
        match self {
            WsMessage::Text(t) => Message::Text(t),
            WsMessage::Binary(b) => Message::Binary(b.to_vec()),
        }
    }
}

impl std::fmt::Display for WsMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_text())
    }
}
impl From<String> for WsMessage {
    fn from(v: String) -> Self {
        WsMessage::Text(v)
    }
}
impl From<&str> for WsMessage {
    fn from(v: &str) -> Self {
        WsMessage::Text(v.to_string())
    }
}
impl From<&String> for WsMessage {
    fn from(v: &String) -> Self {
        WsMessage::Text(v.clone())
    }
}
impl From<serde_json::Value> for WsMessage {
    /// `JSON.stringify(value)` sent as a text frame.
    fn from(v: serde_json::Value) -> Self {
        WsMessage::Text(v.to_string())
    }
}
impl From<Bytes> for WsMessage {
    fn from(v: Bytes) -> Self {
        WsMessage::Binary(v)
    }
}
impl From<Vec<u8>> for WsMessage {
    fn from(v: Vec<u8>) -> Self {
        WsMessage::Binary(Bytes::from(v))
    }
}
impl From<&[u8]> for WsMessage {
    fn from(v: &[u8]) -> Self {
        WsMessage::Binary(Bytes::copy_from_slice(v))
    }
}

/// `ws.readyState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ReadyState {
    Connecting = 0,
    Open = 1,
    Closing = 2,
    Closed = 3,
}

impl ReadyState {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
    /// `'CONNECTING' | 'OPEN' | 'CLOSING' | 'CLOSED'`.
    pub fn name(self) -> &'static str {
        match self {
            ReadyState::Connecting => "CONNECTING",
            ReadyState::Open => "OPEN",
            ReadyState::Closing => "CLOSING",
            ReadyState::Closed => "CLOSED",
        }
    }
    fn from_u8(v: u8) -> Self {
        match v {
            0 => ReadyState::Connecting,
            1 => ReadyState::Open,
            2 => ReadyState::Closing,
            _ => ReadyState::Closed,
        }
    }
}

/// An error of a [`WebSocket`] (`'error'` event, failed `send`, invalid `close` arguments).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsError {
    message: String,
    code: Option<&'static str>,
}

impl WsError {
    fn new(message: impl Into<String>, code: Option<&'static str>) -> Self {
        WsError { message: message.into(), code }
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    /// The `ws` error code (`'WS_ERR_INVALID_UTF8'`, ...) for protocol errors.
    pub fn code(&self) -> Option<&'static str> {
        self.code
    }
}
impl std::fmt::Display for WsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for WsError {}

type MessageListener = Arc<dyn Fn(WsMessage) + Send + Sync>;
type CloseListener = Arc<dyn Fn(u16, &str) + Send + Sync>;
type ErrorListener = Arc<dyn Fn(&WsError) + Send + Sync>;
type BytesListener = Arc<dyn Fn(Bytes) + Send + Sync>;

#[derive(Default)]
struct Listeners {
    message: Vec<MessageListener>,
    close: Vec<CloseListener>,
    error: Vec<ErrorListener>,
    ping: Vec<BytesListener>,
    pong: Vec<BytesListener>,
}

enum Command {
    Send(Message, Option<oneshot::Sender<Result<(), WsError>>>),
    Close(Option<CloseFrame<'static>>),
    Terminate,
}

static SOCKET_IDS: AtomicU64 = AtomicU64::new(1);

struct WsInner {
    id: u64,
    state: AtomicU8,
    protocol: String,
    request: Request,
    tx: mpsc::UnboundedSender<Command>,
    listeners: Mutex<Listeners>,
    closed: watch::Sender<Option<(u16, String)>>,
}

/// A server-side WebSocket connection (`ws`' `WebSocket`). Cheap to clone; clones refer
/// to the same connection (equality and hashing are by connection, so it can be kept
/// in a `HashSet` like `new Set<WebSocket>()`).
///
/// Event listeners (`on_message`, `on_close`, ...) should be registered inside the
/// `handle_upgrade` callback: frames are only read after it returns, so none is lost.
#[derive(Clone)]
pub struct WebSocket {
    inner: Arc<WsInner>,
}

impl PartialEq for WebSocket {
    fn eq(&self, other: &Self) -> bool {
        self.inner.id == other.inner.id
    }
}
impl Eq for WebSocket {}
impl std::hash::Hash for WebSocket {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.inner.id.hash(state)
    }
}
impl std::fmt::Debug for WebSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocket").field("id", &self.inner.id).field("ready_state", &self.ready_state()).finish()
    }
}

impl WebSocket {
    /// A process-unique id of the connection.
    pub fn id(&self) -> u64 {
        self.inner.id
    }
    /// `ws.readyState`.
    pub fn ready_state(&self) -> ReadyState {
        ReadyState::from_u8(self.inner.state.load(Ordering::SeqCst))
    }
    /// `ws.readyState === WebSocket.OPEN`.
    pub fn is_open(&self) -> bool {
        self.ready_state() == ReadyState::Open
    }
    /// `ws.protocol`: the negotiated subprotocol (empty when none).
    pub fn protocol(&self) -> &str {
        &self.inner.protocol
    }
    /// The HTTP upgrade request (`req` of the `connection` event).
    pub fn upgrade_request(&self) -> &Request {
        &self.inner.request
    }
    pub fn remote_addr(&self) -> Option<std::net::SocketAddr> {
        self.inner.request.remote_addr()
    }

    fn not_open(&self) -> WsError {
        let s = self.ready_state();
        WsError::new(format!("WebSocket is not open: readyState {} ({})", s.as_u8(), s.name()), None)
    }

    /// `ws.send(data)`: queues a text (`String`, `&str`, `serde_json::Value`) or binary
    /// (`Vec<u8>`, `Bytes`) frame. Fails if the socket is not `OPEN`.
    pub fn send(&self, data: impl Into<WsMessage>) -> Result<(), WsError> {
        if !self.is_open() {
            return Err(self.not_open());
        }
        self.inner.tx.send(Command::Send(data.into().into_tungstenite(), None)).map_err(|_| self.not_open())
    }

    /// `ws.send(data, (err) => ...)`: like [`send`](Self::send) but resolves once the frame
    /// was written to the socket, with the write error if any.
    pub async fn send_and_wait(&self, data: impl Into<WsMessage>) -> Result<(), WsError> {
        if !self.is_open() {
            return Err(self.not_open());
        }
        let (tx, rx) = oneshot::channel();
        self.inner.tx.send(Command::Send(data.into().into_tungstenite(), Some(tx))).map_err(|_| self.not_open())?;
        rx.await.unwrap_or_else(|_| Err(self.not_open()))
    }

    /// `ws.ping(data)`.
    pub fn ping(&self, data: impl Into<Bytes>) -> Result<(), WsError> {
        if !self.is_open() {
            return Err(self.not_open());
        }
        self.inner.tx.send(Command::Send(Message::Ping(data.into().to_vec()), None)).map_err(|_| self.not_open())
    }

    /// `ws.pong(data)`.
    pub fn pong(&self, data: impl Into<Bytes>) -> Result<(), WsError> {
        if !self.is_open() {
            return Err(self.not_open());
        }
        self.inner.tx.send(Command::Send(Message::Pong(data.into().to_vec()), None)).map_err(|_| self.not_open())
    }

    /// `ws.close([code[, reason]])`: starts the closing handshake. The connection is
    /// destroyed if the peer does not answer within 30 seconds (like `ws`).
    /// Valid codes: 1000, 1001-1003, 1007-1014 and 3000-4999; the reason must fit in 123 bytes.
    pub fn close(&self, code: Option<u16>, reason: &str) -> Result<(), WsError> {
        if let Some(c) = code {
            let valid = matches!(c, 1000..=1003 | 1007..=1014 | 3000..=4999);
            if !valid {
                return Err(WsError::new("First argument must be a valid error code number", None));
            }
        }
        if reason.len() > 123 {
            return Err(WsError::new("The message must not be greater than 123 bytes", None));
        }
        if self.ready_state() != ReadyState::Open {
            return Ok(());
        }
        self.inner.state.store(ReadyState::Closing as u8, Ordering::SeqCst);
        let frame = code.map(|c| CloseFrame { code: CloseCode::from(c), reason: Cow::Owned(reason.to_string()) });
        let _ = self.inner.tx.send(Command::Close(frame));
        Ok(())
    }

    /// `ws.terminate()`: destroys the connection immediately (close code `1006`).
    pub fn terminate(&self) {
        if self.ready_state() == ReadyState::Closed {
            return;
        }
        self.inner.state.store(ReadyState::Closing as u8, Ordering::SeqCst);
        let _ = self.inner.tx.send(Command::Terminate);
    }

    /// `ws.on('message', (data, isBinary) => ...)`.
    pub fn on_message(&self, f: impl Fn(WsMessage) + Send + Sync + 'static) -> &Self {
        self.inner.listeners.lock().unwrap().message.push(Arc::new(f));
        self
    }
    /// `ws.on('close', (code, reason) => ...)`: `1005` when the peer sent no status code,
    /// `1006` when the connection was lost or terminated without a closing handshake.
    pub fn on_close(&self, f: impl Fn(u16, &str) + Send + Sync + 'static) -> &Self {
        self.inner.listeners.lock().unwrap().close.push(Arc::new(f));
        self
    }
    /// `ws.on('error', (err) => ...)`: protocol errors (invalid UTF-8, payload too big, ...).
    pub fn on_error(&self, f: impl Fn(&WsError) + Send + Sync + 'static) -> &Self {
        self.inner.listeners.lock().unwrap().error.push(Arc::new(f));
        self
    }
    /// `ws.on('ping', (data) => ...)` (the pong is sent automatically).
    pub fn on_ping(&self, f: impl Fn(Bytes) + Send + Sync + 'static) -> &Self {
        self.inner.listeners.lock().unwrap().ping.push(Arc::new(f));
        self
    }
    /// `ws.on('pong', (data) => ...)`.
    pub fn on_pong(&self, f: impl Fn(Bytes) + Send + Sync + 'static) -> &Self {
        self.inner.listeners.lock().unwrap().pong.push(Arc::new(f));
        self
    }

    /// The incoming messages as a `Stream` that ends when the socket closes. Call it
    /// inside the `handle_upgrade` callback so no message is missed.
    pub fn messages(&self) -> std::pin::Pin<Box<dyn Stream<Item = WsMessage> + Send + 'static>> {
        let (tx, rx) = mpsc::unbounded_channel::<WsMessage>();
        if self.ready_state() != ReadyState::Closed {
            let tx = Arc::new(Mutex::new(Some(tx)));
            let t1 = tx.clone();
            self.on_message(move |m| {
                if let Some(tx) = t1.lock().unwrap().as_ref() {
                    let _ = tx.send(m);
                }
            });
            self.on_close(move |_, _| {
                tx.lock().unwrap().take();
            });
        }
        Box::pin(futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|m| (m, rx)) }))
    }

    /// Resolves with `(code, reason)` once the connection is closed.
    pub async fn closed(&self) -> (u16, String) {
        let mut rx = self.inner.closed.subscribe();
        loop {
            if let Some(v) = rx.borrow_and_update().clone() {
                return v;
            }
            if rx.changed().await.is_err() {
                return (1006, String::new());
            }
        }
    }

    fn emit_message(&self, m: WsMessage) {
        let ls = self.inner.listeners.lock().unwrap().message.clone();
        for l in ls {
            l(m.clone());
        }
    }
    fn emit_error(&self, e: &WsError) {
        let ls = self.inner.listeners.lock().unwrap().error.clone();
        for l in ls {
            l(e);
        }
    }
    fn emit_bytes(&self, ping: bool, data: Bytes) {
        let ls = {
            let g = self.inner.listeners.lock().unwrap();
            if ping { g.ping.clone() } else { g.pong.clone() }
        };
        for l in ls {
            l(data.clone());
        }
    }
    fn emit_close(&self, code: u16, reason: &str) {
        let ls = std::mem::take(&mut self.inner.listeners.lock().unwrap().close);
        for l in ls {
            l(code, reason);
        }
        // Drop the remaining listeners: they may hold clones of this socket.
        *self.inner.listeners.lock().unwrap() = Listeners::default();
        self.inner.closed.send_replace(Some((code, reason.to_string())));
    }
}

/// Handles the protocol selection (`handleProtocols(protocols, req)`).
pub type HandleProtocols = Arc<dyn Fn(&[String], &Request) -> Option<String> + Send + Sync>;

/// Options of [`WebSocketServer::new`] (the `noServer: true` mode of `ws`).
#[derive(Clone)]
pub struct WebSocketServerOptions {
    /// `maxPayload` in bytes (default 100 MiB).
    pub max_payload: usize,
    /// `clientTracking` (default `true`): keep the connected sockets in `clients()`.
    pub client_tracking: bool,
    /// `handleProtocols`; by default the first requested subprotocol is accepted.
    pub handle_protocols: Option<HandleProtocols>,
}

impl Default for WebSocketServerOptions {
    fn default() -> Self {
        WebSocketServerOptions { max_payload: 100 * 1024 * 1024, client_tracking: true, handle_protocols: None }
    }
}

impl WebSocketServerOptions {
    pub fn max_payload(mut self, bytes: usize) -> Self {
        self.max_payload = bytes;
        self
    }
    pub fn client_tracking(mut self, v: bool) -> Self {
        self.client_tracking = v;
        self
    }
    pub fn handle_protocols(mut self, f: impl Fn(&[String], &Request) -> Option<String> + Send + Sync + 'static) -> Self {
        self.handle_protocols = Some(Arc::new(f));
        self
    }
}

struct WssInner {
    options: WebSocketServerOptions,
    clients: Mutex<HashMap<u64, WebSocket>>,
    closed: AtomicBool,
}

/// `new WebSocketServer({ noServer: true })`: performs WebSocket handshakes for upgrade
/// requests received through `HttpServer::on_upgrade`. Cheap to clone.
#[derive(Clone)]
pub struct WebSocketServer {
    inner: Arc<WssInner>,
}

impl Default for WebSocketServer {
    fn default() -> Self {
        Self::new(WebSocketServerOptions::default())
    }
}

fn abort_handshake(req: &UpgradeRequest, status: u16, message: Option<&str>, extra: &[(&str, &str)]) {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
    let message = message.unwrap_or(status.canonical_reason().unwrap_or("")).to_string();
    let mut resp = Response::bytes(message, "text/html").with_status(status);
    for (k, v) in extra {
        resp.set_header(k, v);
    }
    let _ = req.respond(resp);
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// `subprotocol.parse()` of `ws`.
fn parse_subprotocols(header: &str) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for part in header.split(',') {
        let p = part.trim_matches([' ', '\t']);
        if p.is_empty() || !p.bytes().all(is_token_char) || out.iter().any(|x| x == p) {
            return None;
        }
        out.push(p.to_string());
    }
    Some(out)
}

impl WebSocketServer {
    pub fn new(options: WebSocketServerOptions) -> Self {
        WebSocketServer {
            inner: Arc::new(WssInner { options, clients: Mutex::new(HashMap::new()), closed: AtomicBool::new(false) }),
        }
    }

    /// `wss.clients`: the open connections (requires `client_tracking`).
    pub fn clients(&self) -> Vec<WebSocket> {
        self.inner.clients.lock().unwrap().values().cloned().collect()
    }

    /// `wss.close()`: new handshakes are rejected with `503`; existing connections are
    /// left open (as in `ws` 8; terminate them explicitly if needed).
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
    }

    /// `wss.handleUpgrade(req, socket, head, cb)`: validates the handshake like `ws`
    /// (`405 Invalid HTTP method`, `400 Missing or invalid Sec-WebSocket-Key header`, ...),
    /// answers `101 Switching Protocols` and calls `cb` with the connected socket.
    /// Does nothing (except logging) if the request was already handled.
    pub fn handle_upgrade(&self, req: &UpgradeRequest, cb: impl FnOnce(WebSocket) + Send + 'static) {
        if req.is_handled() {
            Logger::new("WebSocketServer").warn("handleUpgrade() was called for an upgrade request that was already handled");
            return;
        }
        if req.method() != http::Method::GET {
            return abort_handshake(req, 405, Some("Invalid HTTP method"), &[]);
        }
        if !req.header("upgrade").map(|u| u.eq_ignore_ascii_case("websocket")).unwrap_or(false) {
            return abort_handshake(req, 400, Some("Invalid Upgrade header"), &[]);
        }
        let key = req.header("sec-websocket-key").unwrap_or("");
        let key_ok = key.len() == 24 && key.ends_with("==") && key.as_bytes()[..22].iter().all(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'/');
        if !key_ok {
            return abort_handshake(req, 400, Some("Missing or invalid Sec-WebSocket-Key header"), &[]);
        }
        let version = req.header("sec-websocket-version").and_then(|v| v.trim().parse::<u32>().ok());
        if version != Some(13) && version != Some(8) {
            return abort_handshake(
                req,
                400,
                Some("Missing or invalid Sec-WebSocket-Version header"),
                &[("Sec-WebSocket-Version", "13, 8")],
            );
        }
        let protocols = match req.header("sec-websocket-protocol") {
            Some(h) => match parse_subprotocols(h) {
                Some(p) => p,
                None => return abort_handshake(req, 400, Some("Invalid Sec-WebSocket-Protocol header"), &[]),
            },
            None => Vec::new(),
        };
        if self.inner.closed.load(Ordering::SeqCst) {
            return abort_handshake(req, 503, None, &[]);
        }
        let protocol = if protocols.is_empty() {
            None
        } else {
            match &self.inner.options.handle_protocols {
                Some(f) => f(&protocols, req.request()),
                None => protocols.first().cloned(),
            }
        };
        let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
        let mut headers = HeaderMap::new();
        headers.insert(http::header::UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(http::header::CONNECTION, HeaderValue::from_static("Upgrade"));
        if let Ok(v) = HeaderValue::from_str(&accept) {
            headers.insert("sec-websocket-accept", v);
        }
        if let Some(p) = &protocol {
            if let Ok(v) = HeaderValue::from_str(p) {
                headers.insert("sec-websocket-protocol", v);
            }
        }
        let Ok(upgraded) = req.switch_protocols(headers) else {
            Logger::new("WebSocketServer").warn("handleUpgrade() was called for an upgrade request that was already handled");
            return;
        };
        let request = req.request().clone();
        let server = self.clone();
        tokio::spawn(async move {
            let Ok(io) = upgraded.await else { return };
            let max = server.inner.options.max_payload;
            let mut config = WebSocketConfig::default();
            config.max_message_size = Some(max);
            config.max_frame_size = Some(max);
            let stream = WebSocketStream::from_raw_socket(io, Role::Server, Some(config)).await;
            let (tx, rx) = mpsc::unbounded_channel();
            let ws = WebSocket {
                inner: Arc::new(WsInner {
                    id: SOCKET_IDS.fetch_add(1, Ordering::SeqCst),
                    state: AtomicU8::new(ReadyState::Open as u8),
                    protocol: protocol.unwrap_or_default(),
                    request,
                    tx,
                    listeners: Mutex::new(Listeners::default()),
                    closed: watch::channel(None).0,
                }),
            };
            if server.inner.options.client_tracking {
                server.inner.clients.lock().unwrap().insert(ws.id(), ws.clone());
            }
            cb(ws.clone());
            run(ws.clone(), stream, rx).await;
            server.inner.clients.lock().unwrap().remove(&ws.id());
        });
    }
}

fn protocol_error(e: &tokio_tungstenite::tungstenite::Error) -> (WsError, u16) {
    use tokio_tungstenite::tungstenite::Error as E;
    match e {
        E::Capacity(_) => (WsError::new("Max payload size exceeded", Some("WS_ERR_UNSUPPORTED_MESSAGE_LENGTH")), 1009),
        E::Utf8 => (WsError::new("Invalid UTF-8 sequence", Some("WS_ERR_INVALID_UTF8")), 1007),
        other => (WsError::new(other.to_string(), Some("WS_ERR_PROTOCOL")), 1002),
    }
}

async fn run(ws: WebSocket, mut stream: WebSocketStream<UpgradedIo>, mut rx: mpsc::UnboundedReceiver<Command>) {
    use tokio_tungstenite::tungstenite::Error as E;
    let mut received: Option<(u16, String)> = None;
    let mut deadline: Option<tokio::time::Instant> = None;
    loop {
        let current_deadline = deadline;
        let timeout = async move {
            match current_deadline {
                Some(d) => tokio::time::sleep_until(d).await,
                None => futures::future::pending::<()>().await,
            }
        };
        tokio::select! {
            incoming = stream.next() => match incoming {
                None => break,
                Some(Ok(msg)) => match msg {
                    Message::Text(t) => ws.emit_message(WsMessage::Text(t)),
                    Message::Binary(b) => ws.emit_message(WsMessage::Binary(Bytes::from(b))),
                    Message::Ping(p) => ws.emit_bytes(true, Bytes::from(p)),
                    Message::Pong(p) => ws.emit_bytes(false, Bytes::from(p)),
                    Message::Close(frame) => {
                        received = Some(match frame {
                            Some(f) => (u16::from(f.code), f.reason.into_owned()),
                            None => (1005, String::new()),
                        });
                        ws.inner.state.store(ReadyState::Closing as u8, Ordering::SeqCst);
                        deadline.get_or_insert_with(|| tokio::time::Instant::now() + Duration::from_secs(30));
                    }
                    Message::Frame(_) => {}
                },
                Some(Err(E::ConnectionClosed | E::AlreadyClosed | E::Io(_))) => break,
                Some(Err(e)) => {
                    let (err, code) = protocol_error(&e);
                    ws.emit_error(&err);
                    let frame = CloseFrame { code: CloseCode::from(code), reason: Cow::Borrowed("") };
                    let _ = tokio::time::timeout(Duration::from_secs(1), stream.close(Some(frame))).await;
                    break;
                }
            },
            cmd = rx.recv() => match cmd {
                Some(Command::Send(m, ack)) => {
                    let r = stream.send(m).await.map_err(|e| WsError::new(e.to_string(), None));
                    if let Some(a) = ack {
                        let _ = a.send(r);
                    }
                }
                Some(Command::Close(frame)) => {
                    let _ = stream.close(frame).await;
                    deadline.get_or_insert_with(|| tokio::time::Instant::now() + Duration::from_secs(30));
                }
                Some(Command::Terminate) | None => break,
            },
            _ = timeout => break,
        }
    }
    drop(stream);
    ws.inner.state.store(ReadyState::Closed as u8, Ordering::SeqCst);
    // Fail pending `send_and_wait` calls.
    rx.close();
    while let Ok(cmd) = rx.try_recv() {
        if let Command::Send(_, Some(ack)) = cmd {
            let _ = ack.send(Err(ws.not_open()));
        }
    }
    let (code, reason) = received.unwrap_or((1006, String::new()));
    ws.emit_close(code, &reason);
}
