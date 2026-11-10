//! The adapter: where the rooms of a namespace live and how a broadcast reaches its sockets (`socket.io-adapter`).
//!
//! **Unstable.** The trait and [`InMemoryAdapter`] are hidden from the generated documentation and may change until
//! the Redis adapter (#36) has implemented them once; the public operators (`to`, `except`, `emit`, ...) do not
//! expose them. Each namespace owns one adapter, created by the factory of [`IoOptions::adapter`](super::IoOptions).
//!
//! The shape follows what a cluster adapter needs:
//!
//! - Membership of the sockets connected to **this** process is synchronous and local ([`add_all`](Adapter::add_all),
//!   [`del`](Adapter::del), [`del_all`](Adapter::del_all), [`socket_rooms`](Adapter::socket_rooms)), as in Node,
//!   where `socket.join` returns at once even with the Redis adapter: a socket lives on one node and that node owns its
//!   rooms.
//! - Everything that spans the cluster is `async` and returns a `Result`: a remote adapter publishes, waits for
//!   other nodes and can fail or time out. Nothing in a signature needs a borrowed socket across an await, and
//!   nothing assumes a broadcast completes before the call returns.
//! - A broadcast receives the [`Packet`] before it is encoded, with [`BroadcastOptions`] (`rooms`, `except`,
//!   `flags`, the three fields of the Redis adapter's message).
//! - A broadcast with acknowledgements reports through an [`AckSink`]: `expect(clients)` once per server and
//!   `respond(args)` per answer. The caller owns the deadline, the completeness rule and the byte cap of the answers.
//!
//! The memory a broadcast holds while it is delivered is one shared encoding plus, per recipient, one reference to
//! it and one queue entry; see [`InMemoryAdapter`].

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio::time::Instant;

use super::client::Charge;
use super::engine::session::after;
use super::namespace::NamespaceInner;
use super::options::IoOptions;
use super::parser::{self, Body, EncodeError};

pub use super::parser::Packet;
use super::socket::{Handshake, IoSocket, SocketInner};
use super::value::{Args, IoValue};

/// A room name. Cheap to clone.
pub type Room = Arc<str>;
/// A socket id. Cheap to clone.
pub type SocketId = Arc<str>;

/// How many recipients a broadcast delivers before it yields to the runtime, so a very large fan-out does not hold
/// its worker thread for the whole delivery.
const YIELD_EVERY: usize = 512;

/// Why a join was refused. Nothing was joined.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RoomError {
    /// A room name is longer than `max_room_name_length`.
    #[error("a room name is {len} bytes long; the most is {max}")]
    NameTooLong { len: usize, max: usize },
    /// The socket would be in more than `max_rooms_per_socket` rooms.
    #[error("a socket may be in at most {max} rooms")]
    TooManyRooms { max: usize },
    /// The room is named like the id of another connected socket of the namespace: a client that joined it would
    /// receive what is sent to that socket. Node allows it; Corvis refuses it.
    #[error("a room cannot be named like the id of another socket")]
    SocketIdRoom,
    /// A broadcast operator names more rooms (`to`, `within`, `except`, counted together) than
    /// `max_rooms_per_socket` allows.
    #[error("a broadcast operator may name at most {max} rooms")]
    TooManyTargets { max: usize },
}

/// Why a packet could not be encoded for a broadcast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PacketError {
    #[error("the packet has more attachments than max_attachments allows")]
    TooManyAttachments,
    #[error("the arguments are nested deeper than 127 levels")]
    TooDeep,
}

impl From<EncodeError> for PacketError {
    fn from(e: EncodeError) -> Self {
        match e {
            EncodeError::TooManyAttachments => PacketError::TooManyAttachments,
            EncodeError::TooDeep => PacketError::TooDeep,
        }
    }
}

/// Why an operation of the adapter failed. A recipient that cannot take a packet is not an adapter error: that
/// recipient is closed and the broadcast goes on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdapterError {
    /// The adapter did not get an answer in time.
    #[error("the adapter timed out")]
    Timeout,
    /// The adapter is closed.
    #[error("the adapter is closed")]
    Closed,
    /// The backend of the adapter failed or cannot be reached.
    #[error("the adapter is unavailable: {0}")]
    Unavailable(String),
    /// The packet cannot be encoded.
    #[error(transparent)]
    InvalidPacket(#[from] PacketError),
}

/// The flags of a broadcast (`BroadcastFlags` of `socket.io-adapter`). More may be added: `volatile`, `compress`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct BroadcastFlags {
    /// Reach only the sockets connected to this process.
    pub local: bool,
    /// The deadline of a broadcast with acknowledgements.
    pub timeout: Option<Duration>,
}

