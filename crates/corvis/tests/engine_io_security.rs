#![cfg(feature = "socket-io")]
//! Engine.IO resource bounds: each default limit of the `ws/engine-io` spec has a test here, measuring queue sizes
//! and session counts rather than memory. Timing rule: lower bounds use the configured value, upper bounds are
//! generous.

mod engine_support;

use std::time::Duration;

use corvis::socket_io::EngineOptions;
use corvis::socket_io::engine::{CloseReason, EngineMessage};
use engine_support::*;
use futures::StreamExt;

#[tokio::test]
async fn packet_flood_in_one_post() {
    let mut fx = start(EngineOptions::new().max_payload_packets(100)).await;
    // Exactly the limit is accepted and delivered.
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    let mut messages = socket.messages();
    let body = vec!["4a"; 100].join("\x1e");
    assert_eq!(post(fx.addr, &sid, &body).await.status, 200);
    for _ in 0..100 {
        assert_eq!(messages.next().await, Some(EngineMessage::Text("a".into())));
    }
    // One more is refused as a whole: nothing is delivered and the session is closed.
    let body = vec!["4b"; 101].join("\x1e");
    let resp = post(fx.addr, &sid, &body).await;
    assert_eq!(resp.status, 400);
    assert_eq!(socket.closed().await, CloseReason::ParseError);
    assert_eq!(messages.next().await, None);
}

#[tokio::test]
async fn a_body_of_tiny_packets_is_refused_by_the_default_bound() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    // Just under 1 MB of two-byte packets: about 333,000 of them, far over the 10,000 default.
    let body = vec!["4a"; 333_000].join("\x1e");
    assert!(body.len() <= 1_000_000);
    assert_eq!(post(fx.addr, &sid, &body).await.status, 400);
    assert_eq!(socket.closed().await, CloseReason::ParseError);
    assert_eq!(socket.unread_bytes(), 0);
}

#[tokio::test]
async fn large_backlog() {
    let mut fx = start(EngineOptions::new().max_http_buffer_size(1000)).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    for i in 0..50 {
        socket.send(format!("{i:03}{}", "y".repeat(197))).unwrap();
    }
    let mut polls = 0;
    let mut received = Vec::new();
    while received.len() < 50 {
        let resp = poll(fx.addr, &sid).await;
        assert_eq!(resp.status, 200);
        let text = resp.text();
        // A response holds at most max_http_buffer_size bytes of packets plus the one that crossed the line.
        assert!(text.len() <= 1000 + 202, "response of {} bytes", text.len());
        received.extend(packets(&text));
        polls += 1;
        assert!(polls < 50);
    }
    assert!(polls > 1, "the backlog went out in several responses");
    for (i, p) in received.iter().enumerate() {
        assert!(p.starts_with(&format!("4{i:03}")), "packet {i} out of order: {}", &p[..4]);
    }
}

#[tokio::test]
async fn slow_consumer() {
    let mut fx = start(EngineOptions::new().max_buffered_amount(10_000)).await;
    let (_sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    let message = "z".repeat(1000);
    let mut failed = None;
    for i in 0..100 {
        if let Err(e) = socket.send(message.clone()) {
            failed = Some((i, e));
            break;
        }
        // Whatever is queued stays within the bound plus the packet that crosses it.
        assert!(socket.buffered_amount() <= 10_000 + 1100, "queue of {} bytes", socket.buffered_amount());
    }
    let (at, err) = failed.expect("the send buffer is bounded");
    assert!(at < 15, "failed after {at} messages");
    assert_eq!(err, corvis::socket_io::engine::SendError::Full);
    assert_eq!(socket.closed().await, CloseReason::TransportError);
    assert_eq!(socket.buffered_amount(), 0, "the discarded queue is released");
}

#[tokio::test]
async fn application_that_does_not_read_polling() {
    let mut fx = start(EngineOptions::new().max_unread_bytes(1000)).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    let body = format!("4{}", "m".repeat(1500));
    let addr = fx.addr;
    let sid2 = sid.clone();
    let held = tokio::spawn(async move { post(addr, &sid2, &body).await });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!held.is_finished(), "the answer is held while the application has not read");
    assert!(socket.unread_bytes() >= 1500);
    let mut messages = socket.messages();
    assert!(matches!(messages.next().await, Some(EngineMessage::Text(t)) if t.len() == 1500));
    let resp = tokio::time::timeout(Duration::from_secs(10), held).await.expect("released once read").unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(socket.unread_bytes(), 0);
}

