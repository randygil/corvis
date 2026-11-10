//! The Engine.IO v4 server: what Node's `engine.io` is to `engine.attach(httpServer)`.
//!
//! **Unstable and hidden** until the Socket.IO server builds on it: nothing here is a supported API yet. An
//! [`EngineServer`] is created with [`EngineOptions`](options::EngineOptions), attached to the application's
//! [`HttpServer`](crate::http_adapter::HttpServer), and hands every new session to the `on_connection` callback as
//! an [`EngineSocket`].
//!
//! ```ignore
//! let engine = EngineServer::new(EngineOptions::new())?;
//! engine.on_connection(|socket| {
//!     tokio::spawn(async move {
//!         let mut messages = socket.messages();
//!         while let Some(message) = messages.next().await {
//!             let _ = socket.send(message);
//!         }
//!     });
//! });
//! engine.attach(&app.get_http_server())?;
//! ```
//!
//! Requests whose path equals the engine path or starts with it are answered here, before the router, the
//! `ws`-style gateways and the `upgrade` listeners; every other path behaves as without the engine.

pub mod options;
pub mod packet;
pub mod sid;

mod polling;
pub(crate) mod session;
mod websocket;

#[cfg(test)]
mod fuzz;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use futures::{Stream, StreamExt};
use http::{HeaderValue, StatusCode};
use hyper::body::Incoming;

pub use options::{Admission, EngineOptions, EngineOptionsError};
pub use packet::EngineMessage;
pub use session::{CloseReason, SendError};

use self::session::Session;
use crate::http::Request;
use crate::http_adapter::HttpServer;
use crate::logger::Logger;
use crate::server::ResBody;
use crate::ws::UnreadGauge;

/// The transport a session is currently using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Polling,
    WebSocket,
}

impl TransportKind {
    /// The `transport` query value.
    pub fn as_str(self) -> &'static str {
        match self {
            TransportKind::Polling => "polling",
            TransportKind::WebSocket => "websocket",
        }
    }
}

type ConnectionCallback = Arc<dyn Fn(EngineSocket) + Send + Sync + 'static>;

pub(crate) struct ServerInner {
    options: Arc<EngineOptions>,
    path: String,
    /// The sessions, the per-address counts and the `closed` flag live under one lock: a handshake checks the
    /// flag, both caps and inserts atomically with respect to `close()`'s snapshot and to `remove`.
    sessions: RwLock<Store>,
    /// Received bytes waiting for the application, across every session.
    total_unread: Arc<UnreadGauge>,
    on_connection: RwLock<Option<ConnectionCallback>>,
    last_cap_warning: Mutex<Option<std::time::Instant>>,
    last_hook_warning: Mutex<Option<std::time::Instant>>,
}

impl ServerInner {
    pub(crate) fn remove(&self, id: &str) {
        let mut store = self.sessions.write().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = store.map.remove(id) {
            let key = session.addr_key.clone();
            if let Some(count) = store.per_ip.get_mut(&key) {
                *count -= 1;
                if *count == 0 {
                    store.per_ip.remove(&key);
                }
            }
        }
    }

    fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.read().unwrap_or_else(|e| e.into_inner()).map.get(id).cloned()
    }
}

#[derive(Default)]
struct Store {
    map: HashMap<String, Arc<Session>>,
    per_ip: HashMap<String, usize>,
    closed: bool,
}

/// Why a session could not be opened.
enum OpenError {
    Closing,
    TooManySessions,
    TooManyFromAddress,
}

/// Reads an address as a proxy may have written it: `203.0.113.7`, `203.0.113.7:40000`, `2001:db8::1`,
/// `[2001:db8::1]`, `[2001:db8::1]:40000`, with an optional zone (`fe80::1%eth0`).
fn parse_forwarded_ip(raw: &str) -> Option<std::net::IpAddr> {
    let raw = raw.trim();
    let host = if let Some(rest) = raw.strip_prefix('[') {
        rest.split(']').next()?
    } else if raw.matches(':').count() == 1 {
        // `a.b.c.d:port`
        raw.split(':').next()?
    } else {
        raw
    };
    host.split('%').next()?.parse().ok()
}