/// Which sockets a broadcast reaches: the union of `rooms` (every socket of the namespace when empty) without the
/// members of the `except` rooms (`BroadcastOptions` of `socket.io-adapter`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct BroadcastOptions {
    pub rooms: HashSet<Room>,
    pub except: HashSet<Room>,
    pub flags: BroadcastFlags,
    /// Reach exactly this socket: it is the only candidate, `rooms` are ignored and `except` still applies. How a
    /// [`RemoteSocket`](super::RemoteSocket) of a socket held by another process is addressed: by its id, never as
    /// "the members of the room named like the id", which a client may have joined.
    pub socket: Option<SocketId>,
}

impl BroadcastOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rooms(mut self, rooms: impl IntoIterator<Item = Room>) -> Self {
        self.rooms.extend(rooms);
        self
    }

    pub fn except(mut self, rooms: impl IntoIterator<Item = Room>) -> Self {
        self.except.extend(rooms);
        self
    }

    pub fn flags(mut self, flags: BroadcastFlags) -> Self {
        self.flags = flags;
        self
    }

    pub fn socket(mut self, id: SocketId) -> Self {
        self.socket = Some(id);
        self
    }
}

/// A handshake as plain data, the form that crosses processes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PlainHandshake {
    auth: serde_json::Value,
    headers: Vec<(String, String)>,
    url: String,
    address: String,
    issued_at: u64,
}

#[derive(Clone)]
enum HandshakeRepr {
    /// A socket of this process: shares its handshake, copies nothing.
    Local(Handshake),
    /// A socket described by another process.
    Plain(Box<PlainHandshake>),
}

/// What a client sent when it connected (`socket.handshake`). For a socket of this process it shares the socket's own
/// handshake instead of copying it (a cookie-heavy handshake is hundreds of kilobytes); it serializes as plain data
/// so another process can describe a socket it does not hold.
///
/// `Debug` hides credentials: it prints the names of the headers and of the `auth` keys, never their values, and the
/// URL without its query.
#[derive(Clone)]
pub struct HandshakeInfo {
    repr: HandshakeRepr,
}

/// The headers whose values `Debug` never prints.
const SECRET_HEADERS: [&str; 3] = ["cookie", "authorization", "proxy-authorization"];

impl HandshakeInfo {
    /// A handshake described by plain data (what a remote adapter builds from what another process sent).
    pub fn new(auth: serde_json::Value, headers: Vec<(String, String)>, url: String, address: String, issued_at: u64) -> Self {
        HandshakeInfo { repr: HandshakeRepr::Plain(Box::new(PlainHandshake { auth, headers, url, address, issued_at })) }
    }

    pub(crate) fn local(handshake: Handshake) -> Self {
        HandshakeInfo { repr: HandshakeRepr::Local(handshake) }
    }

    /// The `auth` object of the CONNECT packet.
    pub fn auth(&self) -> &serde_json::Value {
        match &self.repr {
            HandshakeRepr::Local(h) => h.auth(),
            HandshakeRepr::Plain(p) => &p.auth,
        }
    }

    /// The headers of the request that opened the connection (those that are valid text), in order.
    pub fn headers(&self) -> Vec<(&str, &str)> {
        match &self.repr {
            HandshakeRepr::Local(h) => h.headers().iter().filter_map(|(k, v)| Some((k.as_str(), v.to_str().ok()?))).collect(),
            HandshakeRepr::Plain(p) => p.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect(),
        }
    }

    /// The URL of the request, query included.
    pub fn url(&self) -> String {
        match &self.repr {
            HandshakeRepr::Local(h) => h.request().url(),
            HandshakeRepr::Plain(p) => p.url.clone(),
        }
    }

    /// The client address.
    pub fn address(&self) -> String {
        match &self.repr {
            HandshakeRepr::Local(h) => h.ip(),
            HandshakeRepr::Plain(p) => p.address.clone(),
        }
    }

    /// Milliseconds since the Unix epoch.
    pub fn issued_at(&self) -> u64 {
        match &self.repr {
            HandshakeRepr::Local(h) => h.issued_at().duration_since(std::time::SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
            HandshakeRepr::Plain(p) => p.issued_at,
        }
    }

    fn to_plain(&self) -> PlainHandshake {
        PlainHandshake {
            auth: self.auth().clone(),
            headers: self.headers().into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            url: self.url(),
            address: self.address(),
            issued_at: self.issued_at(),
        }
    }
}

impl PartialEq for HandshakeInfo {
    fn eq(&self, other: &Self) -> bool {
        self.to_plain() == other.to_plain()
    }
}

impl Serialize for HandshakeInfo {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_plain().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for HandshakeInfo {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(HandshakeInfo { repr: HandshakeRepr::Plain(Box::new(PlainHandshake::deserialize(deserializer)?)) })
    }
}

impl std::fmt::Debug for HandshakeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let auth_keys: Vec<&str> = match self.auth() {
            serde_json::Value::Object(map) => map.keys().map(String::as_str).collect(),
            _ => Vec::new(),
        };
        let headers: Vec<String> = self
            .headers()
            .into_iter()
            .map(|(name, value)| if SECRET_HEADERS.contains(&name.to_ascii_lowercase().as_str()) { format!("{name}: <hidden>") } else { format!("{name}: {value}") })
            .collect();
        let url = self.url();
        let url = match url.split_once('?') {
            Some((path, _)) => format!("{path}?<hidden>"),
            None => url,
        };
        f.debug_struct("HandshakeInfo")
            .field("auth_keys", &auth_keys)
            .field("headers", &headers)
            .field("url", &url)
            .field("address", &self.address())
            .field("issued_at", &self.issued_at())
            .finish()
    }
}

