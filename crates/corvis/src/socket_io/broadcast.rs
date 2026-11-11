//! Broadcast operators: `io.to("room").except("other").emit(...)`, `socket.broadcast().emit(...)`, and the bulk
//! operations over a selection of sockets (`fetch_sockets`, `sockets_join`, `sockets_leave`, `disconnect_sockets`).
//!
//! An operator is a value: `to`, `within`, `except`, `local` and `timeout` return a new operator and leave the one
//! they were called on as it was, so a base operator can be reused. The terminal operations are `async` and return a
//! `Result`, because the adapter behind them may be a cluster one; with the default in-memory adapter they complete
//! without waiting on any client. Which sockets a broadcast reaches is what Node's in-memory adapter selects.
//!
//! A room name over [`max_room_name_length`](super::IoOptions::max_room_name_length) never panics and is never
//! dropped (dropping it from `to` could leave the room list empty, which means "everyone"): the operator keeps the
//! error and every terminal operation returns it without sending or changing anything.
//!
//! ```no_run
//! # use corvis::socket_io::{IoServer, IoSocket};
//! # async fn demo(io: IoServer, socket: IoSocket) -> Result<(), Box<dyn std::error::Error>> {
//! // Everyone in `lobby` or `news`, once each, except the members of `muted` and the socket `other`.
//! io.to(["lobby", "news"]).except("muted").except("other").emit("notice", ("hello",)).await?;
//! // Every socket of the namespace but this one.
//! socket.broadcast().emit("joined", (socket.id(),)).await?;
//! # Ok(()) }
//! ```

use std::sync::Arc;
use std::time::Duration;

use super::adapter::{
    AckShared, AckSink, AdapterError, BroadcastFlags, BroadcastOptions, HandshakeInfo, Room, RoomError, RoomLimits, SocketId, SocketInfo,
};
use super::engine::options::MAX_DURATION;
use super::namespace::IoNamespace;
use super::parser::{self, Body, Packet};
use super::server::IoServer;
use super::socket::{EmitError, IoSocket};
use super::value::{Args, IntoArgs};

/// What can be given as the rooms of `join`, `to`, `within` and `except`: a name, or several in a `Vec`, a slice or
/// an array.
pub trait IntoRooms {
    fn into_rooms(self) -> Vec<Room>;
}

impl IntoRooms for &str {
    fn into_rooms(self) -> Vec<Room> {
        vec![Arc::from(self)]
    }
}

impl IntoRooms for String {
    fn into_rooms(self) -> Vec<Room> {
        vec![Arc::from(self)]
    }
}

impl IntoRooms for &String {
    fn into_rooms(self) -> Vec<Room> {
        vec![Arc::from(self.as_str())]
    }
}

impl IntoRooms for Room {
    fn into_rooms(self) -> Vec<Room> {
        vec![self]
    }
}

impl<S: AsRef<str>> IntoRooms for Vec<S> {
    fn into_rooms(self) -> Vec<Room> {
        self.iter().map(|r| Arc::from(r.as_ref())).collect()
    }
}

impl<S: AsRef<str>> IntoRooms for &Vec<S> {
    fn into_rooms(self) -> Vec<Room> {
        self.iter().map(|r| Arc::from(r.as_ref())).collect()
    }
}

impl<S: AsRef<str>> IntoRooms for &[S] {
    fn into_rooms(self) -> Vec<Room> {
        self.iter().map(|r| Arc::from(r.as_ref())).collect()
    }
}

impl<S: AsRef<str>, const N: usize> IntoRooms for [S; N] {
    fn into_rooms(self) -> Vec<Room> {
        self.iter().map(|r| Arc::from(r.as_ref())).collect()
    }
}

impl<S: AsRef<str>, const N: usize> IntoRooms for &[S; N] {
    fn into_rooms(self) -> Vec<Room> {
        self.iter().map(|r| Arc::from(r.as_ref())).collect()
    }
}

/// Why a broadcast with acknowledgements ended without every answer.
///
/// It is `#[non_exhaustive]`: a variant may be added in a minor release, so a `match` outside the crate needs a wildcard
/// arm.
///
/// ```compile_fail
/// # use corvis::socket_io::BroadcastAckErrorKind;
/// fn every_variant(value: BroadcastAckErrorKind) {
///     match value {
///         BroadcastAckErrorKind::Timeout => {}
///         BroadcastAckErrorKind::ResponsesTooLarge => {}
///         BroadcastAckErrorKind::Emit(_) => {}
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum BroadcastAckErrorKind {
    /// The deadline passed before every recipient answered.
    #[error("operation has timed out")]
    Timeout,
    /// The answers collected passed `max_broadcast_ack_bytes`; the ones that did not fit were dropped.
    #[error("the acknowledgements are larger than max_broadcast_ack_bytes allows")]
    ResponsesTooLarge,
    /// The event could not be sent (a reserved name, an invalid room, the adapter failed).
    #[error(transparent)]
    Emit(EmitError),
}

