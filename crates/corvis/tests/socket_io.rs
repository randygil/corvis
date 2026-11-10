#![cfg(feature = "socket-io")]
//! The Socket.IO layer over real transports: a raw client speaks Engine.IO over WebSocket and over polling. The
//! timer logic and most protocol details are tested without a network in `src/socket_io/tests.rs`; these tests check
//! that the pieces work together on a socket. Timing rule: "not before" bounds use the configured value, "within"
//! bounds are generous.

mod engine_support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use corvis::socket_io::{Binary, IoOptions, IoValue};
use engine_support::io_client::{Msg, Raw, start};
use engine_support::{open_polling, poll, post};

fn echo(io: &corvis::socket_io::IoServer) {
    io.on_connection(|socket| {
        socket.on("echo", |_socket, args, ack| async move {
            if let Some(ack) = ack {
                let _ = ack.send(args);
            }
        });
    });
}

#[tokio::test]
async fn connect_event_and_acknowledgement_over_websocket() {
    let fx = start(IoOptions::new(), |io| {
        io.on_connection(|socket| {
            socket.on("sum", |_socket, args, ack| async move {
                let total: i64 = args.iter().filter_map(IoValue::as_i64).sum();
                if let Some(ack) = ack {
                    let _ = ack.send((total, "ok"));
                }
            });
        });
    })
    .await;
    let mut raw = Raw::connect(fx.addr).await;
    let sid = raw.join("/", Some(r#"{"token":"abc"}"#)).await;
    assert_eq!(sid.len(), 20);
    assert_ne!(sid, raw.open["sid"].as_str().unwrap(), "the socket id is not the session id");
    raw.send(r#"23["sum",1,2]"#).await;
    assert_eq!(raw.recv_text().await, r#"33[3,"ok"]"#);
}

#[tokio::test]
async fn two_namespaces_on_one_session_with_auth_and_distinct_ids() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let fx = start(IoOptions::new(), |io| {
        let admin = io.of("/admin").unwrap();
        for ns in [io.main(), admin] {
            let seen = seen.clone();
            ns.on_connection(move |socket| {
                seen.lock().unwrap().push(format!("{} {}", socket.namespace().name(), socket.handshake().auth()));
            });
        }
    })
    .await;
    let mut raw = Raw::connect(fx.addr).await;
    let main = raw.join("/", None).await;
    let admin = raw.join("/admin", Some(r#"{"token":"x"}"#)).await;
    let session = raw.open["sid"].as_str().unwrap().to_string();
    assert_eq!([&main, &admin, &session].iter().collect::<std::collections::HashSet<_>>().len(), 3);
    assert_eq!(*seen.lock().unwrap(), ["/ {}", r#"/admin {"token":"x"}"#]);
    assert_eq!(fx.io.engine().session_count(), 1, "both namespaces share one session");
    raw.send(r#"0/missing,"#).await;
    assert_eq!(raw.recv_text().await, r#"4/missing,{"message":"Invalid namespace"}"#);
}

#[tokio::test]
async fn binary_echo_over_polling() {
    let fx = start(IoOptions::new(), echo).await;
    let (sid, _) = open_polling(fx.addr).await;
    assert_eq!(post(fx.addr, &sid, "40").await.status, 200);
    assert!(poll(fx.addr, &sid).await.text().starts_with("40{\"sid\":"));
    let data: Vec<u8> = (0..1024u32).map(|i| (i * 7 % 251) as u8).collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
    // The header and its attachment in one POST: `451-5[...]`, then `b<base64>`.
    let body = format!(r#"451-5["echo",{{"name":"f","data":{{"_placeholder":true,"num":0}}}}]{}b{b64}"#, '\x1e');
    assert_eq!(post(fx.addr, &sid, &body).await.status, 200);
    let answer = poll(fx.addr, &sid).await.text();
    let (header, attachment) = answer.split_once('\x1e').expect("the packet and its attachment in one response");
    assert_eq!(header, r#"461-5[{"name":"f","data":{"_placeholder":true,"num":0}}]"#);
    assert_eq!(attachment, format!("b{b64}"));
}

#[tokio::test]
async fn binary_acknowledgement_over_websocket() {
    let fx = start(IoOptions::new(), echo).await;
    let mut raw = Raw::connect(fx.addr).await;
    raw.join("/", None).await;
    raw.send(r#"52-8["echo",{"_placeholder":true,"num":0},{"_placeholder":true,"num":1}]"#).await;
    raw.send_binary(&[1, 2, 3]).await;
    raw.send_binary(&[4, 5]).await;
    assert_eq!(raw.recv_text().await, r#"62-8[{"_placeholder":true,"num":0},{"_placeholder":true,"num":1}]"#);
    assert_eq!(raw.recv().await, Msg::Binary(vec![1, 2, 3]));
    assert_eq!(raw.recv().await, Msg::Binary(vec![4, 5]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_binary_emits_keep_each_packet_with_its_attachments() {
    let fx = start(IoOptions::new(), |io| {
        io.on_connection(|socket| {
            socket.on("go", |socket, _args, _ack| async move {
                let threads: Vec<_> = (0..32u8)
                    .map(|n| {
                        let socket = socket.clone();
                        std::thread::spawn(move || {
                            for round in 0..20u8 {
                                socket.emit("pair", (n, round, Binary::from(vec![n, round, 0]), Binary::from(vec![n, round, 1]))).unwrap();
                            }
                        })
                    })
                    .collect();
                for t in threads {
                    t.join().unwrap();
                }
            });
        });
    })
    .await;
    let mut raw = Raw::connect(fx.addr).await;
    raw.join("/", None).await;
    raw.send(r#"2["go"]"#).await;
    for _ in 0..32 * 20 {
        let header = raw.recv_text().await;
        let tag: Vec<u8> = {
            let args: serde_json::Value = serde_json::from_str(
                header.strip_prefix("52-").unwrap().split_once('[').map(|(_, rest)| format!("[{rest}")).unwrap().as_str(),
            )
            .unwrap();
            vec![args[1].as_u64().unwrap() as u8, args[2].as_u64().unwrap() as u8]
        };
        assert_eq!(raw.recv().await, Msg::Binary(vec![tag[0], tag[1], 0]));
        assert_eq!(raw.recv().await, Msg::Binary(vec![tag[0], tag[1], 1]));
    }
}

#[tokio::test]
async fn the_connect_timeout_closes_a_silent_connection_not_before_it() {
    let fx = start(IoOptions::new().connect_timeout(Duration::from_millis(600)), |_| {}).await;
    let mut raw = Raw::connect(fx.addr).await;
    let started = Instant::now();
    assert!(!raw.closed_within(Duration::from_millis(400)).await, "not before the configured time");
    assert!(raw.closed_within(Duration::from_secs(10)).await);
    assert!(started.elapsed() >= Duration::from_millis(600));
}

#[tokio::test]
async fn shutting_the_application_down_reports_server_shutting_down() {
    let reasons = Arc::new(Mutex::new(Vec::new()));
    let mut fx = start(IoOptions::new(), |io| {
        let reasons = reasons.clone();
        io.on_connection(move |socket| {
            let reasons = reasons.clone();
            socket.on_disconnect(move |_socket, reason| reasons.lock().unwrap().push(reason.as_str().to_string()));
        });
    })
    .await;
    let mut a = Raw::connect(fx.addr).await;
    let mut b = Raw::connect(fx.addr).await;
    a.join("/", None).await;
    b.join("/", None).await;
    fx.app.close().await.unwrap();
    assert_eq!(*reasons.lock().unwrap(), ["server shutting down", "server shutting down"]);
    assert!(a.closed_within(Duration::from_secs(10)).await);
    assert!(b.closed_within(Duration::from_secs(10)).await);
}

#[tokio::test]
async fn disconnect_reasons_from_a_real_client() {
    let reasons = Arc::new(Mutex::new(Vec::new()));
    let fx = start(IoOptions::new(), |io| {
        io.of("/admin").unwrap();
        for ns in [io.main(), io.of("/admin").unwrap()] {
            let reasons = reasons.clone();
            ns.on_connection(move |socket| {
                let reasons = reasons.clone();
                socket.on_disconnect(move |s, reason| reasons.lock().unwrap().push(format!("{} {}", s.namespace().name(), reason)));
            });
        }
    })
    .await;
    let mut raw = Raw::connect(fx.addr).await;
    raw.join("/", None).await;
    raw.join("/admin", None).await;
    raw.send("1/admin,").await;
    for _ in 0..100 {
        if reasons.lock().unwrap().len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(*reasons.lock().unwrap(), ["/admin client namespace disconnect"]);
    raw.close().await;
    for _ in 0..100 {
        if reasons.lock().unwrap().len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(reasons.lock().unwrap()[1], "/ transport close");
}

#[tokio::test]
async fn the_dropped_handle_does_not_stop_the_server() {
    let counter = Arc::new(AtomicUsize::new(0));
    let fx = {
        let counter = counter.clone();
        start(IoOptions::new(), move |io| {
            io.on_connection(move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
            });
        })
        .await
    };
    let engine_sessions = fx.io.engine().clone();
    let addr = fx.addr;
    drop(fx.io);
    let mut raw = Raw::connect(addr).await;
    raw.join("/", None).await;
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert_eq!(engine_sessions.session_count(), 1);
}
