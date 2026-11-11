//! An adapter that panics or fails while a socket connects: the attempt is refused and nothing of it stays.

use std::sync::atomic::AtomicUsize;

use super::super::*;
use super::probe::{Knobs, probe_server};
use crate::socket_io::adapter::MembershipSize;

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
async fn an_error_from_add_all_of_the_own_room_during_connect_refuses_the_attempt_like_a_panic() {
    an_own_room_failure_refuses_the_attempt(|knobs| &knobs.fail_add_all).await;
}