/// A broadcast with acknowledgements that did not collect every answer: the kind, and the answers that did arrive
/// (`err.responses` in Node).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{kind}")]
pub struct BroadcastAckError {
    kind: BroadcastAckErrorKind,
    responses: Vec<Args>,
}

impl BroadcastAckError {
    fn new(kind: BroadcastAckErrorKind, responses: Vec<Args>) -> Self {
        BroadcastAckError { kind, responses }
    }

    pub fn kind(&self) -> &BroadcastAckErrorKind {
        &self.kind
    }

    /// The answers received before the call ended, in the order they arrived.
    pub fn responses(&self) -> &[Args] {
        &self.responses
    }

    pub fn into_responses(self) -> Vec<Args> {
        self.responses
    }
}

impl From<EmitError> for BroadcastAckError {
    fn from(error: EmitError) -> Self {
        BroadcastAckError::new(BroadcastAckErrorKind::Emit(error), Vec::new())
    }
}

/// Ends the collection of a broadcast with acknowledgements however the call ends, even when its future is dropped:
/// the adapter's cleanups run, so no recipient keeps a pending acknowledgement for it.
struct Collection(Arc<AckShared>);

impl Drop for Collection {
    fn drop(&mut self) {
        self.0.finish();
    }
}

/// One call of `to`, `within` or `except`, linked to the ones before it. A chain shares its prefix with every operator
/// built from it, so building one costs the rooms of each call once, not once per call.
struct Step {
    parent: Option<Arc<Step>>,
    rooms: Vec<Room>,
    except: bool,
}

impl Drop for Step {
    /// Iterative, so a long chain is not dropped by recursion.
    fn drop(&mut self) {
        let mut next = self.parent.take();
        while let Some(step) = next {
            match Arc::try_unwrap(step) {
                Ok(mut step) => next = step.parent.take(),
                Err(_) => break,
            }
        }
    }
}

/// A chain of `to` / `within` / `except` / `local` / `timeout` that ends in an emit or a bulk operation. Cheap to
/// clone; each call costs the rooms it adds, however long the chain already is. An operator names at most
/// [`max_rooms_per_socket`](super::IoOptions::max_rooms_per_socket) rooms in all (`to`, `within` and `except`
/// together): past that, every terminal operation fails with `RoomError::TooManyTargets`.
///
/// A `to` or `within` given an empty collection selects no socket (when no other call adds rooms), where Node would
/// reach everyone: an empty list that came from a client must not mean "everyone". An operator with no `to` at all
/// reaches every socket of the namespace.
#[derive(Clone)]
pub struct BroadcastOperator {
    nsp: IoNamespace,
    steps: Option<Arc<Step>>,
    /// The rooms named by `to`, `within` and `except`.
    named: usize,
    /// `to` or `within` was called: with no room in the end the operator selects nobody.
    narrowed: bool,
    flags: BroadcastFlags,
    socket: Option<SocketId>,
    error: Option<RoomError>,
}