/// Rooms as a plain list of names on the wire; in memory each is a shared string.
mod rooms_serde {
    use super::Room;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(rooms: &[Room], serializer: S) -> Result<S::Ok, S::Error> {
        let names: Vec<&str> = rooms.iter().map(|r| &**r).collect();
        names.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Room>, D::Error> {
        let names = Vec::<String>::deserialize(deserializer)?;
        Ok(names.into_iter().map(|n| Room::from(n.as_str())).collect())
    }
}

/// A socket described by data: what `fetch_sockets` returns, whichever process holds the socket. For a socket of this
/// process nothing big is copied: the handshake is shared, and the rooms are shared names (16 bytes each, at most
/// `max_rooms_per_socket`). `data` is a copy of the application's own `socket.data`.
///
/// `Debug` hides credentials (see [`HandshakeInfo`]).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SocketInfo {
    pub id: String,
    pub handshake: HandshakeInfo,
    #[serde(with = "rooms_serde")]
    pub rooms: Vec<Room>,
    pub data: serde_json::Value,
}

impl std::fmt::Debug for SocketInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SocketInfo").field("id", &self.id).field("handshake", &self.handshake).field("rooms", &self.rooms.len()).finish_non_exhaustive()
    }
}

// ----------------------------------------------------------------------------------------- the local namespace

/// What an adapter may reach of its namespace: the sockets connected to this process. It does not keep the
/// namespace alive (the namespace owns the adapter), so a handle used after the namespace is gone finds nothing.
/// Do not call it from inside the factory: the namespace does not exist yet.
#[derive(Clone)]
pub struct LocalNamespace {
    inner: Weak<NamespaceInner>,
}

impl std::fmt::Debug for LocalNamespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalNamespace").field("name", &self.name()).finish()
    }
}

impl LocalNamespace {
    pub(crate) fn new(inner: Weak<NamespaceInner>) -> Self {
        LocalNamespace { inner }
    }

    /// The namespace name, empty when the namespace is gone.
    pub fn name(&self) -> String {
        self.inner.upgrade().map(|n| n.name.clone()).unwrap_or_default()
    }

    /// The connected socket with this id.
    pub fn socket(&self, id: &str) -> Option<IoSocket> {
        self.inner.upgrade().and_then(|n| n.socket(id))
    }

    /// The connected sockets with these ids, under one lock: a snapshot, not a view.
    pub fn sockets_by_ids<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Vec<IoSocket> {
        match self.inner.upgrade() {
            Some(n) => n.sockets_by_ids(ids),
            None => Vec::new(),
        }
    }

    /// Whether the socket is connected here and has not finished disconnecting: the test `add_all` makes under its
    /// own lock, so no membership is left behind by a socket that went away.
    pub fn is_live(&self, id: &str) -> bool {
        self.socket(id).is_some_and(|s| s.is_live())
    }

    /// Whether `id` is the id of a live socket of this namespace (connected, or disconnecting).
    pub(crate) fn is_socket_id(&self, id: &str) -> bool {
        self.is_live(id)
    }

    /// The options of the namespace's server.
    pub(crate) fn options(&self) -> Option<Arc<IoOptions>> {
        self.inner.upgrade().map(|n| n.options.clone())
    }

    pub fn max_rooms_per_socket(&self) -> usize {
        self.options().map_or(super::options::DEFAULT_MAX_ROOMS_PER_SOCKET, |o| o.max_rooms_per_socket)
    }

    pub fn max_room_name_length(&self) -> usize {
        self.options().map_or(super::options::DEFAULT_MAX_ROOM_NAME_LENGTH, |o| o.max_room_name_length)
    }

    /// The next id of a server-side acknowledgement of this namespace.
    pub fn next_ack_id(&self) -> u64 {
        self.inner.upgrade().map_or(0, |n| n.next_ack_id())
    }
}

// ------------------------------------------------------------------------------------------------- the factory

/// Creates the adapter of a namespace.
#[derive(Clone)]
pub struct AdapterFactory(Arc<dyn Fn(LocalNamespace) -> Arc<dyn Adapter> + Send + Sync>);

impl std::fmt::Debug for AdapterFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AdapterFactory")
    }
}

