//! Rooms and broadcasts without a network, on the paused clock: who receives what, rooms around a disconnect, the
//! limits, and the deadline and bounds of a broadcast with acknowledgements.

use super::*;
use crate::socket_io::adapter::MembershipSize;

/// The sockets of `conns` that received an event named `name` (by the text of the packet) since the last read.
pub(super) async fn received(conns: &[&Conn], name: &str) -> Vec<bool> {
    let mut out = Vec::new();
    for conn in conns {
        let sent = conn.sent().await;
        out.push(sent.iter().any(|m| matches!(m, Out::T(s) if s.contains(&format!("[\"{name}\"")))));
    }
    out
}

pub(super) fn size(namespace: &IoNamespace) -> MembershipSize {
    namespace.adapter().membership_size().expect("the in-memory adapter")
}

pub(super) async fn connect(io: &IoServer, nsp: &str) -> (Conn, String) {
    let conn = io.conn();
    let packet = if nsp == "/" { "0".to_string() } else { format!("0{nsp},") };
    let sid = conn.join(&packet).await;
    (conn, sid)
}

#[tokio::test(start_paused = true)]
async fn a_socket_is_in_the_room_of_its_own_id_when_its_connection_handler_runs() {
    let io = server(options());
    let seen = log();
    {
        let seen = seen.clone();
        io.on_connection(move |socket| {
            let mut rooms: Vec<String> = socket.rooms().into_iter().collect();
            rooms.sort();
            seen.lock().unwrap().push(format!("{}|{}", socket.id(), rooms.join(",")));
        });
    }
    let (_conn, sid) = connect(&io, "/").await;
    assert_eq!(entries(&seen), [format!("{sid}|{sid}")]);
}

#[tokio::test(start_paused = true)]
async fn rooms_are_scoped_to_their_namespace() {
    let io = server(options());
    io.of("/admin").unwrap();
    for name in ["/", "/admin"] {
        io.of(name).unwrap().on_connection(|socket| {
            socket.join("r").unwrap();
        });
    }
    let (a, _) = connect(&io, "/").await;
    let (b, _) = connect(&io, "/admin").await;
    io.to("r").emit("hit", ()).await.unwrap();
    assert_eq!(received(&[&a, &b], "hit").await, [true, false]);
}

#[tokio::test(start_paused = true)]
async fn operators_select_what_nodes_in_memory_adapter_selects() {
    let io = server(options());
    io.of("/admin").unwrap();
    let (a, ia) = connect(&io, "/").await;
    let (b, ib) = connect(&io, "/").await;
    let (c, ic) = connect(&io, "/").await;
    let (d, _) = connect(&io, "/admin").await;
    let socket = |id: &str| io.main().socket(id).unwrap();
    socket(&ia).join(["r1", "r2", "admins"]).unwrap();
    socket(&ib).join(["r1"]).unwrap();
    socket(&ic).join(["r1", "r2"]).unwrap();
    let all = [&a, &b, &c, &d];
    for conn in all {
        conn.sent().await;
    }

    // The whole namespace, not the others.
    io.emit("e", ()).await.unwrap();
    assert_eq!(received(&all, "e").await, [true, true, true, false]);
    // A union reaches a socket once.
    io.to(["r1", "r2"]).emit("e", ()).await.unwrap();
    let sent = a.sent().await;
    assert_eq!(sent.len(), 1, "once, not once per room: {sent:?}");
    b.sent().await;
    c.sent().await;
    // `within` is `in`.
    io.within("r2").emit("e", ()).await.unwrap();
    assert_eq!(received(&all, "e").await, [true, false, true, false]);
    // Except a room, and except a socket id inside a room.
    io.except("admins").emit("e", ()).await.unwrap();
    assert_eq!(received(&all, "e").await, [false, true, true, false]);
    io.to("r1").except(ib.as_str()).emit("e", ()).await.unwrap();
    assert_eq!(received(&all, "e").await, [true, false, true, false]);
    // A room nobody is in: nobody, and the emit succeeds.
    io.to("nobody").emit("e", ()).await.unwrap();
    assert_eq!(received(&all, "e").await, [false; 4]);
    // The id of a socket is a room of its own.
    io.to(ib.as_str()).emit("e", ()).await.unwrap();
    assert_eq!(received(&all, "e").await, [false, true, false, false]);
    // An operator is a value: chaining does not change the one it started from.
    let base = io.to("r1");
    let narrowed = base.except("r2");
    narrowed.emit("e", ()).await.unwrap();
    assert_eq!(received(&all, "e").await, [false, true, false, false]);
    base.emit("e", ()).await.unwrap();
    assert_eq!(received(&all, "e").await, [true, true, true, false]);
}

