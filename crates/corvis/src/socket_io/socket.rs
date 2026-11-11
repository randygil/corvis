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

use super::adapter::{AckSink, AdapterError, HandshakeInfo, Room, RoomError, SocketInfo};
use super::broadcast::{BroadcastOperator, IntoRooms};
use super::client::{Charge, ClientShared};
use super::engine::Held;
use super::engine::options::MAX_DURATION;
use super::engine::session::{CloseReason, after};
use super::namespace::IoNamespace;
use super::parser::{self, Body, EncodeError, Encoded, Packet, SharedEncoded};
use super::value::{Args, IntoArgs, IoValue, IoValueError};
use crate::http::Request;

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
    /// A packet of the client waited longer than `admission_timeout` for room in the unread budget, so the server
    /// closed the connection: the budget stayed exhausted by other connections. Not a reason Node has.
    AdmissionTimeout,
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
            DisconnectReason::AdmissionTimeout => "admission timeout",
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
    /// A room named in `to`, `within` or `except` (or given to `sockets_join` / `sockets_leave`) is not valid: nothing
    /// was sent or changed, and the name was not dropped, which could have widened the broadcast.
    #[error("invalid room: {0}")]
    InvalidRoom(#[from] RoomError),
    /// The adapter failed.
    #[error("{0}")]
    Adapter(AdapterError),
}

impl From<AdapterError> for EmitError {
    fn from(error: AdapterError) -> Self {
        match error {
            AdapterError::InvalidPacket(super::adapter::PacketError::TooManyAttachments) => EmitError::TooManyAttachments,
            AdapterError::InvalidPacket(super::adapter::PacketError::TooDeep) => EmitError::TooDeep,
            other => EmitError::Adapter(other),
        }
    }
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
    data: Arc<HandshakeData>,
}

#[derive(Debug)]
struct HandshakeData {
    auth: Value,
    request: Request,
    issued_at: SystemTime,
    /// What `auth` occupies, charged to the connection's session and address for as long as the handshake lives (the
    /// socket's life, or that of a `fetch_sockets` result that shares it): `auth` is a `serde_json::Value`, whose
    /// size is not the one `max_auth_bytes` measures.
    _charge: Option<Held>,
}

impl Handshake {
    pub(crate) fn new(auth: Value, request: Request, charge: Option<Held>) -> Self {
        Handshake { data: Arc::new(HandshakeData { auth, request, issued_at: SystemTime::now(), _charge: charge }) }
    }

    /// The `auth` object of the CONNECT packet (`{}` when the client sent none).
    pub fn auth(&self) -> &Value {
        &self.data.auth
    }

    /// The HTTP request that opened the Engine.IO session: headers, query, URL.
    pub fn request(&self) -> &Request {
        &self.data.request
    }

    pub fn headers(&self) -> &http::HeaderMap {
        self.data.request.headers()
    }

    /// The client address (it follows `set_trust_proxy`).
    pub fn ip(&self) -> String {
        self.data.request.ip()
    }