impl AdapterFactory {
    pub(crate) fn new(f: impl Fn(LocalNamespace) -> Arc<dyn Adapter> + Send + Sync + 'static) -> Self {
        AdapterFactory(Arc::new(f))
    }

    pub(crate) fn in_memory() -> Self {
        AdapterFactory::new(|local| Arc::new(InMemoryAdapter::new(local)) as Arc<dyn Adapter>)
    }

    pub(crate) fn build(&self, local: LocalNamespace) -> Arc<dyn Adapter> {
        (self.0)(local)
    }
}

// ---------------------------------------------------------------------------------------- acknowledgement sink

struct AckState {
    responses: Vec<Args>,
    /// What the answers occupy in the unread budget of the sockets that sent them, held until the call ends.
    charges: Vec<Charge>,
    bytes: usize,
    expected_clients: usize,
    reported: usize,
    servers: Option<usize>,
    too_large: bool,
    finished: bool,
    cleanups: Vec<Box<dyn FnOnce() + Send>>,
}

pub(crate) struct AckShared {
    state: Mutex<AckState>,
    notify: Notify,
    max_bytes: usize,
}

/// How a broadcast with acknowledgements ended, from the collector's side.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AckProgress {
    Complete,
    TooLarge,
}

impl AckShared {
    pub(crate) fn new(max_bytes: usize) -> Arc<AckShared> {
        Arc::new(AckShared {
            state: Mutex::new(AckState {
                responses: Vec::new(),
                charges: Vec::new(),
                bytes: 0,
                expected_clients: 0,
                reported: 0,
                servers: None,
                too_large: false,
                finished: false,
                cleanups: Vec::new(),
            }),
            notify: Notify::new(),
            max_bytes,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AckState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn set_servers(&self, servers: usize) {
        self.lock().servers = Some(servers);
        self.notify.notify_one();
    }

    /// Waits until every server reported its clients and every counted client answered, or the cap was passed.
    pub(crate) async fn wait(&self) -> AckProgress {
        loop {
            let notified = self.notify.notified();
            {
                let s = self.lock();
                if s.too_large {
                    return AckProgress::TooLarge;
                }
                if s.servers.is_some_and(|n| s.reported >= n) && s.responses.len() >= s.expected_clients {
                    return AckProgress::Complete;
                }
            }
            notified.await;
        }
    }

    /// Takes what was collected and ends the collection: later `expect` and `respond` calls do nothing, and the
    /// cleanups registered by the adapter run (outside the lock). Idempotent.
    pub(crate) fn finish(&self) -> Vec<Args> {
        let (responses, charges, cleanups) = {
            let mut s = self.lock();
            s.finished = true;
            s.bytes = 0;
            (std::mem::take(&mut s.responses), std::mem::take(&mut s.charges), std::mem::take(&mut s.cleanups))
        };
        // Released outside the lock: a release wakes the connections that wait for room.
        drop(charges);
        for cleanup in cleanups {
            cleanup();
        }
        responses
    }
}

/// Where an adapter reports the answers of a broadcast with acknowledgements (Node's two callbacks). Cheap to clone;
/// every clone feeds the same collection. Calls after the broadcast ended do nothing.
#[derive(Clone)]
pub struct AckSink {
    shared: Arc<AckShared>,
}

impl std::fmt::Debug for AckSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AckSink").finish_non_exhaustive()
    }
}

impl AckSink {
    pub(crate) fn new(shared: Arc<AckShared>) -> AckSink {
        AckSink { shared }
    }

    /// One server reports how many clients it sent the packet to (the first callback of `broadcastWithAck`). Call
    /// it once per server, with the number of clients whose answer will be sent to [`respond`](Self::respond).
    pub fn expect(&self, clients: usize) {
        {
            let mut s = self.shared.lock();
            if s.finished {
                return;
            }
            s.expected_clients = s.expected_clients.saturating_add(clients);
            s.reported += 1;
        }
        self.shared.notify.notify_one();
    }

    /// A client's answer. The answers of one broadcast are bounded by `max_broadcast_ack_bytes`: the one that would
    /// pass it is dropped and ends the call with the answers that were collected.
    pub fn respond(&self, args: Args) {
        self.collect(args, None);
    }

    /// The answer of a local socket with the room it occupies in that connection's unread budget: held with the
    /// answer until the call ends, dropped at once when the answer is not kept.
    pub(crate) fn respond_charged(&self, args: Args, charge: Charge) {
        self.collect(args, Some(charge));
    }

    fn collect(&self, args: Args, charge: Option<Charge>) {
        let cost = args.iter().map(IoValue::cost).fold(std::mem::size_of::<Args>(), usize::saturating_add);
        {
            let mut s = self.shared.lock();
            if s.finished || s.too_large {
                return;
            }
            if s.bytes.saturating_add(cost) > self.shared.max_bytes {
                s.too_large = true;
            } else {
                s.bytes += cost;
                s.responses.push(args);
                s.charges.extend(charge);
            }
        }
        self.shared.notify.notify_one();
    }

