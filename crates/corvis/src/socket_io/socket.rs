//! Sockets: one client in one namespace.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use futures::future::BoxFuture;
use serde_json::Value;
use tokio::sync::{Notify, oneshot};
use tokio::time::Instant;

use super::client::ClientShared;
use super::engine::options::MAX_DURATION;
use super::engine::session::{CloseReason, after};
use super::namespace::IoNamespace;
use super::parser::{self, Body, EncodeError, Encoded, Packet};
use super::value::{Args, IntoArgs, IoValue, IoValueError};
use crate::http::Request;
use crate::logger::Logger;

pub(crate) type EventHandler = Arc<dyn Fn(IoSocket, Args, Option<AckSender>) -> BoxFuture<'static, ()> + Send + Sync>;
type DisconnectHandler = Arc<dyn Fn(&IoSocket, DisconnectReason) + Send + Sync>;

/// Why a socket disconnected, with the strings Node's `socket.io` reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DisconnectReason {
    /// The client left the namespace (a DISCONNECT packet).
    ClientNamespaceDisconnect,
    /// Server code called `disconnect`.
    ServerNamespaceDisconnect,
    /// The client closed the connection.
    TransportClose,
    /// The connection failed (a protocol violation at the transport, a send buffer over its limit).
    TransportError,
    /// The client stopped answering pings.
    PingTimeout,
    /// An Engine.IO packet could not be parsed.
    ParseError,
    /// A Socket.IO packet could not be decoded, or the server closed the connection.
    ForcedClose,
    /// The server closed the connection because the client broke the protocol: a second CONNECT for a namespace, a
    /// packet for a namespace it had not joined, a CONNECT_ERROR, a binary packet that took too long.
    ForcedServerClose,
    /// The server is shutting down.
    ServerShuttingDown,
}

impl DisconnectReason {
    /// The reason as Node spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            DisconnectReason::ClientNamespaceDisconnect => "client namespace disconnect",
            DisconnectReason::ServerNamespaceDisconnect => "server namespace disconnect",
            DisconnectReason::TransportClose => "transport close",
            DisconnectReason::TransportError => "transport error",
            DisconnectReason::PingTimeout => "ping timeout",
            DisconnectReason::ParseError => "parse error",
            DisconnectReason::ForcedClose => "forced close",
            DisconnectReason::ForcedServerClose => "forced server close",
            DisconnectReason::ServerShuttingDown => "server shutting down",
        }
    }
}

impl std::fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<CloseReason> for DisconnectReason {
    fn from(reason: CloseReason) -> Self {
        match reason {
            CloseReason::TransportClose => DisconnectReason::TransportClose,
            CloseReason::TransportError => DisconnectReason::TransportError,
            CloseReason::PingTimeout => DisconnectReason::PingTimeout,
            CloseReason::ParseError => DisconnectReason::ParseError,
            CloseReason::ForcedClose => DisconnectReason::ForcedClose,
        }
    }
}

/// Why an emit failed. Nothing was sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EmitError {
    /// The event name is one of `connect`, `connect_error`, `disconnect`, `disconnecting`, `newListener`,
    /// `removeListener`.
    #[error("\"{0}\" is a reserved event name")]
    ReservedEvent(String),
    /// The socket is not connected (not yet, or no longer).
    #[error("the socket is not connected")]
    NotConnected,
    /// The arguments hold more binary values than `max_attachments`.
    #[error("the packet has more attachments than max_attachments allows")]
    TooManyAttachments,
    /// The arguments are nested deeper than 127 levels.
    #[error("the arguments are nested deeper than 127 levels")]
    TooDeep,
    /// The send buffer of the connection is over its limit: the connection is closing with `transport error`.
    #[error("the send buffer is full")]
    BufferFull,
    /// The arguments could not be converted.
    #[error("{0}")]
    Value(#[from] IoValueError),
}

/// Why an acknowledged emit did not produce the client's answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AckError {
    /// The client did not answer in time.
    #[error("operation has timed out")]
    Timeout,
    /// The socket disconnected while waiting.
    #[error("the socket disconnected before the acknowledgement arrived")]
    Disconnected,
    /// The socket already waits for `max_pending_acks` answers.
    #[error("too many pending acknowledgements")]
    TooManyPending,
    /// The event could not be sent.
    #[error(transparent)]
    Emit(#[from] EmitError),
}

