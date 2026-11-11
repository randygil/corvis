//! A test adapter: the in-memory adapter behind knobs that make it panic, fail or never return.

use std::collections::HashSet;
use std::sync::atomic::AtomicUsize;

use async_trait::async_trait;

use super::super::*;
use crate::socket_io::adapter::{
    AckSink, Adapter, AdapterError, BroadcastOptions, InMemoryAdapter, LocalNamespace, Packet, Room, SocketId, SocketInfo,
};

#[derive(Default)]
pub(super) struct Knobs {
    pub(super) panic_del_all: AtomicBool,
    pub(super) panic_room_exists: AtomicBool,
    pub(super) panic_add_all: AtomicBool,
    pub(super) fail_add_all: AtomicBool,
    pub(super) hang_init: AtomicBool,
    pub(super) hang_broadcast: AtomicBool,
    /// How many of the next `room_exists` calls say yes.
    pub(super) rooms_exist: AtomicUsize,
    pub(super) room_exists_calls: AtomicUsize,
    pub(super) closes: AtomicUsize,
}

/// The in-memory adapter, with faults on demand.
struct Probe {
    inner: Arc<InMemoryAdapter>,
    knobs: Arc<Knobs>,
}

#[async_trait]
impl Adapter for Probe {
    async fn init(&self) -> Result<(), AdapterError> {
        if self.knobs.hang_init.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        self.inner.init().await
    }
    async fn close(&self) {
        self.knobs.closes.fetch_add(1, Ordering::SeqCst);
        self.inner.close().await
    }
    async fn server_count(&self) -> Result<usize, AdapterError> {
        self.inner.server_count().await
    }
    fn add_all(&self, id: &str, rooms: &[Room]) -> Result<(), RoomError> {
        if self.knobs.panic_add_all.load(Ordering::SeqCst) {
            panic!("the adapter failed in add_all");
        }
        if self.knobs.fail_add_all.load(Ordering::SeqCst) {
            return Err(RoomError::TooManyRooms { max: 0 });
        }
        self.inner.add_all(id, rooms)
    }
    fn del(&self, id: &str, room: &str) {
        self.inner.del(id, room)
    }
    fn del_all(&self, id: &str) {
        if self.knobs.panic_del_all.load(Ordering::SeqCst) {
            panic!("the adapter failed");
        }
        self.inner.del_all(id)
    }
    fn socket_rooms(&self, id: &str) -> Vec<Room> {
        self.inner.socket_rooms(id)
    }
    fn room_exists(&self, room: &str) -> bool {
        if self.knobs.panic_room_exists.load(Ordering::SeqCst) {
            panic!("the adapter failed in room_exists");
        }
        self.knobs.room_exists_calls.fetch_add(1, Ordering::SeqCst);
        let left = self.knobs.rooms_exist.load(Ordering::SeqCst);
        if left > 0 {
            self.knobs.rooms_exist.store(left - 1, Ordering::SeqCst);
            return true;
        }
        self.inner.room_exists(room)
    }
    async fn broadcast(&self, packet: Packet, opts: BroadcastOptions) -> Result<(), AdapterError> {
        if self.knobs.hang_broadcast.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        self.inner.broadcast(packet, opts).await
    }
    async fn broadcast_with_ack(&self, packet: Packet, opts: BroadcastOptions, sink: AckSink) -> Result<(), AdapterError> {
        if self.knobs.hang_broadcast.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        self.inner.broadcast_with_ack(packet, opts, sink).await
    }
    async fn sockets(&self, rooms: &[Room]) -> Result<HashSet<SocketId>, AdapterError> {
        self.inner.sockets(rooms).await
    }
    async fn fetch_sockets(&self, opts: BroadcastOptions) -> Result<Vec<SocketInfo>, AdapterError> {
        self.inner.fetch_sockets(opts).await
    }
    async fn add_sockets(&self, opts: BroadcastOptions, rooms: Vec<Room>) -> Result<(), AdapterError> {
        self.inner.add_sockets(opts, rooms).await
    }
    async fn del_sockets(&self, opts: BroadcastOptions, rooms: Vec<Room>) -> Result<(), AdapterError> {
        self.inner.del_sockets(opts, rooms).await
    }
    async fn disconnect_sockets(&self, opts: BroadcastOptions, close: bool) -> Result<(), AdapterError> {
        self.inner.disconnect_sockets(opts, close).await
    }
}

/// A server whose main namespace runs a [`Probe`], with the knobs and the adapter it wraps.
pub(super) fn probe_server(options: IoOptions) -> (IoServer, Arc<Knobs>, Arc<InMemoryAdapter>) {
    let knobs = Arc::new(Knobs::default());
    let slot: Arc<Mutex<Option<Arc<InMemoryAdapter>>>> = Arc::new(Mutex::new(None));
    let io = {
        let (knobs, slot) = (knobs.clone(), slot.clone());
        server(options.adapter(move |local: LocalNamespace| {
            let inner = Arc::new(InMemoryAdapter::new(local));
            *slot.lock().unwrap() = Some(inner.clone());
            Probe { inner, knobs: knobs.clone() }
        }))
    };
    let inner = slot.lock().unwrap().clone().expect("the main namespace has its adapter");
    (io, knobs, inner)
}
