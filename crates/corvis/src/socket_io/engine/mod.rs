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

use std::sync::{Arc, Mutex, RwLock, Weak};
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
use crate::ws::{UnreadGauge, UnreadTicket};

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
type ClosingCallback = Arc<dyn Fn() + Send + Sync + 'static>;

pub(crate) struct ServerInner {
    options: Arc<EngineOptions>,
    path: String,
    /// The sessions, the per-address counts and the `closed` flag live under one lock: a handshake checks the
    /// flag, both caps and inserts atomically with respect to `close()`'s snapshot and to `remove`.
    sessions: RwLock<Store>,
    /// Received bytes waiting for the application, across every session.
    total_unread: Arc<UnreadGauge>,
    /// The gauge of every client address that holds something, when `max_address_unread_bytes` is set: between its
    /// sessions' gauges and the server-wide one, so everything an address holds counts here, in every state, and
    /// outlives its sessions for as long as anything charged through them does.
    addresses: Arc<AddressTable>,
    /// Handlers of a layer on top that outlive their connection, per client address (`EngineSocket::linger`).
    lingering: Mutex<HashMap<String, usize>>,
    on_connection: RwLock<Option<ConnectionCallback>>,
    /// Callbacks run at the start of `close()`, before any session is closed.
    on_closing: RwLock<Vec<ClosingCallback>>,
    last_cap_warning: Mutex<Option<std::time::Instant>>,
    last_hook_warning: Mutex<Option<std::time::Instant>>,
    /// When the last send-buffer overflow was logged and how many were left out since: a broadcast can close many
    /// sessions at once, so the line is written once a minute with the number it left out.
    overflow_warning: Mutex<(Option<std::time::Instant>, u64)>,
}

type AddressTable = Mutex<HashMap<String, Weak<UnreadGauge>>>;

impl ServerInner {
    /// The line to write for a session closed by a send buffer over its limit, or `None` when one was written less
    /// than a minute ago (the next one says how many were left out).
    pub(crate) fn overflow_line(&self, log_id: u64, now: std::time::Instant) -> Option<String> {
        let mut state = self.overflow_warning.lock().unwrap_or_else(|e| e.into_inner());
        let (last, suppressed) = &mut *state;
        if last.is_some_and(|t| now.saturating_duration_since(t) < Duration::from_secs(60)) {
            *suppressed += 1;
            return None;
        }
        let left_out = std::mem::take(suppressed);
        *last = Some(now);
        Some(if left_out > 0 {
            format!(
                "session {log_id}: send buffer over its limit, session closed ({left_out} more were closed in the last minute without a line)"
            )
        } else {
            format!("session {log_id}: send buffer over its limit, session closed")
        })
    }

    /// What a new session's gauge is chained to: the gauge of its address when the address has a share of the
    /// budget, the server-wide gauge otherwise.
    fn parent_gauge(&self, key: &str) -> Arc<UnreadGauge> {
        let Some(share) = self.options.max_address_unread_bytes else { return self.total_unread.clone() };
        let mut table = self.addresses.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(gauge) = table.get(key).and_then(Weak::upgrade) {
            return gauge;
        }
        let (forget, owner) = (key.to_string(), Arc::downgrade(&self.addresses));
        let gauge = Arc::new(
            UnreadGauge::new(share, Some(self.total_unread.clone()))
                .with_guarantee_cap(share.saturating_add(self.options.address_guarantee()))
                .with_on_drop(move || {
                    // Forget the address once nothing refers to it. A newer gauge of the same address may have replaced
                    // this one already: only a dead entry is removed.
                    if let Some(table) = owner.upgrade() {
                        let mut table = table.lock().unwrap_or_else(|e| e.into_inner());
                        if table.get(&forget).is_some_and(|w| w.strong_count() == 0) {
                            table.remove(&forget);
                        }
                    }
                }),
        );
        table.insert(key.to_string(), Arc::downgrade(&gauge));
        gauge
    }

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
    /// Handshakes that passed the caps and wait for the `allow_request` hook: counted against both caps so
    /// concurrent handshakes cannot all reach the hook past a cap.
    pending: usize,
    pending_ip: HashMap<String, usize>,
    closed: bool,
}

