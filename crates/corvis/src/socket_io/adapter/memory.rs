//! The default adapter: the rooms of a namespace in two maps behind one lock, delivery in this process.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock, Weak};

use async_trait::async_trait;
use tokio::time::Instant;

use crate::socket_io::engine::session::after;
use crate::socket_io::parser::{self, Body};
use crate::socket_io::socket::{IoSocket, SocketInner};

use super::{AckSink, Adapter, AdapterError, BroadcastOptions, LocalNamespace, Packet, PacketError, Room, RoomError, SocketId, SocketInfo};

/// How many recipients a broadcast delivers before it yields to the runtime, so a very large fan-out does not hold
/// its worker thread for the whole delivery.
const YIELD_EVERY: usize = 512;

/// The two maps of the in-memory adapter: the sockets of each room, and the rooms of each socket. They mirror each
/// other; no room and no socket is ever kept with an empty set.
#[derive(Default)]
pub(crate) struct Membership {
    pub(crate) rooms: HashMap<Room, HashSet<SocketId>>,
    pub(crate) sids: HashMap<SocketId, HashSet<Room>>,
    /// The names the last `add` looked at: the work done under the lock, for the tests.
    #[cfg(test)]
    pub(crate) examined: usize,
}

impl Membership {
    /// Joins `id` to `rooms` (distinct names; the caller deduplicates outside the lock), all or nothing: refused, at
    /// the first name that would take the socket past `max_rooms`, without looking at the rest.
    pub(crate) fn add(&mut self, id: &str, rooms: &[Room], max_rooms: usize) -> Result<(), RoomError> {
        let current = self.sids.get(id);
        let held = current.map_or(0, HashSet::len);
        let mut fresh: Vec<&Room> = Vec::new();
        #[cfg(test)]
        {
            self.examined = 0;
        }
        for room in rooms {
            #[cfg(test)]
            {
                self.examined += 1;
            }
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

    /// Removes every member of the room named `id` other than `id`.
    pub(crate) fn expel_others(&mut self, id: &str) {
        let others: Vec<SocketId> =
            self.rooms.get(id).map(|members| members.iter().filter(|m| &***m != id).cloned().collect()).unwrap_or_default();
        for other in others {
            self.remove(&other, id);
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

/// How big the maps of the in-memory adapter are: for the tests that check nothing is left behind after churn (see
/// `testing::membership_size`).
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

    /// Puts a socket in a room without any of the checks of `add_all`: what a bug elsewhere could do, so the tests
    /// can check that a direct operation reaches one socket whatever the room maps say.
    #[cfg(test)]
    pub(crate) fn force_join(&self, id: &str, room: &str) {
        self.write().add(id, &[Room::from(room)], usize::MAX).unwrap();
    }

    /// The names the last `add_all` examined under the lock.
    #[cfg(test)]
    pub(crate) fn examined(&self) -> usize {
        self.read().examined
    }

    /// The size of the two maps; an error when they do not mirror each other.
    pub(crate) fn size(&self) -> Result<MembershipSize, String> {
        let m = self.read();
        let from_rooms: usize = m.rooms.values().map(HashSet::len).sum();
        let from_sids: usize = m.sids.values().map(HashSet::len).sum();
        if from_rooms != from_sids {
            return Err(format!("the rooms hold {from_rooms} memberships and the sockets {from_sids}"));
        }
        Ok(MembershipSize { rooms: m.rooms.len(), sockets: m.sids.len(), memberships: from_rooms })
    }

    /// The invariants of the two maps (they mirror each other, no empty room or socket).
    pub(crate) fn check_invariants(&self) -> Result<(), String> {
        self.read().check()
    }

    /// Whether the maps hold anything for this socket.
    pub(crate) fn has_socket(&self, id: &str) -> bool {
        self.read().sids.contains_key(id)
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
        let limits = self.local.room_limits();
        limits.check(rooms)?;
        let max_rooms = limits.max_rooms;
        let mut seen: HashSet<&str> = HashSet::with_capacity(rooms.len());
        let mut distinct: Vec<Room> = Vec::with_capacity(rooms.len());
        for room in rooms {
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
        // socket. A live socket is always in the room of its own id (it cannot leave it, and `del_all` runs once it is
        // no longer live), so its id is a key of this adapter's own map: checked there, under the same lock, without
        // looking anything up in the namespace. The one live socket not in it yet is one between its registration and
        // its own room, and joining its own room expels whoever got into it meanwhile (below).
        if distinct.iter().any(|room| &**room != id && m.sids.contains_key(&**room)) {
            return Err(RoomError::SocketIdRoom);
        }
        // The room named like the socket is its own: whoever else got into it before the socket was live (the id is
        // known to the connect middleware) is expelled, under the same lock.
        if distinct.iter().any(|room| &**room == id) {
            m.expel_others(id);
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
        let max_attachments = self.local.options().map_or(crate::socket_io::options::DEFAULT_MAX_ATTACHMENTS, |o| o.max_attachments);
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
        let deadline =
            after(Instant::now(), opts.flags.timeout.unwrap_or(options.ack_timeout).min(crate::socket_io::engine::options::MAX_DURATION));
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
                socket.warn("a socket skipped by sockets_join", || {
                    format!("a socket of the namespace {} was not joined: {error}", self.local.name())
                });
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