/// The key sessions are counted under: the address `ip()` resolved (so `set_trust_proxy` is honored), normalized
/// (brackets, port and zone removed), with IPv6 addresses reduced to their `/64` (one subscriber gets at least that
/// much) and IPv4-mapped ones to IPv4. When the resolved value is not an address at all the socket's peer is used,
/// so a client cannot choose a fresh key per request with a malformed header (behind `TrustProxy::All` it still
/// chooses a well-formed one: the per-address cap does not protect there).
pub(crate) fn address_key(ip: &str, peer: Option<std::net::SocketAddr>) -> String {
    let addr = parse_forwarded_ip(ip).or_else(|| peer.map(|p| p.ip())).map(|a| match a {
        std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped().map(std::net::IpAddr::V4).unwrap_or(a),
        v4 => v4,
    });
    match addr {
        Some(std::net::IpAddr::V6(v6)) => {
            let o = v6.octets();
            format!("{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}/64", o[0], o[1], o[2], o[3], o[4], o[5], o[6], o[7])
        }
        Some(v4) => v4.to_string(),
        None => ip.to_string(),
    }
}

/// An Engine.IO server. Cheap to clone; all clones are the same server.
#[derive(Clone)]
pub struct EngineServer {
    inner: Arc<ServerInner>,
}

impl EngineServer {
    /// Creates a server, refusing invalid options with an error that names the option.
    pub fn new(options: EngineOptions) -> Result<EngineServer, EngineOptionsError> {
        options.validate()?;
        let path = options.normalized_path();
        let total_unread = Arc::new(UnreadGauge::new(options.max_total_unread_bytes, None));
        Ok(EngineServer {
            inner: Arc::new(ServerInner {
                options: Arc::new(options),
                path,
                sessions: RwLock::new(Store::default()),
                total_unread,
                on_connection: RwLock::new(None),
                last_cap_warning: Mutex::new(None),
                last_hook_warning: Mutex::new(None),
            }),
        })
    }

    /// `engine.attach(httpServer)`: from now on the server takes every request under its path. Works before or
    /// after the application is initialized and before it listens. Fails when another attached server's path
    /// overlaps this one.
    pub fn attach(&self, server: &HttpServer) -> Result<(), EngineOptionsError> {
        server.attach_engine(self)?;
        if let Some(warning) = self.inner.options.cors.as_ref().and_then(|c| c.startup_warning()) {
            Logger::new("EngineIO").warn(warning);
        }
        Ok(())
    }