impl std::fmt::Debug for BroadcastOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BroadcastOperator")
            .field("namespace", &self.nsp.name())
            .field("rooms_named", &self.named)
            .field("flags", &self.flags)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl BroadcastOperator {
    pub(crate) fn new(nsp: IoNamespace) -> Self {
        BroadcastOperator { nsp, steps: None, named: 0, narrowed: false, flags: BroadcastFlags::default(), socket: None, error: None }
    }

    /// An operator that reaches exactly one socket, by its id (never as a room).
    pub(crate) fn for_socket(nsp: IoNamespace, id: &str) -> Self {
        let mut operator = BroadcastOperator::new(nsp);
        operator.socket = Some(Arc::from(id));
        operator
    }

    fn push(&mut self, rooms: Vec<Room>, except: bool) {
        if !rooms.is_empty() {
            self.steps = Some(Arc::new(Step { parent: self.steps.take(), rooms, except }));
        }
    }

    /// Adds a socket id to the excluded rooms: a socket id is a room name of fixed, valid length. Not counted
    /// against the rooms an operator may name.
    pub(crate) fn except_ids_unchecked(mut self, id: &str) -> Self {
        self.push(vec![Arc::from(id)], true);
        self
    }

    fn with_rooms(&self, rooms: impl IntoRooms, except: bool) -> Self {
        let mut next = self.clone();
        if !except {
            next.narrowed = true;
        }
        let limits = RoomLimits::of(self.nsp.options());
        let mut kept = Vec::new();
        for room in rooms.into_rooms() {
            match limits.check_name(&room) {
                // Kept as an error, not dropped: leaving the name out of `to` could make the broadcast reach everyone.
                Err(error) => {
                    next.error.get_or_insert(error);
                }
                Ok(()) => kept.push(room),
            }
        }
        let most = limits.max_rooms;
        if next.named.saturating_add(kept.len()) > most {
            next.error.get_or_insert(RoomError::TooManyTargets { max: most });
        } else {
            next.named += kept.len();
            next.push(kept, except);
        }
        next
    }

    /// `to(rooms)`: adds rooms to the ones the broadcast reaches; a socket in several of them receives it once.
    pub fn to(&self, rooms: impl IntoRooms) -> Self {
        self.with_rooms(rooms, false)
    }

    /// `in(rooms)` (`in` is a keyword in Rust): the same as [`to`](Self::to).
    pub fn within(&self, rooms: impl IntoRooms) -> Self {
        self.with_rooms(rooms, false)
    }

    /// `except(rooms)`: excludes the members of these rooms; a socket id is a room, so `except(id)` excludes that
    /// socket. An empty collection excludes nobody.
    pub fn except(&self, rooms: impl IntoRooms) -> Self {
        self.with_rooms(rooms, true)
    }

    /// `local`: reach only the sockets connected to this server.
    pub fn local(&self) -> Self {
        let mut next = self.clone();
        next.flags.local = true;
        next
    }

    /// `timeout(duration)`: the deadline of [`emit_with_ack`](Self::emit_with_ack), which is
    /// [`ack_timeout`](super::IoOptions::ack_timeout) otherwise.
    pub fn timeout(&self, timeout: Duration) -> Self {
        let mut next = self.clone();
        next.flags.timeout = Some(timeout);
        next
    }

    fn check(&self) -> Result<(), EmitError> {
        match &self.error {
            Some(error) => Err(error.clone().into()),
            None => Ok(()),
        }
    }

    /// What the adapter is asked to reach, built once for the terminal operation; `None` when the operator selects
    /// nobody (an empty `to`).
    fn selection(&self) -> Result<Option<BroadcastOptions>, EmitError> {
        self.check()?;
        let mut opts = BroadcastOptions::new().flags(self.flags.clone());
        opts.socket = self.socket.clone();
        let mut step = self.steps.as_deref();
        while let Some(s) = step {
            let into = if s.except { &mut opts.except } else { &mut opts.rooms };
            into.extend(s.rooms.iter().cloned());
            step = s.parent.as_deref();
        }
        if self.narrowed && opts.rooms.is_empty() && opts.socket.is_none() {
            return Ok(None);
        }
        Ok(Some(opts))
    }

    fn packet(&self, event: &str, args: impl IntoArgs) -> Result<Packet, EmitError> {
        if parser::is_reserved(event) {
            return Err(EmitError::ReservedEvent(event.to_string()));
        }
        self.check()?;
        let args = args.into_args()?;
        Ok(Packet { nsp: self.nsp.name().to_string(), body: Body::Event { id: None, name: event.to_string(), args } })
    }

    /// `emit(event, ...args)`: sends the event to the sockets selected, once, the same packet a direct emit sends,
    /// encoded once for all of them. It never waits for a recipient: one whose send buffer is full is closed
    /// (`transport error`), and the others and the caller are not affected. Fails, sending nothing, for a reserved
    /// event name, an invalid room, arguments that cannot be sent, or an adapter error (an adapter that takes longer
    /// than [`adapter_timeout`](super::IoOptions::adapter_timeout) fails the call with `AdapterError::Timeout`).
    ///
    /// Order: two broadcasts one caller issues one after the other, the second after the first returned, reach each
    /// recipient they have in common in that order. Broadcasts of concurrent tasks (the handlers of one connection
    /// run concurrently) have no order between them.
    pub async fn emit(&self, event: &str, args: impl IntoArgs) -> Result<(), EmitError> {
        let packet = self.packet(event, args)?;
        let Some(opts) = self.selection()? else { return Ok(()) };
        let adapter = self.nsp.ready_adapter().await?;
        self.nsp.bounded(adapter.broadcast(packet, opts)).await?;
        Ok(())
    }

    /// `timeout(...).emit(event, ...args, callback)`: sends the event and collects the answer of every recipient, in
    /// the order they arrive. It ends when every recipient answered, or at the deadline ([`timeout`](Self::timeout),
    /// else `ack_timeout`), failing with `operation has timed out` and the answers so far. A recipient that
    /// disconnects, or that waits for `max_pending_acks` answers already, does not answer. The answers held are
    /// bounded by `max_broadcast_ack_bytes`; the call fails with `ResponsesTooLarge` once they would pass it. The
    /// answers of all pending calls together are bounded by a server-wide answer budget (a quarter of the engine's
    /// `max_total_unread_bytes`, at least `max_broadcast_ack_bytes`) that belongs to the calls: the connection that
    /// answered is not charged beyond the receipt of its answer, and a call that finds no room does not wait, it ends
    /// with `ResponsesTooLarge`. Node, without `timeout()`, fails at once.
    pub async fn emit_with_ack(&self, event: &str, args: impl IntoArgs) -> Result<Vec<Args>, BroadcastAckError> {
        let packet = self.packet(event, args)?;
        let Some(mut opts) = self.selection()? else { return Ok(Vec::new()) };
        let options = self.nsp.options().clone();
        let timeout = self.flags.timeout.unwrap_or(options.ack_timeout).min(MAX_DURATION);
        opts.flags.timeout = Some(timeout);
        let shared = AckShared::new(options.max_broadcast_ack_bytes, self.nsp.ack_budget().clone());
        let collection = Collection(shared.clone());
        let work = async {
            let adapter = self.nsp.ready_adapter().await?;
            shared.set_servers(self.nsp.bounded(adapter.server_count()).await?);
            self.nsp.bounded(adapter.broadcast_with_ack(packet, opts, AckSink::new(shared.clone()))).await?;
            Ok::<_, AdapterError>(shared.wait().await)
        };
        let outcome = tokio::time::timeout(timeout, work).await;
        let responses = shared.finish();
        drop(collection);
        match outcome {
            Ok(Ok(super::adapter::AckProgress::Complete)) => Ok(responses),
            Ok(Ok(super::adapter::AckProgress::TooLarge)) => {
                Err(BroadcastAckError::new(BroadcastAckErrorKind::ResponsesTooLarge, responses))
            }
            Ok(Err(error)) => Err(BroadcastAckError::new(BroadcastAckErrorKind::Emit(error.into()), responses)),
            Err(_) => Err(BroadcastAckError::new(BroadcastAckErrorKind::Timeout, responses)),
        }
    }

    /// `fetchSockets()`: the sockets selected, as [`RemoteSocket`]s (plain data plus operations that work whether or
    /// not the socket is connected to this process). A snapshot: sockets that join or leave afterwards do not change it.
    /// For sockets of this process the handshake is shared, not copied.
    pub async fn fetch_sockets(&self) -> Result<Vec<RemoteSocket>, EmitError> {
        let Some(opts) = self.selection()? else { return Ok(Vec::new()) };
        let adapter = self.nsp.ready_adapter().await?;
        let infos = self.nsp.bounded(adapter.fetch_sockets(opts)).await?;
        Ok(infos.into_iter().map(|info| RemoteSocket { nsp: self.nsp.clone(), info }).collect())
    }

    /// `socketsJoin(rooms)`: makes every socket selected join these rooms. A socket that would pass
    /// `max_rooms_per_socket`, or that the rooms would make join the room of another socket's id, is skipped and a
    /// warning is logged; the others join (Node has no cap). A call naming more rooms than `max_rooms_per_socket`
    /// fails at once.
    pub async fn sockets_join(&self, rooms: impl IntoRooms) -> Result<(), EmitError> {
        self.check()?;
        let rooms = self.valid_rooms(rooms)?;
        let Some(opts) = self.selection()? else { return Ok(()) };
        let adapter = self.nsp.ready_adapter().await?;
        self.nsp.bounded(adapter.add_sockets(opts, rooms)).await?;
        Ok(())
    }

    /// `socketsLeave(rooms)`: makes every socket selected leave these rooms (never the room of its own id).
    pub async fn sockets_leave(&self, rooms: impl IntoRooms) -> Result<(), EmitError> {
        self.check()?;
        let rooms = self.valid_rooms(rooms)?;
        let Some(opts) = self.selection()? else { return Ok(()) };
        let adapter = self.nsp.ready_adapter().await?;
        self.nsp.bounded(adapter.del_sockets(opts, rooms)).await?;
        Ok(())
    }

    /// `disconnectSockets(close)`: disconnects every socket selected from this namespace (they hear `io server
    /// disconnect`), and with `close` closes their connections too.
    pub async fn disconnect_sockets(&self, close: bool) -> Result<(), EmitError> {
        let Some(opts) = self.selection()? else { return Ok(()) };
        let adapter = self.nsp.ready_adapter().await?;
        self.nsp.bounded(adapter.disconnect_sockets(opts, close)).await?;
        Ok(())
    }

    fn valid_rooms(&self, rooms: impl IntoRooms) -> Result<Vec<Room>, RoomError> {
        let rooms = rooms.into_rooms();
        RoomLimits::of(self.nsp.options()).check(&rooms)?;
        Ok(rooms)
    }
}

