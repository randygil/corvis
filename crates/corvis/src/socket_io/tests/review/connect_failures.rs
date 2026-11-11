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

/// Set when the value is dropped.
struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// A middleware registers `disconnecting` and `disconnect` callbacks that hold a clone of the socket, then the attempt
/// fails (`refuse` decides how). A socket that never connected reports no disconnect, as with the reference server (its
/// `_onclose` returns early for a socket that is not connected), and the callbacks are released: one that holds the
/// socket would otherwise keep it, its handshake and everything they capture alive for good.
async fn callbacks_of_an_attempt_that_never_connected_are_released_without_running(refuse: bool) {
    let (io, knobs, _) = probe_server(options());
    let ran = Arc::new(AtomicUsize::new(0));
    let released = Arc::new(AtomicBool::new(false));
    {
        let (ran, released) = (ran.clone(), released.clone());
        io.use_middleware(move |socket| {
            let flag = Arc::new(DropFlag(released.clone()));
            for disconnecting in [true, false] {
                let (ran, kept, flag) = (ran.clone(), socket.clone(), flag.clone());
                let callback = move |_: &IoSocket, _: DisconnectReason| {
                    let _ = (&kept, &flag);
                    ran.fetch_add(1, Ordering::SeqCst);
                };
                if disconnecting {
                    socket.on_disconnecting(callback);
                } else {
                    socket.on_disconnect(callback);
                }
            }
            async move { if refuse { Err(ConnectError::new("refused")) } else { Ok(()) } }
        });
    }
    knobs.fail_add_all.store(!refuse, Ordering::SeqCst);
    let conn = io.conn();
    conn.text("0");
    let expected = if refuse { r#"4{"message":"refused"}"# } else { r#"4{"message":"Internal server error"}"# };
    assert_eq!(conn.sent().await, [t(expected)]);
    assert_eq!(ran.load(Ordering::SeqCst), 0, "no disconnect callback ran for a socket that never connected");
    assert!(released.load(Ordering::SeqCst), "the callbacks, and the socket they hold, were released");
}

#[tokio::test(start_paused = true)]
async fn the_disconnect_callbacks_of_a_socket_whose_own_room_failed_are_released_without_running() {
    callbacks_of_an_attempt_that_never_connected_are_released_without_running(false).await;
}

#[tokio::test(start_paused = true)]
async fn the_disconnect_callbacks_of_a_socket_a_middleware_refused_are_released_without_running() {
    callbacks_of_an_attempt_that_never_connected_are_released_without_running(true).await;
}