    /// Sets the callback that receives every new session, once its open packet is queued. It runs on the
    /// connection's task: spawn a task for anything slow. A panic closes only that session.
    pub fn on_connection(&self, f: impl Fn(EngineSocket) + Send + Sync + 'static) -> &Self {
        *self.inner.on_connection.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(f));
        self
    }

    /// The path the server takes over (normalized, with its trailing slash).
    pub fn path(&self) -> &str {
        &self.inner.path
    }

    /// The number of open sessions.
    pub fn session_count(&self) -> usize {
        self.inner.sessions.read().unwrap_or_else(|e| e.into_inner()).map.len()
    }

    /// Closes every session with `forced close`, answers each pending poll with a close packet, and refuses new
    /// handshakes with `503`. Idempotent.
    pub fn close(&self) {
        // The flag and the snapshot are taken under the lock that inserts sessions: none can appear after it.
        let sessions: Vec<Arc<Session>> = {
            let mut store = self.inner.sessions.write().unwrap_or_else(|e| e.into_inner());
            store.closed = true;
            store.map.values().cloned().collect()
        };
        for s in sessions {
            s.close(CloseReason::ForcedClose);
        }
    }

    /// `true` when the request path is this server's (equal to its path or starting with it).
    pub(crate) fn matches(&self, path: &str) -> bool {
        path.starts_with(&self.inner.path)
    }

    pub(crate) fn overlaps(&self, other: &EngineServer) -> bool {
        self.inner.path.starts_with(&other.inner.path) || other.inner.path.starts_with(&self.inner.path)
    }

    fn handshake_refused(
        &self,
        handshake: &Request,
        upgrade: bool,
        status: StatusCode,
        code: u8,
        message: &str,
    ) -> hyper::Response<ResBody> {
        let _ = handshake;
        let mut resp = polling::json_error(status, code, message);
        if upgrade {
            resp.headers_mut().insert(http::header::CONNECTION, HeaderValue::from_static("close"));
        }
        resp
    }

    /// Creates a session. The `closed` flag, `max_sessions` and `max_sessions_per_ip` are checked and the session
    /// inserted under the one lock that `close()` and `remove` take, so a handshake cannot slip in after a close
    /// and the counts cannot be overshot by concurrent handshakes.
    fn open_session(&self, transport: TransportKind, handshake: Request) -> Result<Arc<Session>, OpenError> {
        let mut store = self.inner.sessions.write().unwrap_or_else(|e| e.into_inner());
        let key = address_key(&handshake.ip(), handshake.remote_addr());
        self.caps_ok(&store, &key)?;
        let mut id = sid::generate();
        while store.map.contains_key(&id) {
            id = sid::generate();
        }
        let session = Session::open(
            Arc::downgrade(&self.inner),
            id.clone(),
            transport,
            handshake,
            self.inner.options.clone(),
            Some(self.inner.total_unread.clone()),
        );
        *store.per_ip.entry(key).or_insert(0) += 1;
        store.map.insert(id, session.clone());
        Ok(session)
    }

    fn caps_ok(&self, store: &Store, key: &str) -> Result<(), OpenError> {
        if store.closed {
            return Err(OpenError::Closing);
        }
        if store.map.len() >= self.inner.options.max_sessions {
            return Err(OpenError::TooManySessions);
        }
        if store.per_ip.get(key).copied().unwrap_or(0) >= self.inner.options.max_sessions_per_ip {
            return Err(OpenError::TooManyFromAddress);
        }
        Ok(())
    }

    /// The same checks as `open_session` without inserting: run before the `allow_request` hook so a client over a
    /// cap is answered `503` without the hook being invoked. `open_session` checks again under the write lock.
    fn check_caps(&self, handshake: &Request) -> Result<(), OpenError> {
        let store = self.inner.sessions.read().unwrap_or_else(|e| e.into_inner());
        self.caps_ok(&store, &address_key(&handshake.ip(), handshake.remote_addr()))
    }

    /// The response for a handshake that could not open a session.
    fn open_refused(&self, handshake: &Request, upgrade: bool, error: OpenError) -> hyper::Response<ResBody> {
        match error {
            OpenError::Closing => self.handshake_refused(handshake, upgrade, StatusCode::SERVICE_UNAVAILABLE, 3, "Server is closing"),
            OpenError::TooManySessions => {
                self.warn_session_cap(format!("max_sessions ({})", self.inner.options.max_sessions));
                self.handshake_refused(handshake, upgrade, StatusCode::SERVICE_UNAVAILABLE, 3, "Too many sessions")
            }
            OpenError::TooManyFromAddress => {
                self.warn_session_cap(format!("max_sessions_per_ip ({})", self.inner.options.max_sessions_per_ip));
                self.handshake_refused(handshake, upgrade, StatusCode::SERVICE_UNAVAILABLE, 3, "Too many sessions")
            }
        }
    }

    /// Runs the `allow_request` hook, if any. A panic in it rejects the request.
    async fn admitted(&self, handshake: &Request) -> bool {
        let Some(hook) = &self.inner.options.allow_request else { return true };
        use futures::FutureExt;
        let future = std::panic::AssertUnwindSafe(async { (hook.0)(handshake).await });
        let limit = self.inner.options.allow_request_timeout;
        match tokio::time::timeout(limit, future.catch_unwind()).await {
            Ok(Ok(decision)) => decision == options::Admission::Allow,
            Ok(Err(_)) => {
                Logger::new("EngineIO").error("the allow_request hook panicked: the request was refused");
                false
            }
            Err(_) => {
                let mut last = self.inner.last_hook_warning.lock().unwrap_or_else(|e| e.into_inner());
                let now = std::time::Instant::now();
                if last.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(60)) {
                    *last = Some(now);
                    Logger::new("EngineIO").warn(format!("the allow_request hook took longer than {limit:?}: the request was refused"));
                }
                false
            }
        }
    }

    fn warn_session_cap(&self, which: String) {
        let mut last = self.inner.last_cap_warning.lock().unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        if last.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(60)) {
            *last = Some(now);
            Logger::new("EngineIO").warn(format!("the Engine.IO server reached {which}: new handshakes are answered 503"));
        }
    }

    fn notify_connection(&self, session: &Arc<Session>) {
        let callback = self.inner.on_connection.read().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(callback) = callback {
            let socket = EngineSocket { session: session.clone() };
            if !crate::ws::guarded("the Engine.IO on_connection callback", || callback(socket)) {
                session.close(CloseReason::TransportError);
            }
        }
    }

    /// Handles one request under the engine path. The caller has checked [`matches`](Self::matches).
    pub(crate) async fn handle(
        &self,
        mut req: hyper::Request<Incoming>,
        remote: SocketAddr,
        parser: &crate::http::body_parser::BodyParserOptions,
        request_timeout: Option<Duration>,
    ) -> hyper::Response<ResBody> {
        let upgrade = crate::http_adapter::is_upgrade_request(&req);
        let on_upgrade = if upgrade { Some(hyper::upgrade::on(&mut req)) } else { None };
        let (parts, body) = req.into_parts();
        let Ok(handshake) = Request::for_upgrade(parts, remote, parser) else {
            return polling::json_error(StatusCode::BAD_REQUEST, 3, "Bad request");
        };
        let cors = self.inner.options.cors.clone();
        if !upgrade
            && let Some(cors) = &cors
            && cors.is_preflight(&handshake)
        {
            return crate::server::into_hyper(cors.preflight(&handshake), false);
        }
        let mut response = self.dispatch(handshake.clone(), body, on_upgrade, upgrade, request_timeout).await;
        if !upgrade && let Some(cors) = &cors {
            let mut carrier = crate::http::Response::new();
            cors.apply(&handshake, &mut carrier);
            for (name, value) in carrier.headers().iter() {
                if name == http::header::VARY
                    && let Some(current) = response.headers().get(http::header::VARY).and_then(|v| v.to_str().ok())
                    && let Some(merged) = crate::router::cors::vary_append(current, value.to_str().unwrap_or(""))
                    && let Ok(v) = HeaderValue::from_str(&merged)
                {
                    response.headers_mut().insert(name.clone(), v);
                    continue;
                }
                response.headers_mut().insert(name.clone(), value.clone());
            }
        }
        response
    }

    async fn dispatch(
        &self,
        handshake: Request,
        body: Incoming,
        on_upgrade: Option<hyper::upgrade::OnUpgrade>,
        upgrade: bool,
        request_timeout: Option<Duration>,
    ) -> hyper::Response<ResBody> {
        let query = Query::parse(handshake.uri().query());
        let options = &self.inner.options;
        let abort =
            |code: u8| -> hyper::Response<ResBody> { if upgrade { polling::abort_upgrade(code) } else { polling::abort_request(code) } };

        // Node's `verify`, in its order.
        let transport = match query.get("transport") {
            Some("polling") => TransportKind::Polling,
            Some("websocket") => TransportKind::WebSocket,
            _ => return abort(0),
        };
        if let Some(origin) = handshake.headers().get(http::header::ORIGIN)
            && origin.as_bytes().iter().any(|b| (*b < 0x20 && *b != b'\t') || *b == 0x7f)
        {
            return abort(3);
        }
        let sid = query.get("sid").filter(|s| !s.is_empty());
        let session = match sid {
            Some(sid) => {
                // The shape is checked before any lookup: an attacker-sized string never reaches the map.
                let found = if sid::has_valid_shape(sid) { self.inner.get(sid) } else { None };
                let Some(session) = found else { return abort(1) };
                // Without an upgrade, only the polling transport of a session that still uses it is served: a
                // `transport=websocket` request that is no upgrade, or any polling request on a session that
                // moved to WebSocket, is a bad request and delivers nothing.
                if !upgrade && (transport != TransportKind::Polling || session.transport() != TransportKind::Polling) {
                    return abort(3);
                }
                if query.get("EIO") != Some("4") {
                    return abort(3);
                }
                Some(session)
            }
            None => {
                if handshake.method() != http::Method::GET {
                    return abort(2);
                }
                if transport == TransportKind::WebSocket && !upgrade {
                    return abort(3);
                }
                None
            }
        };

        // Origin allowlist: handshakes and upgrades.
        if (sid.is_none() || upgrade)
            && let (Some(allowed), Some(origin)) = (&options.allowed_origins, handshake.headers().get(http::header::ORIGIN))
        {
            let origin = origin.to_str().unwrap_or("");
            if !allowed.iter().any(|a| a.eq_ignore_ascii_case(origin)) {
                return self.handshake_refused(&handshake, upgrade, StatusCode::FORBIDDEN, 4, "Forbidden");
            }
        }

        if upgrade {
            return self.upgrade(handshake, on_upgrade, session, transport, &query).await;
        }
        match session {
            Some(session) => match *handshake.method() {
                http::Method::GET => polling::get(&session).await,
                http::Method::POST => polling::post(&session, handshake.headers(), body, request_timeout).await,
                _ => polling::empty(StatusCode::INTERNAL_SERVER_ERROR),
            },
            None => self.handshake_polling(handshake, &query).await,
        }
    }

    async fn handshake_polling(&self, handshake: Request, query: &Query) -> hyper::Response<ResBody> {
        if query.get("EIO") != Some("4") {
            return polling::abort_request(5);
        }
        // Before the hook: a client over a cap is answered without invoking it.
        if let Err(error) = self.check_caps(&handshake) {
            return self.open_refused(&handshake, false, error);
        }
        if !self.admitted(&handshake).await {
            return self.handshake_refused(&handshake, false, StatusCode::FORBIDDEN, 4, "Forbidden");
        }
        let session = match self.open_session(TransportKind::Polling, handshake.clone()) {
            Ok(session) => session,
            Err(error) => return self.open_refused(&handshake, false, error),
        };
        // The handshake request is the session's first poll: it carries the open packet, and what the connection
        // callback sends goes out with the next poll.
        let response = polling::get(&session).await;
        // From here the client has `first_request_timeout` to make its next request.
        session.arm_first_request();
        self.notify_connection(&session);
        response
    }

    async fn upgrade(
        &self,
        handshake: Request,
        on_upgrade: Option<hyper::upgrade::OnUpgrade>,
        session: Option<Arc<Session>>,
        transport: TransportKind,
        query: &Query,
    ) -> hyper::Response<ResBody> {
        let Some(on_upgrade) = on_upgrade else { return polling::abort_upgrade(3) };
        if !handshake
            .headers()
            .get(http::header::UPGRADE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|u| u.eq_ignore_ascii_case("websocket"))
        {
            return crate::websockets::reject_handshake(400, "Invalid Upgrade header", &[]);
        }
        let key = match crate::websockets::validate_ws_handshake(handshake.headers(), handshake.method(), None) {
            Ok(key) => key,
            Err(resp) => return resp,
        };
        let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(&key);
        let close_timeout = self.inner.options.close_timeout;
        let (session, role) = match session {
            // An upgrade of an existing session; a transport other than `websocket` is closed right away.
            Some(session) => {
                (session, if transport == TransportKind::WebSocket { websocket::Role::Candidate } else { websocket::Role::Refuse })
            }
            None => {
                if transport != TransportKind::WebSocket {
                    return polling::abort_upgrade(3);
                }
                if query.get("EIO") != Some("4") {
                    return polling::abort_upgrade(5);
                }
                if let Err(error) = self.check_caps(&handshake) {
                    return self.open_refused(&handshake, true, error);
                }
                if !self.admitted(&handshake).await {
                    return self.handshake_refused(&handshake, true, StatusCode::FORBIDDEN, 4, "Forbidden");
                }
                let session = match self.open_session(TransportKind::WebSocket, handshake.clone()) {
                    Ok(session) => session,
                    Err(error) => return self.open_refused(&handshake, true, error),
                };
                (session, websocket::Role::First)
            }
        };
        let server = self.clone();
        tokio::spawn(async move {
            let io = match on_upgrade.await {
                Ok(io) => hyper_util::rt::TokioIo::new(io),
                Err(_) => {
                    if role == websocket::Role::First {
                        session.close(CloseReason::TransportError);
                    }
                    return;
                }
            };
            websocket::run(server, session, io, role, close_timeout).await;
        });
        hyper::Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header("upgrade", "websocket")
            .header("connection", "Upgrade")
            .header("sec-websocket-accept", accept)
            .body(crate::server::empty_body())
            .expect("a valid upgrade response")
    }
}