    /// Registers what the adapter must undo when the broadcast ends, however it ends (a pending acknowledgement on
    /// every recipient, a subscription). Runs at once when the broadcast already ended.
    pub fn on_finish(&self, cleanup: impl FnOnce() + Send + 'static) {
        {
            let mut s = self.shared.lock();
            if !s.finished {
                s.cleanups.push(Box::new(cleanup));
                return;
            }
        }
        cleanup();
    }
}

// ----------------------------------------------------------------------------------------------- the trait

/// The adapter of one namespace. See the [module documentation](self); unstable until #36.
#[async_trait]
pub trait Adapter: Send + Sync + 'static {
    /// Runs once, before the first cluster operation of the namespace (Node's `init`).
    async fn init(&self) -> Result<(), AdapterError>;
    /// Releases what the adapter holds (Node's `close`).
    async fn close(&self);
    /// How many servers run the namespace, this one included.
    async fn server_count(&self) -> Result<usize, AdapterError>;

    /// Joins a socket connected to this process to rooms (Node's `addAll`). All or nothing: a name over the limit,
    /// more rooms than the socket may be in (a call naming more than that is refused without examining them), or a
    /// name that is the id of another live socket of the namespace joins none. A socket that is not connected is
    /// ignored.
    fn add_all(&self, id: &str, rooms: &[Room]) -> Result<(), RoomError>;
    /// Removes a socket from a room (Node's `del`). A socket cannot leave the room named like its own id: that is
    /// how a direct emit to it and `RemoteSocket` reach exactly it.
    fn del(&self, id: &str, room: &str);
    /// Removes a socket from every room (Node's `delAll`).
    fn del_all(&self, id: &str);
    /// The rooms of a socket connected to this process (Node's `socketRooms`).
    fn socket_rooms(&self, id: &str) -> Vec<Room>;

    /// Sends the packet to the sockets the options select, on every server unless `flags.local`. A recipient that
    /// cannot take it is closed by its own send buffer bound and does not fail the broadcast.
    async fn broadcast(&self, packet: Packet, opts: BroadcastOptions) -> Result<(), AdapterError>;
    /// Like [`broadcast`](Self::broadcast), reporting the recipients and their answers to `sink`.
    async fn broadcast_with_ack(&self, packet: Packet, opts: BroadcastOptions, sink: AckSink) -> Result<(), AdapterError>;
    /// The ids of the sockets in all of `rooms`' union (every socket when empty), on every server.
    async fn sockets(&self, rooms: &[Room]) -> Result<HashSet<SocketId>, AdapterError>;
    /// The sockets the options select, described by plain data.
    async fn fetch_sockets(&self, opts: BroadcastOptions) -> Result<Vec<SocketInfo>, AdapterError>;
    /// Joins the selected sockets to rooms.
    async fn add_sockets(&self, opts: BroadcastOptions, rooms: Vec<Room>) -> Result<(), AdapterError>;
    /// Removes the selected sockets from rooms.
    async fn del_sockets(&self, opts: BroadcastOptions, rooms: Vec<Room>) -> Result<(), AdapterError>;
    /// Disconnects the selected sockets, closing their connection when `close`.
    async fn disconnect_sockets(&self, opts: BroadcastOptions, close: bool) -> Result<(), AdapterError>;

    /// Whether a room with this name has members here. The namespace refuses to assign a new socket an id that is
    /// already a room, so a room created in advance cannot capture a future socket. An adapter that keeps rooms
    /// should implement it; the default says no.
    fn room_exists(&self, _room: &str) -> bool {
        false
    }

    /// What the local maps hold, for the tests that check nothing is left behind. `None` when the adapter has no such
    /// maps.
    #[doc(hidden)]
    fn membership_size(&self) -> Option<MembershipSize> {
        None
    }

    /// The invariants of the local maps (they mirror each other, no empty room), for the tests.
    #[doc(hidden)]
    fn check_invariants(&self) -> Result<(), String> {
        Ok(())
    }

    /// Whether the local maps hold anything for this socket, for the tests.
    #[doc(hidden)]
    fn has_socket(&self, _id: &str) -> bool {
        false
    }
}

// -------------------------------------------------------------------------------------- the in-memory adapter

/// The two maps of the in-memory adapter: the sockets of each room, and the rooms of each socket. They mirror each
/// other; no room and no socket is ever kept with an empty set.
#[derive(Default)]
pub(crate) struct Membership {
    pub(crate) rooms: HashMap<Room, HashSet<SocketId>>,
    pub(crate) sids: HashMap<SocketId, HashSet<Room>>,
    /// The names the last `add` looked at: the work done under the lock, for the tests.
    pub(crate) examined: usize,
}

