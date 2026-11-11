#![cfg(feature = "socket-io")]
//! What a client can make the server hold or do through rooms and broadcasts, in the style of `ws_security.rs`:
//! room and name limits, a broadcast to many clients that do not read, a join that races a disconnect or a
//! broadcast. Timing rule: nothing here compares durations; a timeout is only the hang detector of a test.

mod engine_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use corvis::socket_io::engine::EngineOptions;
use corvis::socket_io::{IoOptions, RoomError, testing};
use engine_support::io_client::{IoFixture, Msg, Raw, start};
use engine_support::{open_polling, poll, post};

const HANG: Duration = Duration::from_secs(60);

async fn within<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(HANG, future).await.expect("no hang")
}

#[tokio::test]
async fn a_socket_cannot_join_more_rooms_than_the_cap_and_a_refused_join_changes_nothing() {
    let fx = start(IoOptions::new(), |_| {}).await;
    let mut raw = Raw::connect(fx.addr).await;
    let id = raw.join("/", None).await;
    let socket = fx.io.main().socket(&id).unwrap();
    let many: Vec<String> = (0..1_000).map(|n| format!("room-{n}")).collect();
    assert_eq!(socket.join(&many), Err(RoomError::TooManyRooms { max: 1_000 }));
    assert_eq!(socket.rooms().len(), 1, "still only the room of its own id");
    let fits: Vec<String> = (0..999).map(|n| format!("room-{n}")).collect();
    socket.join(&fits).unwrap();
    assert_eq!(socket.rooms().len(), 1_000);
    assert_eq!(socket.join("one-more"), Err(RoomError::TooManyRooms { max: 1_000 }));
    // Leaving frees room.
    socket.leave("room-0");
    socket.join("one-more").unwrap();
    assert_eq!(testing::membership_size(&fx.io.main()).unwrap().memberships, 1_000);
}

#[tokio::test]
async fn a_room_name_over_the_limit_is_refused_in_join() {
    let fx = start(IoOptions::new(), |_| {}).await;
    let mut raw = Raw::connect(fx.addr).await;
    let id = raw.join("/", None).await;
    let socket = fx.io.main().socket(&id).unwrap();
    assert_eq!(socket.join("x".repeat(257)), Err(RoomError::NameTooLong { len: 257, max: 256 }));
    socket.join("x".repeat(256)).unwrap();
    assert_eq!(socket.rooms().len(), 2);
}

