#![cfg(feature = "socket-io")]
//! The connect sequence over real transports, on a runtime with four workers: a socket is registered before its client
//! can know its id, nothing precedes CONNECT, no lock of the connection is held across the adapter, CONNECT leaves in a
//! write of its own, and a connection that ends at any step leaves nothing behind. Servers bind to 127.0.0.1.

mod engine_support;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use corvis::socket_io::adapter::{
    AckSink, Adapter, AdapterError, BroadcastOptions, InMemoryAdapter, LocalNamespace, MembershipSize, Packet, Room, RoomError, SocketId,
    SocketInfo,
};
use corvis::socket_io::testing::{ConnectStep, membership_size, pause_connect};
use corvis::socket_io::{EngineOptions, IoOptions};
use engine_support::io_client::{Raw, start};
use engine_support::{eventually, open_polling, packets, poll, post};

/// Room enough for every client of these tests, which all come from 127.0.0.1.
fn options() -> IoOptions {
    IoOptions::new().engine(EngineOptions::new().max_sessions_per_ip(4_096))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_client_finds_its_socket_by_id_as_soon_as_it_reads_its_connect() {
    const CLIENTS: usize = 1_000;
    const AT_ONCE: usize = 50;
    let fx = start(options(), |_| {}).await;
    let misses = Arc::new(AtomicUsize::new(0));
    for _ in 0..CLIENTS / AT_ONCE {
        let wave: Vec<_> = (0..AT_ONCE)
            .map(|_| {
                let (io, misses, addr) = (fx.io.clone(), misses.clone(), fx.addr);
                tokio::spawn(async move {
                    let mut raw = Raw::connect(addr).await;
                    let id = raw.join("/", None).await;
                    // No retry and no wait: the lookup is made the moment the client read its id.
                    if io.main().socket(&id).is_none() {
                        misses.fetch_add(1, Ordering::SeqCst);
                    }
                    raw.close().await;
                })
            })
            .collect();
        for client in wave {
            client.await.unwrap();
        }
    }
    assert_eq!(misses.load(Ordering::SeqCst), 0, "a lookup by id missed the socket of a client that knew its id");
    fx.io.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_reaches_a_client_before_its_connect_while_the_namespace_is_broadcast_to() {
    const CLIENTS: usize = 200;
    let fx = start(options(), |_| {}).await;
    let stop = Arc::new(AtomicBool::new(false));
    let noise = {
        let (io, stop) = (fx.io.clone(), stop.clone());
        tokio::spawn(async move {
            while !stop.load(Ordering::SeqCst) {
                let _ = io.main().emit("noise", ()).await;
                tokio::task::yield_now().await;
            }
        })
    };
    let clients: Vec<_> = (0..CLIENTS)
        .map(|_| {
            let addr = fx.addr;
            tokio::spawn(async move {
                let mut raw = Raw::connect(addr).await;
                // `join` fails unless the first packet for the namespace is CONNECT.
                raw.join("/", None).await;
                raw.close().await;
            })
        })
        .collect();
    for client in clients {
        client.await.unwrap();
    }
    stop.store(true, Ordering::SeqCst);
    noise.await.unwrap();
    fx.io.close();
}

// ------------------------------------------------------------------------------- an adapter that closes the connection

/// The in-memory adapter, except that joining a socket to the room of its own id first closes the socket's
/// connection, as an application adapter may do from `add_all`.
struct ClosingAdapter {
    inner: InMemoryAdapter,
    local: LocalNamespace,
}

#[async_trait]
impl Adapter for ClosingAdapter {
    async fn init(&self) -> Result<(), AdapterError> {
        self.inner.init().await
    }
    async fn close(&self) {
        self.inner.close().await
    }
    async fn server_count(&self) -> Result<usize, AdapterError> {
        self.inner.server_count().await
    }
    fn add_all(&self, id: &str, rooms: &[Room]) -> Result<(), RoomError> {
        if rooms.iter().any(|room| &**room == id)
            && let Some(socket) = self.local.socket(id)
        {
            socket.disconnect(true);
        }
        self.inner.add_all(id, rooms)
    }
    fn del(&self, id: &str, room: &str) {
        self.inner.del(id, room)
    }
    fn del_all(&self, id: &str) {
        self.inner.del_all(id)
    }
    fn socket_rooms(&self, id: &str) -> Vec<Room> {
        self.inner.socket_rooms(id)
    }
    fn room_exists(&self, room: &str) -> bool {
        self.inner.room_exists(room)
    }
    async fn broadcast(&self, packet: Packet, opts: BroadcastOptions) -> Result<(), AdapterError> {
        self.inner.broadcast(packet, opts).await
    }
    async fn broadcast_with_ack(&self, packet: Packet, opts: BroadcastOptions, sink: AckSink) -> Result<(), AdapterError> {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_adapter_that_closes_the_connection_from_add_all_does_not_deadlock_it() {
    let connected = Arc::new(AtomicUsize::new(0));
    let options = options().adapter(|local: LocalNamespace| ClosingAdapter { inner: InMemoryAdapter::new(local.clone()), local });
    let fx = {
        let connected = connected.clone();
        start(options, move |io| {
            io.on_connection(move |_socket| {
                connected.fetch_add(1, Ordering::SeqCst);
            });
        })
        .await
    };
    let mut raw = Raw::connect(fx.addr).await;
    raw.send("0").await;
    assert!(raw.closed_within(engine_support::EVENTUALLY).await, "the connection closes instead of hanging");
    eventually("the socket to be gone", || fx.io.main().socket_count() == 0).await;
    eventually("the session to be gone", || fx.io.engine().session_count() == 0).await;
    assert_eq!(connected.load(Ordering::SeqCst), 0, "no connection handler ran");
    // The server still serves others: the namespace's lock is not held by the attempt that went away.
    assert_eq!(fx.io.main().sockets().len(), 0);
    fx.io.close();
}

// ------------------------------------------------------------------------------- the connection ends at each step

async fn a_connection_that_ends_at(step: ConnectStep) {
    let handled = Arc::new(AtomicUsize::new(0));
    let fx = {
        let handled = handled.clone();
        start(options(), move |io| {
            io.on_connection(move |_socket| {
                handled.fetch_add(1, Ordering::SeqCst);
            });
        })
        .await
    };
    let pause = pause_connect(&fx.io.main(), step).expect("the first pause of the namespace");
    let mut raw = Raw::connect(fx.addr).await;
    raw.send("0").await;
    pause.reached().await;
    raw.close().await;
    eventually("the session to close", || fx.io.engine().session_count() == 0).await;
    if step != ConnectStep::Slotted {
        // In the namespace's table already: the end of the connection takes it out before the attempt goes on.
        eventually("the connection to tear the socket down", || fx.io.main().socket_count() == 0).await;
    }
    pause.release();
    pause.finished().await;
    assert_eq!(handled.load(Ordering::SeqCst), 0, "no handler of the attempt that ended at {step:?} ran");
    assert_eq!(fx.io.main().socket_count(), 0, "no socket is left (at {step:?})");
    let empty = MembershipSize { rooms: 0, sockets: 0, memberships: 0 };
    assert_eq!(membership_size(&fx.io.main()), Some(empty), "no membership is left (at {step:?})");
    fx.io.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connection_that_ends_once_the_socket_has_its_slot_leaves_nothing() {
    a_connection_that_ends_at(ConnectStep::Slotted).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connection_that_ends_once_the_socket_is_registered_leaves_nothing() {
    a_connection_that_ends_at(ConnectStep::Registered).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connection_that_ends_once_the_socket_joined_its_own_room_leaves_nothing() {
    a_connection_that_ends_at(ConnectStep::JoinedOwnRoom).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connection_that_ends_once_connect_was_queued_runs_no_handler() {
    a_connection_that_ends_at(ConnectStep::Announced).await;
}

// ---------------------------------------------------------------------------------- CONNECT in a write of its own

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn over_websocket_the_handlers_event_follows_connect_as_its_own_frame() {
    let fx = start(options().engine(EngineOptions::new().max_sessions_per_ip(4_096).ping_interval(std::time::Duration::from_secs(60))), |io| {
        io.on_connection(|socket| {
            let _ = socket.emit("auth", (serde_json::json!({}),));
        });
    })
    .await;
    for _ in 0..50 {
        let mut raw = Raw::connect(fx.addr).await;
        raw.join("/", None).await;
        assert_eq!(raw.recv_text().await, r#"2["auth",{}]"#);
        raw.close().await;
    }
    fx.io.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn over_polling_connect_is_answered_alone_and_the_handlers_event_in_the_next_poll() {
    let fx = start(options(), |io| {
        io.on_connection(|socket| {
            let _ = socket.emit("auth", (serde_json::json!({}),));
        });
    })
    .await;
    let (sid, _) = open_polling(fx.addr).await;
    // No poll is pending when CONNECT arrives.
    assert_eq!(post(fx.addr, &sid, "40").await.status, 200);
    let first = poll(fx.addr, &sid).await.text();
    let first = packets(&first);
    assert!(matches!(&first[..], [connect] if connect.starts_with("40{\"sid\":")), "the first poll carries CONNECT alone: {first:?}");
    let second = poll(fx.addr, &sid).await.text();
    assert_eq!(packets(&second), [r#"42["auth",{}]"#], "the handler's event comes with the next poll");
    fx.io.close();
}