impl Membership {
    /// Joins `id` to `rooms` (distinct names; the caller deduplicates outside the lock), all or nothing: refused, at
    /// the first name that would take the socket past `max_rooms`, without looking at the rest.
    pub(crate) fn add(&mut self, id: &str, rooms: &[Room], max_rooms: usize) -> Result<(), RoomError> {
        let current = self.sids.get(id);
        let held = current.map_or(0, HashSet::len);
        let mut fresh: Vec<&Room> = Vec::new();
        self.examined = 0;
        for room in rooms {
            self.examined += 1;
            if current.is_some_and(|set| set.contains(&**room)) {
                continue;
            }
            if held + fresh.len() + 1 > max_rooms {
                return Err(RoomError::TooManyRooms { max: max_rooms });
            }
            fresh.push(room);
        }
        if fresh.is_empty() {
            return Ok(());
        }
        let key: SocketId = match self.sids.get_key_value(id) {
            Some((key, _)) => key.clone(),
            None => Arc::from(id),
        };
        let fresh: Vec<Room> = fresh.into_iter().cloned().collect();
        for room in fresh {
            self.rooms.entry(room.clone()).or_default().insert(key.clone());
            self.sids.entry(key.clone()).or_default().insert(room);
        }
        Ok(())
    }

    pub(crate) fn remove_all(&mut self, id: &str) {
        let Some(rooms) = self.sids.remove(id) else { return };
        for room in rooms {
            if let Some(members) = self.rooms.get_mut(&room) {
                members.remove(id);
                if members.is_empty() {
                    self.rooms.remove(&room);
                }
            }
        }
    }

    pub(crate) fn remove(&mut self, id: &str, room: &str) {
        if let Some(set) = self.sids.get_mut(id) {
            set.remove(room);
            if set.is_empty() {
                self.sids.remove(id);
            }
        }
        if let Some(members) = self.rooms.get_mut(room) {
            members.remove(id);
            if members.is_empty() {
                self.rooms.remove(room);
            }
        }
    }

    /// The invariants: the maps mirror each other, and hold no empty room and no empty entry.
    pub(crate) fn check(&self) -> Result<(), String> {
        for (room, members) in &self.rooms {
            if members.is_empty() {
                return Err(format!("empty room {room}"));
            }
            for id in members {
                if !self.sids.get(id).is_some_and(|rooms| rooms.contains(room)) {
                    return Err(format!("{id} is in {room} but its entry does not say so"));
                }
            }
        }
        for (id, rooms) in &self.sids {
            if rooms.is_empty() {
                return Err(format!("empty entry for {id}"));
            }
            for room in rooms {
                if !self.rooms.get(room).is_some_and(|members| members.contains(id)) {
                    return Err(format!("{id} claims {room} but the room does not have it"));
                }
            }
        }
        Ok(())
    }

    /// The ids `opts` selects (`apply` of Node's in-memory adapter): the union of the rooms, each socket once, or
    /// every socket without rooms, minus the members of the except rooms. Linear in the sockets reached plus the
    /// cheaper of the two ways of testing the except rooms.
    pub(crate) fn select(&self, opts: &BroadcastOptions) -> Vec<SocketId> {
        let except: Vec<&HashSet<SocketId>> = opts.except.iter().filter_map(|r| self.rooms.get(r)).collect();
        if let Some(target) = &opts.socket {
            // Exactly this socket, whoever else is in a room named like it.
            return match self.sids.get_key_value(&**target) {
                Some((key, _)) if !except.iter().any(|set| set.contains(&**key)) => vec![key.clone()],
                _ => Vec::new(),
            };
        }
        let except_members: usize = except.iter().map(|s| s.len()).sum();
        let mut candidates: Vec<&SocketId> = Vec::new();
        if opts.rooms.is_empty() {
            candidates.extend(self.sids.keys());
        } else {
            let mut seen: HashSet<&str> = HashSet::new();
            for room in &opts.rooms {
                if let Some(members) = self.rooms.get(room) {
                    for id in members {
                        if seen.insert(id) {
                            candidates.push(id);
                        }
                    }
                }
            }
        }
        if except.is_empty() {
            return candidates.into_iter().cloned().collect();
        }
        // Testing each candidate against every except room costs `candidates x except rooms`; building the set of
        // excluded ids costs the members of those rooms. Take the cheaper: never the product of two large numbers.
        if candidates.len().saturating_mul(except.len()) <= except_members {
            candidates.into_iter().filter(|id| !except.iter().any(|set| set.contains(*id))).cloned().collect()
        } else {
            let excluded: HashSet<&str> = except.iter().flat_map(|set| set.iter().map(|id| &**id)).collect();
            candidates.into_iter().filter(|id| !excluded.contains(&***id)).cloned().collect()
        }
    }
}

