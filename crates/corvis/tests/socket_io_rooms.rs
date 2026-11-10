#![cfg(feature = "socket-io")]
//! Rooms and broadcasts over real transports, with several clients, once with the default adapter and once with a
//! test adapter that wraps it, yields on every cluster operation, can fail one, and reports a second, simulated
//! server (it uses the public API of the adapter only). Absence of delivery is observed through a marker event that
//! the server emits to everyone after the broadcast, never by waiting.

mod engine_support;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use corvis::socket_io::adapter::{
    AckSink, Adapter, AdapterError, BroadcastOptions, InMemoryAdapter, LocalNamespace, MembershipSize, Packet, Room, SocketId, SocketInfo,
};
use corvis::socket_io::{Args, Binary, BroadcastAckErrorKind, EmitError, IoOptions, IoServer, IoValue};
use engine_support::io_client::{IoFixture, Msg, Raw, start};
use engine_support::{open_polling, poll, post};
use tokio::sync::Notify;

// ------------------------------------------------------------------------------------------- the test adapter

#[derive(Default)]
struct Knobs {
    /// Fail the next `broadcast` and `broadcast_with_ack` with this error.
    fail: AtomicBool,
    /// The simulated second server reports only after `release` is notified.
    hold_remote: AtomicBool,
    release: Notify,
    /// Cluster operations seen.
    calls: std::sync::atomic::AtomicUsize,
}

struct TestAdapter {
    inner: InMemoryAdapter,
    knobs: Arc<Knobs>,
}

impl TestAdapter {
    async fn step(&self, fail: bool) -> Result<(), AdapterError> {
        self.knobs.calls.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        if fail && self.knobs.fail.load(Ordering::SeqCst) {
            return Err(AdapterError::Unavailable("injected".into()));
        }
        Ok(())
    }
}