/// A socket returned by `fetch_sockets`: its data, and operations on it. For a socket of this process they act on
/// that socket directly (as Node returns the local socket); for one held by another process they go to the adapter,
/// addressed to that socket's id and to nothing else.
#[derive(Clone)]
pub struct RemoteSocket {
    nsp: IoNamespace,
    info: SocketInfo,
}

impl std::fmt::Debug for RemoteSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteSocket").field("namespace", &self.nsp.name()).field("info", &self.info).finish()
    }
}

impl RemoteSocket {
    pub fn id(&self) -> &str {
        &self.info.id
    }

    pub fn handshake(&self) -> &HandshakeInfo {
        &self.info.handshake
    }

    /// The rooms the socket was in when it was fetched.
    pub fn rooms(&self) -> &[Room] {
        &self.info.rooms
    }

    /// `socket.data` when it was fetched.
    pub fn data(&self) -> &serde_json::Value {
        &self.info.data
    }

    /// The plain description.
    pub fn info(&self) -> &SocketInfo {
        &self.info
    }

    /// The live socket when it is connected to this process.
    pub fn local(&self) -> Option<IoSocket> {
        self.nsp.socket(&self.info.id)
    }

    /// An operator that targets exactly this socket, by its id: not "the members of the room named like it".
    pub fn operator(&self) -> BroadcastOperator {
        BroadcastOperator::for_socket(self.nsp.clone(), &self.info.id)
    }