/// The default adapter: rooms in two maps behind one lock, delivery in this process.
///
/// - A broadcast selects its recipients under the read lock, releases it, and delivers afterwards: membership is
///   never locked while packets are queued, so handlers can join, leave and broadcast during a broadcast and a large
///   fan-out does not block joins. The selection is the moment of the broadcast: a socket that left a room before
///   it started does not receive it, one that disconnected is never sent a packet (its phase is checked at send).
/// - The packet is encoded once and the same text, and the same attachment buffers, are queued on every recipient.
///   While a broadcast is delivered it holds `E` (the encoding: text plus attachments) once, plus about 24 bytes per
///   recipient for the snapshot of ids and sockets; each recipient's queue holds a reference to the encoding, and
///   its own `max_buffered_amount` bounds what it keeps, so `S` slow recipients hold at most `S x
///   max_buffered_amount` between them (there is no server-wide outbound budget).
/// - A recipient whose queue is over its bound is closed (`transport error`); the others still get the packet and
///   the broadcast succeeds. Nothing tells the broadcaster which recipients were closed.
///
/// Lock order, outermost first: the adapter's membership lock, the namespace's socket table, a socket's state. The
/// membership lock is never held across an await or while application code runs.
pub struct InMemoryAdapter {
    local: LocalNamespace,
    membership: RwLock<Membership>,
}

impl std::fmt::Debug for InMemoryAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryAdapter").field("namespace", &self.local).finish_non_exhaustive()
    }
}

/// How big the maps of an adapter are: for the tests that check nothing is left behind after churn.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MembershipSize {
    pub rooms: usize,
    pub sockets: usize,
    pub memberships: usize,
}

