//! Tests of the Socket.IO layer without a network: a session is fed packets and read directly, and the clock is
//! paused, so every timer is checked at its exact time and nothing here depends on how fast the machine is. The same
//! behaviors are exercised over real transports in `tests/socket_io.rs` and `tests/socket_io_security.rs`.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use serde_json::json;
use tokio::sync::Notify;

use super::engine::{EngineMessage, EngineOptions, EngineSocket};
use super::*;

const DAY: Duration = Duration::from_secs(24 * 3600);

fn options() -> IoOptions {
    IoOptions::new().engine(EngineOptions::new().ping_interval(DAY).ping_timeout(DAY))
}

/// Lets the tasks that are ready run: 40 turns of the single-threaded runtime on the paused clock, neither a sleep nor
/// wall-clock time (the clock does not move). A wait for "nothing left to do" cannot be written here: on the paused clock
/// a timer would be the way, and the timer wheel's 1 ms resolution would move the clock under the exact-time tests.
/// Every packet these tests feed takes far fewer turns than this to be handled.
async fn settle() {
    for _ in 0..40 {
        tokio::task::yield_now().await;
    }
}

/// Lets the runtime turn until `condition` holds, failing loudly after 10,000 turns: a wait for a condition, on the
/// paused clock.
async fn settle_until(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..10_000 {
        if condition() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("{what}: not within 10,000 turns of the runtime");
}

async fn advance(ms: u64) {
    tokio::time::advance(Duration::from_millis(ms)).await;
    settle().await;
}

#[derive(Debug, Clone, PartialEq)]
enum Out {
    T(String),
    B(Vec<u8>),
}

fn t(s: &str) -> Out {
    Out::T(s.to_string())
}

struct Conn {
    socket: EngineSocket,
}

impl Conn {
    fn text(&self, s: &str) {
        self.socket.feed(EngineMessage::Text(s.to_string()));
    }

    fn bin(&self, b: &[u8]) {
        self.socket.feed(EngineMessage::Binary(Bytes::copy_from_slice(b)));
    }

    async fn sent(&self) -> Vec<Out> {
        settle().await;
        self.socket
            .take_sent()
            .into_iter()
            .map(|m| match m {
                EngineMessage::Text(s) => Out::T(s),
                EngineMessage::Binary(b) => Out::B(b.to_vec()),
                // Framed messages carry the Engine.IO type in front.
                EngineMessage::Framed(s) => Out::T(s[1..].to_string()),
                EngineMessage::Shared(s) => Out::T(s[1..].to_string()),
            })
            .collect()
    }

    /// Joins a namespace with the given CONNECT packet and returns the socket id from the answer.
    async fn join(&self, connect: &str) -> String {
        self.text(connect);
        let sent = self.sent().await;
        let Some(Out::T(answer)) = sent.first() else { panic!("no answer to {connect}: {sent:?}") };
        let json_at = answer.find('{').expect("an object");
        let body: serde_json::Value = serde_json::from_str(&answer[json_at..]).unwrap_or_else(|_| panic!("not an object: {answer}"));
        body["sid"].as_str().unwrap_or_else(|| panic!("no sid in {answer}")).to_string()
    }
}

trait Open {
    fn conn(&self) -> Conn;
}

impl Open for IoServer {
    fn conn(&self) -> Conn {
        Conn { socket: self.engine().open_test_socket() }
    }
}

type Log = Arc<Mutex<Vec<String>>>;
type Results<K> = Arc<Mutex<Vec<(K, Result<Args, AckError>)>>>;

fn log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

fn entries(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

fn server(options: IoOptions) -> IoServer {
    IoServer::new(options).unwrap()
}

/// Records the id of every connected socket and the reasons of every disconnect, per namespace.
fn record_disconnects(namespace: &IoNamespace, log: &Log) {
    let log = log.clone();
    namespace.on_connection(move |socket| {
        let log = log.clone();
        let name = socket.namespace().name().to_string();
        socket.on_disconnect(move |_socket, reason| log.lock().unwrap().push(format!("{name}:{reason}")));
    });
}

// ------------------------------------------------------------------------------------- namespaces and connection

#[tokio::test(start_paused = true)]
async fn an_emit_queues_the_encoder_s_buffer_and_a_broadcast_one_shared_buffer() {
    let io = server(options());
    let conns = [io.conn(), io.conn()];
    let mut ids = Vec::new();
    for conn in &conns {
        ids.push(conn.join("0").await);
    }
    let socket = io.main().socket(&ids[0]).expect("connected");
    socket.emit("one", ("x".repeat(1024),)).unwrap();
    let sent = conns[0].socket.take_sent();
    let [EngineMessage::Framed(text)] = &sent[..] else { panic!("one framed message, got {sent:?}") };
    assert!(text.starts_with(r#"42["one","#), "{text}");
    io.main().emit("all", ("y".repeat(1024),)).await.unwrap();
    let mut buffers = Vec::new();
    for conn in &conns {
        let sent = conn.socket.take_sent();
        let [EngineMessage::Shared(text)] = &sent[..] else { panic!("one shared message, got {sent:?}") };
        buffers.push(text.clone());
    }
    assert!(Arc::ptr_eq(&buffers[0], &buffers[1]), "every recipient queues the one encoding");
}

#[tokio::test(start_paused = true)]
async fn a_connection_under_its_limits_never_registers_for_a_release() {
    let io = server(options());
    let handled = Arc::new(AtomicUsize::new(0));
    {
        let handled = handled.clone();
        io.on_connection(move |socket| {
            let handled = handled.clone();
            socket.on("e", move |_socket, _args, _ack| {
                let handled = handled.clone();
                async move {
                    handled.fetch_add(1, Ordering::SeqCst);
                }
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    for i in 0..10_000 {
        conn.text(&format!(r#"2["e",{i}]"#));
        if i % 100 == 99 {
            settle().await;
        }
    }
    settle().await;
    assert_eq!(handled.load(Ordering::SeqCst), 10_000);
    assert_eq!(conn.socket.peak_unread_waiters(), 0, "a reader under its limits registered for a release");
}

#[tokio::test(start_paused = true)]
async fn auth_is_visible_to_the_server_and_the_answer_carries_a_socket_id() {
    let io = server(options());
    let seen = log();
    {
        let seen = seen.clone();
        io.on_connection(move |socket| seen.lock().unwrap().push(socket.handshake().auth().to_string()));
    }
    let conn = io.conn();
    let sid = conn.join(r#"0{"token":"abc"}"#).await;
    assert_eq!(sid.len(), 20);
    assert_eq!(entries(&seen), [r#"{"token":"abc"}"#]);
    // Without auth the handshake exposes an empty object, as Node.
    let conn = io.conn();
    conn.join("0").await;
    assert_eq!(entries(&seen)[1], "{}");
}

#[tokio::test(start_paused = true)]
async fn socket_ids_are_not_session_ids_and_differ() {
    let io = server(options());
    io.of("/admin").unwrap();
    let conn = io.conn();
    let main = conn.join("0").await;
    let admin = conn.join("0/admin,").await;
    assert_eq!(main.len(), 20);
    assert_eq!(admin.len(), 20);
    assert_ne!(main, admin);
    assert_ne!(main, conn.socket.id());
    assert_ne!(admin, conn.socket.id());
    assert!(io.main().socket(&main).is_some());
    assert!(io.of("/admin").unwrap().socket(&admin).is_some());
    assert!(io.main().socket(&admin).is_none(), "an id belongs to its namespace");
}

#[tokio::test(start_paused = true)]
async fn an_unknown_namespace_gets_invalid_namespace_and_is_not_created() {
    let io = server(options());
    let conn = io.conn();
    conn.text("0/missing,");
    assert_eq!(conn.sent().await, [t(r#"4/missing,{"message":"Invalid namespace"}"#)]);
    // The connection stays open and the namespace is still unknown.
    conn.text("0/missing,");
    assert_eq!(conn.sent().await, [t(r#"4/missing,{"message":"Invalid namespace"}"#)]);
    assert!(!conn.socket.is_closing());
    conn.join("0").await;
}

#[tokio::test(start_paused = true)]
async fn two_namespaces_on_one_session_each_receive_their_own_events() {
    let io = server(options());
    let seen = log();
    for name in ["/", "/admin"] {
        let seen = seen.clone();
        io.of(name).unwrap().on_connection(move |socket| {
            let seen = seen.clone();
            socket.on("ping", move |socket, args, _ack| {
                let seen = seen.clone();
                async move { seen.lock().unwrap().push(format!("{}:{}", socket.namespace().name(), args.get::<i64>(0).unwrap())) }
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    conn.join("0/admin,").await;
    conn.text(r#"2["ping",1]"#);
    conn.text(r#"2/admin,["ping",2]"#);
    settle().await;
    assert_eq!(entries(&seen), ["/:1", "/admin:2"]);
}

#[tokio::test(start_paused = true)]
async fn connection_handlers_run_after_connect_and_before_the_first_event() {
    let io = server(options());
    let seen = log();
    {
        let seen = seen.clone();
        io.on_connection(move |socket| {
            let seen = seen.clone();
            // The welcome event goes out after the CONNECT packet.
            socket.emit("welcome", ("hi",)).unwrap();
            socket.on("early", move |_s, _a, _k| {
                let seen = seen.clone();
                async move { seen.lock().unwrap().push("early".into()) }
            });
        });
    }
    let conn = io.conn();
    conn.text("0");
    let sent = conn.sent().await;
    conn.text(r#"2["early"]"#);
    settle().await;
    assert_eq!(entries(&seen), ["early"], "the handler registered by the connection handler sees the first event");
    assert!(matches!(&sent[0], Out::T(s) if s.starts_with("0{\"sid\":")));
    assert_eq!(sent[1], t(r#"2["welcome","hi"]"#));
}

/// Security review of #185, F1: the connection handlers are where an application authenticates a socket and
/// disconnects it. They must run as soon as the socket is announced, not when its client chooses to poll: a client that
/// posts CONNECT and never polls must not stay connected, unauthenticated, receiving the namespace's broadcasts.
#[tokio::test(start_paused = true)]
async fn the_connection_handler_runs_before_the_client_polls_and_a_socket_it_refuses_gets_no_broadcast() {
    let io = server(options());
    let chat = io.of("/chat").unwrap();
    let ran = Arc::new(AtomicUsize::new(0));
    {
        let ran = ran.clone();
        chat.on_connection(move |socket| {
            ran.fetch_add(1, Ordering::SeqCst);
            if socket.handshake().auth().get("token").is_none() {
                socket.disconnect(false);
            }
        });
    }
    // A member, so the broadcasts have somewhere to go.
    let member = io.conn();
    member.join(r#"0/chat,{"token":"t"}"#).await;
    let stop = Arc::new(AtomicBool::new(false));
    let broadcaster = {
        let (chat, stop) = (chat.clone(), stop.clone());
        tokio::spawn(async move {
            while !stop.load(Ordering::SeqCst) {
                let _ = chat.emit("news", ()).await;
                tokio::task::yield_now().await;
            }
        })
    };
    // CONNECT without a token, and no poll: nothing takes what is queued for this client.
    let conn = io.conn();
    conn.text("0/chat,");
    settle_until("the connection handler of the client that does not poll", || ran.load(Ordering::SeqCst) == 2).await;
    let sent = conn.sent().await;
    stop.store(true, Ordering::SeqCst);
    broadcaster.await.unwrap();
    assert!(matches!(sent.first(), Some(Out::T(t)) if t.starts_with("0/chat,{\"sid\"")), "{sent:?}");
    assert!(!sent.iter().any(|m| matches!(m, Out::T(t) if t.contains("news"))), "a broadcast reached a refused socket: {sent:?}");
    assert_eq!(sent.last(), Some(&t("1/chat,")), "the socket was disconnected: {sent:?}");
    assert!(member.sent().await.iter().any(|m| matches!(m, Out::T(t) if t.contains("news"))), "the broadcasts did run");
}

// ---------------------------------------------------------------------------------------------------- middleware

#[tokio::test(start_paused = true)]
async fn middleware_runs_in_order_and_the_first_error_stops_the_chain() {
    let io = server(options());
    let order = log();
    for (i, fail) in [(1, false), (2, true), (3, false)] {
        let order = order.clone();
        io.use_middleware(move |_socket| {
            let order = order.clone();
            async move {
                order.lock().unwrap().push(format!("m{i}"));
                if fail { Err(ConnectError::new("stop")) } else { Ok(()) }
            }
        });
    }
    let connected = log();
    {
        let connected = connected.clone();
        io.on_connection(move |_| connected.lock().unwrap().push("connected".into()));
    }
    let conn = io.conn();
    conn.text("0");
    assert_eq!(conn.sent().await, [t(r#"4{"message":"stop"}"#)]);
    assert_eq!(entries(&order), ["m1", "m2"]);
    assert!(entries(&connected).is_empty(), "connection handlers do not run for a refused socket");
    assert_eq!(io.main().socket_count(), 0);
    // The slot is free again: a retry runs the middleware again.
    conn.text("0");
    assert_eq!(conn.sent().await, [t(r#"4{"message":"stop"}"#)]);
    assert!(!conn.socket.is_closing());
}

#[tokio::test(start_paused = true)]
async fn middleware_errors_carry_data_only_when_given() {
    let io = server(options());
    io.of("/a").unwrap().use_middleware(|_| async { Err(ConnectError::new("not authorized").data(json!({ "content": "retry later" }))) });
    io.of("/b").unwrap().use_middleware(|_| async { Err(ConnectError::new("not authorized")) });
    let conn = io.conn();
    conn.text("0/a,");
    conn.text("0/b,");
    assert_eq!(
        conn.sent().await,
        [t(r#"4/a,{"message":"not authorized","data":{"content":"retry later"}}"#), t(r#"4/b,{"message":"not authorized"}"#)]
    );
}

#[tokio::test(start_paused = true)]
async fn middleware_panics_and_hangs_are_contained_and_answered_uniformly() {
    let io = server(options().middleware_timeout(Duration::from_secs(2)));
    io.of("/panics").unwrap().use_middleware(|_| async { panic!("secret internal detail") });
    io.of("/panics-early").unwrap().use_middleware(|_| -> std::future::Ready<Result<(), ConnectError>> { panic!("also secret") });
    io.of("/hangs").unwrap().use_middleware(|_| std::future::pending::<Result<(), ConnectError>>());
    let conn = io.conn();
    conn.text("0/panics,");
    conn.text("0/panics-early,");
    conn.text("0/hangs,");
    let sent = conn.sent().await;
    assert_eq!(sent.len(), 2, "the hanging middleware has not answered yet: {sent:?}");
    for out in &sent {
        let Out::T(text) = out else { panic!() };
        assert!(text.ends_with(r#"{"message":"Internal server error"}"#), "{text}");
        assert!(!text.contains("secret"));
    }
    advance(1_999).await;
    assert!(conn.sent().await.is_empty(), "not before the middleware timeout");
    advance(1).await;
    assert_eq!(conn.sent().await, [t(r#"4/hangs,{"message":"Internal server error"}"#)]);
    // Other namespaces still work.
    conn.join("0").await;
}

#[tokio::test(start_paused = true)]
async fn a_socket_in_middleware_cannot_emit_but_can_carry_the_user() {
    let io = server(options());
    let result = log();
    {
        let result = result.clone();
        io.use_middleware(move |socket| {
            let result = result.clone();
            async move {
                result.lock().unwrap().push(format!("{:?}", socket.emit("x", ())));
                result.lock().unwrap().push(format!("{}", socket.connected()));
                socket.insert(String::from("alice"));
                socket.set_data(json!({ "user": "alice" }));
                Ok(())
            }
        });
    }
    let user = log();
    {
        let user = user.clone();
        io.on_connection(move |socket| user.lock().unwrap().push(format!("{:?} {}", socket.get::<String>(), socket.data())));
    }
    let conn = io.conn();
    conn.join("0").await;
    assert_eq!(entries(&result), ["Err(NotConnected)", "false"]);
    assert_eq!(entries(&user), [r#"Some("alice") {"user":"alice"}"#]);
}

#[tokio::test(start_paused = true)]
async fn a_connection_that_closes_during_middleware_gets_no_handlers_and_no_error() {
    let io = server(options());
    let gate = Arc::new(Notify::new());
    {
        let gate = gate.clone();
        io.use_middleware(move |_| {
            let gate = gate.clone();
            async move {
                gate.notified().await;
                Ok(())
            }
        });
    }
    let connected = log();
    record_disconnects(&io.main(), &connected);
    let conn = io.conn();
    conn.text("0");
    settle().await;
    conn.socket.close();
    advance(31_000).await; // the close timeout of the session: no transport drains it here
    gate.notify_waiters();
    settle().await;
    assert!(entries(&connected).is_empty(), "no connection handler, so no disconnect either");
    assert_eq!(io.main().socket_count(), 0);
    assert!(conn.sent().await.is_empty(), "nothing was queued after the close");
}

// --------------------------------------------------------------------------------------- one socket per namespace

#[tokio::test(start_paused = true)]
async fn a_second_connect_for_a_joined_namespace_closes_the_connection() {
    let io = server(options());
    let reasons = log();
    record_disconnects(&io.main(), &reasons);
    let conn = io.conn();
    conn.join("0").await;
    conn.text("0");
    settle().await;
    assert!(conn.socket.is_closing());
    assert_eq!(entries(&reasons), ["/:forced server close"]);
}

#[tokio::test(start_paused = true)]
async fn a_connect_flood_during_slow_middleware_runs_it_once_and_closes_at_the_second_connect() {
    let io = server(options());
    let runs = Arc::new(AtomicUsize::new(0));
    {
        let runs = runs.clone();
        io.of("/admin").unwrap().use_middleware(move |_| {
            let runs = runs.clone();
            async move {
                runs.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_secs(1)).await;
                Ok(())
            }
        });
    }
    let conn = io.conn();
    for _ in 0..1_000 {
        conn.text("0/admin,");
    }
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the middleware ran once");
    assert!(conn.socket.is_closing(), "the second CONNECT closed the connection");
    advance(2_000).await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert_eq!(io.of("/admin").unwrap().socket_count(), 0, "the attempt was dropped with the connection");
}

#[tokio::test(start_paused = true)]
async fn packets_for_a_namespace_that_is_not_joined_close_the_connection() {
    for first in [r#"2["echo",1]"#, "1", r#"30["x"]"#, r#"2/admin,["echo"]"#] {
        let io = server(options());
        io.of("/admin").unwrap();
        let conn = io.conn();
        conn.text(first);
        settle().await;
        assert!(conn.socket.is_closing(), "{first}");
    }
    // A packet for a namespace that is still in middleware is not a joined namespace either.
    let io = server(options());
    io.use_middleware(|_| std::future::pending::<Result<(), ConnectError>>());
    let conn = io.conn();
    conn.text("0");
    conn.text(r#"2["echo"]"#);
    settle().await;
    assert!(conn.socket.is_closing());
}

#[tokio::test(start_paused = true)]
async fn connect_error_from_a_client_closes_the_connection() {
    let io = server(options());
    let reasons = log();
    record_disconnects(&io.main(), &reasons);
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"4{"message":"x"}"#);
    settle().await;
    assert!(conn.socket.is_closing());
    assert_eq!(entries(&reasons), ["/:forced server close"]);
}

#[tokio::test(start_paused = true)]
async fn a_connection_that_joins_no_namespace_is_closed_at_connect_timeout_not_before() {
    let io = server(options().connect_timeout(Duration::from_millis(1_000)));
    let conn = io.conn();
    settle().await;
    advance(999).await;
    assert!(!conn.socket.is_closing());
    advance(1).await;
    assert!(conn.socket.is_closing());
    // Joining in time clears the timer for good, even when the socket leaves again.
    let conn = io.conn();
    settle().await;
    advance(500).await;
    conn.join("0").await;
    conn.text("1");
    advance(5_000).await;
    assert!(!conn.socket.is_closing());
}

#[tokio::test(start_paused = true)]
async fn a_pending_connection_attempt_does_not_stop_the_connect_timeout() {
    let io = server(options().connect_timeout(Duration::from_millis(1_000)));
    io.use_middleware(|_| std::future::pending::<Result<(), ConnectError>>());
    let conn = io.conn();
    settle().await;
    conn.text("0");
    settle().await;
    advance(1_000).await;
    assert!(conn.socket.is_closing());
}

// ------------------------------------------------------------------------------------------------------ events

#[tokio::test(start_paused = true)]
async fn events_carry_any_number_of_arguments_and_unhandled_events_are_ignored() {
    let io = server(options());
    let seen = log();
    {
        let seen = seen.clone();
        io.on_connection(move |socket| {
            let seen = seen.clone();
            socket.on("many", move |_s, args, ack| {
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push(format!(
                        "{} {}",
                        args.len(),
                        args.to_json().unwrap().iter().map(|v| v.to_string()).collect::<Vec<_>>().join("|")
                    ));
                    assert!(ack.is_none());
                }
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"2["many"]"#);
    conn.text(r#"2["many",1,"two",{"three":3},null,[4]]"#);
    conn.text(r#"2["nobody-listens",1]"#);
    conn.text(r#"22["nobody-listens-with-ack"]"#);
    settle().await;
    assert_eq!(entries(&seen), ["0 ", r#"5 1|"two"|{"three":3}|null|[4]"#]);
    assert!(conn.sent().await.is_empty(), "an event without a handler is not acknowledged");
}

#[tokio::test(start_paused = true)]
async fn the_server_emits_events_with_several_arguments_and_refuses_reserved_names() {
    let io = server(options());
    let result = log();
    {
        let result = result.clone();
        io.on_connection(move |socket| {
            socket.emit("update", (1, "two", json!({ "three": 3 }))).unwrap();
            for name in ["connect", "connect_error", "disconnect", "disconnecting", "newListener", "removeListener"] {
                let err = socket.emit(name, ()).unwrap_err();
                result.lock().unwrap().push(err.to_string());
            }
        });
    }
    let conn = io.conn();
    conn.text("0");
    let sent = conn.sent().await;
    assert_eq!(sent.len(), 2, "only the CONNECT and the update were sent: {sent:?}");
    assert_eq!(sent[1], t(r#"2["update",1,"two",{"three":3}]"#));
    assert_eq!(entries(&result)[2], "\"disconnect\" is a reserved event name");
    assert_eq!(entries(&result).len(), 6);
}

#[tokio::test(start_paused = true)]
async fn the_client_ack_carries_its_id_and_several_values_and_is_sent_once() {
    let io = server(options());
    io.on_connection(|socket| {
        socket.on("sum", |_s, args, ack| async move {
            let total: i64 = args.iter().filter_map(IoValue::as_i64).sum();
            let ack = ack.expect("asked for an ack");
            ack.send((total, "ok")).unwrap();
        });
        socket.on("silent", |_s, _args, _ack| async {});
    });
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"2127["sum",1,2]"#);
    conn.text(r#"29007199254740991["silent"]"#);
    assert_eq!(conn.sent().await, [t(r#"3127[3,"ok"]"#)]);
}

#[tokio::test(start_paused = true)]
async fn handlers_start_in_arrival_order() {
    let io = server(options());
    let order = log();
    {
        let order = order.clone();
        io.on_connection(move |socket| {
            for name in ["set", "get"] {
                let order = order.clone();
                socket.on(name, move |_s, _a, _k| {
                    let order = order.clone();
                    async move {
                        order.lock().unwrap().push(format!("{name} started"));
                        tokio::task::yield_now().await;
                        order.lock().unwrap().push(format!("{name} done"));
                    }
                });
            }
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"2["set"]"#);
    conn.text(r#"2["get"]"#);
    settle().await;
    let order = entries(&order);
    assert_eq!(&order[..2], ["set started", "get started"]);
}

#[tokio::test(start_paused = true)]
async fn a_panicking_handler_is_contained_and_the_next_event_is_handled() {
    let io = server(options());
    let seen = log();
    {
        let seen = seen.clone();
        io.on_connection(move |socket| {
            socket.on("boom", |_s, _a, _k| async { panic!("handler detail") });
            let seen = seen.clone();
            socket.on("ok", move |_s, _a, _k| {
                let seen = seen.clone();
                async move { seen.lock().unwrap().push("ok".into()) }
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"2["boom"]"#);
    conn.text(r#"2["ok"]"#);
    settle().await;
    assert_eq!(entries(&seen), ["ok"]);
    assert!(!conn.socket.is_closing());
    assert_eq!(io.main().socket_count(), 1);
}

#[tokio::test(start_paused = true)]
async fn blocked_handlers_are_bounded_per_connection_and_reads_stop() {
    let io = server(options().max_inflight(4));
    io.of("/other").unwrap();
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    for name in ["/", "/other"] {
        let (running, peak, release, released) = (running.clone(), peak.clone(), release.clone(), released.clone());
        io.of(name).unwrap().on_connection(move |socket| {
            let (running, peak, release, released) = (running.clone(), peak.clone(), release.clone(), released.clone());
            socket.on("block", move |_s, _a, _k| {
                let (running, peak, release, released) = (running.clone(), peak.clone(), release.clone(), released.clone());
                async move {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    while !released.load(Ordering::SeqCst) {
                        release.notified().await;
                    }
                    running.fetch_sub(1, Ordering::SeqCst);
                }
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    conn.join("0/other,").await;
    // Other connections are served meanwhile.
    for i in 0..200 {
        conn.text(&format!(r#"2{}["block"]"#, if i % 2 == 0 { "" } else { "/other," }));
    }
    settle().await;
    assert_eq!(running.load(Ordering::SeqCst), 4, "four handlers across both namespaces, no more");
    let unread = conn.socket.unread_bytes();
    assert!(unread >= 196 * (10 + 64), "the packets that were not taken stay accounted: {unread}");
    let other = io.conn();
    other.join("0").await;
    other.text(r#"2["nothing"]"#);
    settle().await;
    assert!(!other.socket.is_closing());
    // Releasing the handlers lets the rest through, still four at a time.
    released.store(true, Ordering::SeqCst);
    release.notify_waiters();
    settle().await;
    assert_eq!(running.load(Ordering::SeqCst), 0);
    assert_eq!(peak.load(Ordering::SeqCst), 4);
    assert_eq!(conn.socket.unread_bytes(), 0, "everything is released when the handlers finish");
}

#[tokio::test(start_paused = true)]
async fn decoded_packets_are_charged_until_their_handler_finishes() {
    let io = server(options());
    let release = Arc::new(Notify::new());
    {
        let release = release.clone();
        io.on_connection(move |socket| {
            let release = release.clone();
            socket.on("slow", move |_s, _a, _k| {
                let release = release.clone();
                async move { release.notified().await }
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    let before = conn.socket.unread_bytes();
    // 5,000 empty arrays: 20 kB of text but 5,000 nodes of structure.
    let arrays = vec!["[]"; 5_000].join(",");
    conn.text(&format!(r#"2["slow",[{arrays}]]"#));
    settle().await;
    let charged = conn.socket.unread_bytes() - before;
    assert!(charged >= 5_000 * 40, "empty structures cost their real size: {charged}");
    release.notify_waiters();
    settle().await;
    assert_eq!(conn.socket.unread_bytes(), before);
}

// ------------------------------------------------------------------------------------------------ acknowledgements

#[tokio::test(start_paused = true)]
async fn a_server_emit_with_ack_resolves_with_the_clients_answer() {
    let io = server(options());
    let result: Arc<Mutex<Option<Result<Args, AckError>>>> = Arc::new(Mutex::new(None));
    {
        let result = result.clone();
        io.on_connection(move |socket| {
            let result = result.clone();
            tokio::spawn(async move {
                let answer = socket.emit_with_ack("ask", ("question",)).await;
                *result.lock().unwrap() = Some(answer);
            });
        });
    }
    let conn = io.conn();
    conn.text("0");
    let sent = conn.sent().await;
    assert_eq!(sent[1], t(r#"20["ask","question"]"#), "the first id of a namespace is 0");
    assert!(result.lock().unwrap().is_none());
    conn.text(r#"30["yes",7]"#);
    settle().await;
    let answer = result.lock().unwrap().take().unwrap().unwrap();
    assert_eq!(answer.to_json().unwrap(), vec![json!("yes"), json!(7)]);
    let socket = io.main().sockets().pop().unwrap();
    assert_eq!(socket.pending_acks(), 0);
    // A replay of the same answer, and an answer for an id never issued, are ignored.
    conn.text(r#"30["again"]"#);
    conn.text(r#"3999["never issued"]"#);
    settle().await;
    assert!(!conn.socket.is_closing());
}

#[tokio::test(start_paused = true)]
async fn the_acknowledgement_ids_count_per_namespace_from_zero() {
    let io = server(options());
    io.of("/b").unwrap();
    for name in ["/", "/b"] {
        io.of(name).unwrap().on_connection(|socket| {
            for _ in 0..2 {
                let socket = socket.clone();
                tokio::spawn(async move {
                    let _ = socket.timeout(Duration::from_secs(5)).emit_with_ack("e", ()).await;
                });
            }
        });
    }
    let first = io.conn();
    first.text("0");
    first.text("0/b,");
    let sent = first.sent().await;
    assert!(sent.contains(&t(r#"20["e"]"#)) && sent.contains(&t(r#"21["e"]"#)));
    assert!(sent.contains(&t(r#"2/b,0["e"]"#)) && sent.contains(&t(r#"2/b,1["e"]"#)));
    // A second connection continues the namespace's counter.
    let second = io.conn();
    second.text("0");
    let sent = second.sent().await;
    assert!(sent.contains(&t(r#"22["e"]"#)) && sent.contains(&t(r#"23["e"]"#)), "{sent:?}");
}

#[tokio::test(start_paused = true)]
async fn an_unanswered_emit_fails_at_its_timeout_and_the_default_is_30_seconds() {
    let io = server(options());
    let results: Results<&'static str> = Arc::new(Mutex::new(Vec::new()));
    let sockets: Arc<Mutex<Vec<IoSocket>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let (results, sockets) = (results.clone(), sockets.clone());
        io.on_connection(move |socket| {
            sockets.lock().unwrap().push(socket.clone());
            let (results_short, s) = (results.clone(), socket.clone());
            tokio::spawn(async move {
                let r = s.timeout(Duration::from_millis(200)).emit_with_ack("short", ()).await;
                results_short.lock().unwrap().push(("short", r));
            });
            let (results, s) = (results.clone(), socket);
            tokio::spawn(async move {
                let r = s.emit_with_ack("default", ()).await;
                results.lock().unwrap().push(("default", r));
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    advance(199).await;
    assert!(results.lock().unwrap().is_empty(), "not before the timeout");
    advance(1).await;
    {
        let results = results.lock().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.as_ref().unwrap_err().to_string(), "operation has timed out");
    }
    advance(29_000).await;
    assert_eq!(results.lock().unwrap().len(), 1, "the default deadline is 30 s");
    advance(799).await;
    assert_eq!(results.lock().unwrap().len(), 1);
    advance(1).await;
    assert_eq!(results.lock().unwrap().len(), 2);
    assert_eq!(results.lock().unwrap()[1].1, Err(AckError::Timeout));
    assert_eq!(sockets.lock().unwrap()[0].pending_acks(), 0, "the slots are free");
}

#[tokio::test(start_paused = true)]
async fn the_pending_cap_fails_the_extra_call_at_once_and_sends_nothing_for_it() {
    let io = server(options().max_pending_acks(2));
    let results: Results<usize> = Arc::new(Mutex::new(Vec::new()));
    {
        let results = results.clone();
        io.on_connection(move |socket| {
            for i in 0..3 {
                let (results, socket) = (results.clone(), socket.clone());
                tokio::spawn(async move {
                    let r = socket.emit_with_ack("ask", (i,)).await;
                    results.lock().unwrap().push((i, r));
                });
            }
        });
    }
    let conn = io.conn();
    conn.text("0");
    let sent = conn.sent().await;
    assert_eq!(sent.len(), 3, "CONNECT and two events: {sent:?}");
    assert_eq!(sent[1], t(r#"20["ask",0]"#));
    assert_eq!(sent[2], t(r#"21["ask",1]"#));
    {
        let results = results.lock().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], (2, Err(AckError::TooManyPending)));
    }
    conn.text(r#"30["a"]"#);
    conn.text(r#"31["b"]"#);
    settle().await;
    let results = results.lock().unwrap();
    assert_eq!(results.len(), 3);
    assert!(results.iter().filter(|(_, r)| r.is_ok()).count() == 2);
}

#[tokio::test(start_paused = true)]
async fn an_ack_from_another_socket_does_not_resolve_the_call() {
    let io = server(options());
    let result: Arc<Mutex<Option<Result<Args, AckError>>>> = Arc::new(Mutex::new(None));
    let first = Arc::new(AtomicBool::new(true));
    {
        let (result, first) = (result.clone(), first.clone());
        io.on_connection(move |socket| {
            if first.swap(false, Ordering::SeqCst) {
                let result = result.clone();
                tokio::spawn(async move {
                    let answer = socket.timeout(Duration::from_secs(10)).emit_with_ack("ask", ()).await;
                    *result.lock().unwrap() = Some(answer);
                });
            }
        });
    }
    let a = io.conn();
    a.join("0").await;
    let b = io.conn();
    b.join("0").await;
    b.text(r#"30["forged"]"#);
    settle().await;
    assert!(result.lock().unwrap().is_none(), "B cannot answer A's call");
    a.text(r#"30["real"]"#);
    settle().await;
    assert_eq!(result.lock().unwrap().take().unwrap().unwrap().to_json().unwrap(), vec![json!("real")]);
}

#[tokio::test(start_paused = true)]
async fn pending_calls_fail_at_once_when_the_socket_disconnects() {
    let io = server(options());
    let result: Arc<Mutex<Option<Result<Args, AckError>>>> = Arc::new(Mutex::new(None));
    {
        let result = result.clone();
        io.on_connection(move |socket| {
            let result = result.clone();
            tokio::spawn(async move {
                let answer = socket.emit_with_ack("ask", ()).await;
                *result.lock().unwrap() = Some(answer);
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    assert!(result.lock().unwrap().is_none());
    conn.text("1");
    settle().await;
    assert_eq!(result.lock().unwrap().take().unwrap(), Err(AckError::Disconnected));
}

#[tokio::test(start_paused = true)]
async fn an_abandoned_call_frees_its_slot_and_expired_slots_are_purged() {
    let io = server(options().max_pending_acks(1));
    let sockets: Arc<Mutex<Vec<IoSocket>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let sockets = sockets.clone();
        io.on_connection(move |socket| sockets.lock().unwrap().push(socket));
    }
    let conn = io.conn();
    conn.join("0").await;
    let socket = sockets.lock().unwrap()[0].clone();
    // A call that is dropped after it started gives its slot back.
    let call = tokio::spawn({
        let socket = socket.clone();
        async move { socket.emit_with_ack("a", ()).await }
    });
    settle().await;
    assert_eq!(socket.pending_acks(), 1);
    call.abort();
    settle().await;
    assert_eq!(socket.pending_acks(), 0);
    // A call whose future is parked, never polled again, still expires and does not hold the cap.
    let parked_emitter = socket.timeout(Duration::from_millis(100));
    let mut parked = Box::pin(parked_emitter.emit_with_ack("b", ()));
    assert!(futures::poll!(parked.as_mut()).is_pending());
    assert_eq!(socket.pending_acks(), 1);
    advance(150).await;
    let second_emitter = socket.timeout(Duration::from_secs(5));
    let second = second_emitter.emit_with_ack("c", ());
    tokio::pin!(second);
    assert!(futures::poll!(second.as_mut()).is_pending(), "the expired slot did not count against max_pending_acks");
    assert_eq!(socket.pending_acks(), 1);
}

// ----------------------------------------------------------------------------------------------------- binary

#[tokio::test(start_paused = true)]
async fn a_binary_event_is_reassembled_and_a_binary_ack_is_sent_whole() {
    let io = server(options());
    io.on_connection(|socket| {
        socket.on("echo", |_s, args, ack| async move {
            ack.unwrap().send(args).unwrap();
        });
    });
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"51-3["echo",{"name":"a","data":{"_placeholder":true,"num":0}}]"#);
    conn.bin(&[1, 2, 3]);
    assert_eq!(conn.sent().await, [t(r#"61-3[{"name":"a","data":{"_placeholder":true,"num":0}}]"#), Out::B(vec![1, 2, 3])]);
}

#[tokio::test(start_paused = true)]
async fn binary_arguments_reach_the_handler_in_place_and_deserialize_into_structs() {
    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct Upload {
        name: String,
        data: Binary,
    }
    let io = server(options());
    let seen = log();
    {
        let seen = seen.clone();
        io.on_connection(move |socket| {
            let seen = seen.clone();
            socket.on("upload", move |_s, args, _k| {
                let seen = seen.clone();
                async move {
                    let upload: Upload = args.get(0).unwrap();
                    seen.lock().unwrap().push(format!("{} {:?}", upload.name, upload.data.as_bytes()));
                }
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"51-["upload",{"name":"f","data":{"_placeholder":true,"num":0}}]"#);
    conn.bin(&[9, 8]);
    settle().await;
    assert_eq!(entries(&seen), ["f [9, 8]"]);
}

#[tokio::test(start_paused = true)]
async fn the_server_emits_binary_events_with_placeholders_and_refuses_too_many_attachments() {
    let io = server(options().max_attachments(2));
    let result = log();
    {
        let result = result.clone();
        io.on_connection(move |socket| {
            socket.emit("file", (json!({ "k": 1 }), Binary::from(vec![1u8, 2]), IoValue::from(Bytes::from_static(b"zz")))).unwrap();
            let three = (Binary::from(vec![1u8]), Binary::from(vec![2u8]), Binary::from(vec![3u8]));
            result.lock().unwrap().push(format!("{:?}", socket.emit("too-many", three)));
        });
    }
    let conn = io.conn();
    conn.text("0");
    let sent = conn.sent().await;
    assert_eq!(sent.len(), 4, "CONNECT, then the packet and its two attachments together: {sent:?}");
    assert_eq!(sent[1], t(r#"52-["file",{"k":1},{"_placeholder":true,"num":0},{"_placeholder":true,"num":1}]"#));
    assert_eq!(sent[2], Out::B(vec![1, 2]));
    assert_eq!(sent[3], Out::B(b"zz".to_vec()));
    assert_eq!(entries(&result), ["Err(TooManyAttachments)"]);
}

#[tokio::test(start_paused = true)]
async fn too_many_attachments_close_the_connection_before_any_is_accepted() {
    let io = server(options());
    let reasons = log();
    record_disconnects(&io.main(), &reasons);
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"511-["x"]"#);
    settle().await;
    assert!(conn.socket.is_closing());
    assert_eq!(entries(&reasons), ["/:forced close"]);
}

#[tokio::test(start_paused = true)]
async fn a_binary_packet_that_never_completes_is_closed_at_attachment_timeout() {
    let io = server(options().attachment_timeout(Duration::from_secs(10)));
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"510-["x",{"_placeholder":true,"num":0}]"#);
    conn.bin(&[1]);
    settle().await;
    advance(9_999).await;
    assert!(!conn.socket.is_closing());
    // More attachments do not extend the deadline of the packet.
    conn.bin(&[2]);
    settle().await;
    advance(1).await;
    assert!(conn.socket.is_closing(), "closed at the deadline although the client kept sending");
}

#[tokio::test(start_paused = true)]
async fn a_completed_binary_packet_stops_its_timer() {
    let io = server(options().attachment_timeout(Duration::from_secs(10)));
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"51-["x",{"_placeholder":true,"num":0}]"#);
    advance(5_000).await;
    conn.bin(&[1]);
    advance(60_000).await;
    assert!(!conn.socket.is_closing());
}

#[tokio::test(start_paused = true)]
async fn the_attachments_of_an_incomplete_packet_stay_accounted_and_are_freed_with_the_session() {
    let io = server(options().attachment_timeout(Duration::from_secs(10)));
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"510-["x"]"#);
    for _ in 0..9 {
        conn.bin(&vec![0u8; 100_000]);
    }
    settle().await;
    assert!(conn.socket.unread_bytes() >= 9 * 100_000, "the attachments count against max_unread_bytes");
    advance(10_000).await;
    assert!(conn.socket.is_closing());
}

#[tokio::test(start_paused = true)]
async fn an_attachment_without_a_header_and_text_during_reassembly_close_the_connection() {
    let io = server(options());
    let conn = io.conn();
    conn.join("0").await;
    conn.bin(&[1]);
    settle().await;
    assert!(conn.socket.is_closing());
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"51-["x",{"_placeholder":true,"num":0}]"#);
    conn.text(r#"2["y"]"#);
    settle().await;
    assert!(conn.socket.is_closing());
}

// ----------------------------------------------------------------------------------------- malformed and limits

#[tokio::test(start_paused = true)]
async fn undecodable_packets_close_the_connection_with_forced_close() {
    for bad in [
        "9",
        "",
        r#"2{"a":1}"#,
        r#"2["disconnect"]"#,
        r#"2abc["x"]"#,
        r#"2["x""#,
        "5-[\"x\"]",
        "2[\"x\"] x",
        "20000000000000000000[\"x\"]",
        "29007199254740992[\"x\"]",
    ] {
        let io = server(options());
        let reasons = log();
        record_disconnects(&io.main(), &reasons);
        let conn = io.conn();
        conn.join("0").await;
        conn.text(bad);
        settle().await;
        assert!(conn.socket.is_closing(), "{bad:?}");
        assert_eq!(entries(&reasons), ["/:forced close"], "{bad:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn nesting_above_the_limit_and_extreme_nesting_close_the_connection() {
    for depth in [200usize, 100_000] {
        let io = server(options());
        let conn = io.conn();
        conn.join("0").await;
        conn.text(&format!("2[\"x\",{}{}]", "[".repeat(depth), "]".repeat(depth)));
        settle().await;
        assert!(conn.socket.is_closing(), "{depth}");
    }
}

#[tokio::test(start_paused = true)]
async fn an_oversized_ack_id_closes_the_connection_and_the_largest_is_echoed() {
    let io = server(options());
    io.on_connection(|socket| {
        socket.on("echo", |_s, args, ack| async move {
            ack.unwrap().send(args).unwrap();
        });
    });
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"29007199254740991["echo",1]"#);
    assert_eq!(conn.sent().await, [t(r#"39007199254740991[1]"#)]);
    conn.text(r#"212345678901234567890["echo",1]"#);
    settle().await;
    assert!(conn.socket.is_closing());
}

#[tokio::test(start_paused = true)]
async fn event_name_and_argument_limits_close_the_connection() {
    let io = server(options().max_event_name_len(8).max_args(3));
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"2["12345678",1,2,3]"#);
    settle().await;
    assert!(!conn.socket.is_closing());
    for bad in [r#"2["123456789"]"#, r#"2["x",1,2,3,4]"#, r#"30["a",1,2,3,4]"#] {
        let conn = io.conn();
        conn.join("0").await;
        conn.text(bad);
        settle().await;
        assert!(conn.socket.is_closing(), "{bad}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_huge_namespace_in_a_packet_closes_the_connection() {
    let io = server(options());
    let conn = io.conn();
    conn.text(&format!("0/{},", "a".repeat(300)));
    settle().await;
    assert!(conn.socket.is_closing());
}

// --------------------------------------------------------------------------------------------------- disconnect

#[tokio::test(start_paused = true)]
async fn the_client_leaves_one_namespace_and_can_join_it_again() {
    let io = server(options());
    io.of("/admin").unwrap();
    let reasons = log();
    record_disconnects(&io.main(), &reasons);
    record_disconnects(&io.of("/admin").unwrap(), &reasons);
    let conn = io.conn();
    conn.join("0").await;
    conn.join("0/admin,").await;
    conn.text("1/admin,");
    settle().await;
    assert_eq!(entries(&reasons), ["/admin:client namespace disconnect"]);
    assert!(!conn.socket.is_closing());
    assert_eq!(io.of("/admin").unwrap().socket_count(), 0);
    assert_eq!(io.main().socket_count(), 1);
    conn.join("0/admin,").await;
}

#[tokio::test(start_paused = true)]
async fn the_server_leaves_one_namespace_or_closes_the_connection() {
    let io = server(options());
    io.of("/admin").unwrap();
    let reasons = log();
    record_disconnects(&io.main(), &reasons);
    record_disconnects(&io.of("/admin").unwrap(), &reasons);
    let conn = io.conn();
    conn.join("0").await;
    let admin = conn.join("0/admin,").await;
    io.of("/admin").unwrap().socket(&admin).unwrap().disconnect(false);
    assert_eq!(conn.sent().await, [t("1/admin,")]);
    assert_eq!(entries(&reasons), ["/admin:server namespace disconnect"]);
    assert!(!conn.socket.is_closing());
    io.main().sockets()[0].disconnect(true);
    assert_eq!(conn.sent().await, [t("1")]);
    assert!(conn.socket.is_closing());
    assert_eq!(entries(&reasons), ["/admin:server namespace disconnect", "/:server namespace disconnect"]);
    // Disconnecting twice does nothing.
    if let Some(socket) = io.main().sockets().first() {
        socket.disconnect(false);
    }
}

#[tokio::test(start_paused = true)]
async fn disconnecting_handlers_find_the_socket_and_disconnect_handlers_do_not_each_once() {
    let io = server(options());
    let order = log();
    {
        let order = order.clone();
        io.on_connection(move |socket| {
            let (a, b) = (order.clone(), order.clone());
            socket.on_disconnecting(move |s, reason| {
                a.lock().unwrap().push(format!("disconnecting {} {}", s.namespace().socket(s.id()).is_some(), reason))
            });
            socket.on_disconnect(move |s, reason| {
                b.lock().unwrap().push(format!("disconnect {} {}", s.namespace().socket(s.id()).is_some(), reason))
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    conn.socket.close();
    let socket = io.main().sockets().pop().unwrap();
    socket.disconnect(false);
    socket.disconnect(false);
    conn.text("1");
    settle().await;
    assert_eq!(entries(&order), ["disconnecting true server namespace disconnect", "disconnect false server namespace disconnect"]);
}

#[tokio::test(start_paused = true)]
async fn the_engine_reason_reaches_the_sockets() {
    let io = server(options());
    let reasons = log();
    record_disconnects(&io.main(), &reasons);
    let conn = io.conn();
    conn.join("0").await;
    // A session that closes by itself: here through the server closing the engine path of a graceful close.
    conn.socket.close();
    advance(31_000).await;
    assert_eq!(entries(&reasons), ["/:forced close"]);
}

#[tokio::test(start_paused = true)]
async fn closing_the_server_reports_server_shutting_down_before_the_engine_closes_and_fails_acks() {
    let io = server(options());
    let reasons = log();
    record_disconnects(&io.main(), &reasons);
    let result: Arc<Mutex<Option<Result<Args, AckError>>>> = Arc::new(Mutex::new(None));
    {
        let result = result.clone();
        io.on_connection(move |socket| {
            let result = result.clone();
            tokio::spawn(async move {
                *result.lock().unwrap() = Some(socket.emit_with_ack("ask", ()).await);
            });
        });
    }
    let (a, b) = (io.conn(), io.conn());
    a.join("0").await;
    b.join("0").await;
    io.close();
    settle().await;
    assert_eq!(entries(&reasons), ["/:server shutting down", "/:server shutting down"]);
    assert!(a.socket.is_closing() && b.socket.is_closing());
    assert_eq!(result.lock().unwrap().take().unwrap(), Err(AckError::Disconnected));
    io.close();
}

#[tokio::test(start_paused = true)]
async fn the_engine_hook_reports_the_shutdown_when_the_application_closes_the_engine() {
    let io = server(options());
    let reasons = log();
    record_disconnects(&io.main(), &reasons);
    let conn = io.conn();
    conn.join("0").await;
    io.engine().close();
    settle().await;
    assert_eq!(entries(&reasons), ["/:server shutting down"]);
    assert!(conn.socket.is_closing());
}

#[tokio::test(start_paused = true)]
async fn emits_after_the_disconnect_fail_and_send_nothing() {
    let io = server(options());
    let sockets: Arc<Mutex<Vec<IoSocket>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let sockets = sockets.clone();
        io.on_connection(move |socket| sockets.lock().unwrap().push(socket));
    }
    let conn = io.conn();
    conn.join("0").await;
    conn.text("1");
    settle().await;
    let socket = sockets.lock().unwrap()[0].clone();
    assert!(!socket.connected());
    assert_eq!(socket.emit("x", ()), Err(EmitError::NotConnected));
    assert_eq!(socket.timeout(Duration::from_secs(1)).emit_with_ack("x", ()).await, Err(AckError::Emit(EmitError::NotConnected)));
    assert!(conn.sent().await.is_empty());
    // An event that was already on its way when the namespace was left is dropped (Node does the same): the
    // connection stays open, and so do the client's other namespaces.
    conn.text(r#"2["late"]"#);
    conn.text("31[]");
    settle().await;
    assert!(!conn.socket.is_closing(), "a packet for a namespace the connection joined and left is dropped");
    // A namespace it never joined is still a violation.
    conn.text(r#"2/never,["x"]"#);
    settle().await;
    assert!(conn.socket.is_closing());
}

#[tokio::test(start_paused = true)]
async fn packets_for_a_namespace_the_server_just_left_are_dropped_and_the_other_namespaces_survive() {
    let io = server(options());
    let chat = io.of("/chat").unwrap();
    let held: Arc<Mutex<Vec<IoSocket>>> = Arc::new(Mutex::new(Vec::new()));
    let events = log();
    {
        let (held, events) = (held.clone(), events.clone());
        chat.on_connection(move |socket| {
            held.lock().unwrap().push(socket.clone());
            let events = events.clone();
            socket.on("say", move |_s, _a, _k| {
                let events = events.clone();
                async move { events.lock().unwrap().push("chat:say".into()) }
            });
        });
    }
    {
        let events = events.clone();
        io.on_connection(move |socket| {
            let events = events.clone();
            socket.on("say", move |_s, _a, _k| {
                let events = events.clone();
                async move { events.lock().unwrap().push("main:say".into()) }
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    conn.join("0/chat,").await;
    // The server leaves the namespace; the client's packets for it were already on their way.
    let socket = held.lock().unwrap()[0].clone();
    socket.disconnect(false);
    settle().await;
    let _ = conn.sent().await;
    conn.text(r#"2/chat,["say"]"#);
    conn.text(r#"3/chat,5[]"#);
    conn.text(r#"1/chat,"#);
    // A binary event: the packet and its attachment are consumed and dropped, delivered to nobody.
    conn.text(r#"51-/chat,["say",{"_placeholder":true,"num":0}]"#);
    conn.bin(b"attachment");
    settle().await;
    assert!(!conn.socket.is_closing(), "the connection survives");
    assert_eq!(entries(&events), Vec::<String>::new(), "nothing is delivered for the namespace that was left");
    assert_eq!(io.engine().total_unread_bytes(), 0, "the dropped packets released everything they held");
    // The other namespace works, and the left one may be joined again (the entry is cleared by the CONNECT).
    conn.text(r#"2["say"]"#);
    settle().await;
    assert_eq!(entries(&events), ["main:say"]);
    conn.join("0/chat,").await;
    conn.text(r#"2/chat,["say"]"#);
    settle().await;
    assert_eq!(entries(&events), ["main:say", "chat:say"]);
    // A namespace the connection never joined is still a violation, with the same other state around it.
    chat.on_connection(|_| {});
    conn.text(r#"2/other,["say"]"#);
    settle().await;
    assert!(conn.socket.is_closing());
}

#[tokio::test(start_paused = true)]
async fn a_refused_connect_leaves_no_trace_so_its_packets_still_close_the_connection() {
    let io = server(options());
    io.of("/chat").unwrap().use_middleware(|_| async { Err(ConnectError::new("no")) });
    let conn = io.conn();
    conn.text("0/chat,");
    let _ = conn.sent().await;
    conn.text(r#"2/chat,["say"]"#);
    settle().await;
    assert!(conn.socket.is_closing(), "a namespace the connection was refused is not one it left");
}

/// A gate that holds the handler of `slow` until it is released.
fn slow_handler(io: &IoServer, started: &Arc<AtomicUsize>, finished: &Arc<AtomicUsize>) {
    let (started, finished) = (started.clone(), finished.clone());
    io.on_connection(move |socket| {
        let (started, finished) = (started.clone(), finished.clone());
        socket.on("slow", move |_s, _a, _k| {
            let (started, finished) = (started.clone(), finished.clone());
            async move {
                started.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_secs(5)).await;
                finished.fetch_add(1, Ordering::SeqCst);
            }
        });
    });
}

#[tokio::test(start_paused = true)]
async fn handlers_are_cancelled_at_once_when_the_client_drops_the_connection_unless_the_application_opted_in() {
    for (opt_in, expected_finished) in [(false, 0usize), (true, 3)] {
        let io = server(options().finish_handlers_after_client_close(opt_in));
        let (started, finished) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        slow_handler(&io, &started, &finished);
        let conn = io.conn();
        conn.join("0").await;
        for _ in 0..3 {
            conn.text(r#"2["slow"]"#);
        }
        settle().await;
        assert_eq!(started.load(Ordering::SeqCst), 3);
        // The client's transport closes.
        conn.socket.kill();
        settle().await;
        advance(5_000).await;
        assert_eq!(finished.load(Ordering::SeqCst), expected_finished, "opted in: {opt_in}");
        assert_eq!(io.engine().total_unread_bytes(), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn handlers_get_their_grace_when_the_server_closes_the_connection_and_count_against_the_address_meanwhile() {
    let engine = EngineOptions::new().ping_interval(DAY).ping_timeout(DAY).max_sessions_per_ip(2);
    let io = server(IoOptions::new().engine(engine).max_inflight(4));
    let (started, finished) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    slow_handler(&io, &started, &finished);
    assert_eq!(io.inner.options.max_lingering_per_ip(), 8, "max_sessions_per_ip x max_inflight");
    let conn = io.conn_from("203.0.113.40");
    conn.join("0").await;
    for _ in 0..3 {
        conn.text(r#"2["slow"]"#);
    }
    settle().await;
    assert_eq!(io.engine().lingering("203.0.113.40"), 0, "running handlers of an open connection do not linger");
    // The server closes the connection (a graceful close, as a shutdown does): the handlers get `close_timeout`.
    assert_eq!(started.load(Ordering::SeqCst), 3);
    conn.socket.force_close();
    settle().await;
    assert!(conn.socket.is_closing());
    assert_eq!(io.engine().lingering("203.0.113.40"), 3);
    advance(5_000).await;
    assert_eq!(finished.load(Ordering::SeqCst), 3, "they finished");
    assert_eq!(io.engine().lingering("203.0.113.40"), 0, "and no longer count");
}

#[tokio::test(start_paused = true)]
async fn an_address_with_too_many_lingering_handlers_is_refused_new_sessions_until_they_end() {
    let engine = EngineOptions::new().ping_interval(DAY).ping_timeout(DAY).max_sessions_per_ip(1);
    let io = server(IoOptions::new().engine(engine).max_inflight(2));
    let (started, finished) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    slow_handler(&io, &started, &finished);
    // The cap is 1 x 2 handlers. Two lingering handlers reach it.
    let conn = io.conn_from("203.0.113.41");
    conn.join("0").await;
    conn.text(r#"2["slow"]"#);
    conn.text(r#"2["slow"]"#);
    settle().await;
    // The connection is at `max_inflight`, so it reads nothing more: the server ends it (a shutdown does this).
    conn.socket.force_close();
    settle().await;
    assert!(conn.socket.is_closing());
    assert_eq!(io.engine().lingering("203.0.113.41"), 2);
    assert!(io.engine().refuses_new_sessions_from("203.0.113.41"), "the address is at its cap");
    assert!(!io.engine().refuses_new_sessions_from("203.0.113.42"), "another address is not");
    advance(5_000).await;
    assert!(!io.engine().refuses_new_sessions_from("203.0.113.41"), "served again when they end");
}

#[tokio::test(start_paused = true)]
async fn an_event_that_runs_when_the_connection_ends_gets_a_grace_period() {
    let io = server(options());
    let done = Arc::new(AtomicBool::new(false));
    {
        let done = done.clone();
        io.on_connection(move |socket| {
            let done = done.clone();
            socket.on("slow", move |_s, _a, _k| {
                let done = done.clone();
                async move {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    done.store(true, Ordering::SeqCst);
                }
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"2["slow"]"#);
    settle().await;
    // The server ends the connection, as a shutdown does: the handler keeps its grace period.
    conn.socket.force_close();
    settle().await;
    advance(5_000).await;
    assert!(done.load(Ordering::SeqCst), "a handler already running is not cancelled when the server closes the connection");
}

/// A connection the server closes because the client broke the protocol is the client's doing: nobody is left to answer,
/// so its handlers are cancelled at once, like those of a client that dropped the connection.
#[tokio::test(start_paused = true)]
async fn an_event_that_runs_when_the_client_breaks_the_protocol_is_cancelled() {
    let io = server(options());
    let done = Arc::new(AtomicBool::new(false));
    {
        let done = done.clone();
        io.on_connection(move |socket| {
            let done = done.clone();
            socket.on("slow", move |_s, _a, _k| {
                let done = done.clone();
                async move {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    done.store(true, Ordering::SeqCst);
                }
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"2["slow"]"#);
    settle().await;
    conn.text("0"); // a second CONNECT to a namespace already joined: a violation
    settle().await;
    assert!(conn.socket.is_closing());
    advance(5_000).await;
    assert!(!done.load(Ordering::SeqCst), "the handler was dropped with the connection");
    assert_eq!(io.engine().total_unread_bytes(), 0);
}

// -------------------------------------------------------------------------------- the decoded-packet budget

/// A server whose server-wide budget is 1 MiB and whose packets cost at most 256 KiB, so the rules of `budget.rs`
/// are reached with small packets: one address may hold one maximum packet (312 kB: a decoded packet and one wire message), and a session may always
/// hold the fair share of 16 KiB.
fn tight() -> IoOptions {
    let engine = EngineOptions::new()
        .ping_interval(DAY)
        .ping_timeout(DAY)
        .max_http_buffer_size(50_000)
        .max_unread_bytes(1024 * 1024)
        .max_total_unread_bytes(1024 * 1024);
    IoOptions::new().engine(engine).max_decoded_bytes(256 * 1024).max_attachments(1)
}

/// What a client address holds in all (the gauge between its sessions and the server-wide one).
fn address_held(io: &IoServer, address: &str) -> usize {
    io.engine().address_unread_bytes(address)
}

/// `n` empty objects in one argument: 3 bytes of text each, 128 bytes of structure each.
fn hold_event(n: usize) -> String {
    format!(r#"2["hold",[{}]]"#, vec!["{}"; n].join(","))
}

/// The cost of `hold_event(1_500)`: about 205 kB.
const BIG: usize = 1_500;

struct Gate {
    entered: Arc<AtomicUsize>,
    small: Arc<AtomicUsize>,
    release: Arc<Notify>,
}

/// `hold` keeps its arguments until `release` is notified; `small` only counts.
fn gated(io: &IoServer) -> Gate {
    let gate = Gate { entered: Arc::new(AtomicUsize::new(0)), small: Arc::new(AtomicUsize::new(0)), release: Arc::new(Notify::new()) };
    let (entered, small, release) = (gate.entered.clone(), gate.small.clone(), gate.release.clone());
    io.on_connection(move |socket| {
        let (entered, release) = (entered.clone(), release.clone());
        socket.on("hold", move |_s, args, _a| {
            let (entered, release) = (entered.clone(), release.clone());
            async move {
                entered.fetch_add(1, Ordering::SeqCst);
                release.notified().await;
                drop(args);
            }
        });
        let small = small.clone();
        socket.on("small", move |_s, _a, _k| {
            small.fetch_add(1, Ordering::SeqCst);
            async {}
        });
    });
    gate
}

impl Gate {
    fn entered(&self) -> usize {
        self.entered.load(Ordering::SeqCst)
    }

    fn small(&self) -> usize {
        self.small.load(Ordering::SeqCst)
    }

    async fn open(&self) {
        self.release.notify_waiters();
        settle().await;
    }

    /// Opens the gate until nothing new enters: every handler that was waiting for room has run and ended.
    async fn drain(&self) {
        for _ in 0..30 {
            self.open().await;
        }
    }
}

trait OpenFrom {
    fn conn_from(&self, address: &str) -> Conn;
}

impl OpenFrom for IoServer {
    fn conn_from(&self, address: &str) -> Conn {
        Conn { socket: self.engine().open_test_socket_from(address) }
    }
}

#[tokio::test(start_paused = true)]
async fn a_decoded_packet_is_charged_at_its_cost_not_at_what_was_reserved_for_it() {
    let io = server(tight());
    let gate = gated(&io);
    let conn = io.conn();
    conn.join("0").await;
    // 2,000 times the same key: the text says up to 2,000 entries (about 230 kB reserved), the object has one.
    let repeated = vec![r#""a":1"#; 2_000].join(",");
    conn.text(&format!(r#"2["hold",{{{repeated}}}]"#));
    settle().await;
    assert_eq!(gate.entered(), 1);
    let held = io.engine().total_unread_bytes();
    assert!(held < 2_048, "the reservation shrank to the cost of the object that exists: {held} bytes");
    assert_eq!(conn.socket.unread_bytes(), held, "the wire message was released too");
    gate.open().await;
    assert_eq!(io.engine().total_unread_bytes(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_packet_that_finds_no_room_waits_and_the_connection_takes_nothing_else() {
    let io = server(tight());
    let gate = gated(&io);
    let first = io.conn_from("203.0.113.1");
    let second = io.conn_from("203.0.113.1");
    for c in [&first, &second] {
        c.join("0").await;
    }
    first.text(&hold_event(BIG));
    settle().await;
    assert_eq!(gate.entered(), 1);
    let one_packet = io.engine().total_unread_bytes();
    assert!(one_packet > 200_000 && one_packet < 210_000, "{one_packet}");
    // The same address cannot hold a second packet of this size (its quota is 256 kB): it waits, with its message
    // still charged, and the connection takes no further message from the engine.
    second.text(&hold_event(BIG));
    second.text(r#"2["small"]"#);
    second.text(r#"2["small"]"#);
    settle().await;
    assert_eq!(gate.entered(), 1, "the second packet is parked");
    assert_eq!(gate.small(), 0, "and nothing behind it is taken");
    assert!(second.socket.unread_bytes() > 4_500, "the waiting message and the queued ones stay accounted");
    assert!(!second.socket.is_closing(), "waiting is not an error");
    // Room released: it goes in, and the two behind it follow in order.
    gate.open().await;
    assert_eq!(gate.entered(), 2);
    assert_eq!(gate.small(), 2);
    gate.open().await;
    assert_eq!(io.engine().total_unread_bytes(), 0);
    assert_eq!(second.socket.unread_bytes(), 0);
}

#[tokio::test(start_paused = true)]
async fn an_honest_address_is_served_while_another_holds_its_maximum() {
    let io = server(tight());
    let gate = gated(&io);
    let attackers: Vec<Conn> = (0..8).map(|_| io.conn_from("203.0.113.1")).collect();
    for c in &attackers {
        c.join("0").await;
        c.text(&hold_event(BIG));
    }
    settle().await;
    assert_eq!(gate.entered(), 1, "one address holds one packet of this size: its share is one maximum packet");
    // Another address, and a packet that needs room in the server-wide budget (above the fair share).
    let honest = io.conn_from("198.51.100.7");
    honest.join("0").await;
    honest.text(&hold_event(BIG));
    honest.text(r#"2["small"]"#);
    settle().await;
    assert_eq!(gate.entered(), 2, "the honest packet is decoded while the attacker's seven wait");
    assert_eq!(gate.small(), 1);
    // A small packet is always taken, even by a session of the attacking address (the guaranteed share).
    attackers[7].text(r#"2["small"]"#);
    settle().await;
    assert_eq!(gate.small(), 1, "queued behind its own parked packet, as any connection");
    let fresh = io.conn_from("203.0.113.1");
    fresh.join("0").await;
    fresh.text(r#"2["small"]"#);
    settle().await;
    assert_eq!(gate.small(), 2, "a session with nothing waiting is served from its guaranteed share");
    gate.drain().await;
    assert_eq!(gate.entered(), 9, "the attacking address's seven waiting packets run after, one at a time");
    assert_eq!(io.engine().total_unread_bytes(), 0);
}

#[tokio::test(start_paused = true)]
async fn the_charge_of_a_running_handler_outlives_its_session_and_is_not_multiplied_by_reconnecting() {
    let io = server(tight());
    let gate = gated(&io);
    let first = io.conn_from("203.0.113.1");
    first.join("0").await;
    first.text(&hold_event(BIG));
    settle().await;
    let charge = io.engine().total_unread_bytes();
    assert_eq!(gate.entered(), 1);
    assert_eq!(address_held(&io, "203.0.113.1"), charge);
    // The session ends; the handler keeps running (it gets its grace period) and keeps its charge.
    // (A graceful close ends the session after `close_timeout`, 30 s; the handler's own grace period starts then.)
    first.socket.close();
    advance(31_000).await;
    assert!(first.socket.is_closing());
    assert_eq!(io.engine().total_unread_bytes(), charge, "the server-wide gauge still counts the running handler");
    assert_eq!(address_held(&io, "203.0.113.1"), charge, "and so does the address");
    // Reconnecting from the same address does not give it a new quota: the packet waits.
    let again = io.conn_from("203.0.113.1");
    again.join("0").await;
    again.text(&hold_event(BIG));
    settle().await;
    assert_eq!(gate.entered(), 1);
    // Other addresses are not affected.
    let other = io.conn_from("198.51.100.7");
    other.join("0").await;
    other.text(&hold_event(BIG));
    settle().await;
    assert_eq!(gate.entered(), 2);
    // The handler ends: the room is released and the new session's packet goes in.
    gate.open().await;
    assert_eq!(gate.entered(), 3);
    gate.open().await;
    assert_eq!(io.engine().total_unread_bytes(), 0);
    assert_eq!(address_held(&io, "203.0.113.1"), 0);
}

#[tokio::test(start_paused = true)]
async fn reconnecting_over_and_over_cannot_multiply_what_an_address_holds_even_in_guaranteed_shares() {
    // Five open sessions per address: the guaranteed shares an address may use add up to 5 x 16 KiB. Sessions that
    // close at once keep their packets charged while the handlers run, so each reconnect would give the address a
    // fresh share if the count belonged to the session.
    let engine = EngineOptions::new()
        .ping_interval(DAY)
        .ping_timeout(DAY)
        .max_http_buffer_size(50_000)
        .max_unread_bytes(1024 * 1024)
        .max_total_unread_bytes(1024 * 1024)
        .max_sessions_per_ip(5);
    // The handlers outlive their client here (the application asked for it): the charges are what is measured.
    let io =
        server(IoOptions::new().engine(engine).max_decoded_bytes(256 * 1024).max_attachments(1).finish_handlers_after_client_close(true));
    let gate = gated(&io);
    // The share is the floor here (one maximum packet: the decoded packet and one 50 kB message), above 1/16.
    let cap = io.inner.options.address_share() + io.inner.options.engine.address_guarantee();
    assert_eq!(cap, 256 * 1024 + (50_000 + 64) + 5 * 16 * 1024);
    // 100 empty objects: about 14 kB decoded, below the guaranteed share.
    for _ in 0..60 {
        let conn = io.conn_from("203.0.113.9");
        conn.join("0").await;
        conn.text(&hold_event(100));
        settle().await;
        conn.socket.kill();
        settle().await;
    }
    let held = io.engine().total_unread_bytes();
    assert!(held <= cap, "the address holds {held} bytes, above its cap of {cap}");
    assert!(gate.entered() >= 20 && gate.entered() <= 28, "{} packets admitted before the cap", gate.entered());
    // Another address is untouched.
    let admitted = gate.entered();
    let other = io.conn_from("198.51.100.7");
    other.join("0").await;
    other.text(&hold_event(100));
    settle().await;
    assert_eq!(gate.entered(), admitted + 1, "an address that holds nothing is served at once");
    gate.drain().await;
    assert_eq!(io.engine().total_unread_bytes(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_handler_that_never_ends_loses_its_charge_when_the_grace_period_does() {
    let io = server(tight());
    let gate = gated(&io);
    let conn = io.conn_from("203.0.113.1");
    conn.join("0").await;
    conn.text(&hold_event(BIG));
    settle().await;
    assert!(io.engine().total_unread_bytes() > 200_000);
    // The session ends after `close_timeout` (a graceful close); the handler's own grace period of `close_timeout`
    // starts then, and is measured from that moment, so the clock is moved in steps the tasks can follow.
    conn.socket.close();
    advance(31_000).await;
    assert!(io.engine().total_unread_bytes() > 200_000, "the handler gets its grace period");
    advance(28_000).await;
    assert!(io.engine().total_unread_bytes() > 200_000, "still inside close_timeout");
    advance(3_000).await;
    assert_eq!(io.engine().total_unread_bytes(), 0, "after close_timeout the handler is dropped and the room is free");
    assert_eq!(gate.entered(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_waiting_packet_is_released_with_its_session() {
    let io = server(tight());
    let gate = gated(&io);
    let first = io.conn_from("203.0.113.1");
    let second = io.conn_from("203.0.113.1");
    for c in [&first, &second] {
        c.join("0").await;
    }
    first.text(&hold_event(BIG));
    second.text(&hold_event(BIG));
    settle().await;
    let both = io.engine().total_unread_bytes();
    assert_eq!(gate.entered(), 1);
    second.socket.close();
    advance(31_000).await;
    assert!(io.engine().total_unread_bytes() < both - 4_500, "the parked wire message went with its session");
    gate.open().await;
    assert_eq!(io.engine().total_unread_bytes(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_connection_over_its_unread_limit_takes_no_more_messages() {
    // The session limit is 64 KiB (the engine's own test, not a counter of the layer): four packets of about 20 kB
    // pass it, and the fifth stays in the engine until a handler ends.
    let engine = EngineOptions::new()
        .ping_interval(DAY)
        .ping_timeout(DAY)
        .max_http_buffer_size(10_000)
        .max_unread_bytes(64 * 1024)
        .max_total_unread_bytes(1024 * 1024);
    let io = server(IoOptions::new().engine(engine).max_decoded_bytes(256 * 1024).max_inflight(100).max_attachments(1));
    let gate = gated(&io);
    let conn = io.conn();
    conn.join("0").await;
    for _ in 0..8 {
        conn.text(&hold_event(150));
    }
    settle().await;
    assert_eq!(gate.entered(), 4, "reads stop once the session holds more than its limit");
    assert!(conn.socket.unread_bytes() > 64 * 1024);
    gate.open().await;
    assert_eq!(gate.entered(), 8, "when room is released the rest is taken");
    gate.open().await;
    assert_eq!(conn.socket.unread_bytes(), 0);
}

// ----------------------------------------------------------------------------------- round 2: one accounting path

#[tokio::test(start_paused = true)]
async fn a_packet_that_waits_longer_than_the_admission_timeout_closes_the_connection_with_its_own_reason() {
    let io = server(tight().admission_timeout(Duration::from_secs(5)));
    let reasons = log();
    record_disconnects(&io.main(), &reasons);
    let gate = gated(&io);
    let first = io.conn_from("203.0.113.1");
    let second = io.conn_from("203.0.113.1");
    for c in [&first, &second] {
        c.join("0").await;
    }
    first.text(&hold_event(BIG));
    second.text(&hold_event(BIG));
    settle().await;
    assert_eq!(gate.entered(), 1, "the second packet is parked");
    advance(4_900).await;
    assert!(!second.socket.is_closing(), "waiting is not an error before the limit");
    advance(200).await;
    assert!(second.socket.is_closing(), "the wait is bounded");
    assert!(!first.socket.is_closing(), "the connection that holds the room is not the one that waited");
    assert_eq!(entries(&reasons), ["/:admission timeout"]);
    assert_eq!(DisconnectReason::AdmissionTimeout.as_str(), "admission timeout");
    gate.drain().await;
    assert_eq!(io.engine().total_unread_bytes(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_parked_packet_is_admitted_as_soon_as_room_is_released_and_its_deadline_is_forgotten() {
    let io = server(tight().admission_timeout(Duration::from_secs(5)));
    let gate = gated(&io);
    let first = io.conn_from("203.0.113.1");
    let second = io.conn_from("203.0.113.1");
    for c in [&first, &second] {
        c.join("0").await;
    }
    first.text(&hold_event(BIG));
    second.text(&hold_event(BIG));
    advance(3_000).await;
    assert_eq!(gate.entered(), 1);
    // The handler of the first returns: no time passes, and the parked packet is taken (no polling loop, no lost wakeup).
    gate.release.notify_waiters();
    settle().await;
    assert_eq!(gate.entered(), 2, "woken by the release, without any time passing");
    // Its wait is over: the old deadline (2 s from now) must not close the connection.
    advance(10_000).await;
    assert!(!second.socket.is_closing());
    gate.drain().await;
    assert_eq!(io.engine().total_unread_bytes(), 0);
}

#[tokio::test(start_paused = true)]
async fn binary_attachments_count_against_the_address_in_every_state() {
    let io = server(tight());
    let gate = gated(&io);
    let header = r#"51-["hold",{"_placeholder":true,"num":0}]"#;
    let conn = io.conn_from("203.0.113.1");
    conn.join("0").await;
    conn.text(header);
    conn.bin(&vec![7u8; 40_000]);
    settle().await;
    assert_eq!(gate.entered(), 1);
    let held = address_held(&io, "203.0.113.1");
    assert_eq!(held, io.engine().total_unread_bytes());
    assert!(held >= 40_064, "the attachment the handler keeps is charged to the address: {held}");
    // Eight sessions of one address, each with two binary events for handlers that keep them: the address cannot
    // take them all, because what it already holds (attachments included) is above its quota.
    let quota = io.inner.options.address_share();
    let sessions: Vec<Conn> = (0..8).map(|_| io.conn_from("203.0.113.1")).collect();
    for c in &sessions {
        c.join("0").await;
        for _ in 0..2 {
            c.text(header);
            c.bin(&vec![7u8; 40_000]);
        }
    }
    settle().await;
    assert!(gate.entered() <= 1 + quota / 40_000 + 1, "{} binary events admitted from one address; quota {quota}", gate.entered());
    assert!(gate.entered() >= 2, "{}", gate.entered());
    // Another address is served.
    let honest = io.conn_from("198.51.100.7");
    honest.join("0").await;
    let before = gate.entered();
    honest.text(header);
    honest.bin(&vec![7u8; 40_000]);
    settle().await;
    assert_eq!(gate.entered(), before + 1);
    for _ in 0..4 {
        gate.drain().await;
    }
    // What the address still holds is the unread bytes of sessions whose packets wait: they are admitted as room
    // appears, and a packet that cannot be admitted is closed at the admission timeout (30 s here), which frees them.
    advance(31_000).await;
    for _ in 0..4 {
        gate.drain().await;
    }
    assert_eq!(io.engine().total_unread_bytes(), 0);
    assert_eq!(address_held(&io, "203.0.113.1"), 0);
}

#[tokio::test(start_paused = true)]
async fn the_handshake_auth_is_charged_for_the_life_of_the_socket_at_the_size_of_the_stored_value() {
    let io = server(tight());
    let conn = io.conn_from("203.0.113.1");
    assert_eq!(io.engine().total_unread_bytes(), 0);
    // 40 objects with one entry each: about 4 kB of text, well inside max_auth_bytes, a map of its own each as a `Value`.
    let auth = format!("[{}]", vec![r#"{"":0}"#; 40].join(","));
    conn.join(&format!("0{{\"a\":{auth}}}")).await;
    let held = address_held(&io, "203.0.113.1");
    // The stored value is built from the decoded packet with exact capacities.
    let parsed = serde_json::from_str::<serde_json::Value>(&format!("{{\"a\":{auth}}}")).unwrap();
    let stored = super::value::json_cost(&IoValue::from_serialize(&parsed).unwrap().to_json().unwrap());
    assert!(stored > 40 * 250, "every one-entry object owns a map of its own: {stored}");
    assert_eq!(held, stored, "the socket holds exactly what its auth occupies");
    // The socket ends: the charge goes with it.
    conn.socket.kill();
    settle().await;
    assert_eq!(address_held(&io, "203.0.113.1"), 0);
    assert_eq!(io.engine().total_unread_bytes(), 0);
}

#[tokio::test(start_paused = true)]
async fn the_auth_of_many_sockets_in_many_namespaces_adds_up_on_the_address_and_leaves_with_them() {
    let io = server(tight());
    for n in 0..10 {
        io.of(&format!("/n{n}")).unwrap();
    }
    let auth = format!(r#"{{"a":[{}],"b":"{}"}}"#, [r#"{"":0}"#; 8].join(","), "x".repeat(100));
    let one = super::value::json_cost(&serde_json::from_str::<serde_json::Value>(&auth).unwrap());
    let conns: Vec<Conn> = (0..3).map(|_| io.conn_from("203.0.113.1")).collect();
    for conn in &conns {
        for n in 0..10 {
            conn.text(&format!("0/n{n},{auth}"));
            settle().await;
        }
        settle().await;
        let sent = conn.sent().await;
        assert_eq!(sent.len(), 10, "ten CONNECT answers: {sent:?}");
    }
    assert_eq!(address_held(&io, "203.0.113.1"), 30 * one, "thirty sockets, each charged for its own auth");
    assert_eq!(io.engine().total_unread_bytes(), 30 * one);
    // A session ending releases its ten sockets' charges, and nobody else's.
    conns[0].socket.kill();
    settle().await;
    assert_eq!(address_held(&io, "203.0.113.1"), 20 * one);
    for conn in &conns[1..] {
        conn.socket.kill();
    }
    settle().await;
    assert_eq!(address_held(&io, "203.0.113.1"), 0);
}

mod gateways;
mod review;
mod rooms;