#[tokio::test(start_paused = true)]
async fn a_broadcast_from_a_socket_excludes_it() {
    let io = server(options());
    let (a, ia) = connect(&io, "/").await;
    let (b, ib) = connect(&io, "/").await;
    let (c, _) = connect(&io, "/").await;
    for id in [&ia, &ib] {
        io.main().socket(id).unwrap().join("r").unwrap();
    }
    let sa = io.main().socket(&ia).unwrap();
    sa.to("r").emit("e", ()).await.unwrap();
    assert_eq!(received(&[&a, &b, &c], "e").await, [false, true, false]);
    sa.broadcast().emit("e", ()).await.unwrap();
    assert_eq!(received(&[&a, &b, &c], "e").await, [false, true, true]);
    sa.except("r").emit("e", ()).await.unwrap();
    assert_eq!(received(&[&a, &b, &c], "e").await, [false, false, true]);
}

#[tokio::test(start_paused = true)]
async fn a_broadcast_is_the_packet_of_a_direct_emit_and_reserved_names_are_refused() {
    let io = server(options());
    let (a, ia) = connect(&io, "/").await;
    io.main().socket(&ia).unwrap().emit("e", (1, "x")).unwrap();
    let direct = a.sent().await;
    io.emit("e", (1, "x")).await.unwrap();
    assert_eq!(a.sent().await, direct);
    for name in ["connect", "connect_error", "disconnect", "disconnecting", "newListener", "removeListener"] {
        assert_eq!(io.emit(name, ()).await, Err(EmitError::ReservedEvent(name.to_string())));
        assert!(
            matches!(io.emit_with_ack(name, ()).await, Err(e) if *e.kind() == BroadcastAckErrorKind::Emit(EmitError::ReservedEvent(name.to_string())))
        );
    }
    assert!(a.sent().await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_binary_broadcast_shares_one_encoding() {
    let io = server(options());
    let (a, _) = connect(&io, "/").await;
    let (b, _) = connect(&io, "/").await;
    io.emit("file", (Binary::from(vec![1u8, 2, 3]),)).await.unwrap();
    let expected = vec![t(r#"51-["file",{"_placeholder":true,"num":0}]"#), Out::B(vec![1, 2, 3])];
    assert_eq!(a.sent().await, expected);
    assert_eq!(b.sent().await, expected);
}

#[tokio::test(start_paused = true)]
async fn leave_takes_effect_at_once_and_an_empty_room_is_deleted() {
    let io = server(options());
    let (a, ia) = connect(&io, "/").await;
    let sa = io.main().socket(&ia).unwrap();
    sa.join("r").unwrap();
    sa.leave("r");
    sa.leave("never joined");
    io.to("r").emit("e", ()).await.unwrap();
    assert!(a.sent().await.is_empty());
    assert_eq!(size(&io.main()).rooms, 1, "only the room of the socket's own id is left");
    // The room of its own id cannot be left: it is how the socket is addressed (listed difference from Node).
    sa.leave(&ia);
    assert_eq!(size(&io.main()).rooms, 1);
    io.to(ia.as_str()).emit("own", ()).await.unwrap();
    assert_eq!(a.sent().await.len(), 1);
    drop(sa);
    io.disconnect_sockets(false).await.unwrap();
    assert_eq!(size(&io.main()), MembershipSize { rooms: 0, sockets: 0, memberships: 0 });
}

#[tokio::test(start_paused = true)]
async fn rooms_are_intact_while_disconnecting_and_gone_at_disconnect() {
    let io = server(options());
    let seen = log();
    {
        let seen = seen.clone();
        io.on_connection(move |socket| {
            socket.join("r").unwrap();
            let (during, after) = (seen.clone(), seen.clone());
            socket.on_disconnecting(move |s, _| {
                let mut rooms: Vec<String> = s.rooms().into_iter().collect();
                rooms.sort();
                during.lock().unwrap().push(format!("disconnecting:{}", rooms.len()));
            });
            socket.on_disconnect(move |s, _| after.lock().unwrap().push(format!("disconnect:{}", s.rooms().len())));
        });
    }
    let (a, _) = connect(&io, "/").await;
    a.socket.kill();
    settle().await;
    assert_eq!(entries(&seen), ["disconnecting:2", "disconnect:0"]);
    assert_eq!(size(&io.main()), MembershipSize { rooms: 0, sockets: 0, memberships: 0 });
}

#[tokio::test(start_paused = true)]
async fn a_farewell_from_the_disconnecting_handler_reaches_the_rooms_of_the_socket() {
    let io = server(options());
    io.on_connection(|socket| {
        socket.join("r").unwrap();
        socket.on_disconnecting(|s, _| {
            let rooms: Vec<String> = s.rooms().into_iter().collect();
            let s = s.clone();
            // The handler is synchronous: the broadcast runs in a task, from a copy of the rooms it had.
            tokio::spawn(async move { s.to(rooms).emit("left", ()).await.unwrap() });
        });
    });
    let (a, _) = connect(&io, "/").await;
    let (b, _) = connect(&io, "/").await;
    let (c, _) = connect(&io, "/").await;
    a.sent().await;
    b.sent().await;
    c.sent().await;
    a.socket.kill();
    settle().await;
    assert_eq!(received(&[&b, &c], "left").await, [true, true]);
}

#[tokio::test(start_paused = true)]
async fn joining_a_socket_that_disconnected_does_nothing() {
    let io = server(options());
    let (a, ia) = connect(&io, "/").await;
    let sa = io.main().socket(&ia).unwrap();
    a.socket.kill();
    settle().await;
    assert_eq!(sa.join("r"), Ok(()));
    assert_eq!(size(&io.main()), MembershipSize { rooms: 0, sockets: 0, memberships: 0 });
    assert!(sa.rooms().is_empty());
}

#[tokio::test(start_paused = true)]
async fn the_room_cap_and_the_name_length_are_enforced_all_or_nothing() {
    let io = server(options().max_rooms_per_socket(5).max_room_name_length(30));
    let (_a, ia) = connect(&io, "/").await;
    let sa = io.main().socket(&ia).unwrap();
    sa.join(["a", "b", "c", "d"]).unwrap();
    assert_eq!(sa.rooms().len(), 5, "four rooms and its own id");
    assert_eq!(sa.join(["e", "f"]), Err(RoomError::TooManyRooms { max: 5 }));
    assert_eq!(sa.rooms().len(), 5, "nothing of the refused call joined");
    assert!(!sa.rooms().contains("e"));
    assert_eq!(sa.join(["a", "b"]), Ok(()), "rooms already joined do not count again");
    sa.join(["a", "a", "a"]).unwrap();
    assert_eq!(sa.join("x".repeat(31)), Err(RoomError::NameTooLong { len: 31, max: 30 }));
    sa.leave("a");
    sa.join("e").unwrap();
    assert_eq!(sa.rooms().len(), 5);
}

#[tokio::test(start_paused = true)]
async fn a_long_room_name_in_an_operator_fails_every_operation_and_never_widens_the_broadcast() {
    let io = server(options().max_room_name_length(30));
    let (a, _) = connect(&io, "/").await;
    let long = "x".repeat(31);
    let expected = EmitError::InvalidRoom(RoomError::NameTooLong { len: 31, max: 30 });
    for op in [
        io.to(long.as_str()),
        io.to(["ok", long.as_str()]),
        io.except(long.as_str()),
        io.within(long.as_str()).local(),
        io.to("ok").to(long.as_str()).timeout(Duration::from_secs(1)),
    ] {
        assert_eq!(op.emit("e", ()).await, Err(expected.clone()));
        assert!(matches!(op.emit_with_ack("e", ()).await, Err(e) if *e.kind() == BroadcastAckErrorKind::Emit(expected.clone())));
        assert_eq!(op.sockets_join("r").await, Err(expected.clone()));
        assert_eq!(op.sockets_leave("r").await, Err(expected.clone()));
        assert_eq!(op.disconnect_sockets(false).await.unwrap_err(), expected);
        assert!(op.fetch_sockets().await.is_err());
    }
    assert_eq!(io.sockets_join(long.as_str()).await, Err(expected.clone()));
    assert!(a.sent().await.is_empty());
    assert_eq!(io.main().socket_count(), 1, "nobody was disconnected");
}

#[tokio::test(start_paused = true)]
async fn bulk_operations_act_on_the_selection() {
    let io = server(options());
    io.of("/admin").unwrap();
    let (a, ia) = connect(&io, "/").await;
    let (b, ib) = connect(&io, "/").await;
    let (c, ic) = connect(&io, "/").await;
    let sock = |id: &str| io.main().socket(id).unwrap();
    sock(&ia).join("r1").unwrap();
    sock(&ib).join("r1").unwrap();
    // Make a room join another.
    io.to("r1").sockets_join("r2").await.unwrap();
    io.to("r2").emit("e", ()).await.unwrap();
    assert_eq!(received(&[&a, &b, &c], "e").await, [true, true, false]);
    io.to("r1").sockets_leave("r2").await.unwrap();
    io.to("r2").emit("e", ()).await.unwrap();
    assert_eq!(received(&[&a, &b, &c], "e").await, [false; 3]);
    // Fetch sockets: ids and rooms of the selection.
    let mut fetched = io.to("r1").except(ib.as_str()).fetch_sockets().await.unwrap();
    assert_eq!(fetched.len(), 1);
    let one = fetched.pop().unwrap();
    assert_eq!(one.id(), ia);
    let mut rooms: Vec<&str> = one.rooms().iter().map(|r| &**r).collect();
    rooms.sort();
    let mut expected = vec![ia.clone(), "r1".to_string()];
    expected.sort();
    assert_eq!(rooms.into_iter().map(str::to_string).collect::<Vec<_>>(), expected);
    assert!(one.local().is_some());
    // A remote socket does its operations through the adapter.
    one.emit("direct", ()).await.unwrap();
    assert_eq!(received(&[&a, &b, &c], "direct").await, [true, false, false]);
    one.join("r9").await.unwrap();
    assert!(sock(&ia).rooms().contains("r9"));
    one.leave("r9").await.unwrap();
    assert!(!sock(&ia).rooms().contains("r9"));
    // Disconnect a room, keeping the connection.
    for conn in [&a, &b, &c] {
        conn.sent().await;
    }
    io.to("r1").disconnect_sockets(false).await.unwrap();
    assert_eq!(a.sent().await, [t("1")]);
    assert_eq!(b.sent().await, [t("1")]);
    assert!(c.sent().await.is_empty());
    assert!(io.main().socket(&ic).is_some());
    assert!(io.main().socket(&ia).is_none());
    assert!(!a.socket.is_closing(), "the connection stays open");
    // With close, the connection goes too.
    io.disconnect_sockets(true).await.unwrap();
    settle().await;
    assert!(c.socket.is_closing());
}

#[tokio::test(start_paused = true)]
async fn a_socket_at_its_room_cap_is_skipped_by_sockets_join() {
    let io = server(options().max_rooms_per_socket(2));
    let (_a, ia) = connect(&io, "/").await;
    let (_b, ib) = connect(&io, "/").await;
    io.main().socket(&ia).unwrap().join("x").unwrap();
    io.sockets_join("y").await.unwrap();
    let a_rooms = io.main().socket(&ia).unwrap().rooms();
    let b_rooms = io.main().socket(&ib).unwrap().rooms();
    assert!(!a_rooms.contains("y"), "full: {a_rooms:?}");
    assert!(b_rooms.contains("y"), "the others still join: {b_rooms:?}");
}

// ------------------------------------------------------------------------------ broadcast with acknowledgements

#[tokio::test(start_paused = true)]
async fn every_recipient_answers() {
    let io = server(options());
    let (a, _) = connect(&io, "/").await;
    let (b, _) = connect(&io, "/").await;
    let (c, _) = connect(&io, "/").await;
    let task = tokio::spawn({
        let io = io.clone();
        async move { io.timeout(Duration::from_secs(5)).emit_with_ack("q", ("?",)).await }
    });
    settle().await;
    for (conn, answer) in [(&a, 1), (&b, 2), (&c, 3)] {
        let sent = conn.sent().await;
        assert_eq!(sent, [t(r#"20["q","?"]"#)], "one packet, with the broadcast's id");
        assert!(!task.is_finished(), "waits for the others");
        conn.text(&format!("30[{answer}]"));
        settle().await;
    }
    let answers = task.await.unwrap().unwrap();
    let mut got: Vec<i64> = answers.iter().map(|a| a.get::<i64>(0).unwrap()).collect();
    got.sort();
    assert_eq!(got, [1, 2, 3]);
}

#[tokio::test(start_paused = true)]
async fn a_silent_recipient_ends_the_call_at_the_deadline_with_the_answers_so_far() {
    let io = server(options());
    let (a, ia) = connect(&io, "/").await;
    let (b, ib) = connect(&io, "/").await;
    let (_c, ic) = connect(&io, "/").await;
    let start = tokio::time::Instant::now();
    let task = tokio::spawn({
        let io = io.clone();
        async move { (io.timeout(Duration::from_millis(200)).emit_with_ack("q", ()).await, tokio::time::Instant::now()) }
    });
    settle().await;
    a.sent().await;
    b.sent().await;
    a.text("30[1]");
    b.text("30[2]");
    advance(199).await;
    assert!(!task.is_finished());
    advance(2).await;
    let (outcome, ended) = task.await.unwrap();
    let error = outcome.unwrap_err();
    assert_eq!(error.to_string(), "operation has timed out");
    assert_eq!(*error.kind(), BroadcastAckErrorKind::Timeout);
    assert_eq!(error.responses().len(), 2);
    assert!(ended - start >= Duration::from_millis(200), "not before the configured value");
    for id in [&ia, &ib, &ic] {
        assert_eq!(io.main().socket(id).unwrap().pending_acks(), 0, "nothing stays pending after the deadline");
    }
}

#[tokio::test(start_paused = true)]
async fn without_a_timeout_the_default_deadline_applies() {
    let io = server(options());
    let (_a, ia) = connect(&io, "/").await;
    let task = tokio::spawn({
        let io = io.clone();
        async move { io.emit_with_ack("q", ()).await }
    });
    settle().await;
    assert_eq!(io.main().socket(&ia).unwrap().pending_acks(), 1);
    advance(29_900).await;
    assert!(!task.is_finished());
    advance(200).await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(*error.kind(), BroadcastAckErrorKind::Timeout);
    assert!(error.responses().is_empty());
    assert_eq!(io.main().socket(&ia).unwrap().pending_acks(), 0);
}

#[tokio::test(start_paused = true)]
async fn an_outsider_is_ignored_and_a_second_answer_counts_once() {
    let io = server(options());
    let (a, ia) = connect(&io, "/").await;
    let (b, ib) = connect(&io, "/").await;
    let (outsider, _) = connect(&io, "/").await;
    for id in [&ia, &ib] {
        io.main().socket(id).unwrap().join("r").unwrap();
    }
    let task = tokio::spawn({
        let io = io.clone();
        async move { io.to("r").timeout(Duration::from_secs(5)).emit_with_ack("q", ()).await }
    });
    settle().await;
    assert!(outsider.sent().await.is_empty(), "the outsider was not sent the packet");
    outsider.text("30[9]");
    a.text("30[1]");
    a.text("30[1]");
    settle().await;
    assert!(!task.is_finished(), "one of two answered; the outsider and the repeat do not complete it");
    b.text("30[2]");
    let answers = task.await.unwrap().unwrap();
    let got: Vec<i64> = answers.iter().map(|a| a.get::<i64>(0).unwrap()).collect();
    assert_eq!(got, [1, 2]);
}

#[tokio::test(start_paused = true)]
async fn answers_past_the_cap_end_the_call_with_what_fit() {
    let io = server(options().max_broadcast_ack_bytes(1024 * 1024));
    let mut conns: Vec<Conn> = Vec::new();
    for _ in 0..20 {
        conns.push(connect(&io, "/").await.0);
    }
    let task = tokio::spawn({
        let io = io.clone();
        async move { io.timeout(Duration::from_secs(5)).emit_with_ack("q", ()).await }
    });
    settle().await;
    let big = "x".repeat(100_000);
    for conn in &conns {
        conn.sent().await;
        conn.text(&format!(r#"30["{big}"]"#));
        settle().await;
    }
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(*error.kind(), BroadcastAckErrorKind::ResponsesTooLarge);
    assert!(!error.responses().is_empty());
    let held: usize = error.responses().iter().map(|a| a.iter().map(|v| v.cost()).sum::<usize>()).sum();
    assert!(held <= 1024 * 1024, "at most the cap was held: {held}");
    for socket in io.main().sockets() {
        assert_eq!(socket.pending_acks(), 0, "nothing stays pending");
    }
}

#[tokio::test(start_paused = true)]
async fn a_recipient_that_disconnects_never_answers_and_the_call_ends_at_the_deadline() {
    let io = server(options());
    let (a, _) = connect(&io, "/").await;
    let (b, _) = connect(&io, "/").await;
    let task = tokio::spawn({
        let io = io.clone();
        async move { io.timeout(Duration::from_secs(1)).emit_with_ack("q", ()).await }
    });
    settle().await;
    a.sent().await;
    b.sent().await;
    a.text("30[1]");
    b.socket.kill();
    advance(1100).await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(*error.kind(), BroadcastAckErrorKind::Timeout);
    assert_eq!(error.responses().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn dropping_the_call_frees_the_recipients_entries() {
    let io = server(options());
    let (_a, ia) = connect(&io, "/").await;
    let task = tokio::spawn({
        let io = io.clone();
        async move { io.emit_with_ack("q", ()).await }
    });
    settle().await;
    assert_eq!(io.main().socket(&ia).unwrap().pending_acks(), 1);
    task.abort();
    let _ = task.await;
    settle().await;
    assert_eq!(io.main().socket(&ia).unwrap().pending_acks(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_recipient_with_its_pending_table_full_gets_the_event_but_is_not_awaited() {
    let io = server(options().max_pending_acks(1));
    let (a, ia) = connect(&io, "/").await;
    let (b, _) = connect(&io, "/").await;
    // `a` already waits for one answer.
    let sa = io.main().socket(&ia).unwrap();
    let own = tokio::spawn({
        let sa = sa.clone();
        async move { sa.emit_with_ack("own", ()).await }
    });
    settle().await;
    a.sent().await;
    b.sent().await;
    let task = tokio::spawn({
        let io = io.clone();
        async move { io.timeout(Duration::from_secs(5)).emit_with_ack("q", ()).await }
    });
    settle().await;
    assert_eq!(a.sent().await.len(), 1, "it still gets the event");
    // The broadcast took id 1 (the socket's own emit took 0): `b` answers it.
    b.text("31[7]");
    settle().await;
    let answers = task.await.unwrap().unwrap();
    assert_eq!(answers.len(), 1, "completes without waiting for the one that cannot answer");
    own.abort();
}
