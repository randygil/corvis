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
    io.main().set_dispatcher(Arc::new(move |_socket, _event, _args, _ack| {
        let (s, a, gate) = (s.clone(), a.clone(), gate.clone());
        Some(
            async move {
                s.fetch_add(1, Ordering::SeqCst);
                if gate.admitted().await {
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
        io.main().set_dispatcher(Arc::new(move |_socket, event, _args, _ack| {
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

#[tokio::test(start_paused = true)]
async fn a_ping_due_while_a_layer_queues_its_packets_waits_for_it_and_is_never_lost() {
    let engine = EngineOptions::new().ping_interval(Duration::from_millis(300)).ping_timeout(Duration::from_secs(3600));
    let io = server(IoOptions::new().engine(engine));
    let conn = io.conn();
    // CONNECT and what the connection handlers emit are queued under a hold: no ping lands between them.
    let hold = conn.socket.hold_ping();
    advance(400).await;
    assert_eq!(conn.socket.take_pings(), 0, "the ping that fell due is held back");
    drop(hold);
    settle().await;
    assert_eq!(conn.socket.take_pings(), 1, "and queued when the hold ends: the heartbeat still fires");
    // A hold that is never released cannot postpone the heartbeat without limit.
    let io = server(
        IoOptions::new().engine(EngineOptions::new().ping_interval(Duration::from_millis(300)).ping_timeout(Duration::from_secs(3600))),
    );
    let stuck = io.conn();
    let _forgotten = stuck.socket.hold_ping();
    advance(400).await;
    assert_eq!(stuck.socket.take_pings(), 0);
    advance(1_100).await;
    assert_eq!(stuck.socket.take_pings(), 1, "after one second the held ping goes out anyway");
}
