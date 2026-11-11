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
//!
//! **Lock order**, outermost first: the adapter's membership lock, a namespace's socket table, a socket's state, then the
//! Engine.IO session's state and outbox (leaves). Under the membership lock `add_all` takes the table and the state of
//! the joining socket only (its liveness; the "id of another socket" test reads the adapter's own map), so a
//! synchronous adapter method never runs while a lock of the connection is held, and may itself call anything of the
//! socket API, even close the connection. The connection's table of namespaces is held around a socket's state lock
//! once (when a socket that passed its middleware takes its slot), and nothing takes it while holding a state lock.

use std::collections::HashSet;
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;

use super::namespace::NamespaceInner;
use super::options::IoOptions;
use super::parser::EncodeError;

pub use super::parser::Packet;
use super::socket::IoSocket;

mod memory;

#[cfg(test)]
pub(crate) use self::memory::Membership;
pub use self::memory::{InMemoryAdapter, MembershipSize};
pub use super::ack::AckSink;
pub(crate) use super::ack::{AckBudget, AckProgress, AckShared};
pub use super::info::{HandshakeInfo, SocketInfo};

/// A room name. Cheap to clone.
pub type Room = Arc<str>;
/// A socket id. Cheap to clone.
pub type SocketId = Arc<str>;

/// Why a join was refused. Nothing was joined.
///
/// It is `#[non_exhaustive]`: a variant may be added in a minor release, so a `match` outside the crate needs a wildcard
/// arm.
///
/// ```compile_fail
/// # use corvis::socket_io::RoomError;
/// fn every_variant(value: RoomError) {
///     match value {
///         RoomError::NameTooLong { .. } => {}
///         RoomError::TooManyRooms { .. } => {}
///         RoomError::SocketIdRoom => {}
///         RoomError::TooManyTargets { .. } => {}
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
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

/// The bounds on the room names a call may give: every place that takes room names from application code (a join, the
/// in-memory adapter, a broadcast operator) applies them, the same way, before it looks at anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RoomLimits {
    pub(crate) max_rooms: usize,
    pub(crate) max_name_length: usize,
}

impl RoomLimits {
    pub(crate) fn of(options: &IoOptions) -> RoomLimits {
        RoomLimits { max_rooms: options.max_rooms_per_socket, max_name_length: options.max_room_name_length }
    }

    /// The count first (a call naming more rooms than a socket may be in is refused without looking at the names),
    /// then each name.
    pub(crate) fn check(&self, rooms: &[Room]) -> Result<(), RoomError> {
        if rooms.len() > self.max_rooms {
            return Err(RoomError::TooManyRooms { max: self.max_rooms });
        }
        rooms.iter().try_for_each(|room| self.check_name(room))
    }

    pub(crate) fn check_name(&self, room: &str) -> Result<(), RoomError> {
        if room.len() > self.max_name_length {
            return Err(RoomError::NameTooLong { len: room.len(), max: self.max_name_length });
        }
        Ok(())
    }
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
///
/// It is `#[non_exhaustive]`: a variant may be added in a minor release, so a `match` outside the crate needs a wildcard
/// arm.
///
/// ```compile_fail
/// # use corvis::socket_io::AdapterError;
/// fn every_variant(value: AdapterError) {
///     match value {
///         AdapterError::Timeout => {}
///         AdapterError::Closed => {}
///         AdapterError::Unavailable(_) => {}
///         AdapterError::InvalidPacket(_) => {}
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
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
        #[cfg(test)]
        if let Some(n) = self.inner.upgrade() {
            n.liveness_lookups.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        self.socket(id).is_some_and(|s| s.is_live())
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

    pub(crate) fn room_limits(&self) -> RoomLimits {
        RoomLimits { max_rooms: self.max_rooms_per_socket(), max_name_length: self.max_room_name_length() }
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

// ----------------------------------------------------------------------------------------------- the trait

/// The adapter of one namespace. See the [module documentation](self); unstable until #36.
#[async_trait]
pub trait Adapter: std::any::Any + Send + Sync + 'static {
    /// Runs once, before the first cluster operation of the namespace (Node's `init`).
    async fn init(&self) -> Result<(), AdapterError>;
    /// Releases what the adapter holds (Node's `close`).
    async fn close(&self);
    /// How many servers run the namespace, this one included.
    async fn server_count(&self) -> Result<usize, AdapterError>;

    /// Joins a socket connected to this process to rooms (Node's `addAll`). All or nothing: a name over the limit,
    /// more rooms than the socket may be in (a call naming more than that is refused without examining them), or a
    /// name that is the id of another live socket of the namespace joins none. A socket that is not connected is
    /// ignored. Adding the socket to the room named like its own id (what the namespace does when the socket
    /// connects) expels every other member of that room, so a name joined before the socket was live does not
    /// survive it.
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
    /// must implement it (the default says no, which silently disables the rule, so a distributed adapter has to
    /// answer for the rooms of the whole cluster it can see). It is synchronous and so are `add_all`, `del` and
    /// `del_all`: none of them can be bounded by `adapter_timeout`, so they must not block or wait for the network.
    fn room_exists(&self, _room: &str) -> bool {
        false
    }
}
