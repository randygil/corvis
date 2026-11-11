//! Regression tests of the first security review of rooms and broadcast (#34), on the paused clock and without a
//! network: a client that joins the room of another socket's id, what a `RemoteSocket` reaches, answers collected by
//! a broadcast against the unread budget, the work a join does under the lock, adapters that panic or never return,
//! empty selections, operators that name too many rooms, adapter close and the order of broadcasts.

use std::collections::HashSet;
use std::sync::atomic::AtomicUsize;

use async_trait::async_trait;

use super::rooms::{connect, received, size};
use super::*;
use crate::socket_io::adapter::Membership;
use crate::socket_io::adapter::{
    AckSink, Adapter, AdapterError, BroadcastOptions, InMemoryAdapter, LocalNamespace, MembershipSize, Packet, Room, SocketId, SocketInfo,
};

// ------------------------------------------------------------------------------------------------ a probe adapter

#[derive(Default)]
struct Knobs {
    panic_del_all: AtomicBool,
    panic_room_exists: AtomicBool,
    panic_add_all: AtomicBool,
    fail_add_all: AtomicBool,
    hang_init: AtomicBool,
    hang_broadcast: AtomicBool,
    /// How many of the next `room_exists` calls say yes.
    rooms_exist: AtomicUsize,
    room_exists_calls: AtomicUsize,
    closes: AtomicUsize,
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
fn probe_server(options: IoOptions) -> (IoServer, Arc<Knobs>, Arc<InMemoryAdapter>) {
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

// ---------------------------------------------------------------------------------------------------------- F1

#[tokio::test(start_paused = true)]
async fn a_client_cannot_join_the_room_of_another_sockets_id() {
    let io = server(options());
    let (x, ix) = connect(&io, "/").await;
    let (v, iv) = connect(&io, "/").await;
    let sx = io.main().socket(&ix).unwrap();
    let sv = io.main().socket(&iv).unwrap();
    assert_eq!(sx.join(iv.as_str()), Err(RoomError::SocketIdRoom));
    assert_eq!(sx.join(["harmless", iv.as_str()]), Err(RoomError::SocketIdRoom), "all or nothing");
    assert_eq!(sx.rooms(), HashSet::from([ix.clone()]));
    // Its own id is not another socket's: allowed, and a no-op.
    sx.join(ix.as_str()).unwrap();
    // The bulk operation skips the one it may not join, and the others take part.
    io.sockets_join(iv.as_str()).await.unwrap();
    assert!(!sx.rooms().contains(&iv), "{:?}", sx.rooms());
    assert!(sv.rooms().contains(&iv));
    for conn in [&x, &v] {
        conn.sent().await;
    }
    io.to(iv.as_str()).emit("private", ()).await.unwrap();
    assert_eq!(received(&[&x, &v], "private").await, [false, true]);
    // A socket cannot leave its own id room either: it is how it is addressed.
    sx.leave(&ix);
    assert!(sx.rooms().contains(&ix));
    io.to(ix.as_str()).emit("own", ()).await.unwrap();
    assert_eq!(received(&[&x, &v], "own").await, [true, false]);
    // A socket that is gone is no longer protected: its id is just a name, but nobody can reach it.
    sv.disconnect(false);
    settle().await;
    assert_eq!(size(&io.main()).sockets, 1);
}

#[tokio::test(start_paused = true)]
async fn no_socket_is_given_an_id_that_is_already_a_room() {
    let (io, knobs, _) = probe_server(options());
    knobs.rooms_exist.store(2, Ordering::SeqCst);
    let (_c, id) = connect(&io, "/").await;
    assert_eq!(knobs.room_exists_calls.load(Ordering::SeqCst), 3, "two ids were refused and a third assigned");
    assert!(io.main().socket(&id).is_some());
}

#[tokio::test(start_paused = true)]
async fn a_remote_socket_reaches_only_its_own_socket_even_when_another_is_in_its_id_room() {
    let (io, _knobs, adapter) = probe_server(options());
    let (x, ix) = connect(&io, "/").await;
    let (v, iv) = connect(&io, "/").await;
    // The hardening bypassed (a bug elsewhere): the attacker is in the victim's id room.
    adapter.force_join(&ix, &iv);
    let victim = io.fetch_sockets().await.unwrap().into_iter().find(|s| s.id() == iv).unwrap();
    for conn in [&x, &v] {
        conn.sent().await;
    }
    // The four effects, directly on the local socket.
    victim.emit("dm", ()).await.unwrap();
    assert_eq!(received(&[&x, &v], "dm").await, [false, true]);
    victim.join("admins").await.unwrap();
    assert!(io.main().socket(&iv).unwrap().rooms().contains("admins"));
    assert!(!io.main().socket(&ix).unwrap().rooms().contains("admins"), "the attacker did not become an admin");
    victim.leave("admins").await.unwrap();
    assert!(!io.main().socket(&iv).unwrap().rooms().contains("admins"));
    // The same through the adapter, addressed by id as a single-socket target (what a remote socket does).
    victim.operator().emit("dm2", ()).await.unwrap();
    assert_eq!(received(&[&x, &v], "dm2").await, [false, true]);
    victim.operator().sockets_join("admins2").await.unwrap();
    assert!(io.main().socket(&iv).unwrap().rooms().contains("admins2"));
    assert!(!io.main().socket(&ix).unwrap().rooms().contains("admins2"));
    assert_eq!(victim.operator().fetch_sockets().await.unwrap().len(), 1);
    victim.operator().disconnect_sockets(false).await.unwrap();
    settle().await;
    assert!(io.main().socket(&iv).is_none(), "the victim left");
    assert!(io.main().socket(&ix).is_some(), "the attacker was not disconnected with it");
    // And the direct path.
    let other = io.fetch_sockets().await.unwrap().into_iter().find(|s| s.id() == ix).unwrap();
    other.disconnect(false).await.unwrap();
    settle().await;
    assert_eq!(io.main().socket_count(), 0);
}

// ---------------------------------------------------------------------------------------------------------- F2

/// The answer of a recipient to the broadcast with acknowledgement id `id`.
fn answer(id: u64, text: &str) -> String {
    format!(r#"3{id}["{text}"]"#)
}

#[tokio::test(start_paused = true)]
async fn the_answers_a_broadcast_holds_are_charged_to_the_call_and_never_to_the_answering_connection() {
    let io = server(options());
    let mut conns = Vec::new();
    for _ in 0..3 {
        conns.push(connect(&io, "/").await.0);
    }
    let call = |io: &IoServer, seconds: u64| {
        let io = io.clone();
        tokio::spawn(async move { io.timeout(Duration::from_secs(seconds)).emit_with_ack("q", ()).await })
    };
    let big = "x".repeat(100_000);
    let engine = io.engine().clone();
    let held = || io.main().ack_budget().used();
    assert_eq!(engine.total_unread_bytes(), 0);
    assert_eq!(held(), 0);

    // 1. One answers, one is silent: the answer is held by the call, and the connection's budget, its address's and
    // the server's are free again the moment the call has it.
    let task = call(&io, 5);
    settle().await;
    conns[0].sent().await;
    conns[0].text(&answer(0, &big));
    settle().await;
    assert!(held() >= 100_000, "held by the call: {}", held());
    assert_eq!(engine.total_unread_bytes(), 0, "not on the server-wide unread gauge");
    assert_eq!(engine.address_unread_bytes("127.0.0.1"), 0, "not on the address");
    assert_eq!(conns[0].socket.unread_bytes(), 0, "not on the session");
    advance(5_000).await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(*error.kind(), BroadcastAckErrorKind::Timeout);
    assert_eq!(error.responses().len(), 1);
    drop(error);
    assert_eq!(held(), 0, "released when the call ended");

    // 2. The answering connection closes while its answer is held: nothing of it remains on the connection side,
    // and the call keeps the answer until it ends.
    let task = call(&io, 120);
    settle().await;
    conns[0].text(&answer(1, &big));
    settle().await;
    assert!(held() >= 100_000);
    conns[0].socket.close();
    advance(31_000).await; // the close timeout of the session: no transport drains it here
    assert_eq!(io.main().socket_count(), 2, "the answering socket is gone");
    assert!(held() >= 100_000, "the call still holds the answer");
    assert_eq!(engine.total_unread_bytes(), 0);
    conns[1].text(&answer(1, "b"));
    conns[2].text(&answer(1, "c"));
    settle().await;
    let answers = task.await.unwrap().unwrap();
    assert_eq!(answers.len(), 3);
    drop(answers);
    settle().await;
    assert_eq!(held(), 0);
    assert_eq!(engine.total_unread_bytes(), 0);

    // 3. A call dropped before its end releases what it holds.
    let (_d, _) = connect(&io, "/").await;
    let task = call(&io, 5);
    settle().await;
    conns[1].text(&answer(2, &big));
    settle().await;
    assert!(held() >= 100_000);
    task.abort();
    let _ = task.await;
    settle().await;
    assert_eq!(held(), 0);
    assert_eq!(engine.total_unread_bytes(), 0);
}

#[test]
fn the_answers_of_all_calls_together_are_bounded_by_the_server_answer_budget_without_waiting() {
    use crate::socket_io::adapter::{AckBudget, AckShared};
    let budget = AckBudget::new(1_000);
    let one = AckShared::new(10_000, budget.clone());
    let two = AckShared::new(10_000, budget.clone());
    let (a, b) = (AckSink::new(one.clone()), AckSink::new(two.clone()));
    let answer = |n: usize| Args::new(vec![IoValue::String("x".repeat(n))]);
    a.respond(answer(300));
    assert!(budget.used() >= 300 && budget.used() <= 1_000);
    // The second call finds no room: nothing waits, it keeps what it had (nothing).
    b.respond(answer(900));
    assert_eq!(two.finish().len(), 0);
    // A call that finished frees its room for the others.
    assert_eq!(one.finish().len(), 1);
    assert_eq!(budget.used(), 0);
    let three = AckShared::new(10_000, budget.clone());
    AckSink::new(three.clone()).respond(answer(600));
    assert_eq!(three.finish().len(), 1);
    assert_eq!(budget.used(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_call_that_finds_no_room_in_the_answer_budget_ends_with_responses_too_large() {
    let io = server(options());
    let (a, _) = connect(&io, "/").await;
    let (_b, _) = connect(&io, "/").await;
    // Another call holds nearly the whole budget.
    let budget = io.main().ack_budget().clone();
    let cap = options().ack_budget();
    let full = crate::socket_io::adapter::AckShared::new(cap, budget.clone());
    AckSink::new(full.clone()).respond(Args::new(vec![IoValue::String("x".repeat(cap - 1_000))]));
    let task = tokio::spawn({
        let io = io.clone();
        async move { io.timeout(Duration::from_secs(5)).emit_with_ack("q", ()).await }
    });
    settle().await;
    a.text(&answer(0, &"y".repeat(10_000)));
    settle().await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(*error.kind(), BroadcastAckErrorKind::ResponsesTooLarge);
    assert_eq!(io.engine().total_unread_bytes(), 0, "the answerer's unread budget is not what ran out");
    drop(error);
    full.finish();
    assert_eq!(budget.used(), 0);
}

#[tokio::test(start_paused = true)]
async fn answers_dropped_by_the_cap_release_their_charge_too() {
    let io = server(options().max_broadcast_ack_bytes(150_000));
    let mut conns = Vec::new();
    for _ in 0..4 {
        conns.push(connect(&io, "/").await.0);
    }
    let task = tokio::spawn({
        let io = io.clone();
        async move { io.timeout(Duration::from_secs(5)).emit_with_ack("q", ()).await }
    });
    settle().await;
    let big = "x".repeat(100_000);
    conns[0].text(&answer(0, &big));
    settle().await;
    conns[1].text(&answer(0, &big));
    settle().await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(*error.kind(), BroadcastAckErrorKind::ResponsesTooLarge);
    drop(error);
    settle().await;
    assert_eq!(io.engine().total_unread_bytes(), 0);
    assert_eq!(io.engine().address_unread_bytes("127.0.0.1"), 0);
}

// ---------------------------------------------------------------------------------------------------------- B1

#[test]
fn a_join_stops_at_the_first_name_that_passes_the_cap() {
    let mut m = Membership::default();
    let rooms = |n: usize| -> Vec<Room> { (0..n).map(|i| Room::from(format!("r{i}"))).collect() };
    m.add("s", &rooms(4), 5).unwrap();
    assert_eq!(m.examined, 4);
    // Four held, room for one more: the second new name is the first that passes the cap.
    let more: Vec<Room> = (100..10_100).map(|i| Room::from(format!("n{i}"))).collect();
    assert_eq!(m.add("s", &more, 5), Err(RoomError::TooManyRooms { max: 5 }));
    assert_eq!(m.examined, 2, "ten thousand names, two examined");
    assert_eq!(m.sids["s"].len(), 4, "nothing joined");
    m.check().unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_join_naming_more_rooms_than_the_cap_is_refused_before_the_lock() {
    let (io, _knobs, adapter) = probe_server(options().max_rooms_per_socket(50));
    let (_c, id) = connect(&io, "/").await;
    let socket = io.main().socket(&id).unwrap();
    let before = adapter.examined();
    assert_eq!(before, 1, "the room of its own id");
    let names: Vec<String> = (0..200_000).map(|i| format!("room-{i}")).collect();
    assert_eq!(socket.join(&names), Err(RoomError::TooManyRooms { max: 50 }));
    assert_eq!(io.sockets_join(&names).await, Err(EmitError::InvalidRoom(RoomError::TooManyRooms { max: 50 })));
    assert_eq!(adapter.examined(), before, "no name was examined under the lock");
    // At the adapter itself too (a trait caller that skips the front doors).
    let rooms: Vec<Room> = names.iter().map(|n| Room::from(n.as_str())).collect();
    assert_eq!(io.main().adapter().add_all(&id, &rooms), Err(RoomError::TooManyRooms { max: 50 }));
    assert_eq!(adapter.examined(), before);
    // Duplicates are folded outside the lock: 50 copies of one name examine one name.
    socket.join(vec!["same"; 50]).unwrap();
    assert_eq!(adapter.examined(), 1);
    assert_eq!(socket.rooms().len(), 2);
}

// ---------------------------------------------------------------------------------------------------------- B3

#[tokio::test(start_paused = true)]
async fn fetch_sockets_shares_the_handshake_of_a_local_socket() {
    let io = server(options());
    let conn = io.conn();
    let id = conn.join(r#"0{"token":"SECRET-AUTH-VALUE"}"#).await;
    let socket = io.main().socket(&id).unwrap();
    let fetched = io.fetch_sockets().await.unwrap();
    assert_eq!(fetched.len(), 1);
    let info = fetched[0].handshake();
    assert!(std::ptr::eq(info.auth(), socket.handshake().auth()), "the auth object is shared, not copied");
    assert_eq!(info.auth()["token"], "SECRET-AUTH-VALUE");
    // It still serializes as plain data and reads back equal.
    let wire = serde_json::to_string(fetched[0].info()).unwrap();
    let back: SocketInfo = serde_json::from_str(&wire).unwrap();
    assert_eq!(&back, fetched[0].info());
    assert_eq!(back.handshake.auth()["token"], "SECRET-AUTH-VALUE");
}

#[tokio::test(start_paused = true)]
async fn debug_output_hides_credentials() {
    let io = server(options());
    let conn = io.conn();
    let id = conn.join(r#"0{"token":"SECRET-AUTH-VALUE"}"#).await;
    let fetched = io.fetch_sockets().await.unwrap();
    for text in [format!("{:?}", fetched[0]), format!("{:?}", fetched[0].info()), format!("{:?}", fetched[0].handshake())] {
        assert!(!text.contains("SECRET-AUTH-VALUE"), "{text}");
        assert!(text.contains(&id) || !text.contains("RemoteSocket") || text.contains("id"), "{text}");
    }
    assert!(format!("{:?}", fetched[0].handshake()).contains("token"), "the key is shown");
    // A handshake described by another process.
    let remote = HandshakeInfo::new(
        json!({ "token": "SECRET-AUTH-VALUE" }),
        vec![
            ("cookie".into(), "session=SECRET-COOKIE".into()),
            ("Authorization".into(), "Bearer SECRET-BEARER".into()),
            ("proxy-authorization".into(), "Basic SECRET-PROXY".into()),
            ("x-api-key".into(), "SECRET-APIKEY".into()),
            ("X-Auth-Token".into(), "SECRET-TOKEN".into()),
            ("x-custom-credential".into(), "SECRET-CUSTOM".into()),
            ("proxy-authorization ".into(), "SECRET-SPACE".into()),
            ("user-agent".into(), "tests".into()),
        ],
        "/socket.io/?EIO=4&token=SECRET-QUERY".into(),
        "10.0.0.1".into(),
        1,
    );
    let text = format!("{remote:?}");
    for secret in [
        "SECRET-AUTH-VALUE",
        "SECRET-COOKIE",
        "SECRET-BEARER",
        "SECRET-PROXY",
        "SECRET-QUERY",
        "SECRET-APIKEY",
        "SECRET-TOKEN",
        "SECRET-CUSTOM",
        "SECRET-SPACE",
    ] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
    for shown in ["cookie", "Authorization", "user-agent: tests", "/socket.io/", "10.0.0.1"] {
        assert!(text.contains(shown), "{shown} missing from {text}");
    }
}

// ---------------------------------------------------------------------------------------------------------- B4

#[tokio::test(start_paused = true)]
async fn a_panic_in_the_adapter_on_disconnect_does_not_leak_the_socket() {
    let (io, knobs, _) = probe_server(options());
    let conn = io.conn();
    let id = conn.join(r#"0{"token":"a token that is charged"}"#).await;
    assert!(io.main().socket(&id).is_some());
    assert!(io.engine().total_unread_bytes() > 0, "the handshake auth is charged");
    knobs.panic_del_all.store(true, Ordering::SeqCst);
    conn.socket.close();
    advance(31_000).await; // the close timeout of the session: no transport drains it here
    assert_eq!(io.main().socket_count(), 0, "the socket left the namespace");
    assert_eq!(io.engine().total_unread_bytes(), 0, "its charges were released");
    drop(conn);
}

#[tokio::test(start_paused = true)]
async fn an_adapter_that_never_returns_fails_emit_at_the_adapter_timeout() {
    let (io, knobs, _) = probe_server(options().adapter_timeout(Duration::from_secs(5)));
    let (c, _) = connect(&io, "/").await;
    knobs.hang_broadcast.store(true, Ordering::SeqCst);
    let start = tokio::time::Instant::now();
    assert_eq!(io.emit("e", ()).await, Err(EmitError::Adapter(AdapterError::Timeout)));
    assert_eq!(start.elapsed(), Duration::from_secs(5), "on the paused clock the deadline is exact");
    let also = io.to("r").emit("e", ()).await;
    assert_eq!(also, Err(EmitError::Adapter(AdapterError::Timeout)));
    assert_eq!(
        io.timeout(Duration::from_secs(60)).emit_with_ack("e", ()).await.unwrap_err().kind(),
        &BroadcastAckErrorKind::Emit(EmitError::Adapter(AdapterError::Timeout))
    );
    knobs.hang_broadcast.store(false, Ordering::SeqCst);
    io.emit("later", ()).await.unwrap();
    assert!(received(&[&c], "later").await[0]);
}

#[tokio::test(start_paused = true)]
async fn an_adapter_whose_init_never_returns_fails_the_call_and_is_tried_again() {
    let (io, knobs, _) = probe_server(options().adapter_timeout(Duration::from_secs(2)));
    let (c, _) = connect(&io, "/").await;
    knobs.hang_init.store(true, Ordering::SeqCst);
    assert_eq!(io.emit("e", ()).await, Err(EmitError::Adapter(AdapterError::Timeout)));
    assert!(io.fetch_sockets().await.is_err());
    knobs.hang_init.store(false, Ordering::SeqCst);
    io.emit("ok", ()).await.unwrap();
    assert!(received(&[&c], "ok").await[0]);
}

// ---------------------------------------------------------------------------------------------------------- B5

#[tokio::test(start_paused = true)]
async fn an_empty_list_of_rooms_selects_nobody_and_no_to_at_all_selects_everybody() {
    let io = server(options());
    let (a, ia) = connect(&io, "/").await;
    let (b, ib) = connect(&io, "/").await;
    io.main().socket(&ia).unwrap().join("r").unwrap();
    let all = [&a, &b];
    for conn in all {
        conn.sent().await;
    }
    let empty: Vec<String> = Vec::new();
    io.to(empty.clone()).emit("e", ()).await.unwrap();
    io.within(empty.clone()).emit("e", ()).await.unwrap();
    io.to(empty.clone()).except("r").emit("e", ()).await.unwrap();
    assert_eq!(received(&all, "e").await, [false, false]);
    assert_eq!(io.to(empty.clone()).emit_with_ack("e", ()).await.unwrap().len(), 0);
    assert!(io.to(empty.clone()).fetch_sockets().await.unwrap().is_empty());
    io.to(empty.clone()).sockets_join("x").await.unwrap();
    io.to(empty.clone()).disconnect_sockets(false).await.unwrap();
    assert_eq!(io.main().socket_count(), 2, "nobody was disconnected");
    assert!(!io.main().socket(&ib).unwrap().rooms().contains("x"));
    // A later `to` with rooms adds to the (empty) union.
    io.to(empty.clone()).to("r").emit("e", ()).await.unwrap();
    assert_eq!(received(&all, "e").await, [true, false]);
    // `socket.to(empty)` selects nobody either.
    io.main().socket(&ia).unwrap().to(empty.clone()).emit("e", ()).await.unwrap();
    assert_eq!(received(&all, "e").await, [false, false]);
    // `except` with nothing has no effect, and no `to` at all is everyone.
    io.except(empty.clone()).emit("e", ()).await.unwrap();
    assert_eq!(received(&all, "e").await, [true, true]);
    io.emit("e", ()).await.unwrap();
    assert_eq!(received(&all, "e").await, [true, true]);
}

// ----------------------------------------------------------------------------------------------------- the notes

#[tokio::test(start_paused = true)]
async fn an_operator_names_at_most_as_many_rooms_as_a_socket_may_be_in() {
    let io = server(options().max_rooms_per_socket(5));
    let (a, ia) = connect(&io, "/").await;
    io.main().socket(&ia).unwrap().join("a").unwrap();
    a.sent().await;
    let too_many = EmitError::InvalidRoom(RoomError::TooManyTargets { max: 5 });
    assert_eq!(io.to(["a", "b", "c"]).to(["d", "e", "f"]).emit("e", ()).await, Err(too_many.clone()));
    assert_eq!(io.to(["a", "b", "c"]).except(["d", "e", "f"]).emit("e", ()).await, Err(too_many.clone()));
    assert_eq!(io.to(["a", "b", "c", "d", "e", "f"]).fetch_sockets().await.unwrap_err(), too_many);
    assert!(a.sent().await.is_empty(), "nothing was sent");
    // Up to the cap, one call at a time or in one, works; the base operator is not changed by a failing branch.
    let base = io.to("a");
    let failing = base.to(["b", "c", "d", "e", "f"]);
    assert!(failing.emit("e", ()).await.is_err());
    base.emit("e", ()).await.unwrap();
    io.to(["a", "b"]).to("c").to("d").to("e").emit("e", ()).await.unwrap();
    assert_eq!(received(&[&a], "e").await, [true]);
}

#[tokio::test(start_paused = true)]
async fn broadcasting_from_a_socket_does_not_use_up_the_rooms_an_operator_may_name() {
    let io = server(options().max_rooms_per_socket(1));
    let (_a, ia) = connect(&io, "/").await;
    let (b, ib) = connect(&io, "/").await;
    b.sent().await;
    // One room named: the room of `b`'s id. The exclusion of the sender is not counted against the cap.
    io.main().socket(&ia).unwrap().to(ib.as_str()).emit("e", ()).await.unwrap();
    assert!(received(&[&b], "e").await[0]);
}

#[tokio::test(start_paused = true)]
async fn closing_the_server_closes_each_adapter_once() {
    let (io, knobs, _) = probe_server(options());
    io.of("/other").unwrap();
    let (_c, _) = connect(&io, "/").await;
    io.close();
    settle().await;
    assert_eq!(knobs.closes.load(Ordering::SeqCst), 2, "one close for each of the two namespaces, not two each");
}

#[tokio::test(start_paused = true)]
async fn two_broadcasts_of_one_caller_reach_every_recipient_in_order_whatever_runs_beside_them() {
    // More recipients than one delivery slice (512), so a broadcast yields in the middle of its delivery.
    const RECIPIENTS: usize = 1_300;
    const ROUNDS: usize = 20;
    let io = server(options());
    let mut conns = Vec::new();
    for n in 0..RECIPIENTS {
        // An address holds 256 sessions: spread them.
        let conn = io.conn_from(&format!("10.0.{}.{}", n / 200, n % 200 + 1));
        let id = conn.join("0").await;
        io.main().socket(&id).unwrap().join("r").unwrap();
        conn.sent().await;
        conns.push(conn);
    }
    // Another caller broadcasts to the same sockets meanwhile; there is no order between its events and ours.
    let noise = tokio::spawn({
        let io = io.clone();
        async move {
            for n in 0..ROUNDS * 2 {
                io.to("r").emit("noise", (n,)).await.unwrap();
            }
        }
    });
    for n in 0..ROUNDS {
        io.to("r").emit("seq", (n,)).await.unwrap();
    }
    noise.await.unwrap();
    for (i, conn) in conns.iter().enumerate() {
        let sent = conn.sent().await;
        let seen: Vec<usize> = sent
            .into_iter()
            .filter_map(|m| match m {
                Out::T(s) => s.strip_prefix("2[\"seq\",").and_then(|rest| rest.trim_end_matches(']').parse().ok()),
                Out::B(_) => None,
            })
            .collect();
        assert_eq!(seen, (0..ROUNDS).collect::<Vec<_>>(), "recipient {i}");
    }
}

// ------------------------------------------------------------------------------------------------------ round 2

#[tokio::test(start_paused = true)]
async fn a_panic_in_room_exists_during_connect_refuses_the_attempt_and_leaves_no_session_behind() {
    let (io, knobs, _) = probe_server(options());
    knobs.panic_room_exists.store(true, Ordering::SeqCst);
    let conn = io.conn();
    conn.text("0");
    assert_eq!(conn.sent().await, [t(r#"4{"message":"Internal server error"}"#)]);
    assert_eq!(io.main().socket_count(), 0);
    assert_eq!(io.engine().total_unread_bytes(), 0, "the handshake charge went with the attempt");
    // The slot is free: the client may try again, and once the adapter works the attempt succeeds.
    knobs.panic_room_exists.store(false, Ordering::SeqCst);
    let id = conn.join("0").await;
    assert!(io.main().socket(&id).is_some());
}

/// The own room of a connecting socket could not be joined (the adapter panicked, or refused): the attempt is refused
/// with `Internal server error` before any CONNECT, nothing of the socket stays, and the connection stays open, so the
/// client may try again.
async fn an_own_room_failure_refuses_the_attempt(knob: impl Fn(&Knobs) -> &AtomicBool) {
    let (io, knobs, adapter) = probe_server(options());
    let connected = Arc::new(AtomicUsize::new(0));
    {
        let connected = connected.clone();
        io.on_connection(move |_socket| {
            connected.fetch_add(1, Ordering::SeqCst);
        });
    }
    knob(&knobs).store(true, Ordering::SeqCst);
    let conn = io.conn();
    let before = io.engine().total_unread_bytes();
    conn.text(r#"0{"token":"a token that is charged"}"#);
    assert_eq!(conn.sent().await, [t(r#"4{"message":"Internal server error"}"#)], "CONNECT_ERROR and no CONNECT");
    assert!(!conn.socket.is_closing(), "the connection stays open");
    assert_eq!(io.main().socket_count(), 0, "no socket stays in the namespace");
    assert_eq!(adapter.size(), Ok(MembershipSize { rooms: 0, sockets: 0, memberships: 0 }), "nor in the adapter");
    assert!(!crate::socket_io::testing::adapter_has_socket(&adapter, "anyone"));
    assert_eq!(io.engine().total_unread_bytes(), before, "its charges were released");
    assert_eq!(connected.load(Ordering::SeqCst), 0, "no connection handler ran");
    // The slot is free: once the adapter works the client joins.
    knob(&knobs).store(false, Ordering::SeqCst);
    let id = conn.join("0").await;
    assert!(io.main().socket(&id).is_some());
    assert_eq!(connected.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn a_panic_in_add_all_of_the_own_room_during_connect_refuses_the_attempt_and_leaves_nothing_behind() {
    an_own_room_failure_refuses_the_attempt(|knobs| &knobs.panic_add_all).await;
}

#[tokio::test(start_paused = true)]
async fn the_own_room_of_a_connecting_socket_expels_whoever_joined_its_id_during_the_middleware() {
    let io = server(options());
    let (x, ix) = connect(&io, "/").await;
    let attacker = io.main().socket(&ix).unwrap();
    // The middleware of the next connection leaks the id the socket was just given (a log line, a "connecting"
    // message): the attacker joins that name while it is not yet the id of a live socket.
    let joined = log();
    {
        let (attacker, joined) = (attacker.clone(), joined.clone());
        io.use_middleware(move |socket| {
            let (attacker, joined) = (attacker.clone(), joined.clone());
            async move {
                joined.lock().unwrap().push(format!("{:?}", attacker.join(socket.id())));
                Ok(())
            }
        });
    }
    let (v, iv) = connect(&io, "/").await;
    assert_eq!(entries(&joined), ["Ok(())"], "the join of a name that is not yet an id goes through");
    assert!(!attacker.rooms().contains(&iv), "but the socket's own room expelled the attacker: {:?}", attacker.rooms());
    crate::socket_io::testing::check_invariants(&io.main()).unwrap();
    io.to(iv.as_str()).emit("private", ()).await.unwrap();
    assert_eq!(received(&[&x, &v], "private").await, [false, true]);
}

// ------------------------------------------------------------------------------------------------------ round 3

/// The ack id of the first `q` event in what a connection was sent.
fn question_id(sent: &[Out]) -> u64 {
    for out in sent {
        if let Out::T(text) = out
            && let Some(rest) = text.strip_prefix('2')
        {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if rest[digits.len()..].starts_with("[\"q\"") {
                return digits.parse().unwrap();
            }
        }
    }
    panic!("no question in {sent:?}")
}

#[tokio::test(start_paused = true)]
async fn one_address_cannot_fill_the_answer_budget_and_make_the_calls_of_others_fail() {
    const MIB: usize = 1 << 20;
    let io = server(
        IoOptions::new()
            .max_decoded_bytes(MIB)
            .max_broadcast_ack_bytes(MIB)
            .engine(EngineOptions::new().ping_interval(DAY).ping_timeout(DAY).max_total_unread_bytes(16 * MIB)),
    );
    let budget = io.main().ack_budget().clone();
    // The attacker (one address) has a socket that answers and a silent one in the same room; honest members are on
    // two other addresses.
    let attacker = io.conn_from("203.0.113.1");
    let ia = attacker.join("0").await;
    let silent = io.conn_from("203.0.113.1");
    let isilent = silent.join("0").await;
    let mut honest: Vec<(Conn, String)> = Vec::new();
    for address in ["203.0.113.2", "203.0.113.3"] {
        let c = io.conn_from(address);
        let id = c.join("0").await;
        honest.push((c, id));
    }
    for id in [&ia, &isilent] {
        io.main().socket(id).unwrap().join("r1").unwrap();
    }
    for (_, id) in &honest {
        io.main().socket(id).unwrap().join("r2").unwrap();
    }
    let calls = 40;
    let mut tasks = Vec::new();
    for _ in 0..calls {
        let io = io.clone();
        tasks.push(tokio::spawn(async move { io.to("r1").timeout(Duration::from_secs(60)).emit_with_ack("q", ()).await }));
        settle().await;
    }
    attacker.sent().await;
    let big = "x".repeat(100_000);
    for id in 0..calls as u64 {
        attacker.text(&answer(id, &big));
        settle().await;
    }
    // 40 answers of 100 kB would be 4 MB, the whole budget (4 MiB here): the address share stops it far before.
    assert!(budget.used() <= budget.address_share(), "{} held, share {}", budget.used(), budget.address_share());
    assert_eq!(budget.address_entries(), 1);
    // An honest call, answered from other addresses, completes.
    let honest_call = {
        let io = io.clone();
        tokio::spawn(async move { io.to("r2").timeout(Duration::from_secs(5)).emit_with_ack("q", ()).await })
    };
    settle().await;
    for (conn, _) in &honest {
        let id = question_id(&conn.sent().await);
        conn.text(&answer(id, "ok"));
    }
    settle().await;
    let answers = honest_call.await.unwrap().expect("the honest call completes");
    assert_eq!(answers.len(), 2);
    drop(answers);
    // The calls of the attacker end (some with too much, the others at the deadline) and the map empties.
    advance(61_000).await;
    let mut too_large = 0;
    for task in tasks {
        if let Err(e) = task.await.unwrap()
            && *e.kind() == BroadcastAckErrorKind::ResponsesTooLarge
        {
            too_large += 1;
        }
    }
    assert!(too_large > 0, "the answers over the share ended their calls with ResponsesTooLarge");
    assert_eq!(budget.used(), 0);
    assert_eq!(budget.address_entries(), 0, "an address with nothing held is forgotten");
}

#[tokio::test(start_paused = true)]
async fn an_error_from_add_all_of_the_own_room_during_connect_refuses_the_attempt_like_a_panic() {
    an_own_room_failure_refuses_the_attempt(|knobs| &knobs.fail_add_all).await;
}