#[async_trait]
impl Adapter for TestAdapter {
    async fn init(&self) -> Result<(), AdapterError> {
        self.step(false).await?;
        self.inner.init().await
    }
    async fn close(&self) {
        self.inner.close().await
    }
    async fn server_count(&self) -> Result<usize, AdapterError> {
        self.step(false).await?;
        Ok(2)
    }
    fn add_all(&self, id: &str, rooms: &[Room]) -> Result<(), corvis::socket_io::RoomError> {
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
    async fn broadcast(&self, packet: Packet, opts: BroadcastOptions) -> Result<(), AdapterError> {
        self.step(true).await?;
        self.inner.broadcast(packet, opts).await
    }
    async fn broadcast_with_ack(&self, packet: Packet, opts: BroadcastOptions, sink: AckSink) -> Result<(), AdapterError> {
        self.step(true).await?;
        self.inner.broadcast_with_ack(packet, opts, sink.clone()).await?;
        // The second server: one client, which answers `"remote"`; it reports when released.
        let knobs = self.knobs.clone();
        tokio::spawn(async move {
            if knobs.hold_remote.load(Ordering::SeqCst) {
                knobs.release.notified().await;
            }
            tokio::task::yield_now().await;
            sink.expect(1);
            sink.respond(Args::new(vec![IoValue::from("remote")]));
        });
        Ok(())
    }
    async fn sockets(&self, rooms: &[Room]) -> Result<HashSet<SocketId>, AdapterError> {
        self.step(false).await?;
        self.inner.sockets(rooms).await
    }
    async fn fetch_sockets(&self, opts: BroadcastOptions) -> Result<Vec<SocketInfo>, AdapterError> {
        self.step(false).await?;
        self.inner.fetch_sockets(opts).await
    }
    async fn add_sockets(&self, opts: BroadcastOptions, rooms: Vec<Room>) -> Result<(), AdapterError> {
        self.step(false).await?;
        self.inner.add_sockets(opts, rooms).await
    }
    async fn del_sockets(&self, opts: BroadcastOptions, rooms: Vec<Room>) -> Result<(), AdapterError> {
        self.step(false).await?;
        self.inner.del_sockets(opts, rooms).await
    }
    async fn disconnect_sockets(&self, opts: BroadcastOptions, close: bool) -> Result<(), AdapterError> {
        self.step(false).await?;
        self.inner.disconnect_sockets(opts, close).await
    }
    fn membership_size(&self) -> Option<MembershipSize> {
        self.inner.membership_size()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Variant {
    InMemory,
    Replaced,
}

const VARIANTS: [Variant; 2] = [Variant::InMemory, Variant::Replaced];

impl Variant {
    fn options(self, knobs: &Arc<Knobs>) -> IoOptions {
        match self {
            Variant::InMemory => IoOptions::new(),
            Variant::Replaced => {
                let knobs = knobs.clone();
                IoOptions::new()
                    .adapter(move |local: LocalNamespace| TestAdapter { inner: InMemoryAdapter::new(local), knobs: knobs.clone() })
            }
        }
    }

    /// Answers a broadcast with acknowledgements collects from the simulated second server.
    fn remote_answers(self) -> usize {
        usize::from(self == Variant::Replaced)
    }
}

// --------------------------------------------------------------------------------------------------- helpers

async fn app(variant: Variant) -> (IoFixture, Arc<Knobs>) {
    let knobs = Arc::new(Knobs::default());
    let fx = start(variant.options(&knobs), |io| {
        io.of("/admin").unwrap();
    })
    .await;
    (fx, knobs)
}

/// A raw client connected to a namespace.
struct Client {
    raw: Raw,
    id: String,
    nsp: &'static str,
}

impl Client {
    async fn new(fx: &IoFixture, nsp: &'static str) -> Client {
        let mut raw = Raw::connect(fx.addr).await;
        let id = raw.join(nsp, None).await;
        Client { raw, id, nsp }
    }

    fn socket(&self, fx: &IoFixture) -> corvis::socket_io::IoSocket {
        fx.io.of(self.nsp).unwrap().socket(&self.id).expect("connected")
    }

    /// The text packets received up to the marker (excluded).
    async fn until_marker(&mut self) -> Vec<String> {
        let mut seen = Vec::new();
        loop {
            match self.raw.recv().await {
                Msg::Text(t) if t.contains("[\"marker\"") => return seen,
                Msg::Text(t) => seen.push(t),
                Msg::Binary(b) => seen.push(format!("binary:{}", b.len())),
            }
        }
    }
}

/// Emits the marker to every socket of both namespaces, then reads each client up to it.
async fn collect(fx: &IoFixture, clients: &mut [&mut Client]) -> Vec<Vec<String>> {
    fx.io.main().emit("marker", ()).await.unwrap();
    fx.io.of("/admin").unwrap().emit("marker", ()).await.unwrap();
    let mut out = Vec::new();
    for c in clients.iter_mut() {
        out.push(c.until_marker().await);
    }
    out
}

fn hits(seen: &[String], name: &str) -> usize {
    seen.iter().filter(|s| s.contains(&format!("[\"{name}\""))).count()
}

// ------------------------------------------------------------------------------------------------ scenarios

#[tokio::test]
async fn the_same_room_in_two_namespaces_is_two_rooms() {
    for v in VARIANTS {
        let (fx, _k) = app(v).await;
        let mut a = Client::new(&fx, "/").await;
        let mut b = Client::new(&fx, "/admin").await;
        a.socket(&fx).join("r").unwrap();
        b.socket(&fx).join("r").unwrap();
        fx.io.to("r").emit("hit", ()).await.unwrap();
        let seen = collect(&fx, &mut [&mut a, &mut b]).await;
        assert_eq!((hits(&seen[0], "hit"), hits(&seen[1], "hit")), (1, 0), "{v:?}");
    }
}

#[tokio::test]
async fn emit_to_a_socket_id_and_the_rooms_right_after_connecting() {
    for v in VARIANTS {
        let (fx, _k) = app(v).await;
        let mut a = Client::new(&fx, "/").await;
        let mut b = Client::new(&fx, "/").await;
        fx.io.to(a.id.as_str()).emit("direct", ()).await.unwrap();
        let seen = collect(&fx, &mut [&mut a, &mut b]).await;
        assert_eq!((hits(&seen[0], "direct"), hits(&seen[1], "direct")), (1, 0), "{v:?}");
        assert_eq!(a.socket(&fx).rooms(), HashSet::from([a.id.clone()]));
    }
}

#[tokio::test]
async fn whole_namespace_union_and_exclusions() {
    for v in VARIANTS {
        let (fx, _k) = app(v).await;
        let mut a = Client::new(&fx, "/").await;
        let mut b = Client::new(&fx, "/").await;
        let mut c = Client::new(&fx, "/").await;
        let mut d = Client::new(&fx, "/admin").await;
        a.socket(&fx).join(["r1", "r2", "admins"]).unwrap();
        b.socket(&fx).join("r1").unwrap();
        c.socket(&fx).join(["r1", "r2"]).unwrap();
        let counts = |seen: &[Vec<String>]| seen.iter().map(|s| hits(s, "e")).collect::<Vec<_>>();

        fx.io.emit("e", ()).await.unwrap();
        assert_eq!(counts(&collect(&fx, &mut [&mut a, &mut b, &mut c, &mut d]).await), [1, 1, 1, 0], "{v:?} whole namespace");
        fx.io.to(["r1", "r2"]).emit("e", ()).await.unwrap();
        assert_eq!(counts(&collect(&fx, &mut [&mut a, &mut b, &mut c, &mut d]).await), [1, 1, 1, 0], "{v:?} union, once each");
        fx.io.except("admins").emit("e", ()).await.unwrap();
        assert_eq!(counts(&collect(&fx, &mut [&mut a, &mut b, &mut c, &mut d]).await), [0, 1, 1, 0], "{v:?} except a room");
        fx.io.to("r1").except(b.id.as_str()).emit("e", ()).await.unwrap();
        assert_eq!(counts(&collect(&fx, &mut [&mut a, &mut b, &mut c, &mut d]).await), [1, 0, 1, 0], "{v:?} except an id");
        fx.io.to("nobody").emit("e", ()).await.unwrap();
        assert_eq!(counts(&collect(&fx, &mut [&mut a, &mut b, &mut c, &mut d]).await), [0, 0, 0, 0], "{v:?} an empty room");
    }
}

#[tokio::test]
async fn a_broadcast_from_a_socket_excludes_it_and_a_farewell_reaches_the_rooms() {
    for v in VARIANTS {
        let knobs = Arc::new(Knobs::default());
        let fx = start(v.options(&knobs), |io| {
            io.on_connection(|socket| {
                socket.on("shout", |socket, _args, _ack| async move {
                    let _ = socket.to("r").emit("heard", ()).await;
                });
                socket.on_disconnecting(|socket, _| {
                    let rooms: Vec<String> = socket.rooms().into_iter().collect();
                    let socket = socket.clone();
                    tokio::spawn(async move {
                        let _ = socket.to(rooms).emit("left", ()).await;
                    });
                });
            });
        })
        .await;
        let mut a = Client::new(&fx, "/").await;
        let mut b = Client::new(&fx, "/").await;
        let mut c = Client::new(&fx, "/").await;
        for client in [&a, &b, &c] {
            client.socket(&fx).join("r").unwrap();
        }
        a.raw.send(r#"2["shout"]"#).await;
        // The handler runs asynchronously: wait for the delivery through B, then close the round with the marker.
        assert!(b.raw.recv_text().await.contains("heard"), "{v:?}");
        assert!(c.raw.recv_text().await.contains("heard"));
        let seen = collect(&fx, &mut [&mut a, &mut b, &mut c]).await;
        assert_eq!(hits(&seen[0], "heard"), 0, "{v:?} the sender is excluded");
        a.raw.close().await;
        assert!(b.raw.recv_text().await.contains("left"), "{v:?}");
        assert!(c.raw.recv_text().await.contains("left"));
    }
}

#[tokio::test]
async fn a_binary_broadcast_reaches_websocket_and_polling_clients() {
    for v in VARIANTS {
        let (fx, _k) = app(v).await;
        let mut a = Client::new(&fx, "/").await;
        let (sid, _) = open_polling(fx.addr).await;
        assert_eq!(post(fx.addr, &sid, "40").await.status, 200);
        let connect = poll(fx.addr, &sid).await.text();
        let polling_id = connect.split("\"sid\":\"").nth(1).and_then(|s| s.split('"').next()).unwrap().to_string();
        fx.io.main().socket(&polling_id).unwrap().join("r").unwrap();
        a.socket(&fx).join("r").unwrap();
        let data: Vec<u8> = (0..1024u32).map(|i| (i * 7 % 251) as u8).collect();
        fx.io.to("r").emit("file", (Binary::from(data.clone()),)).await.unwrap();
        let seen = collect(&fx, &mut [&mut a]).await;
        assert_eq!(seen[0], [r#"51-["file",{"_placeholder":true,"num":0}]"#.to_string(), "binary:1024".to_string()], "{v:?}");
        let body = poll(fx.addr, &sid).await.text();
        assert!(body.starts_with(r#"451-["file",{"_placeholder":true,"num":0}]"#), "{body}");
        let attachment = body.split('\x1e').nth(1).unwrap();
        use base64::Engine as _;
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&attachment[1..]).unwrap(), data);
    }
}

#[tokio::test]
async fn packets_of_one_task_arrive_in_the_order_they_were_emitted() {
    for v in VARIANTS {
        let (fx, _k) = app(v).await;
        let mut a = Client::new(&fx, "/").await;
        a.socket(&fx).join("r").unwrap();
        for n in 0..20 {
            fx.io.to("r").emit("a", (n,)).await.unwrap();
            a.socket(&fx).emit("b", (n,)).unwrap();
        }
        let seen = collect(&fx, &mut [&mut a]).await;
        let expected: Vec<String> = (0..20).flat_map(|n| [format!(r#"2["a",{n}]"#), format!(r#"2["b",{n}]"#)]).collect();
        assert_eq!(seen[0], expected, "{v:?}");
    }
}

#[tokio::test]
async fn leave_and_rooms_during_disconnecting() {
    for v in VARIANTS {
        let knobs = Arc::new(Knobs::default());
        let log = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let fx = start(v.options(&knobs), |io| {
            let log = log.clone();
            io.on_connection(move |socket| {
                socket.join("r").unwrap();
                let (during, after) = (log.clone(), log.clone());
                socket.on_disconnecting(move |s, _| {
                    let mut rooms: Vec<String> = s.rooms().into_iter().collect();
                    rooms.sort();
                    during.lock().unwrap().push(format!("during:{}", rooms.len()));
                });
                socket.on_disconnect(move |s, _| after.lock().unwrap().push(format!("after:{}", s.rooms().len())));
            });
        })
        .await;
        let mut a = Client::new(&fx, "/").await;
        let mut b = Client::new(&fx, "/").await;
        a.socket(&fx).leave("r");
        fx.io.to("r").emit("e", ()).await.unwrap();
        let seen = collect(&fx, &mut [&mut a, &mut b]).await;
        assert_eq!((hits(&seen[0], "e"), hits(&seen[1], "e")), (0, 1), "{v:?}");
        b.raw.close().await;
        let deadline = Instant::now() + Duration::from_secs(10);
        while log.lock().unwrap().len() < 2 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(*log.lock().unwrap(), ["during:2", "after:0"], "{v:?}");
        assert_eq!(fx.io.main().adapter().membership_size().unwrap().sockets, 1, "only `a` is left");
    }
}

#[tokio::test]
async fn bulk_operations() {
    for v in VARIANTS {
        let (fx, _k) = app(v).await;
        let mut a = Client::new(&fx, "/").await;
        let mut b = Client::new(&fx, "/").await;
        let mut c = Client::new(&fx, "/").await;
        a.socket(&fx).join("r1").unwrap();
        b.socket(&fx).join("r1").unwrap();
        fx.io.to("r1").sockets_join("r2").await.unwrap();
        fx.io.to("r2").emit("e", ()).await.unwrap();
        let seen = collect(&fx, &mut [&mut a, &mut b, &mut c]).await;
        assert_eq!(seen.iter().map(|s| hits(s, "e")).collect::<Vec<_>>(), [1, 1, 0], "{v:?}");
        // Fetch the sockets in `r1` except `b`.
        let fetched = fx.io.to("r1").except(b.id.as_str()).fetch_sockets().await.unwrap();
        assert_eq!(fetched.len(), 1, "{v:?}");
        assert_eq!(fetched[0].id(), a.id);
        let rooms: HashSet<&str> = fetched[0].rooms().iter().map(String::as_str).collect();
        assert_eq!(rooms, HashSet::from([a.id.as_str(), "r1", "r2"]));
        // Disconnect a room without closing the connections: `io server disconnect` for that namespace only.
        fx.io.to("r1").disconnect_sockets(false).await.unwrap();
        assert_eq!(a.raw.recv_text().await, "1", "{v:?}");
        assert_eq!(b.raw.recv_text().await, "1");
        let seen = collect(&fx, &mut [&mut c]).await;
        assert_eq!(hits(&seen[0], "e"), 0);
        assert!(fx.io.main().socket(&a.id).is_none());
        assert!(fx.io.main().socket(&c.id).is_some());
    }
}

#[tokio::test]
async fn an_adapter_failure_is_returned_and_disconnects_nobody() {
    let (fx, knobs) = app(Variant::Replaced).await;
    let mut a = Client::new(&fx, "/").await;
    knobs.fail.store(true, Ordering::SeqCst);
    let error = fx.io.emit("e", ()).await.unwrap_err();
    assert_eq!(error, EmitError::Adapter(AdapterError::Unavailable("injected".into())));
    let error = fx.io.emit_with_ack("e", ()).await.unwrap_err();
    assert_eq!(*error.kind(), BroadcastAckErrorKind::Emit(EmitError::Adapter(AdapterError::Unavailable("injected".into()))));
    knobs.fail.store(false, Ordering::SeqCst);
    assert_eq!(fx.io.main().socket_count(), 1);
    fx.io.emit("e", ()).await.unwrap();
    let seen = collect(&fx, &mut [&mut a]).await;
    assert_eq!(hits(&seen[0], "e"), 1);
    assert!(knobs.calls.load(Ordering::SeqCst) > 0, "the cluster operations went through the test adapter");
}

/// The answer a client gives to the broadcast it was sent: `3<id>[<value>]`.
fn answer_to(packet: &str, value: &str) -> String {
    let id: String = packet[1..].chars().take_while(char::is_ascii_digit).collect();
    format!("3{id}[{value}]")
}

#[tokio::test]
async fn every_recipient_answers_a_broadcast() {
    for v in VARIANTS {
        let (fx, _k) = app(v).await;
        let mut clients = vec![Client::new(&fx, "/").await, Client::new(&fx, "/").await, Client::new(&fx, "/").await];
        for c in &clients {
            c.socket(&fx).join("r").unwrap();
        }
        let io = fx.io.clone();
        let call = tokio::spawn(async move { io.to("r").timeout(Duration::from_secs(5)).emit_with_ack("q", ()).await });
        for (n, c) in clients.iter_mut().enumerate() {
            let packet = c.raw.recv_text().await;
            assert!(packet.contains("\"q\""), "{packet}");
            c.raw.send(&answer_to(&packet, &n.to_string())).await;
        }
        let answers = call.await.unwrap().unwrap();
        assert_eq!(answers.len(), 3 + v.remote_answers(), "{v:?}");
    }
}

#[tokio::test]
async fn one_silent_recipient_fails_the_call_at_the_timeout_with_the_other_answers() {
    for v in VARIANTS {
        let (fx, _k) = app(v).await;
        let mut clients = vec![Client::new(&fx, "/").await, Client::new(&fx, "/").await, Client::new(&fx, "/").await];
        for c in &clients {
            c.socket(&fx).join("r").unwrap();
        }
        let io = fx.io.clone();
        let started = Instant::now();
        let call = tokio::spawn(async move { io.to("r").timeout(Duration::from_millis(200)).emit_with_ack("q", ()).await });
        for (n, c) in clients.iter_mut().enumerate() {
            let packet = c.raw.recv_text().await;
            if n < 2 {
                c.raw.send(&answer_to(&packet, "1")).await;
            }
        }
        let error = call.await.unwrap().unwrap_err();
        assert!(started.elapsed() >= Duration::from_millis(200), "not before the timeout");
        assert_eq!(error.to_string(), "operation has timed out");
        assert_eq!(error.responses().len(), 2 + v.remote_answers(), "{v:?}");
    }
}

#[tokio::test]
async fn the_second_server_must_report_before_the_call_completes() {
    let (fx, knobs) = app(Variant::Replaced).await;
    let mut a = Client::new(&fx, "/").await;
    knobs.hold_remote.store(true, Ordering::SeqCst);
    let io = fx.io.clone();
    let call = tokio::spawn(async move { io.timeout(Duration::from_secs(10)).emit_with_ack("q", ()).await });
    let packet = a.raw.recv_text().await;
    a.raw.send(&answer_to(&packet, "1")).await;
    // Everyone local answered, but the other server has not reported its clients: the call is not complete.
    assert!(a.raw.recv_within(Duration::from_millis(300)).await.is_none());
    assert!(!call.is_finished());
    knobs.release.notify_one();
    let answers = call.await.unwrap().unwrap();
    assert_eq!(answers.len(), 2);
}

#[tokio::test]
async fn a_long_name_in_to_sends_nothing() {
    for v in VARIANTS {
        let (fx, _k) = app(v).await;
        let mut a = Client::new(&fx, "/").await;
        let long = "x".repeat(257);
        let error = fx.io.to(long.as_str()).emit("e", ()).await.unwrap_err();
        assert!(matches!(error, EmitError::InvalidRoom(_)), "{v:?}");
        let seen = collect(&fx, &mut [&mut a]).await;
        assert_eq!(hits(&seen[0], "e"), 0);
    }
}

#[tokio::test]
async fn the_default_adapter_is_the_in_memory_one() {
    let io = IoServer::new(IoOptions::new()).unwrap();
    assert!(io.main().adapter().membership_size().is_some());
}