#[tokio::test]
async fn application_that_does_not_read_websocket() {
    let mut fx = start(EngineOptions::new().max_unread_bytes(2000)).await;
    let mut ws = ws_connect(fx.addr, None).await;
    let _ = next_text(&mut ws).await;
    let socket = fx.socket().await;
    let message = format!("4{}", "m".repeat(499));
    for _ in 0..200 {
        send_text(&mut ws, &message).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Reads paused once the unread bytes passed the limit: at most the limit plus a few frames.
    let unread = socket.unread_bytes();
    assert!(unread >= 2000, "the limit was reached ({unread})");
    assert!(unread <= 2000 + 8 * 500, "reads paused near the limit ({unread})");
    // Taking the messages resumes the reads and nothing is lost.
    let mut messages = socket.messages();
    for _ in 0..200 {
        let next = tokio::time::timeout(Duration::from_secs(10), messages.next()).await.expect("a message");
        assert!(matches!(next, Some(EngineMessage::Text(t)) if t.len() == 499));
    }
}

#[tokio::test]
async fn session_cap() {
    let mut fx = start(EngineOptions::new().max_sessions(2)).await;
    let (first, _) = open_polling(fx.addr).await;
    let _s1 = fx.socket().await;
    let _ws = ws_connect(fx.addr, None).await;
    let _s2 = fx.socket().await;
    // Polling over the cap: 503 and the documented body, before any state.
    let resp = http(fx.addr, "GET", POLLING, &[], b"").await;
    assert_eq!(resp.status, 503);
    assert_eq!(resp.json(), serde_json::json!({"code": 3, "message": "Too many sessions"}));
    assert_eq!(fx.engine.session_count(), 2);
    // WebSocket-first over the cap: refused before the upgrade.
    let err = tokio_tungstenite::connect_async(format!("ws://{}/socket.io/?EIO=4&transport=websocket", fx.addr)).await.unwrap_err();
    assert!(matches!(err, tokio_tungstenite::tungstenite::Error::Http(ref r) if r.status() == 503), "{err:?}");
    assert_eq!(fx.engine.session_count(), 2);
    // A closed session frees its place.
    assert_eq!(post(fx.addr, &first, "1").await.status, 200);
    let resp = http(fx.addr, "GET", POLLING, &[], b"").await;
    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn sids_that_are_not_ids_never_reach_the_store() {
    let fx = start(EngineOptions::new()).await;
    for sid in ["short", &"a".repeat(21), &"a".repeat(100_000), "aaaaaaaaaaaaaaaaaaa+", "aaaaaaaaaaaaaaaaaaaé"] {
        let resp = http(fx.addr, "GET", &format!("{POLLING}&sid={sid}"), &[], b"").await;
        assert!(resp.status == 400 || resp.status == 431, "{} -> {}", sid.len(), resp.status);
        if resp.status == 400 {
            assert_eq!(resp.json()["code"], 1);
        }
    }
}

// ------------------------------------------------------------------- security review fixes

/// Waits until `cond` holds, within 10 s.
async fn eventually(what: &str, mut cond: impl FnMut() -> bool) {
    let end = std::time::Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(std::time::Instant::now() < end, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn empty_messages_are_charged_over_polling() {
    let mut fx = start(EngineOptions::new().max_unread_bytes(1000)).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    // 100 empty text packets are 399 bytes of body, but each is charged its fixed overhead: 6,400 against 1,000.
    let body = vec!["4"; 100].join("\x1e");
    let addr = fx.addr;
    let sid2 = sid.clone();
    let held = tokio::spawn(async move { post(addr, &sid2, &body).await });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!held.is_finished(), "the answer is held while empty messages wait unread");
    assert_eq!(socket.unread_bytes(), 100 * 64);
    let mut messages = socket.messages();
    for _ in 0..100 {
        assert_eq!(messages.next().await, Some(EngineMessage::Text(String::new())));
    }
    let resp = tokio::time::timeout(Duration::from_secs(10), held).await.expect("released once read").unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(socket.unread_bytes(), 0);
}

#[tokio::test]
async fn empty_messages_stop_being_read_over_websocket() {
    use tokio_tungstenite::tungstenite::Message;
    let mut fx = start(EngineOptions::new().max_unread_bytes(1000)).await;
    let mut ws = ws_connect(fx.addr, None).await;
    let _ = next_text(&mut ws).await;
    let socket = fx.socket().await;
    for i in 0..400 {
        use futures::SinkExt;
        // Empty text messages and one-byte binary ones.
        let frame = if i % 2 == 0 { Message::Text("4".into()) } else { Message::Binary(vec![1]) };
        ws.send(frame).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let unread = socket.unread_bytes();
    assert!(unread > 1000, "the limit was reached ({unread})");
    assert!(unread <= 1000 + 2 * 65, "reads paused at the limit, not after 400 messages ({unread})");
    let mut messages = socket.messages();
    for _ in 0..400 {
        let next = tokio::time::timeout(Duration::from_secs(10), messages.next()).await.expect("a message");
        assert!(next.is_some());
    }
}

#[tokio::test]
async fn two_sessions_share_the_total_unread_budget() {
    let mut fx = start(EngineOptions::new().max_unread_bytes(2000).max_total_unread_bytes(3000)).await;
    let (a, _) = open_polling(fx.addr).await;
    let socket_a = fx.socket().await;
    let (b, _) = open_polling(fx.addr).await;
    let socket_b = fx.socket().await;
    let body = format!("4{}", "m".repeat(1500));
    // 1,564 bytes charged: under the session limit and under the total.
    assert_eq!(post(fx.addr, &a, &body).await.status, 200);
    // The second pushes the total to 3,128: its answer is held although its own gauge is under the limit.
    let addr = fx.addr;
    let b2 = b.clone();
    let held = tokio::spawn(async move { post(addr, &b2, &body).await });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!held.is_finished(), "held by the budget of the whole server");
    assert!(socket_b.unread_bytes() < 2000);
    // The application reading session A frees the budget.
    let mut messages = socket_a.messages();
    assert!(messages.next().await.is_some());
    let resp = tokio::time::timeout(Duration::from_secs(10), held).await.expect("released").unwrap();
    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn the_total_budget_pauses_websocket_readers_too() {
    let mut fx = start(EngineOptions::new().max_unread_bytes(2000).max_total_unread_bytes(3000)).await;
    let (a, _) = open_polling(fx.addr).await;
    let socket_a = fx.socket().await;
    assert_eq!(post(fx.addr, &a, &format!("4{}", "m".repeat(1500))).await.status, 200);
    let mut ws = ws_connect(fx.addr, None).await;
    let _ = next_text(&mut ws).await;
    let socket_b = fx.socket().await;
    for _ in 0..10 {
        send_text(&mut ws, &format!("4{}", "m".repeat(1000))).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    // One message of B fits (1,564 + 1,064 < 3,000); the next ones push the total over and the reader pauses.
    assert!(socket_b.unread_bytes() <= 3 * 1064, "{}", socket_b.unread_bytes());
    drop(socket_a.messages());
    let mut messages = socket_b.messages();
    for _ in 0..10 {
        let next = tokio::time::timeout(Duration::from_secs(10), messages.next()).await.expect("a message");
        assert!(next.is_some());
    }
}

#[tokio::test]
async fn sessions_per_address_are_capped() {
    let mut fx = start(EngineOptions::new().max_sessions_per_ip(3)).await;
    let mut sids = Vec::new();
    for _ in 0..3 {
        sids.push(open_polling(fx.addr).await.0);
        let _ = fx.socket().await;
    }
    let resp = http(fx.addr, "GET", POLLING, &[], b"").await;
    assert_eq!(resp.status, 503);
    assert_eq!(resp.json(), serde_json::json!({"code": 3, "message": "Too many sessions"}));
    let err = tokio_tungstenite::connect_async(format!("ws://{}/socket.io/?EIO=4&transport=websocket", fx.addr)).await.unwrap_err();
    assert!(matches!(err, tokio_tungstenite::tungstenite::Error::Http(ref r) if r.status() == 503), "{err:?}");
    assert_eq!(fx.engine.session_count(), 3);
    // A closed session releases its place.
    assert_eq!(post(fx.addr, &sids[0], "1").await.status, 200);
    eventually("the session to be removed", || fx.engine.session_count() == 2).await;
    assert_eq!(http(fx.addr, "GET", POLLING, &[], b"").await.status, 200);
}

#[tokio::test]
async fn the_address_cap_follows_trust_proxy() {
    let mut fx = start_with(EngineOptions::new().max_sessions_per_ip(1), |app| {
        app.set_trust_proxy(corvis::TrustProxy::parse("127.0.0.1").unwrap());
    })
    .await;
    for client in ["203.0.113.1", "203.0.113.2"] {
        let resp = http(fx.addr, "GET", POLLING, &[("X-Forwarded-For", client)], b"").await;
        assert_eq!(resp.status, 200, "{client}");
        let _ = fx.socket().await;
    }
    // The same forwarded client again is over its cap; another one is not.
    assert_eq!(http(fx.addr, "GET", POLLING, &[("X-Forwarded-For", "203.0.113.1")], b"").await.status, 503);
    assert_eq!(http(fx.addr, "GET", POLLING, &[("X-Forwarded-For", "203.0.113.3")], b"").await.status, 200);
}

#[tokio::test]
async fn allow_request_runs_before_a_session_is_created() {
    use corvis::socket_io::Admission;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let mut fx = start(EngineOptions::new().allow_request(move |req| {
        counted.fetch_add(1, Ordering::SeqCst);
        let token = req.headers().get("x-token").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        async move {
            match token.as_str() {
                "ok" => Admission::Allow,
                "panic" => panic!("hook failure"),
                _ => Admission::Reject,
            }
        }
    }))
    .await;
    // Polling: rejected with Node's answer, and no session exists.
    let resp = http(fx.addr, "GET", POLLING, &[], b"").await;
    assert_eq!(resp.status, 403);
    assert_eq!(resp.json(), serde_json::json!({"code": 4, "message": "Forbidden"}));
    assert_eq!(fx.engine.session_count(), 0);
    // A panicking hook rejects.
    assert_eq!(http(fx.addr, "GET", POLLING, &[("X-Token", "panic")], b"").await.status, 403);
    // WebSocket-first: refused before the upgrade.
    let err = tokio_tungstenite::connect_async(format!("ws://{}/socket.io/?EIO=4&transport=websocket", fx.addr)).await.unwrap_err();
    assert!(matches!(err, tokio_tungstenite::tungstenite::Error::Http(ref r) if r.status() == 403), "{err:?}");
    assert_eq!(fx.engine.session_count(), 0);
    // Allowed.
    let resp = http(fx.addr, "GET", POLLING, &[("X-Token", "ok")], b"").await;
    assert_eq!(resp.status, 200);
    let _ = fx.socket().await;
    let before = calls.load(Ordering::SeqCst);
    // Requests of an existing session do not run it again.
    let sid = {
        let text = resp.text();
        serde_json::from_str::<serde_json::Value>(&text[1..]).unwrap()["sid"].as_str().unwrap().to_string()
    };
    assert_eq!(post(fx.addr, &sid, "4x").await.status, 200);
    assert_eq!(calls.load(Ordering::SeqCst), before);
}

#[tokio::test]
async fn a_polling_session_that_never_makes_a_second_request_is_closed() {
    let mut fx = start(EngineOptions::new().first_request_timeout(Duration::from_millis(400))).await;
    let started = std::time::Instant::now();
    let (_sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    assert_eq!(socket.closed().await, CloseReason::PingTimeout);
    assert!(started.elapsed() >= Duration::from_millis(400), "not before first_request_timeout");
    eventually("the slot to be released", || fx.engine.session_count() == 0).await;
}

#[tokio::test]
async fn a_second_request_or_a_websocket_first_session_is_not_closed_by_the_first_request_timer() {
    let mut fx = start(EngineOptions::new().first_request_timeout(Duration::from_millis(300))).await;
    let (sid, _) = open_polling(fx.addr).await;
    let polling_socket = fx.socket().await;
    let addr = fx.addr;
    let sid2 = sid.clone();
    // A pending poll is a request: the session lives on.
    let pending = tokio::spawn(async move { poll(addr, &sid2).await });
    let mut ws = ws_connect(fx.addr, None).await;
    let _ = next_text(&mut ws).await;
    let ws_socket = fx.socket().await;
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(polling_socket.close_reason().is_none());
    assert!(ws_socket.close_reason().is_none());
    polling_socket.send("hi").unwrap();
    assert_eq!(pending.await.unwrap().text(), "4hi");
}

#[tokio::test]
async fn transport_confusion_is_a_bad_request() {
    let mut fx = start(EngineOptions::new()).await;
    // A `transport=websocket` request that is not an upgrade is refused whatever the session.
    let resp = http(fx.addr, "GET", "/socket.io/?EIO=4&transport=websocket", &[], b"").await;
    assert_eq!(resp.status, 400);
    assert_eq!(resp.json()["code"], 3);
    // A session that moved to WebSocket does not answer polling requests, and a `POST` delivers nothing.
    let mut ws = ws_connect(fx.addr, None).await;
    let open = next_text(&mut ws).await;
    let sid = serde_json::from_str::<serde_json::Value>(&open[1..]).unwrap()["sid"].as_str().unwrap().to_string();
    let socket = fx.socket().await;
    let mut messages = socket.messages();
    for transport in ["websocket", "polling"] {
        let target = format!("/socket.io/?EIO=4&transport={transport}&sid={sid}");
        let got = http(fx.addr, "GET", &target, &[], b"").await;
        assert_eq!((got.status, got.json()["code"].clone()), (400, serde_json::json!(3)), "GET {transport}");
        let posted = http(fx.addr, "POST", &target, &[("Content-Type", "text/plain;charset=UTF-8")], b"4injected").await;
        assert_eq!((posted.status, posted.json()["code"].clone()), (400, serde_json::json!(3)), "POST {transport}");
    }
    assert!(socket.close_reason().is_none());
    send_text(&mut ws, "4real").await;
    assert_eq!(messages.next().await, Some(EngineMessage::Text("real".into())), "nothing was delivered by the refused requests");
}

#[tokio::test]
async fn huge_durations_are_refused_at_startup() {
    use corvis::socket_io::engine::EngineServer;
    for (name, options) in [
        ("ping_interval", EngineOptions::new().ping_interval(Duration::MAX)),
        ("ping_timeout", EngineOptions::new().ping_timeout(Duration::from_secs(2 * 24 * 3600))),
        ("upgrade_timeout", EngineOptions::new().upgrade_timeout(Duration::from_secs(u64::MAX))),
    ] {
        let err = EngineServer::new(options).err().unwrap_or_else(|| panic!("{name} accepted"));
        assert!(err.to_string().contains(name), "{err}");
    }
}

#[tokio::test]
async fn handshakes_racing_a_close_leave_no_session_behind() {
    let fx = start(EngineOptions::new().max_sessions_per_ip(100_000).max_sessions(100_000)).await;
    let addr = fx.addr;
    let clients: Vec<_> = (0..64).map(|_| tokio::spawn(async move { http(addr, "GET", POLLING, &[], b"").await.status })).collect();
    tokio::time::sleep(Duration::from_millis(5)).await;
    fx.engine.close();
    for client in clients {
        let status = client.await.unwrap();
        assert!(status == 200 || status == 503, "{status}");
    }
    // Every session that was admitted before the close was closed by it; none was admitted after.
    eventually("every session to be closed", || fx.engine.session_count() == 0).await;
    assert_eq!(http(fx.addr, "GET", POLLING, &[], b"").await.status, 503);
}