#[tokio::test]
async fn churn_leaves_the_maps_empty() {
    let fx = start(IoOptions::new(), |_| {}).await;
    let namespace = fx.io.main();
    let mut raw = Raw::connect(fx.addr).await;
    let id = raw.join("/", None).await;
    let socket = fx.io.main().socket(&id).unwrap();
    // 100,000 join / leave cycles of distinct room names leave nothing behind.
    for n in 0..100_000u32 {
        let room = format!("r{n}");
        socket.join(room.as_str()).unwrap();
        socket.leave(&room);
    }
    assert_eq!(testing::membership_size(&namespace).unwrap().rooms, 1, "only the room of the socket's id");
    // Sockets that join many rooms and disconnect (or just drop): they are gone with their rooms.
    for round in 0..200u32 {
        let mut other = Raw::connect(fx.addr).await;
        let other_id = other.join("/", None).await;
        let other_socket = fx.io.main().socket(&other_id).unwrap();
        other_socket.join((0..50).map(|n| format!("shared-{}", (round + n) % 80)).collect::<Vec<_>>()).unwrap();
        if round % 2 == 0 {
            other_socket.disconnect(false);
        } else {
            other.close().await;
        }
    }
    socket.disconnect(false);
    within(async {
        while fx.io.main().socket_count() > 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    let size = testing::membership_size(&namespace).unwrap();
    assert_eq!((size.rooms, size.sockets, size.memberships), (0, 0, 0));
    assert_eq!(testing::check_invariants(&namespace), Ok(()));
}

/// A client that connected over long polling and never polls again: what is queued for it stays queued.
async fn polling_client(fx: &IoFixture) -> String {
    let (sid, _) = open_polling(fx.addr).await;
    assert_eq!(post(fx.addr, &sid, "40").await.status, 200);
    let connect = poll(fx.addr, &sid).await.text();
    connect.split("\"sid\":\"").nth(1).and_then(|s| s.split('"').next()).unwrap().to_string()
}

#[tokio::test]
async fn many_slow_clients_never_hold_the_broadcast_back_and_each_is_cut_at_its_own_bound() {
    const BOUND: usize = 1 << 20;
    const EVENT: usize = 64 * 1024;
    const EVENTS: usize = 256;
    let options = IoOptions::new().engine(EngineOptions::new().max_buffered_amount(BOUND));
    let fx = start(options, |_| {}).await;
    let mut slow = Vec::new();
    for _ in 0..50 {
        slow.push(polling_client(&fx).await);
    }
    let mut reader = Raw::connect(fx.addr).await;
    let reader_id = reader.join("/", None).await;
    for id in slow.iter().chain([&reader_id]) {
        fx.io.main().socket(id).unwrap().join("room").unwrap();
    }
    // The reader counts what it receives, and every emit waits for the reader (not for the slow ones), so the
    // healthy client's own queue stays within its bound.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let reading = tokio::spawn(async move {
        loop {
            match reader.recv_within(Duration::from_secs(30)).await {
                Some(Msg::Text(t)) if t.contains("\"chunk\"") => {
                    let _ = tx.send(());
                }
                Some(Msg::Text(t)) if t.contains("\"end\"") => return true,
                Some(_) => {}
                None => return false,
            }
        }
    });
    let chunk = "x".repeat(EVENT);
    let mut max_queue = 0;
    for n in 0..EVENTS {
        within(fx.io.to("room").emit("chunk", (chunk.as_str(),))).await.unwrap();
        within(rx.recv()).await.expect("the reader received the event");
        for id in &slow {
            if let Some(socket) = fx.io.main().socket(id) {
                max_queue = max_queue.max(testing::buffered_amount(&socket));
            }
        }
        assert!(n < EVENTS);
    }
    within(fx.io.to("room").emit("end", ())).await.unwrap();
    assert!(within(reading).await.unwrap(), "the reading client received every event and the end marker");
    // No session queued more than its bound plus the packet that crossed it.
    assert!(max_queue <= BOUND + EVENT + 64, "a slow session queued {max_queue} bytes");
    // Every slow session was cut off ("transport error"), the reader is still there.
    let remaining: Vec<_> = slow.iter().filter(|id| fx.io.main().socket(id).is_some()).collect();
    assert!(remaining.is_empty(), "{} slow sessions survived 16 MiB", remaining.len());
    assert!(fx.io.main().socket(&reader_id).is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_join_during_a_broadcast_and_a_broadcast_during_a_join_both_complete() {
    let fx = start(IoOptions::new(), |_| {}).await;
    let mut raws = Vec::new();
    let mut ids = Vec::new();
    for _ in 0..40 {
        let mut raw = Raw::connect(fx.addr).await;
        ids.push(raw.join("/", None).await);
        raws.push(raw);
    }
    let mut sockets = Vec::new();
    for id in &ids {
        sockets.push(fx.io.main().socket(id).expect("registered before its CONNECT"));
    }
    for s in &sockets {
        s.join("all").unwrap();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let sent = Arc::new(AtomicUsize::new(0));
    let broadcaster = tokio::spawn({
        let (io, stop, sent) = (fx.io.clone(), stop.clone(), sent.clone());
        async move {
            while !stop.load(Ordering::SeqCst) {
                io.to("all").except("muted").emit("tick", (1,)).await.unwrap();
                sent.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
            }
        }
    });
    let joiners: Vec<_> = sockets
        .iter()
        .map(|s| {
            let s = s.clone();
            tokio::spawn(async move {
                for n in 0..200 {
                    let room = format!("room-{}", n % 7);
                    s.join(room.as_str()).unwrap();
                    s.join("muted").unwrap();
                    s.leave(&room);
                    s.leave("muted");
                    tokio::task::yield_now().await;
                }
            })
        })
        .collect();
    for j in joiners {
        within(j).await.unwrap();
    }
    stop.store(true, Ordering::SeqCst);
    within(broadcaster).await.unwrap();
    assert!(sent.load(Ordering::SeqCst) > 0);
    assert_eq!(testing::check_invariants(&fx.io.main()), Ok(()));
    drop(raws);
}

/// 4.2: many tasks join a socket to rooms while it disconnects, 1,000 times: after the disconnect the adapter holds
/// nothing for the socket and no empty room.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_join_racing_a_disconnect_leaves_no_membership_behind() {
    let fx = start(IoOptions::new(), |_| {}).await;
    let namespace = fx.io.main();
    for round in 0..1_000u32 {
        let mut raw = Raw::connect(fx.addr).await;
        let id = raw.join("/", None).await;
        let socket = fx.io.main().socket(&id).expect("registered before its CONNECT");
        let joiners: Vec<_> = (0..4)
            .map(|t| {
                let socket = socket.clone();
                tokio::spawn(async move {
                    for n in 0..50u32 {
                        let _ = socket.join(format!("r{}", (n + t) % 9));
                        tokio::task::yield_now().await;
                    }
                })
            })
            .collect();
        let closer = tokio::spawn({
            let socket = socket.clone();
            async move {
                tokio::task::yield_now().await;
                socket.disconnect(false);
            }
        });
        within(closer).await.unwrap();
        // `disconnect` returned: the socket is out of every room, whatever the joiners do from now on.
        assert!(!testing::has_socket(&namespace, &id).unwrap(), "round {round}: a membership of a disconnected socket");
        for j in joiners {
            within(j).await.unwrap();
        }
        assert!(!testing::has_socket(&namespace, &id).unwrap(), "round {round}: a join after the disconnect left a membership");
        assert_eq!(testing::check_invariants(&namespace), Ok(()), "round {round}");
        drop(raw);
    }
    let size = testing::membership_size(&namespace).unwrap();
    assert_eq!(size.sockets, 0);
    assert_eq!(size.rooms, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_broadcast_racing_a_disconnect_never_panics_and_sends_nothing_after_it() {
    let fx = start(IoOptions::new(), |_| {}).await;
    for _ in 0..100 {
        let mut raw = Raw::connect(fx.addr).await;
        let id = raw.join("/", None).await;
        let socket = fx.io.main().socket(&id).expect("registered before its CONNECT");
        socket.join("r").unwrap();
        let io = fx.io.clone();
        let broadcasting = tokio::spawn(async move {
            for _ in 0..20 {
                io.to("r").emit("e", ()).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        socket.disconnect(false);
        within(broadcasting).await.unwrap();
        assert!(!socket.connected());
        let before = testing::buffered_amount(&socket);
        fx.io.to("r").emit("after", ()).await.unwrap();
        // Not `==`: the socket holds its connection weakly, so once the connection is gone the amount reads zero.
        assert!(testing::buffered_amount(&socket) <= before, "nothing is queued for a socket that disconnected");
    }
}