/// A slot reserved for a handshake while the hook runs. Dropped (rejection, timeout, panic, cancelled request)
/// it gives the slot back; `open_session` turns it into the real session under the same lock.
struct Reservation {
    inner: Arc<ServerInner>,
    key: String,
    done: bool,
}

impl Reservation {
    fn release(&mut self, store: &mut Store) {
        if self.done {
            return;
        }
        self.done = true;
        store.pending = store.pending.saturating_sub(1);
        if let Some(count) = store.pending_ip.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                store.pending_ip.remove(&self.key);
            }
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.done {
            let inner = self.inner.clone();
            let mut store = inner.sessions.write().unwrap_or_else(|e| e.into_inner());
            self.release(&mut store);
        }
    }
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
        let total_unread = Arc::new(UnreadGauge::new(options.max_total_unread_bytes, None).pooled());
        Ok(EngineServer {
            inner: Arc::new(ServerInner {
                options: Arc::new(options),
                path,
                sessions: RwLock::new(Store::default()),
                total_unread,
                addresses: Arc::new(Mutex::new(HashMap::new())),
                lingering: Mutex::new(HashMap::new()),
                on_connection: RwLock::new(None),
                on_closing: RwLock::new(Vec::new()),
                last_cap_warning: Mutex::new(None),
                last_hook_warning: Mutex::new(None),
                overflow_warning: Mutex::new((None, 0)),
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

    /// Adds a callback that runs at the start of [`close`](Self::close), after new handshakes are refused and
    /// before any session is closed, so a layer on top can report the shutdown to its own users first. A panic in
    /// it is logged and does not stop the close. It runs on every call of `close`: make it idempotent.
    pub fn on_closing(&self, f: impl Fn() + Send + Sync + 'static) -> &Self {
        self.inner.on_closing.write().unwrap_or_else(|e| e.into_inner()).push(Arc::new(f));
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
        let hooks: Vec<ClosingCallback> = self.inner.on_closing.read().unwrap_or_else(|e| e.into_inner()).clone();
        for hook in hooks {
            crate::ws::guarded("the Engine.IO on_closing callback", || hook());
        }
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

    /// Reserves a slot for a handshake under the lock that `close()` and `remove` take, checking the `closed`
    /// flag and both caps against the sessions and the other reservations: a client over a cap is answered `503`
    /// without the `allow_request` hook running, and concurrent handshakes cannot overshoot a cap.
    fn reserve(&self, handshake: &Request) -> Result<Reservation, OpenError> {
        let mut store = self.inner.sessions.write().unwrap_or_else(|e| e.into_inner());
        let key = address_key(&handshake.ip(), handshake.remote_addr());
        self.caps_ok(&store, &key)?;
        store.pending += 1;
        *store.pending_ip.entry(key.clone()).or_insert(0) += 1;
        Ok(Reservation { inner: self.inner.clone(), key, done: false })
    }

    /// Creates a session from a reservation. The `closed` flag is checked again and the session inserted under
    /// the one lock that `close()` and `remove` take, so a handshake cannot slip in after a close.
    fn open_session(&self, transport: TransportKind, handshake: Request, mut reservation: Reservation) -> Result<Arc<Session>, OpenError> {
        let mut store = self.inner.sessions.write().unwrap_or_else(|e| e.into_inner());
        let key = reservation.key.clone();
        reservation.release(&mut store);
        if store.closed {
            return Err(OpenError::Closing);
        }
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
            Some(self.inner.parent_gauge(&key)),
        );
        *store.per_ip.entry(key).or_insert(0) += 1;
        store.map.insert(id, session.clone());
        Ok(session)
    }

    fn caps_ok(&self, store: &Store, key: &str) -> Result<(), OpenError> {
        if store.closed {
            return Err(OpenError::Closing);
        }
        if store.map.len() + store.pending >= self.inner.options.max_sessions {
            return Err(OpenError::TooManySessions);
        }
        let held = store.per_ip.get(key).copied().unwrap_or(0) + store.pending_ip.get(key).copied().unwrap_or(0);
        if held >= self.inner.options.max_sessions_per_ip {
            return Err(OpenError::TooManyFromAddress);
        }
        if let Some(cap) = self.inner.options.max_lingering_per_ip
            && self.inner.lingering.lock().unwrap_or_else(|e| e.into_inner()).get(key).is_some_and(|n| *n >= cap)
        {
            return Err(OpenError::TooManyFromAddress);
        }
        Ok(())
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
        // Before the hook: a client over a cap is answered without invoking it, and the slot stays reserved while
        // the hook runs.
        let reservation = match self.reserve(&handshake) {
            Ok(reservation) => reservation,
            Err(error) => return self.open_refused(&handshake, false, error),
        };
        if !self.admitted(&handshake).await {
            return self.handshake_refused(&handshake, false, StatusCode::FORBIDDEN, 4, "Forbidden");
        }
        let session = match self.open_session(TransportKind::Polling, handshake.clone(), reservation) {
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
                let reservation = match self.reserve(&handshake) {
                    Ok(reservation) => reservation,
                    Err(error) => return self.open_refused(&handshake, true, error),
                };
                if !self.admitted(&handshake).await {
                    return self.handshake_refused(&handshake, true, StatusCode::FORBIDDEN, 4, "Forbidden");
                }
                let session = match self.open_session(TransportKind::WebSocket, handshake.clone(), reservation) {
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

/// Handlers that outlive their session, counted against its client address (`EngineSocket::linger`).
pub(crate) struct Lingering {
    server: Arc<ServerInner>,
    key: String,
    remaining: usize,
}

impl Lingering {
    /// One of the handlers ended.
    pub(crate) fn one_done(&mut self) {
        self.give_back(1);
    }

    fn give_back(&mut self, count: usize) {
        let count = count.min(self.remaining);
        if count == 0 {
            return;
        }
        self.remaining -= count;
        let mut table = self.server.lingering.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = table.get_mut(&self.key) {
            *n = n.saturating_sub(count);
            if *n == 0 {
                table.remove(&self.key);
            }
        }
    }
}

impl Drop for Lingering {
    fn drop(&mut self) {
        self.give_back(self.remaining);
    }
}

/// Bytes charged against a session's unread budget; they are released when this is dropped.
pub struct Held(UnreadTicket);

impl Held {
    /// Lowers the charge to `bytes` (never raises it) and wakes whatever waits for room.
    pub(crate) fn shrink_to(&mut self, bytes: usize) {
        self.0.shrink_to(bytes);
    }

    /// The bytes this guard charges.
    pub(crate) fn bytes(&self) -> usize {
        self.0.size()
    }
}

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Held")
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

    /// Queues several messages as one unit: nothing else is queued between them, and the group is accepted or
    /// refused as a whole (a group that would pass `max_buffered_amount` fails, discards the queue and closes the
    /// session with `transport error`, like [`send`](Self::send)). A Socket.IO binary packet is such a group.
    pub fn send_all(&self, messages: Vec<EngineMessage>) -> Result<(), SendError> {
        self.session.send_all(messages)
    }

    /// Holds the heartbeat's ping back until the guard is dropped, for at most one second (see `session::PingHold`): what
    /// a layer queues meanwhile is not separated by a ping.
    pub(crate) fn hold_ping(&self) -> session::PingHold {
        self.session.hold_ping()
    }

    /// Counts `count` handlers that outlive this session against its client address, until the guard says they are
    /// done or is dropped. At `max_lingering_per_ip` of them an address gets no new sessions (`503`). `None` when the
    /// server is gone.
    pub(crate) fn linger(&self, count: usize) -> Option<Lingering> {
        let server = self.session.server()?;
        let key = self.session.addr_key.clone();
        *server.lingering.lock().unwrap_or_else(|e| e.into_inner()).entry(key.clone()).or_insert(0) += count;
        Some(Lingering { server, key, remaining: count })
    }

    /// Charges `bytes` against this session's unread budget (and the server's) until the returned guard is
    /// dropped. A layer that keeps something decoded from received messages waiting charges its real size here.
    pub fn hold(&self, bytes: usize) -> Held {
        Held(UnreadTicket::new(&self.session.unread, bytes))
    }

    /// Charges `bytes` only if the session stays at or below its fair share of the server-wide budget (and its address
    /// at or below its quota plus the guaranteed shares): what a session may always take, whatever others hold. An
    /// atomic check and charge; `None` leaves every gauge unchanged.
    pub(crate) fn hold_within_share(&self, bytes: usize, credit: usize) -> Option<Held> {
        UnreadTicket::try_new_within_share(&self.session.unread, bytes, credit).map(Held)
    }

    /// Charges `bytes` only if the server-wide budget keeps room for them (an atomic check and charge, so racing
    /// sessions cannot overshoot it together). `None` leaves every gauge unchanged.
    pub(crate) fn hold_within_total(&self, bytes: usize, credit: usize) -> Option<Held> {
        UnreadTicket::try_new_within_ancestors(&self.session.unread, bytes, credit).map(Held)
    }

    /// The test the transports use to stop reading: the session is over its own limit, or over its fair share while
    /// the server-wide budget is exceeded.
    pub(crate) fn unread_over_limit(&self) -> bool {
        self.session.unread.over_limit()
    }

    /// Resolves at the next release of this session's gauge or the server-wide one. Create it before checking
    /// [`unread_over_limit`](Self::unread_over_limit), so a release in between is not lost.
    pub(crate) fn unread_changed(&self) -> impl std::future::Future<Output = ()> + '_ {
        self.session.unread.changed()
    }

    /// Like [`messages`](Self::messages), but each message keeps counting against `max_unread_bytes` until its
    /// [`Held`] guard is dropped, instead of until it is taken: what the caller keeps waiting for a handler or for
    /// more messages stays inside the budget. Shares the single stream with `messages`.
    pub fn messages_held(&self) -> Pin<Box<dyn Stream<Item = (EngineMessage, Held)> + Send + 'static>> {
        match self.session.take_inbox() {
            Some(mut rx) => Box::pin(futures::stream::poll_fn(move |cx| rx.poll_recv(cx)).map(|(message, ticket)| (message, Held(ticket)))),
            None => Box::pin(futures::stream::empty()),
        }
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

/// Helpers for the tests of the layers built on this one: a session with no network, fed and read directly.
#[cfg(test)]
impl EngineServer {
    /// Opens a WebSocket-first session without any transport and hands it to the connection callback.
    pub(crate) fn open_test_socket(&self) -> EngineSocket {
        self.open_test_socket_from("127.0.0.1")
    }

    /// Bytes held against the server-wide unread budget by every session.
    pub(crate) fn total_unread_bytes(&self) -> usize {
        self.inner.total_unread.unread()
    }

    /// Bytes held by every session of `address` together (0 when it holds nothing).
    pub(crate) fn address_unread_bytes(&self, address: &str) -> usize {
        let key = address_key(address, None);
        let table = self.inner.addresses.lock().unwrap_or_else(|e| e.into_inner());
        table.get(&key).and_then(Weak::upgrade).map_or(0, |gauge| gauge.unread())
    }

    /// Handlers that outlive their connection and are counted against `address`.
    pub(crate) fn lingering(&self, address: &str) -> usize {
        let key = address_key(address, None);
        self.inner.lingering.lock().unwrap_or_else(|e| e.into_inner()).get(&key).copied().unwrap_or(0)
    }

    /// Whether a handshake from `address` would be answered `503` now.
    pub(crate) fn refuses_new_sessions_from(&self, address: &str) -> bool {
        let store = self.inner.sessions.read().unwrap_or_else(|e| e.into_inner());
        self.caps_ok(&store, &address_key(address, None)).is_err()
    }

    /// The same, as if the client connected from `address`.
    pub(crate) fn open_test_socket_from(&self, address: &str) -> EngineSocket {
        let (parts, _) = http::Request::builder().uri("/socket.io/?EIO=4&transport=websocket").body(()).unwrap().into_parts();
        let peer: std::net::SocketAddr = format!("{address}:1").parse().unwrap();
        let handshake = Request::for_upgrade(parts, peer, &Default::default()).unwrap();
        let reservation = self.reserve(&handshake).ok().expect("a free slot");
        let session = self.open_session(TransportKind::WebSocket, handshake, reservation).ok().expect("an open session");
        self.notify_connection(&session);
        EngineSocket { session }
    }
}

#[cfg(test)]
impl EngineSocket {
    /// As if the client had sent this message.
    pub(crate) fn feed(&self, message: EngineMessage) {
        self.session.on_packet(packet::Packet::Message(message));
    }

    /// The connection drops at once, as if the transport had closed under it.
    pub(crate) fn kill(&self) {
        self.session.close(session::CloseReason::TransportClose);
    }

    /// The server ends the connection at once, as a shutdown does.
    pub(crate) fn force_close(&self) {
        self.session.close(session::CloseReason::ForcedClose);
    }

    /// `true` once the session is closing or closed.
    pub(crate) fn is_closing(&self) -> bool {
        self.session.ready() != session::Ready::Open
    }

    /// The ping packets queued for the client since the last call (they are dropped by `take_sent`, so read them first).
    #[cfg(test)]
    pub(crate) fn take_pings(&self) -> usize {
        self.session
            .outbox
            .take_batch(usize::MAX, |p| p.polling_len() + 1)
            .into_iter()
            .filter(|p| matches!(p, packet::Packet::Ping(_)))
            .count()
    }

    /// Everything queued for the client since the last call, messages only (no open or ping packets).
    pub(crate) fn take_sent(&self) -> Vec<EngineMessage> {
        self.session
            .outbox
            .take_batch(usize::MAX, |p| p.polling_len() + 1)
            .into_iter()
            .filter_map(|p| match p {
                packet::Packet::Message(m) => Some(m),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::packet::Packet;
    use super::*;

    #[test]
    fn sessions_closed_by_an_overflow_are_logged_once_a_minute_with_how_many_were_left_out() {
        let server = EngineServer::new(EngineOptions::new()).unwrap();
        let start = std::time::Instant::now();
        let first = server.inner.overflow_line(1, start).expect("the first line is written");
        assert!(!first.contains("without a line"), "{first}");
        for id in 2..=1_000 {
            assert_eq!(server.inner.overflow_line(id, start + Duration::from_secs(59)), None);
        }
        let next = server.inner.overflow_line(1_001, start + Duration::from_secs(60)).expect("a minute later");
        assert!(next.contains("999 more were closed"), "{next}");
        assert_eq!(server.inner.overflow_line(1_002, start + Duration::from_secs(61)), None);
    }

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
        let reservation = server.reserve(&handshake).ok().unwrap();
        assert!(server.open_session(TransportKind::Polling, handshake.clone(), reservation).is_ok());
        let late = server.reserve(&handshake).ok().unwrap();
        server.close();
        // A reservation taken before the close does not open a session after it.
        assert!(matches!(server.open_session(TransportKind::Polling, handshake.clone(), late), Err(OpenError::Closing)));
        assert!(matches!(server.reserve(&handshake), Err(OpenError::Closing)));
        assert_eq!(server.session_count(), 0);
        assert!(server.inner.sessions.read().unwrap().per_ip.is_empty(), "the address counts are released with the sessions");
    }

    fn test_handshake() -> Request {
        let (parts, _) = http::Request::builder().uri("/socket.io/?EIO=4&transport=polling").body(()).unwrap().into_parts();
        Request::for_upgrade(parts, "127.0.0.1:1".parse().unwrap(), &Default::default()).unwrap()
    }

    fn open_socket(server: &EngineServer, transport: TransportKind) -> EngineSocket {
        let handshake = test_handshake();
        let reservation = server.reserve(&handshake).ok().unwrap();
        let session = server.open_session(transport, handshake, reservation).ok().unwrap();
        EngineSocket { session }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_group_is_queued_whole_and_never_interleaved_by_32_concurrent_senders() {
        let server = EngineServer::new(EngineOptions::new()).unwrap();
        let socket = open_socket(&server, TransportKind::WebSocket);
        let senders: Vec<_> = (0..32u8)
            .map(|n| {
                let socket = socket.clone();
                std::thread::spawn(move || {
                    for round in 0..50u8 {
                        let tag = format!("{n}:{round}");
                        let group = vec![
                            EngineMessage::Text(format!("head {tag}")),
                            EngineMessage::Binary(bytes::Bytes::from(format!("a {tag}"))),
                            EngineMessage::Binary(bytes::Bytes::from(format!("b {tag}"))),
                        ];
                        socket.send_all(group).unwrap();
                    }
                })
            })
            .collect();
        for sender in senders {
            sender.join().unwrap();
        }
        let queued = socket.session.outbox.take_batch(usize::MAX, |p| p.polling_len() + 1);
        // The open packet, then 32 * 50 groups of three.
        assert_eq!(queued.len(), 1 + 32 * 50 * 3);
        let mut groups = 0;
        for chunk in queued[1..].chunks(3) {
            let [
                Packet::Message(EngineMessage::Text(head)),
                Packet::Message(EngineMessage::Binary(a)),
                Packet::Message(EngineMessage::Binary(b)),
            ] = chunk
            else {
                panic!("a group was split: {chunk:?}")
            };
            let tag = head.strip_prefix("head ").unwrap();
            assert_eq!(a.as_ref(), format!("a {tag}").as_bytes());
            assert_eq!(b.as_ref(), format!("b {tag}").as_bytes());
            groups += 1;
        }
        assert_eq!(groups, 1600);
    }

    #[tokio::test]
    async fn a_group_over_the_send_buffer_is_refused_whole_and_closes_the_session() {
        let server = EngineServer::new(EngineOptions::new().max_buffered_amount(300)).unwrap();
        let socket = open_socket(&server, TransportKind::WebSocket);
        // The open packet is already queued; a group that does not fit is refused whole.
        let group = vec![EngineMessage::Text("x".repeat(100)), EngineMessage::Binary(bytes::Bytes::from(vec![0u8; 200]))];
        assert_eq!(socket.send_all(group), Err(SendError::Full));
        assert_eq!(socket.close_reason(), Some(CloseReason::TransportError));
        assert_eq!(socket.buffered_amount(), 0, "nothing of the group stayed queued");
        assert_eq!(socket.send_all(vec![EngineMessage::Text("late".into())]), Err(SendError::Closed));
    }

    #[tokio::test]
    async fn received_bytes_stay_charged_until_the_guard_is_released() {
        let server = EngineServer::new(EngineOptions::new()).unwrap();
        let socket = open_socket(&server, TransportKind::WebSocket);
        socket.session.on_packet(Packet::Message(EngineMessage::Text("hello".into())));
        let charged = 5 + session::UNREAD_MESSAGE_OVERHEAD;
        assert_eq!(socket.unread_bytes(), charged);
        let mut messages = socket.messages_held();
        let (message, held) = messages.next().await.unwrap();
        assert_eq!(message, EngineMessage::Text("hello".into()));
        assert_eq!(socket.unread_bytes(), charged, "taking the message does not release it");
        let extra = socket.hold(1000);
        assert_eq!(socket.unread_bytes(), charged + 1000);
        drop(held);
        assert_eq!(socket.unread_bytes(), 1000, "the gauge falls on release only");
        drop(extra);
        assert_eq!(socket.unread_bytes(), 0);
        assert_eq!(server.inner.total_unread.unread(), 0, "the server-wide gauge follows");
    }

    #[tokio::test]
    async fn the_closing_hook_runs_before_any_session_is_closed() {
        let server = EngineServer::new(EngineOptions::new()).unwrap();
        let socket = open_socket(&server, TransportKind::WebSocket);
        let seen = Arc::new(Mutex::new(Vec::new()));
        {
            let seen = seen.clone();
            let probe = socket.clone();
            server.on_closing(move || seen.lock().unwrap().push(probe.close_reason()));
        }
        server.on_closing(|| panic!("a hook that panics does not stop the close"));
        server.close();
        assert_eq!(*seen.lock().unwrap(), vec![None], "the session was still open when the hook ran");
        assert_eq!(socket.close_reason(), Some(CloseReason::ForcedClose));
    }

    #[tokio::test]
    async fn a_dropped_reservation_gives_its_slot_back() {
        let server = EngineServer::new(EngineOptions::new().max_sessions_per_ip(1)).unwrap();
        let (parts, _) = http::Request::builder().uri("/socket.io/?EIO=4&transport=polling").body(()).unwrap().into_parts();
        let handshake = Request::for_upgrade(parts, "127.0.0.1:1".parse().unwrap(), &Default::default()).unwrap();
        let held = server.reserve(&handshake).ok().unwrap();
        assert!(matches!(server.reserve(&handshake), Err(OpenError::TooManyFromAddress)));
        drop(held);
        let again = server.reserve(&handshake).ok().unwrap();
        drop(again);
        let store = server.inner.sessions.read().unwrap();
        assert!(store.pending == 0 && store.pending_ip.is_empty());
    }
}