/// The query string as Node's `URLSearchParams` makes it into an object: a parameter given more than once keeps
/// its last value.
pub(crate) struct Query(HashMap<String, String>);

impl Query {
    fn parse(query: Option<&str>) -> Query {
        let mut map = HashMap::new();
        for (k, v) in form_urlencoded::parse(query.unwrap_or("").as_bytes()) {
            map.insert(k.into_owned(), v.into_owned());
        }
        Query(map)
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

/// One session as the application sees it.
#[derive(Clone)]
pub struct EngineSocket {
    session: Arc<Session>,
}

impl std::fmt::Debug for EngineSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineSocket").field("session", &self.session.log_id).finish()
    }
}

impl EngineSocket {
    /// The session id (`socket.id`). It is the session's bearer credential: do not log it.
    pub fn id(&self) -> &str {
        &self.session.id
    }

    /// The request that opened the session, built with the application's request settings.
    pub fn handshake(&self) -> &Request {
        &self.session.handshake
    }

    /// The client address, resolved once when the session was opened (it follows `set_trust_proxy`).
    pub fn ip(&self) -> &str {
        &self.session.ip
    }

    /// The transport in use now.
    pub fn transport(&self) -> TransportKind {
        self.session.transport()
    }

    /// Queues a message. A send that would pass `max_buffered_amount` fails, discards the queue and closes the
    /// session with `transport error`.
    pub fn send(&self, message: impl Into<EngineMessage>) -> Result<(), SendError> {
        self.session.send(message.into())
    }