    /// Sends an event to the socket. A socket that is gone receives nothing and the call succeeds.
    pub async fn emit(&self, event: &str, args: impl IntoArgs) -> Result<(), EmitError> {
        if let Some(socket) = self.local() {
            return match socket.emit(event, args) {
                Err(EmitError::NotConnected) => Ok(()),
                other => other,
            };
        }
        self.operator().emit(event, args).await
    }

    /// Makes the socket join rooms, under the same rules as [`IoSocket::join`].
    pub async fn join(&self, rooms: impl IntoRooms) -> Result<(), EmitError> {
        if let Some(socket) = self.local() {
            return Ok(socket.join(rooms)?);
        }
        self.operator().sockets_join(rooms).await
    }

    /// Makes the socket leave rooms (never the room of its own id).
    pub async fn leave(&self, rooms: impl IntoRooms) -> Result<(), EmitError> {
        if let Some(socket) = self.local() {
            for room in rooms.into_rooms() {
                socket.leave(&room);
            }
            return Ok(());
        }
        self.operator().sockets_leave(rooms).await
    }

    /// Disconnects the socket from the namespace, and closes its connection with `close`.
    pub async fn disconnect(&self, close: bool) -> Result<(), EmitError> {
        if let Some(socket) = self.local() {
            socket.disconnect(close);
            return Ok(());
        }
        self.operator().disconnect_sockets(close).await
    }
}

impl IoNamespace {
    /// The operator that reaches every socket of the namespace.
    pub fn operator(&self) -> BroadcastOperator {
        BroadcastOperator::new(self.clone())
    }

    /// `nsp.emit(event, ...args)`: sends an event to every socket of the namespace.
    pub async fn emit(&self, event: &str, args: impl IntoArgs) -> Result<(), EmitError> {
        self.operator().emit(event, args).await
    }

