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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use futures::{Stream, StreamExt};
use http::{HeaderValue, StatusCode};
use hyper::body::Incoming;

pub use options::{EngineOptions, EngineOptionsError};
pub use packet::EngineMessage;
pub use session::{CloseReason, SendError};

use self::session::Session;
use crate::http::Request;
use crate::http_adapter::HttpServer;
use crate::logger::Logger;
use crate::server::ResBody;

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
    sessions: RwLock<HashMap<String, Arc<Session>>>,
    closed: AtomicBool,
    on_connection: RwLock<Option<ConnectionCallback>>,
    last_cap_warning: Mutex<Option<std::time::Instant>>,
}

impl ServerInner {
    pub(crate) fn remove(&self, id: &str) {
        self.sessions.write().unwrap_or_else(|e| e.into_inner()).remove(id);
    }

    fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.read().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
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
        Ok(EngineServer {
            inner: Arc::new(ServerInner {
                options: Arc::new(options),
                path,
                sessions: RwLock::new(HashMap::new()),
                closed: AtomicBool::new(false),
                on_connection: RwLock::new(None),
                last_cap_warning: Mutex::new(None),
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
        self.inner.sessions.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Closes every session with `forced close`, answers each pending poll with a close packet, and refuses new
    /// handshakes with `503`. Idempotent.
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        let sessions: Vec<Arc<Session>> = self.inner.sessions.read().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
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

    /// Creates a session (checking `max_sessions` under the same lock that inserts it).
    fn open_session(&self, transport: TransportKind, handshake: Request) -> Option<Arc<Session>> {
        let mut sessions = self.inner.sessions.write().unwrap_or_else(|e| e.into_inner());
        if sessions.len() >= self.inner.options.max_sessions {
            return None;
        }
        let mut id = sid::generate();
        while sessions.contains_key(&id) {
            id = sid::generate();
        }
        let session = Session::open(Arc::downgrade(&self.inner), id.clone(), transport, handshake, self.inner.options.clone());
        sessions.insert(id, session.clone());
        Some(session)
    }

    fn warn_session_cap(&self) {
        let mut last = self.inner.last_cap_warning.lock().unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        if last.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(60)) {
            *last = Some(now);
            Logger::new("EngineIO").warn(format!(
                "the Engine.IO server reached max_sessions ({}): new handshakes are answered 503",
                self.inner.options.max_sessions
            ));
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
                if !upgrade && session.transport() != transport {
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
        if self.inner.closed.load(Ordering::SeqCst) {
            return self.handshake_refused(&handshake, false, StatusCode::SERVICE_UNAVAILABLE, 3, "Server is closing");
        }
        let Some(session) = self.open_session(TransportKind::Polling, handshake.clone()) else {
            self.warn_session_cap();
            return self.handshake_refused(&handshake, false, StatusCode::SERVICE_UNAVAILABLE, 3, "Too many sessions");
        };
        // The handshake request is the session's first poll: it carries the open packet, and what the connection
        // callback sends goes out with the next poll.
        let response = polling::get(&session).await;
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
                if self.inner.closed.load(Ordering::SeqCst) {
                    return self.handshake_refused(&handshake, true, StatusCode::SERVICE_UNAVAILABLE, 3, "Server is closing");
                }
                let Some(session) = self.open_session(TransportKind::WebSocket, handshake.clone()) else {
                    self.warn_session_cap();
                    return self.handshake_refused(&handshake, true, StatusCode::SERVICE_UNAVAILABLE, 3, "Too many sessions");
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
