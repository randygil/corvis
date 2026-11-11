//! The gateway layer's hold on events, without a network, on the paused clock: events that wait for the connection hooks
//! occupy handler slots (so reads pause at `max_inflight`), nothing is lost when the gate opens, and a discarded gate
//! lets the waiting events go without running anything.

use futures::FutureExt;

use super::*;
use crate::socket_io::gateway::Gate;

/// A namespace whose dispatcher holds every event on `gate` and counts how many started and how many were admitted.
fn held(io: &IoServer, gate: &Arc<Gate>) -> (Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let started = Arc::new(AtomicUsize::new(0));
    let admitted = Arc::new(AtomicUsize::new(0));
    let (s, a, gate) = (started.clone(), admitted.clone(), gate.clone());
    io.main().set_dispatcher(Arc::new(move |_socket, _event, _args, _ack, charge| {
        let (s, a) = (s.clone(), a.clone());
        let mut ticket = gate.ticket();
        Some(
            async move {
                let _charge = charge;
                s.fetch_add(1, Ordering::SeqCst);
                if ticket.admitted().await {
                    a.fetch_add(1, Ordering::SeqCst);
                }
            }
            .boxed(),
        )
    }));
    (started, admitted)
}

#[tokio::test(start_paused = true)]
async fn events_waiting_for_the_gate_count_as_handlers_in_flight_and_reads_stop_at_the_limit() {
    let io = server(options().max_inflight(3));
    let gate = Arc::new(Gate::new());
    let (started, admitted) = held(&io, &gate);
    let conn = io.conn();
    conn.join("0").await;
    for _ in 0..50 {
        conn.text(r#"2["held"]"#);
    }
    settle().await;
    assert_eq!(started.load(Ordering::SeqCst), 3, "three events wait, the connection reads no more");
    assert_eq!(admitted.load(Ordering::SeqCst), 0);
    assert!(conn.socket.unread_bytes() > 0, "the packets that were not taken stay accounted for");
    gate.open();
    settle().await;
    assert_eq!(started.load(Ordering::SeqCst), 50, "reads resumed and no event was lost");
    assert_eq!(admitted.load(Ordering::SeqCst), 50);
    assert_eq!(conn.socket.unread_bytes(), 0, "everything is released when the events finish");
}

#[tokio::test(start_paused = true)]
async fn a_discarded_gate_releases_the_waiting_events_without_admitting_them() {
    let io = server(options().max_inflight(2));
    let gate = Arc::new(Gate::new());
    let (started, admitted) = held(&io, &gate);
    let conn = io.conn();
    conn.join("0").await;
    for _ in 0..6 {
        conn.text(r#"2["held"]"#);
    }
    settle().await;
    assert_eq!(started.load(Ordering::SeqCst), 2);
    gate.discard();
    settle().await;
    assert_eq!(admitted.load(Ordering::SeqCst), 0, "no waiting event is admitted after a discard");
    assert_eq!(conn.socket.unread_bytes(), 0, "and every charge is released");
}

#[tokio::test(start_paused = true)]
async fn an_event_the_dispatcher_does_not_know_is_dropped_and_a_socket_handler_wins() {
    let io = server(options());
    let calls = Arc::new(AtomicUsize::new(0));
    {
        let calls = calls.clone();
        io.main().set_dispatcher(Arc::new(move |_socket, event, _args, _ack, _charge| {
            calls.fetch_add(1, Ordering::SeqCst);
            (event == "known").then(|| async {}.boxed())
        }));
    }
    let own = Arc::new(AtomicUsize::new(0));
    {
        let own = own.clone();
        io.on_connection(move |socket| {
            let own = own.clone();
            socket.on("mine", move |_s, _a, _k| {
                let own = own.clone();
                async move {
                    own.fetch_add(1, Ordering::SeqCst);
                }
            });
        });
    }
    let conn = io.conn();
    conn.join("0").await;
    conn.text(r#"2["unknown"]"#);
    conn.text(r#"2["mine"]"#);
    conn.text(r#"2["known"]"#);
    settle().await;
    assert_eq!(own.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2, "the dispatcher is consulted for events without a handler of the socket only");
    assert!(!conn.socket.is_closing(), "an unknown event does not close the connection");
}

/// A socket that waits for its disconnect hooks: its `auth` charge leaves the connection's unread budget when it is
/// queued (the queue's budget charges it), it counts against its address until its hooks end, and it does not keep its
/// connection's Engine.IO session alive.
#[tokio::test(start_paused = true)]
async fn a_socket_queued_for_its_disconnect_hooks_gives_its_auth_charge_back_and_keeps_no_session() {
    use crate::socket_io::gateway::disconnect::{HookQueue, pending_cost};
    const ADDRESS: &str = "203.0.113.70";
    let io = server(options());
    let captured: Arc<Mutex<Option<IoSocket>>> = Arc::new(Mutex::new(None));
    {
        let captured = captured.clone();
        io.on_connection(move |socket| *captured.lock().unwrap() = Some(socket));
    }
    let conn = io.conn_from(ADDRESS);
    let pad = "p".repeat(10_000);
    conn.join(&format!(r#"0{{"pad":"{pad}"}}"#)).await;
    let socket = captured.lock().unwrap().take().expect("the socket");
    let auth = crate::socket_io::value::json_cost(socket.handshake().auth());
    let before = io.engine().total_unread_bytes();
    assert!(before >= auth, "the auth is charged to the unread budget: {before} < {auth}");
    // A hook that never ends keeps the socket pending.
    let queue = Arc::new(HookQueue::new(10, 1 << 20, Duration::from_secs(3600)));
    socket.disconnect(false);
    let cost = pending_cost(&socket);
    queue.push(Arc::new(Vec::new()), socket.clone());
    assert_eq!(io.engine().total_unread_bytes(), before - auth, "the auth charge left the connection's budget");
    let linger = io.engine().linger_of(ADDRESS);
    assert!(linger.hooks <= 1 && linger.hook_bytes <= cost, "{linger:?}");
    // The connection ends: the socket the queue (or anyone) keeps does not keep its session.
    conn.socket.kill();
    drop(conn);
    settle().await;
    assert!(socket.engine().is_none(), "the session went with the connection");
    assert_eq!(socket.id().len(), 20, "the socket still works as the hook's handle");
    assert_eq!(socket.handshake().auth()["pad"], serde_json::Value::String(pad));
    // Without gateways the hooks end at once and the address's count goes.
    assert_eq!(io.engine().linger_of(ADDRESS), Default::default());
}