/// What a client sent in its CONNECT packet and how it arrived (`socket.handshake` in Node).
#[derive(Debug, Clone)]
pub struct Handshake {
    auth: Value,
    request: Request,
    issued_at: SystemTime,
}

impl Handshake {
    pub(crate) fn new(auth: Value, request: Request) -> Self {
        Handshake { auth, request, issued_at: SystemTime::now() }
    }

    /// The `auth` object of the CONNECT packet (`{}` when the client sent none).
    pub fn auth(&self) -> &Value {
        &self.auth
    }

    /// The HTTP request that opened the Engine.IO session: headers, query, URL.
    pub fn request(&self) -> &Request {
        &self.request
    }

    pub fn headers(&self) -> &http::HeaderMap {
        self.request.headers()
    }

    /// The client address (it follows `set_trust_proxy`).
    pub fn ip(&self) -> String {
        self.request.ip()
    }

    /// When the CONNECT packet was handled.
    pub fn issued_at(&self) -> SystemTime {
        self.issued_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Created, running middleware.
    Pending,
    Connected,
    /// The `disconnecting` handlers are running.
    Disconnecting,
    Disconnected,
}

struct PendingAck {
    tx: oneshot::Sender<Result<Args, AckError>>,
    deadline: Instant,
}

struct SocketState {
    phase: Phase,
    acks: HashMap<u64, PendingAck>,
}

pub(crate) struct SocketInner {
    id: String,
    nsp: IoNamespace,
    client: Arc<ClientShared>,
    handshake: Handshake,
    state: Mutex<SocketState>,
    handlers: RwLock<HashMap<String, EventHandler>>,
    disconnecting: Mutex<Vec<DisconnectHandler>>,
    disconnect: Mutex<Vec<DisconnectHandler>>,
    data: Mutex<Value>,
    extensions: Mutex<HashMap<TypeId, Box<dyn Any + Send + Sync>>>,
    /// Set once the connection handlers have run: the connection does not dispatch a packet to the socket before.
    ready: AtomicBool,
    ready_notify: Notify,
}

/// One client in one namespace (`socket` in Node). Cheap to clone; all clones are the same socket.
///
/// ```
/// # use corvis::socket_io::{IoOptions, IoServer};
/// # let io = IoServer::new(IoOptions::new()).unwrap();
/// io.main().on_connection(|socket| {
///     // `sum` with a callback: the handler answers with the total.
///     socket.on("sum", |_socket, args, ack| async move {
///         let total: i64 = args.iter().filter_map(|a| a.as_i64()).sum();
///         if let Some(ack) = ack {
///             let _ = ack.send((total, "ok"));
///         }
///     });
/// });
/// ```
#[derive(Clone)]
pub struct IoSocket {
    pub(crate) inner: Arc<SocketInner>,
}

impl std::fmt::Debug for IoSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IoSocket").field("id", &self.inner.id).field("namespace", &self.inner.nsp.name()).finish()
    }
}

impl PartialEq for IoSocket {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}
impl Eq for IoSocket {}

fn emit_error(e: EncodeError) -> EmitError {
    match e {
        EncodeError::TooManyAttachments => EmitError::TooManyAttachments,
        EncodeError::TooDeep => EmitError::TooDeep,
    }
}