impl InMemoryAdapter {
    pub fn new(local: LocalNamespace) -> Self {
        InMemoryAdapter { local, membership: RwLock::new(Membership::default()) }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Membership> {
        self.membership.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Membership> {
        self.membership.write().unwrap_or_else(|e| e.into_inner())
    }

    /// The sockets `opts` selects, connected to this process: a snapshot taken under the read lock.
    pub fn select(&self, opts: &BroadcastOptions) -> Vec<IoSocket> {
        let ids = self.read().select(opts);
        self.local.sockets_by_ids(ids.iter().map(|id| &**id))
    }

    /// The size of the two maps, and whether they mirror each other.
    #[doc(hidden)]
    pub fn size(&self) -> MembershipSize {
        let m = self.read();
        let from_rooms: usize = m.rooms.values().map(HashSet::len).sum();
        let from_sids: usize = m.sids.values().map(HashSet::len).sum();
        assert_eq!(from_rooms, from_sids, "the two maps of the adapter mirror each other");
        MembershipSize { rooms: m.rooms.len(), sockets: m.sids.len(), memberships: from_rooms }
    }

    async fn deliver(&self, sockets: &[IoSocket], encoded: &parser::SharedEncoded) {
        for (i, socket) in sockets.iter().enumerate() {
            if i > 0 && i % YIELD_EVERY == 0 {
                tokio::task::yield_now().await;
            }
            // A recipient that is gone, or whose queue is full (it is closed by the Engine.IO layer), is not the
            // broadcaster's problem.
            let _ = socket.send_shared(encoded);
        }
    }
}

#[async_trait]
impl Adapter for InMemoryAdapter {
    fn membership_size(&self) -> Option<MembershipSize> {
        Some(self.size())
    }

    fn check_invariants(&self) -> Result<(), String> {
        self.read().check()
    }

    fn has_socket(&self, id: &str) -> bool {
        self.read().sids.contains_key(id)
    }

    async fn init(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn close(&self) {
        let mut m = self.write();
        m.rooms.clear();
        m.sids.clear();
    }

    async fn server_count(&self) -> Result<usize, AdapterError> {
        Ok(1)
    }

    fn add_all(&self, id: &str, rooms: &[Room]) -> Result<(), RoomError> {
        // The input is bounded and validated before the lock is taken: a call naming more rooms than a socket may be
        // in is refused at once, and names are checked and deduplicated outside it.
        let max_rooms = self.local.max_rooms_per_socket();
        if rooms.len() > max_rooms {
            return Err(RoomError::TooManyRooms { max: max_rooms });
        }
        let max_name = self.local.max_room_name_length();
        let mut seen: HashSet<&str> = HashSet::with_capacity(rooms.len());
        let mut distinct: Vec<Room> = Vec::with_capacity(rooms.len());
        for room in rooms {
            if room.len() > max_name {
                return Err(RoomError::NameTooLong { len: room.len(), max: max_name });
            }
            if seen.insert(room) {
                distinct.push(room.clone());
            }
        }
        let mut m = self.write();
        // Under the write lock: a socket that disconnects marks itself before it calls `del_all`, which waits for
        // this lock, so nothing is left behind whatever order the two run in.
        if !self.local.is_live(id) {
            return Ok(());
        }
        // The id of another live socket is not a room a client may join: it would receive what is sent to that
        // socket. Checked under the same lock, so the whole call stays all or nothing.
        if distinct.iter().any(|room| &**room != id && self.local.is_socket_id(room)) {
            return Err(RoomError::SocketIdRoom);
        }
        m.add(id, &distinct, max_rooms)
    }

    fn del(&self, id: &str, room: &str) {
        if id == room {
            return;
        }
        self.write().remove(id, room);
    }

    fn room_exists(&self, room: &str) -> bool {
        self.read().rooms.contains_key(room)
    }

    fn del_all(&self, id: &str) {
        self.write().remove_all(id);
    }

    fn socket_rooms(&self, id: &str) -> Vec<Room> {
        self.read().sids.get(id).map(|rooms| rooms.iter().cloned().collect()).unwrap_or_default()
    }

    async fn broadcast(&self, packet: Packet, opts: BroadcastOptions) -> Result<(), AdapterError> {
        let max_attachments = self.local.options().map_or(super::options::DEFAULT_MAX_ATTACHMENTS, |o| o.max_attachments);
        let encoded = parser::encode(&packet, max_attachments).map_err(PacketError::from)?.share();
        drop(packet);
        let sockets = self.select(&opts);
        self.deliver(&sockets, &encoded).await;
        Ok(())
    }

    async fn broadcast_with_ack(&self, packet: Packet, opts: BroadcastOptions, sink: AckSink) -> Result<(), AdapterError> {
        let Some(options) = self.local.options() else {
            sink.expect(0);
            return Ok(());
        };
        let id = self.local.next_ack_id();
        let mut packet = packet;
        if let Body::Event { id: slot, .. } = &mut packet.body {
            *slot = Some(id);
        }
        let encoded = parser::encode(&packet, options.max_attachments).map_err(PacketError::from)?.share();
        drop(packet);
        let deadline = after(Instant::now(), opts.flags.timeout.unwrap_or(options.ack_timeout).min(super::engine::options::MAX_DURATION));
        let sockets = self.select(&opts);
        let mut registered: Vec<Weak<SocketInner>> = Vec::with_capacity(sockets.len());
        for (i, socket) in sockets.iter().enumerate() {
            if i > 0 && i % YIELD_EVERY == 0 {
                tokio::task::yield_now().await;
            }
            // The entry first: the answer may come back before this loop ends. A recipient that cannot hold one more
            // pending answer still gets the event, and its answer is not awaited.
            if !socket.register_broadcast_ack(id, sink.clone(), deadline) {
                let _ = socket.send_shared(&encoded);
                continue;
            }
            if socket.send_shared(&encoded).is_ok() {
                registered.push(socket.downgrade());
            } else {
                socket.cancel_ack(id);
            }
        }
        let clients = registered.len();
        sink.on_finish(move || {
            for socket in registered {
                if let Some(socket) = socket.upgrade() {
                    IoSocket { inner: socket }.cancel_ack(id);
                }
            }
        });
        sink.expect(clients);
        Ok(())
    }

    async fn sockets(&self, rooms: &[Room]) -> Result<HashSet<SocketId>, AdapterError> {
        let opts = BroadcastOptions::new().rooms(rooms.iter().cloned());
        Ok(self.read().select(&opts).into_iter().collect())
    }

    async fn fetch_sockets(&self, opts: BroadcastOptions) -> Result<Vec<SocketInfo>, AdapterError> {
        Ok(self.select(&opts).iter().map(|socket| socket.info(self.socket_rooms(socket.id()))).collect())
    }

    async fn add_sockets(&self, opts: BroadcastOptions, rooms: Vec<Room>) -> Result<(), AdapterError> {
        let sockets = self.select(&opts);
        for (i, socket) in sockets.iter().enumerate() {
            if i > 0 && i % YIELD_EVERY == 0 {
                tokio::task::yield_now().await;
            }
            if let Err(error) = self.add_all(socket.id(), &rooms) {
                // One socket at its cap must not block a bulk operation: it is skipped and logged.
                socket.warn(
                    "a socket skipped by sockets_join",
                    &format!("a socket of the namespace {} was not joined: {error}", self.local.name()),
                );
            }
        }
        Ok(())
    }

    async fn del_sockets(&self, opts: BroadcastOptions, rooms: Vec<Room>) -> Result<(), AdapterError> {
        let sockets = self.select(&opts);
        for (i, socket) in sockets.iter().enumerate() {
            if i > 0 && i % YIELD_EVERY == 0 {
                tokio::task::yield_now().await;
            }
            for room in &rooms {
                self.del(socket.id(), room);
            }
        }
        Ok(())
    }

    async fn disconnect_sockets(&self, opts: BroadcastOptions, close: bool) -> Result<(), AdapterError> {
        let sockets = self.select(&opts);
        for (i, socket) in sockets.iter().enumerate() {
            if i > 0 && i % YIELD_EVERY == 0 {
                tokio::task::yield_now().await;
            }
            socket.disconnect(close);
        }
        Ok(())
    }
}
