//! What the connection asks of an adapter and how its failures are contained: the work a join does under the lock,
//! panics and adapters that never return, empty selections, the rooms an operator may name, closing, the order of
//! broadcasts.

use super::super::rooms::{connect, received};
use super::super::*;
use super::probe::probe_server;
use crate::socket_io::adapter::Membership;
use crate::socket_io::adapter::{AdapterError, Room};

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