    /// `nsp.emit(event, ...args, callback)` with the default deadline: see [`BroadcastOperator::emit_with_ack`].
    pub async fn emit_with_ack(&self, event: &str, args: impl IntoArgs) -> Result<Vec<Args>, BroadcastAckError> {
        self.operator().emit_with_ack(event, args).await
    }

    /// `nsp.to(rooms)`.
    pub fn to(&self, rooms: impl IntoRooms) -> BroadcastOperator {
        self.operator().to(rooms)
    }

    /// `nsp.in(rooms)`.
    pub fn within(&self, rooms: impl IntoRooms) -> BroadcastOperator {
        self.operator().within(rooms)
    }

    /// `nsp.except(rooms)`.
    pub fn except(&self, rooms: impl IntoRooms) -> BroadcastOperator {
        self.operator().except(rooms)
    }

    /// `nsp.local`.
    pub fn local(&self) -> BroadcastOperator {
        self.operator().local()
    }

    /// `nsp.timeout(duration)`.
    pub fn timeout(&self, timeout: Duration) -> BroadcastOperator {
        self.operator().timeout(timeout)
    }

    /// `nsp.fetchSockets()`.
    pub async fn fetch_sockets(&self) -> Result<Vec<RemoteSocket>, EmitError> {
        self.operator().fetch_sockets().await
    }

    /// `nsp.socketsJoin(rooms)`.
    pub async fn sockets_join(&self, rooms: impl IntoRooms) -> Result<(), EmitError> {
        self.operator().sockets_join(rooms).await
    }

    /// `nsp.socketsLeave(rooms)`.
    pub async fn sockets_leave(&self, rooms: impl IntoRooms) -> Result<(), EmitError> {
        self.operator().sockets_leave(rooms).await
    }

    /// `nsp.disconnectSockets(close)`.
    pub async fn disconnect_sockets(&self, close: bool) -> Result<(), EmitError> {
        self.operator().disconnect_sockets(close).await
    }
}

impl IoServer {
    /// `io.emit(event, ...args)`: sends an event to every socket of the main namespace `/`.
    ///
    /// ```no_run
    /// # use corvis::socket_io::IoServer;
    /// # async fn demo(io: IoServer) -> Result<(), Box<dyn std::error::Error>> {
    /// io.emit("tick", (1,)).await?;
    /// io.to("lobby").emit("hello", ("lobby",)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn emit(&self, event: &str, args: impl IntoArgs) -> Result<(), EmitError> {
        self.main().emit(event, args).await
    }

    /// `io.emit` with acknowledgements: see [`BroadcastOperator::emit_with_ack`].
    pub async fn emit_with_ack(&self, event: &str, args: impl IntoArgs) -> Result<Vec<Args>, BroadcastAckError> {
        self.main().emit_with_ack(event, args).await
    }

    /// `io.to(rooms)` on the main namespace.
    pub fn to(&self, rooms: impl IntoRooms) -> BroadcastOperator {
        self.main().to(rooms)
    }

    /// `io.in(rooms)` on the main namespace.
    pub fn within(&self, rooms: impl IntoRooms) -> BroadcastOperator {
        self.main().within(rooms)
    }

    /// `io.except(rooms)` on the main namespace.
    pub fn except(&self, rooms: impl IntoRooms) -> BroadcastOperator {
        self.main().except(rooms)
    }

    /// `io.local` on the main namespace.
    pub fn local(&self) -> BroadcastOperator {
        self.main().local()
    }

    /// `io.timeout(duration)` on the main namespace.
    pub fn timeout(&self, timeout: Duration) -> BroadcastOperator {
        self.main().timeout(timeout)
    }

    /// `io.fetchSockets()` on the main namespace.
    pub async fn fetch_sockets(&self) -> Result<Vec<RemoteSocket>, EmitError> {
        self.main().fetch_sockets().await
    }

    /// `io.socketsJoin(rooms)` on the main namespace.
    pub async fn sockets_join(&self, rooms: impl IntoRooms) -> Result<(), EmitError> {
        self.main().sockets_join(rooms).await
    }

    /// `io.socketsLeave(rooms)` on the main namespace.
    pub async fn sockets_leave(&self, rooms: impl IntoRooms) -> Result<(), EmitError> {
        self.main().sockets_leave(rooms).await
    }

    /// `io.disconnectSockets(close)` on the main namespace.
    pub async fn disconnect_sockets(&self, close: bool) -> Result<(), EmitError> {
        self.main().disconnect_sockets(close).await
    }
}