    /// The messages received from the client. Only the first call gets the stream; later calls get an empty one.
    /// Bytes not yet taken count against `max_unread_bytes`.
    pub fn messages(&self) -> Pin<Box<dyn Stream<Item = EngineMessage> + Send + 'static>> {
        match self.session.take_inbox() {
            Some(mut rx) => Box::pin(futures::stream::poll_fn(move |cx| rx.poll_recv(cx)).map(|(message, _ticket)| message)),
            None => Box::pin(futures::stream::empty()),
        }
    }

    /// Writes what is queued, then closes the session with `forced close`.
    pub fn close(&self) {
        self.session.close_graceful();
    }

    /// Resolves with the reason once the session is closed.
    pub async fn closed(&self) -> CloseReason {
        let mut rx = self.session.closed_watch();
        loop {
            if let Some(reason) = *rx.borrow_and_update() {
                return reason;
            }
            if rx.changed().await.is_err() {
                return self.session.close_reason().unwrap_or(CloseReason::TransportClose);
            }
        }
    }

    /// The reason, if the session is already closed.
    pub fn close_reason(&self) -> Option<CloseReason> {
        self.session.close_reason()
    }

    /// Bytes queued for the client and not yet written.
    pub fn buffered_amount(&self) -> usize {
        self.session.outbox.buffered()
    }

    /// Bytes received and not yet taken from [`messages`](Self::messages).
    pub fn unread_bytes(&self) -> usize {
        self.session.unread.unread()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_keys_are_normalized_whatever_form_a_proxy_wrote() {
        let peer: Option<std::net::SocketAddr> = Some("198.51.100.9:5000".parse().unwrap());
        // IPv4 with and without a port.
        assert_eq!(address_key("203.0.113.7", peer), "203.0.113.7");
        assert_eq!(address_key("203.0.113.7:40000", peer), "203.0.113.7");
        // IPv6: bare, bracketed, bracketed with a port, with a zone: all one /64.
        let key = address_key("2001:db8:1:2::1", peer);
        assert_eq!(address_key("[2001:db8:1:2::1]", peer), key);
        assert_eq!(address_key("[2001:db8:1:2::1]:40000", peer), key);
        assert_eq!(address_key("2001:db8:1:2:ffff::7%eth0", peer), key);
        assert_eq!(address_key("[2001:db8:1:2::1%eth0]:1", peer), key);
        assert_ne!(address_key("2001:db8:1:3::1", peer), key);
        // IPv4-mapped IPv6 is IPv4.
        assert_eq!(address_key("::ffff:203.0.113.7", peer), "203.0.113.7");
        assert_eq!(address_key("[::ffff:203.0.113.7]:9", peer), "203.0.113.7");
        // Garbage falls back to the socket peer, not to a key of the client's choosing.
        assert_eq!(address_key("not an address", peer), "198.51.100.9");
        assert_eq!(address_key("", peer), "198.51.100.9");
        assert_eq!(address_key("999.1.1.1", peer), "198.51.100.9");
    }

    #[test]
    fn a_client_cannot_pick_a_new_key_per_request_with_a_malformed_forwarded_address() {
        let peer: Option<std::net::SocketAddr> = Some("198.51.100.9:5000".parse().unwrap());
        let keys: std::collections::HashSet<String> = (0..50).map(|i| address_key(&format!("junk-{i}"), peer)).collect();
        assert_eq!(keys.len(), 1);
    }

    #[tokio::test]
    async fn no_session_is_opened_after_close() {
        let server = EngineServer::new(EngineOptions::new()).unwrap();
        let (parts, _) = http::Request::builder().uri("/socket.io/?EIO=4&transport=polling").body(()).unwrap().into_parts();
        let handshake = Request::for_upgrade(parts, "127.0.0.1:1".parse().unwrap(), &Default::default()).unwrap();
        assert!(server.open_session(TransportKind::Polling, handshake.clone()).is_ok());
        server.close();
        assert!(matches!(server.open_session(TransportKind::Polling, handshake), Err(OpenError::Closing)));
        assert_eq!(server.session_count(), 0);
        assert!(server.inner.sessions.read().unwrap().per_ip.is_empty(), "the address counts are released with the sessions");
    }
}