    /// When the CONNECT packet was handled.
    pub fn issued_at(&self) -> SystemTime {
        self.data.issued_at
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

/// Who is waiting for an acknowledgement: one call of `emit_with_ack`, or the collection of a broadcast.
enum AckTarget {
    One(oneshot::Sender<Result<Args, AckError>>),
    Broadcast(AckSink),
}

struct PendingAck {
    target: AckTarget,
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
    /// `None` when the adapter panicked while the id was checked: the connection attempt is refused.
    pub(crate) fn new(nsp: IoNamespace, client: Arc<ClientShared>, handshake: Handshake) -> Option<IoSocket> {
        let mut drawn = None;
        // Never an id that is already a room: a room created in advance cannot capture a future socket. The adapter
        // is application code: a panic in it is contained and refuses the attempt.
        client.warnings.guard("an adapter's room_exists", nsp.name(), || {
            let mut id = super::engine::sid::generate();
            while nsp.socket(&id).is_some() || nsp.adapter().room_exists(&id) {
                id = super::engine::sid::generate();
            }
            drawn = Some(id);
        });
        let id = drawn?;
        Some(IoSocket {
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
        })
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
    ///
    /// Do not `unwrap()` or `expect()` anything derived from the client's data: containing the panic does not silence
    /// the standard library's default panic hook, which writes the message (client data included, if the handler
    /// formatted it) to standard error every time, at the rate the client sends. An application that wants silence
    /// installs its own hook with `std::panic::set_hook`; the framework installs none.
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

    /// Queues an encoding shared by the recipients of a broadcast. The phase is checked first, so a socket that
    /// disconnected is never sent a packet.
    pub(crate) fn send_shared(&self, encoded: &SharedEncoded) -> Result<(), EmitError> {
        if self.state().phase != Phase::Connected {
            return Err(EmitError::NotConnected);
        }
        self.inner.client.send_shared(encoded)
    }

    pub(crate) fn downgrade(&self) -> std::sync::Weak<SocketInner> {
        Arc::downgrade(&self.inner)
    }

    /// Connected, or disconnecting with its `disconnecting` handlers still running: the sockets that may be in rooms.
    pub(crate) fn is_live(&self) -> bool {
        matches!(self.state().phase, Phase::Connected | Phase::Disconnecting)
    }

    /// A rate limited warning of the connection's server.
    pub(crate) fn warn(&self, kind: &'static str, message: &str) {
        self.inner.client.warnings.warn(kind, self.inner.nsp.name(), message);
    }

    /// The socket as plain data (what `fetch_sockets` returns).
    pub(crate) fn info(&self, rooms: Vec<Room>) -> SocketInfo {
        SocketInfo { id: self.inner.id.clone(), handshake: HandshakeInfo::local(self.inner.handshake.clone()), rooms, data: self.data() }
    }

    // ------------------------------------------------------------------------------------------------- rooms

    /// The room named after the socket's id, joined when the socket connects.
    /// `false` when the adapter panicked (contained and logged): the socket is then in no room and must not be kept.
    pub(crate) fn join_own_room(&self) -> bool {
        let own: Room = Arc::from(self.inner.id.as_str());
        self.inner.client.warnings.guard("an adapter's add_all", self.inner.nsp.name(), || {
            let _ = self.inner.nsp.adapter().add_all(&self.inner.id, &[own]);
        })
    }

    /// `socket.join(room)`: joins one room or several (a `&str`, a `String`, a `Vec` or an array of names). Every
    /// name is checked before anything changes: a name longer than
    /// [`max_room_name_length`](super::IoOptions::max_room_name_length) fails with `RoomError::NameTooLong`, and
    /// joining more rooms than [`max_rooms_per_socket`](super::IoOptions::max_rooms_per_socket) (the room of the
    /// socket's own id counts) fails with `RoomError::TooManyRooms`, in both cases joining none. Node accepts both.
    /// A socket that is not connected, or has disconnected, joins nothing and the call succeeds, as in Node. A call
    /// naming more than `max_rooms_per_socket` rooms is refused without looking at them, and a room named like the id
    /// of another socket of the namespace fails with `RoomError::SocketIdRoom` (Node allows it), joining none.
    ///
    /// ```
    /// # use corvis::socket_io::{IoOptions, IoServer};
    /// # let io = IoServer::new(IoOptions::new()).unwrap();
    /// io.main().on_connection(|socket| {
    ///     let _ = socket.join(["news", "sports"]);
    ///     let _ = socket.join("chat");
    /// });
    /// ```
    pub fn join(&self, rooms: impl IntoRooms) -> Result<(), RoomError> {
        let rooms = rooms.into_rooms();
        let options = self.inner.nsp.options();
        // The input is bounded before anything looks at it: more names than a socket may hold is refused at once.
        if rooms.len() > options.max_rooms_per_socket {
            return Err(RoomError::TooManyRooms { max: options.max_rooms_per_socket });
        }
        for room in &rooms {
            if room.len() > options.max_room_name_length {
                return Err(RoomError::NameTooLong { len: room.len(), max: options.max_room_name_length });
            }
        }
        self.inner.nsp.adapter().add_all(&self.inner.id, &rooms)
    }

    /// `socket.leave(room)`: leaves a room at once; a room that was not joined is ignored. A socket cannot leave the
    /// room named like its own id (Node lets it): that room is how the socket is addressed.
    pub fn leave(&self, room: &str) {
        self.inner.nsp.adapter().del(&self.inner.id, room);
    }

    /// `socket.rooms`: the rooms the socket is in (its own id's included), none once it has disconnected. While the
    /// `disconnecting` handlers run it still holds them.
    pub fn rooms(&self) -> std::collections::HashSet<String> {
        self.inner.nsp.adapter().socket_rooms(&self.inner.id).iter().map(|r| r.to_string()).collect()
    }

    /// `socket.broadcast`: an operator that reaches every socket of the namespace except this one.
    pub fn broadcast(&self) -> BroadcastOperator {
        BroadcastOperator::new(self.namespace()).except_ids_unchecked(&self.inner.id)
    }

    /// `socket.to(rooms)`: the sockets in these rooms except this one.
    pub fn to(&self, rooms: impl IntoRooms) -> BroadcastOperator {
        self.broadcast().to(rooms)
    }

    /// `socket.in(rooms)` (`in` is a keyword in Rust): the same as [`to`](Self::to).
    pub fn within(&self, rooms: impl IntoRooms) -> BroadcastOperator {
        self.broadcast().within(rooms)
    }

    /// `socket.except(rooms)`: every socket except these rooms' and this one.
    pub fn except(&self, rooms: impl IntoRooms) -> BroadcastOperator {
        self.broadcast().except(rooms)
    }

    /// `socket.local`: a broadcast that stays on this server.
    pub fn local(&self) -> BroadcastOperator {
        self.broadcast().local()
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
            state.acks.insert(id, PendingAck { target: AckTarget::One(tx), deadline: after(now, timeout) });
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

    /// The bytes queued for this socket's connection and not yet written. Unstable and hidden: for the tests that
    /// check what a broadcast holds.
    #[doc(hidden)]
    pub fn buffered_amount(&self) -> usize {
        self.inner.client.engine.buffered_amount()
    }

    /// The number of acknowledged emits still waiting for the client.
    pub fn pending_acks(&self) -> usize {
        self.state().acks.len()
    }

    /// A client acknowledgement for an id this socket sent; an id that is not pending (never issued, already
    /// answered, expired, issued to another socket) is ignored.
    ///
    /// `charge` is what the answer occupies in the unread budget of this connection. It is released as soon as the
    /// collector of a broadcast has taken the answer (what the call keeps is charged to the server-wide answer budget
    /// instead), and dropped here for any other answer.
    pub(crate) fn resolve_ack(&self, id: u64, args: Vec<IoValue>, charge: Charge) {
        let entry = self.state().acks.remove(&id);
        match entry {
            Some(PendingAck { target: AckTarget::One(tx), .. }) => {
                drop(charge);
                let _ = tx.send(Ok(Args::new(args)));
            }
            Some(PendingAck { target: AckTarget::Broadcast(sink), .. }) => sink.respond_charged(Args::new(args), charge),
            None => drop(charge),
        }
    }

    /// Registers this socket as a recipient of the broadcast with acknowledgements `id`: its answer goes to `sink`.
    /// `false` when the socket is not connected or already waits for `max_pending_acks` answers (it then gets the
    /// event and its answer is not awaited).
    pub(crate) fn register_broadcast_ack(&self, id: u64, sink: AckSink, deadline: Instant) -> bool {
        let now = Instant::now();
        let mut state = self.state();
        if state.phase != Phase::Connected {
            return false;
        }
        state.acks.retain(|_, a| a.deadline > now);
        if state.acks.len() >= self.inner.nsp.options().max_pending_acks {
            return false;
        }
        state.acks.insert(id, PendingAck { target: AckTarget::Broadcast(sink), deadline });
        true
    }

    /// Forgets the acknowledgement `id` without answering it.
    pub(crate) fn cancel_ack(&self, id: u64) {
        self.state().acks.remove(&id);
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
            self.inner.client.warnings.guard("a disconnecting handler", self.inner.nsp.name(), || h(self, reason));
        }
        // Marked disconnected first, then out of every room: a join that ran before the mark is removed here, one that
        // runs after it is refused under the adapter's own lock, so no membership of this socket survives.
        let acks = {
            let mut state = self.state();
            state.phase = Phase::Disconnected;
            std::mem::take(&mut state.acks)
        };
        // A panic in the adapter must not leave the socket in the namespace with its charges: it is contained, logged,
        // and the socket is removed all the same.
        let adapter = self.inner.nsp.adapter();
        self.inner.client.warnings.guard("an adapter's del_all", self.inner.nsp.name(), || adapter.del_all(&self.inner.id));
        self.inner.nsp.remove_socket(&self.inner.id);
        self.inner.client.forget(self.inner.nsp.name(), self);
        for (_, ack) in acks {
            if let AckTarget::One(tx) = ack.target {
                let _ = tx.send(Err(AckError::Disconnected));
            }
        }
        let handlers: Vec<DisconnectHandler> = std::mem::take(&mut *self.inner.disconnect.lock().unwrap_or_else(|e| e.into_inner()));
        for h in handlers {
            self.inner.client.warnings.guard("a disconnect handler", self.inner.nsp.name(), || h(self, reason));
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
