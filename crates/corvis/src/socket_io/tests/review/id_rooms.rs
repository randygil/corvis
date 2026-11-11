//! The room named like a socket's id: no client joins another socket's, no socket is given an id that is already a
//! room, its own room expels whoever got in before it was live, and a `RemoteSocket` reaches its own socket only.

use std::collections::HashSet;

use super::super::rooms::{connect, received, size};
use super::super::*;
use super::probe::probe_server;

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
