#![cfg(feature = "socket-io")]
//! Socket.IO resource bounds and hostile input over real sockets: each default limit of the `ws/socket-io` spec has a
//! test here or in `src/socket_io/tests.rs` (the timer-driven ones, on a paused clock). Timing rule: lower bounds use
//! the configured value, upper bounds are generous.

mod engine_support;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvis::socket_io::{ConnectError, IoOptions, IoServer};
use engine_support::io_client::{Raw, start};
use tokio::sync::Notify;

const WAIT: Duration = Duration::from_secs(10);

fn echo(io: &IoServer) {
    io.on_connection(|socket| {
        socket.on("echo", |_s, args, ack| async move {
            if let Some(ack) = ack {
                let _ = ack.send(args);
            }
        });
    });
}

#[tokio::test]
async fn hostile_packets_close_the_connection_and_never_reach_a_handler() {
    let reached = Arc::new(AtomicUsize::new(0));
    let fx = {
        let reached = reached.clone();
        start(IoOptions::new(), move |io| {
            io.on_connection(move |socket| {
                let reached = reached.clone();
                socket.on("x", move |_s, _a, _k| {
                    let reached = reached.clone();
                    async move {
                        reached.fetch_add(1, Ordering::SeqCst);
                    }
                });
            });
        })
        .await
    };
    let nested = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
    let placeholders = |n: usize| (0..n).map(|i| format!(r#"{{"_placeholder":true,"num":{i}}}"#)).collect::<Vec<_>>().join(",");
    let bad: Vec<String> = vec![
        "9".into(),
        r#"2{"a":1}"#.into(),
        r#"2["disconnect"]"#.into(),
        r#"2["connect_error"]"#.into(),
        r#"2abc["x"]"#.into(),
        format!(r#"2["x",{}]"#, nested(200)),
        format!(r#"2["x",{}"#, "[".repeat(100_000)),
        r#"212345678901234567890["x"]"#.into(),
        r#"29007199254740992["x"]"#.into(),
        r#"3["x"]"#.into(),
        format!(r#"511-["x",{}]"#, placeholders(11)),
        r#"5999999999999999999999-["x"]"#.into(),
        r#"5-["x"]"#.into(),
        r#"50-["x"]"#.into(),
        format!(r#"2["{}"]"#, "n".repeat(300)),
        format!(r#"2["x",{}]"#, vec!["1"; 200].join(",")),
        format!("2/{},[\"x\"]", "a".repeat(400)),
        "0[]".into(),
        "1{}".into(),
        "4{}".into(),
    ];
    for packet in &bad {
        let mut raw = Raw::connect(fx.addr).await;
        raw.join("/", None).await;
        raw.send(packet).await;
        assert!(raw.closed_within(WAIT).await, "{:?} did not close the connection", &packet[..packet.len().min(40)]);
    }
    // A placeholder out of range is found when the last attachment arrives; so is text during reassembly.
    for (header, then_text) in [(r#"51-["x",{"_placeholder":true,"num":3}]"#, false), (r#"51-["x",{"_placeholder":true,"num":0}]"#, true)] {
        let mut raw = Raw::connect(fx.addr).await;
        raw.join("/", None).await;
        raw.send(header).await;
        if then_text {
            raw.send(r#"2["x"]"#).await;
        } else {
            raw.send_binary(&[1]).await;
        }
        assert!(raw.closed_within(WAIT).await, "{header}");
    }
    assert_eq!(reached.load(Ordering::SeqCst), 0);
    // And the server still serves.
    let mut raw = Raw::connect(fx.addr).await;
    raw.join("/", None).await;
    raw.send(r#"2["x"]"#).await;
    engine_support::eventually("the handler", || reached.load(Ordering::SeqCst) == 1).await;
    assert_eq!(reached.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_declared_count_without_data_holds_only_the_header_until_the_attachment_timeout() {
    let fx = start(IoOptions::new().attachment_timeout(Duration::from_millis(700)).connect_timeout(Duration::from_secs(30)), echo).await;
    let mut clients = Vec::new();
    for _ in 0..100 {
        let mut raw = Raw::connect(fx.addr).await;
        raw.join("/", None).await;
        raw.send(r#"510-["x"]"#).await;
        clients.push(raw);
    }
    assert_eq!(fx.io.engine().session_count(), 100);
    // All of them are held (not before the configured time) and all of them are closed afterwards: a "not before"
    // bound below the configured attachment timeout, over real sockets.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fx.io.engine().session_count(), 100, "the packets wait for their attachments");
    for raw in &mut clients {
        assert!(raw.closed_within(WAIT).await);
    }
}

#[tokio::test]
async fn a_client_that_keeps_answering_pings_cannot_hold_a_half_sent_binary_event_forever() {
    let fx = start(
        IoOptions::new().attachment_timeout(Duration::from_millis(500)).engine(
            corvis::socket_io::EngineOptions::new().ping_interval(Duration::from_millis(100)).ping_timeout(Duration::from_millis(200)),
        ),
        echo,
    )
    .await;
    let mut raw = Raw::connect(fx.addr).await;
    raw.join("/", None).await;
    raw.send(r#"52-["x",{"_placeholder":true,"num":0},{"_placeholder":true,"num":1}]"#).await;
    raw.send_binary(&[1]).await;
    // `closed_within` answers every ping, so the heartbeat is healthy throughout.
    assert!(!raw.closed_within(Duration::from_millis(300)).await);
    assert!(raw.closed_within(WAIT).await);
}

#[tokio::test]
async fn a_connect_flood_during_slow_middleware_runs_it_once() {
    let runs = Arc::new(AtomicUsize::new(0));
    let fx = {
        let runs = runs.clone();
        start(IoOptions::new(), move |io| {
            io.of("/admin").unwrap().use_middleware(move |_socket| {
                let runs = runs.clone();
                async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    // Slow middleware, so the CONNECT flood arrives while it runs: the load, not a synchronisation.
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    Ok(())
                }
            });
        })
        .await
    };
    let mut raw = Raw::connect(fx.addr).await;
    for _ in 0..1_000 {
        raw.send("0/admin,").await;
    }
    assert!(raw.closed_within(WAIT).await);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn blocked_handlers_are_capped_and_other_connections_are_served() {
    let running = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    let fx = {
        let (running, release, released) = (running.clone(), release.clone(), released.clone());
        start(IoOptions::new(), move |io| {
            io.on_connection(move |socket| {
                let (running, release, released) = (running.clone(), release.clone(), released.clone());
                socket.on("block", move |_s, _a, _k| {
                    let (running, release, released) = (running.clone(), release.clone(), released.clone());
                    async move {
                        running.fetch_add(1, Ordering::SeqCst);
                        while !released.load(Ordering::SeqCst) {
                            release.notified().await;
                        }
                        running.fetch_sub(1, Ordering::SeqCst);
                    }
                });
                socket.on("echo", |_s, args, ack| async move {
                    if let Some(ack) = ack {
                        let _ = ack.send(args);
                    }
                });
            });
        })
        .await
    };
    let mut raw = Raw::connect(fx.addr).await;
    raw.join("/", None).await;
    for _ in 0..10_000 {
        raw.send(r#"2["block"]"#).await;
    }
    engine_support::eventually("32 handlers to run", || running.load(Ordering::SeqCst) == 32).await;
    // No 33rd: the absence of a handler over real sockets, a quiet period, not a condition.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(running.load(Ordering::SeqCst), 32, "32 handlers run, the rest wait");
    let mut other = Raw::connect(fx.addr).await;
    other.join("/", None).await;
    other.send(r#"21["echo","served"]"#).await;
    assert_eq!(other.recv_text().await, r#"31["served"]"#);
    released.store(true, Ordering::SeqCst);
    release.notify_waiters();
}

#[tokio::test]
async fn panics_in_middleware_and_handlers_are_contained() {
    let fx = start(IoOptions::new(), |io| {
        io.of("/boom").unwrap().use_middleware(|_| async { panic!("internal detail") });
        io.of("/slow").unwrap().use_middleware(|_| async { Err(ConnectError::new("nope")) });
        io.on_connection(|socket| {
            socket.on("boom", |_s, _a, _k| async { panic!("handler detail") });
            socket.on("echo", |_s, args, ack| async move {
                if let Some(ack) = ack {
                    let _ = ack.send(args);
                }
            });
        });
    })
    .await;
    let mut raw = Raw::connect(fx.addr).await;
    raw.send("0/boom,").await;
    assert_eq!(raw.recv_text().await, r#"4/boom,{"message":"Internal server error"}"#);
    raw.send("0/slow,").await;
    assert_eq!(raw.recv_text().await, r#"4/slow,{"message":"nope"}"#);
    raw.join("/", None).await;
    raw.send(r#"2["boom"]"#).await;
    raw.send(r#"22["echo","still alive"]"#).await;
    assert_eq!(raw.recv_text().await, r#"32["still alive"]"#);
}

#[tokio::test]
async fn client_ack_ids_may_repeat_and_each_event_is_answered_once() {
    let fx = start(IoOptions::new(), echo).await;
    let mut raw = Raw::connect(fx.addr).await;
    raw.join("/", None).await;
    // A duplicate id is the client's problem: both events are answered with the id they carried.
    raw.send(r#"25["echo","a"]"#).await;
    raw.send(r#"25["echo","b"]"#).await;
    let answers = vec![raw.recv_text().await, raw.recv_text().await];
    assert!(answers.contains(&r#"35["a"]"#.to_string()) && answers.contains(&r#"35["b"]"#.to_string()), "{answers:?}");
    // An acknowledgement for an id the server never issued is ignored, and so is a replay.
    raw.send(r#"3123["nobody asked"]"#).await;
    raw.send(r#"3123["nobody asked"]"#).await;
    raw.send(r#"26["echo","after"]"#).await;
    assert_eq!(raw.recv_text().await, r#"36["after"]"#);
}

#[tokio::test]
async fn emitting_a_reserved_event_sends_nothing() {
    let result = Arc::new(Mutex::new(None));
    let fx = {
        let result = result.clone();
        start(IoOptions::new(), move |io| {
            io.on_connection(move |socket| {
                *result.lock().unwrap() = Some(socket.emit("disconnect", ()));
                socket.emit("after", ()).unwrap();
            });
        })
        .await
    };
    let mut raw = Raw::connect(fx.addr).await;
    raw.join("/", None).await;
    assert_eq!(raw.recv_text().await, r#"2["after"]"#);
    assert!(result.lock().unwrap().take().unwrap().is_err());
}
