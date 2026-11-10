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