impl IoSocket {
    pub(crate) fn new(nsp: IoNamespace, client: Arc<ClientShared>, handshake: Handshake) -> IoSocket {
        let mut id = super::engine::sid::generate();
        while nsp.socket(&id).is_some() {
            id = super::engine::sid::generate();
        }
        IoSocket {
            inner: Arc::new(SocketInner {
                id,
                nsp,
                client,
                handshake,
                state: Mutex::new(SocketState { phase: Phase::Pending, acks: HashMap::new() }),
                handlers: RwLock::new(HashMap::new()),
                disconnecting: Mutex::new(Vec::new()),
                disconnect: Mutex::new(Vec::new()),
                data: Mutex::new(Value::Object(serde_json::Map::new())),
                extensions: Mutex::new(HashMap::new()),
                ready: AtomicBool::new(false),
                ready_notify: Notify::new(),
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, SocketState> {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The socket id: 20 characters, 120 random bits, unrelated to the Engine.IO session id (which is a credential
    /// and never appears in a Socket.IO packet).
    pub fn id(&self) -> &str {
        &self.inner.id
    }

    /// The namespace the socket belongs to.
    pub fn namespace(&self) -> IoNamespace {
        self.inner.nsp.clone()
    }

    /// `socket.handshake`: the client's `auth` and the request that opened the connection.
    pub fn handshake(&self) -> &Handshake {
        &self.inner.handshake
    }

    /// The client address.
    pub fn ip(&self) -> String {
        self.inner.handshake.ip()
    }

    /// `socket.connected`.
    pub fn connected(&self) -> bool {
        self.state().phase == Phase::Connected
    }

    /// `socket.data`: free-form data of the application (an empty object at first).
    pub fn data(&self) -> Value {
        self.inner.data.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_data(&self, data: Value) {
        *self.inner.data.lock().unwrap_or_else(|e| e.into_inner()) = data;
    }

    /// Attaches a typed value to the socket, replacing one of the same type (middleware stores the authenticated
    /// user here).
    pub fn insert<T: Clone + Send + Sync + 'static>(&self, value: T) {
        self.inner.extensions.lock().unwrap_or_else(|e| e.into_inner()).insert(TypeId::of::<T>(), Box::new(value));
    }

    /// The value of type `T` attached with [`insert`](Self::insert).
    pub fn get<T: Clone + Send + Sync + 'static>(&self) -> Option<T> {
        self.inner.extensions.lock().unwrap_or_else(|e| e.into_inner()).get(&TypeId::of::<T>()).and_then(|v| v.downcast_ref::<T>()).cloned()
    }

    /// `socket.on(event, handler)`: registers the handler of an event, replacing the one registered before for the
    /// same name. The handler gets the socket, the arguments and, when the client asked for an acknowledgement, an
    /// [`AckSender`] that answers once. Handlers of one connection start in the order their packets arrived and run
    /// concurrently, at most [`IoOptions::max_inflight`](super::IoOptions::max_inflight) at a time; a panic is logged
    /// and the socket stays connected. An event without a handler is ignored. The reserved names never fire.
    pub fn on<F, Fut>(&self, event: impl Into<String>, handler: F) -> &Self
    where
        F: Fn(IoSocket, Args, Option<AckSender>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handler: EventHandler = Arc::new(move |socket, args, ack| Box::pin(handler(socket, args, ack)));
        self.inner.handlers.write().unwrap_or_else(|e| e.into_inner()).insert(event.into(), handler);
        self
    }

    /// `socket.off(event)`: removes the handler of an event.
    pub fn off(&self, event: &str) -> &Self {
        self.inner.handlers.write().unwrap_or_else(|e| e.into_inner()).remove(event);
        self
    }

    /// Adds a function that runs when the socket starts to disconnect, while it is still in its namespace (and, with
    /// rooms, in its rooms). Synchronous; a panic is logged.
    pub fn on_disconnecting(&self, f: impl Fn(&IoSocket, DisconnectReason) + Send + Sync + 'static) -> &Self {
        self.inner.disconnecting.lock().unwrap_or_else(|e| e.into_inner()).push(Arc::new(f));
        self
    }

    /// Adds a function that runs once the socket left its namespace. Synchronous; a panic is logged. Each socket
    /// reports each of the two exactly once.
    pub fn on_disconnect(&self, f: impl Fn(&IoSocket, DisconnectReason) + Send + Sync + 'static) -> &Self {
        self.inner.disconnect.lock().unwrap_or_else(|e| e.into_inner()).push(Arc::new(f));
        self
    }

    pub(crate) fn handler(&self, event: &str) -> Option<EventHandler> {
        self.inner.handlers.read().unwrap_or_else(|e| e.into_inner()).get(event).cloned()
    }

    // ----------------------------------------------------------------------------------------------- emitting

    fn check_emittable(&self, event: &str) -> Result<(), EmitError> {
        if parser::is_reserved(event) {
            return Err(EmitError::ReservedEvent(event.to_string()));
        }
        if self.state().phase != Phase::Connected {
            return Err(EmitError::NotConnected);
        }
        Ok(())
    }

    fn send(&self, packet: &Packet) -> Result<(), EmitError> {
        let encoded = parser::encode(packet, self.inner.nsp.options().max_attachments).map_err(emit_error)?;
        self.inner.client.send_encoded(&encoded)
    }

    /// Sends an already encoded packet (one encoding shared by many sockets).
    #[allow(dead_code)]
    pub(crate) fn send_encoded(&self, encoded: &Encoded) -> Result<(), EmitError> {
        if self.state().phase != Phase::Connected {
            return Err(EmitError::NotConnected);
        }
        self.inner.client.send_encoded(encoded)
    }

    /// `socket.emit(event, ...args)`: sends an event to the client. The arguments are `()`, a tuple of values that
    /// implement `Serialize`, or anything [`IntoArgs`] accepts; binary values anywhere in them become attachments.
    /// Fails, sending nothing, for a reserved event name, a socket that is not connected, more attachments than
    /// `max_attachments`, or a send buffer that is full.
    pub fn emit(&self, event: &str, args: impl IntoArgs) -> Result<(), EmitError> {
        self.check_emittable(event)?;
        let args = args.into_args()?;
        self.send(&Packet { nsp: self.inner.nsp.name().to_string(), body: Body::Event { id: None, name: event.to_string(), args } })
    }

    /// `socket.emit(event, ...args, callback)`: sends an event and waits for the client's answer.
    ///
    /// The call has a deadline, [`IoOptions::ack_timeout`](super::IoOptions::ack_timeout) unless
    /// [`timeout`](Self::timeout) sets one, after which it fails with `operation has timed out` and frees its
    /// slot; a socket waits for at most `max_pending_acks` answers at once, and a call over that fails at once
    /// without sending; when the socket disconnects its pending calls fail at once.
    ///
    /// ```
    /// # use corvis::socket_io::{IoSocket, AckError};
    /// # async fn ask(socket: IoSocket) -> Result<(), AckError> {
    /// let answer = socket.emit_with_ack("ask", ("question",)).await?;
    /// let yes: String = answer.get(0).map_err(|_| AckError::Disconnected)?;
    /// # let _ = yes; Ok(()) }
    /// ```
    pub async fn emit_with_ack(&self, event: &str, args: impl IntoArgs) -> Result<Args, AckError> {
        self.emit_with_ack_within(event, args, self.inner.nsp.options().ack_timeout).await
    }

    /// `socket.timeout(duration)`: the deadline of one acknowledged emit.
    pub fn timeout(&self, timeout: Duration) -> AckEmitter {
        AckEmitter { socket: self.clone(), timeout }
    }

    async fn emit_with_ack_within(&self, event: &str, args: impl IntoArgs, timeout: Duration) -> Result<Args, AckError> {
        self.check_emittable(event)?;
        let args = args.into_args().map_err(EmitError::from)?;
        let timeout = timeout.min(MAX_DURATION);
        let (tx, rx) = oneshot::channel();
        let now = Instant::now();
        let id = {
            let mut state = self.state();
            if state.phase != Phase::Connected {
                return Err(EmitError::NotConnected.into());
            }
            // An entry whose future was abandoned without being polled again still expires.
            state.acks.retain(|_, a| a.deadline > now);
            if state.acks.len() >= self.inner.nsp.options().max_pending_acks {
                return Err(AckError::TooManyPending);
            }
            let id = self.inner.nsp.next_ack_id();
            state.acks.insert(id, PendingAck { tx, deadline: after(now, timeout) });
            id
        };
        let _slot = AckSlot { socket: self.clone(), id };
        self.send(&Packet { nsp: self.inner.nsp.name().to_string(), body: Body::Event { id: Some(id), name: event.to_string(), args } })?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(AckError::Disconnected),
            Err(_) => Err(AckError::Timeout),
        }
    }

    /// The number of acknowledged emits still waiting for the client.
    pub fn pending_acks(&self) -> usize {
        self.state().acks.len()
    }

    /// A client acknowledgement for an id this socket sent; an id that is not pending (never issued, already
    /// answered, expired, issued to another socket) is ignored.
    pub(crate) fn resolve_ack(&self, id: u64, args: Vec<IoValue>) {
        let entry = self.state().acks.remove(&id);
        if let Some(entry) = entry {
            let _ = entry.tx.send(Ok(Args::new(args)));
        }
    }

    // --------------------------------------------------------------------------------------------- disconnecting

    /// `socket.disconnect(close)`: with `false`, leaves this namespace: a DISCONNECT packet is sent and the reason is
    /// `server namespace disconnect`. With `true`, does that for every socket of the connection and closes the
    /// connection. Does nothing on a socket that is not connected.
    pub fn disconnect(&self, close: bool) {
        if close {
            self.inner.client.terminate(DisconnectReason::ServerNamespaceDisconnect, true, true);
        } else {
            self.disconnect_with(DisconnectReason::ServerNamespaceDisconnect, true);
        }
    }

    /// Moves the socket to connected: called with the connection's namespace table locked, after CONNECT is queued.
    pub(crate) fn mark_connected(&self) {
        let mut state = self.state();
        if state.phase == Phase::Pending {
            state.phase = Phase::Connected;
        }
    }

    pub(crate) fn mark_ready(&self) {
        self.inner.ready.store(true, Ordering::SeqCst);
        self.inner.ready_notify.notify_waiters();
    }

    pub(crate) async fn wait_ready(&self) {
        loop {
            let notified = self.inner.ready_notify.notified();
            if self.inner.ready.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }

    pub(crate) fn is_phase(&self, phase: Phase) -> bool {
        self.state().phase == phase
    }

    /// The whole disconnect sequence, once: `disconnecting` handlers (still in the namespace), removal, pending
    /// acknowledgements failed, `disconnect` handlers.
    pub(crate) fn disconnect_with(&self, reason: DisconnectReason, send_packet: bool) {
        {
            let mut state = self.state();
            if state.phase != Phase::Connected {
                return;
            }
            state.phase = Phase::Disconnecting;
        }
        if send_packet {
            let _ = self.send(&Packet { nsp: self.inner.nsp.name().to_string(), body: Body::Disconnect });
        }
        let handlers: Vec<DisconnectHandler> = std::mem::take(&mut *self.inner.disconnecting.lock().unwrap_or_else(|e| e.into_inner()));
        for h in handlers {
            crate::ws::guarded("a Socket.IO disconnecting handler", || h(self, reason));
        }
        self.inner.nsp.remove_socket(&self.inner.id);
        self.inner.client.forget(self.inner.nsp.name(), self);
        let acks = {
            let mut state = self.state();
            state.phase = Phase::Disconnected;
            std::mem::take(&mut state.acks)
        };
        for (_, ack) in acks {
            let _ = ack.tx.send(Err(AckError::Disconnected));
        }
        let handlers: Vec<DisconnectHandler> = std::mem::take(&mut *self.inner.disconnect.lock().unwrap_or_else(|e| e.into_inner()));
        for h in handlers {
            crate::ws::guarded("a Socket.IO disconnect handler", || h(self, reason));
        }
        // The handlers may hold clones of the socket: drop them so nothing keeps the socket alive.
        self.inner.handlers.write().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// Removes a pending acknowledgement when its call ends, however it ends.
struct AckSlot {
    socket: IoSocket,
    id: u64,
}

impl Drop for AckSlot {
    fn drop(&mut self) {
        self.socket.state().acks.remove(&self.id);
    }
}

/// `socket.timeout(duration)`: an acknowledged emit with its own deadline.
#[derive(Debug, Clone)]
pub struct AckEmitter {
    socket: IoSocket,
    timeout: Duration,
}

impl AckEmitter {
    /// Like [`IoSocket::emit_with_ack`], failing with `operation has timed out` after the duration given to
    /// `timeout`.
    pub async fn emit_with_ack(&self, event: &str, args: impl IntoArgs) -> Result<Args, AckError> {
        self.socket.emit_with_ack_within(event, args, self.timeout).await
    }
}

/// The right to answer one event that the client sent with an acknowledgement callback. Answering consumes it, so an
/// event is acknowledged at most once; dropping it unanswered sends nothing, as an event handler that never calls the
/// callback in Node.
pub struct AckSender {
    socket: IoSocket,
    id: u64,
}

impl std::fmt::Debug for AckSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AckSender").field("id", &self.id).finish()
    }
}

impl AckSender {
    pub(crate) fn new(socket: IoSocket, id: u64) -> AckSender {
        AckSender { socket, id }
    }

    /// Sends the acknowledgement with these values (binary values become attachments). Fails when the socket is no
    /// longer connected or the values cannot be sent.
    pub fn send(self, args: impl IntoArgs) -> Result<(), EmitError> {
        if self.socket.state().phase != Phase::Connected {
            return Err(EmitError::NotConnected);
        }
        let args = args.into_args()?;
        self.socket.send(&Packet { nsp: self.socket.inner.nsp.name().to_string(), body: Body::Ack { id: self.id, args } })
    }
}

/// Logs a panic of a handler without the client's data: only the namespace, which the application registered.
pub(crate) fn log_panic(what: &str, namespace: &str) {
    Logger::new("SocketIO").error(format!("{what} of the namespace {namespace} panicked"));
}
