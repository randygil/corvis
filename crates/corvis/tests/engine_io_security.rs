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
    eventually("the POST to deliver its message", || socket.unread_bytes() >= 1500).await;
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
    // Absence of reads past the pause: the frames are all written, and a bounded wait gives a broken pause the time
    // to read on; nothing else would show it.
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
    eventually("the POST to deliver its messages", || socket.unread_bytes() >= 100 * 64).await;
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
    // Absence of reads past the pause (see above).
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
    // Not before: past the configured first-request timeout, the sessions are still open.
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
    // The close lands among the handshakes, wherever they are: no order is needed, every outcome below is accepted, and
    // what is checked holds for any interleaving. This only lets some of them start first.
    tokio::task::yield_now().await;
    fx.engine.close();
    for client in clients {
        let status = client.await.unwrap();
        assert!(status == 200 || status == 503, "{status}");
    }
    // Every session that was admitted before the close was closed by it; none was admitted after.
    eventually("every session to be closed", || fx.engine.session_count() == 0).await;
    assert_eq!(http(fx.addr, "GET", POLLING, &[], b"").await.status, 503);
}

// ------------------------------------------------------- security review fixes, second round

#[tokio::test]
async fn a_session_above_its_fair_share_waits_while_the_total_is_over_and_resumes_when_it_drops() {
    // Fair share 16 KiB (the floor). Two 20 KB messages: the second pushes the total over 40,000 and its session
    // holds more than the fair share, so its answer is held although its own limit (30,000) is not reached.
    let mut fx = start(EngineOptions::new().max_unread_bytes(30_000).max_total_unread_bytes(40_000)).await;
    let (a, _) = open_polling(fx.addr).await;
    let socket_a = fx.socket().await;
    let (b, _) = open_polling(fx.addr).await;
    let socket_b = fx.socket().await;
    let body = format!("4{}", "m".repeat(20_000));
    assert_eq!(post(fx.addr, &a, &body).await.status, 200);
    let addr = fx.addr;
    let b2 = b.clone();
    let held = tokio::spawn(async move { post(addr, &b2, &body).await });
    eventually("B's POST to deliver its message", || socket_b.unread_bytes() >= 20_000).await;
    assert!(!held.is_finished(), "held: the total is over and B is above its fair share");
    assert!(socket_b.unread_bytes() < 30_000);
    // The application reading session A drops the total under its limit.
    let mut messages = socket_a.messages();
    assert!(messages.next().await.is_some());
    let resp = tokio::time::timeout(Duration::from_secs(10), held).await.expect("released").unwrap();
    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn the_total_budget_pauses_only_the_websocket_readers_above_their_fair_share() {
    let mut fx = start(EngineOptions::new().max_unread_bytes(30_000).max_total_unread_bytes(40_000)).await;
    let (a, _) = open_polling(fx.addr).await;
    let socket_a = fx.socket().await;
    assert_eq!(post(fx.addr, &a, &format!("4{}", "m".repeat(20_000))).await.status, 200);
    let mut ws = ws_connect(fx.addr, None).await;
    let _ = next_text(&mut ws).await;
    let socket_b = fx.socket().await;
    for _ in 0..10 {
        send_text(&mut ws, &format!("4{}", "m".repeat(20_000))).await;
    }
    // Absence of reads past the pause (see above).
    tokio::time::sleep(Duration::from_millis(500)).await;
    // B's first message fits the total (20,064 + 20,064 > 40,000 only by the second): at most two are read.
    assert!(socket_b.unread_bytes() <= 2 * 20_064, "{}", socket_b.unread_bytes());
    drop(socket_a.messages());
    let mut messages = socket_b.messages();
    for _ in 0..10 {
        let next = tokio::time::timeout(Duration::from_secs(10), messages.next()).await.expect("a message");
        assert!(next.is_some());
    }
}

#[tokio::test]
async fn a_hoarder_filling_the_total_does_not_starve_a_drained_session() {
    // The review's PoC: hoarders fill the whole budget; an innocent session that is drained and answers its pings
    // must stay alive past several ping cycles and keep receiving.
    let mut fx = start(
        EngineOptions::new()
            .ping_interval(Duration::from_millis(300))
            .ping_timeout(Duration::from_millis(200))
            .max_unread_bytes(50_000)
            .max_total_unread_bytes(50_000),
    )
    .await;
    let mut hoarders = Vec::new();
    let mut held = Vec::new();
    for _ in 0..3 {
        let mut ws = ws_connect(fx.addr, None).await;
        let _ = next_text(&mut ws).await;
        let socket = fx.socket().await;
        for _ in 0..4 {
            send_text(&mut ws, &format!("4{}", "h".repeat(20_000))).await;
        }
        hoarders.push((ws, socket));
    }
    eventually("the hoarders to go over the total", || hoarders.iter().map(|(_, s)| s.unread_bytes()).sum::<usize>() > 50_000).await;
    let hoarded: usize = hoarders.iter().map(|(_, s)| s.unread_bytes()).sum();
    assert!(hoarded > 50_000, "the total is over its budget ({hoarded})");
    // Keep the hoarders' own pings answered too, without reading their messages.
    for (mut ws, socket) in hoarders {
        held.push(socket);
        tokio::spawn(async move {
            loop {
                match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
                    Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) if t == "2" => send_text(&mut ws, "3").await,
                    Ok(Some(Ok(_))) => {}
                    _ => break,
                }
            }
        });
    }
    // The innocent session.
    let mut ws = ws_connect(fx.addr, None).await;
    let _ = next_text(&mut ws).await;
    let innocent = fx.socket().await;
    let received = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = received.clone();
    let mut messages = innocent.messages();
    tokio::spawn(async move {
        while messages.next().await.is_some() {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    // The innocent session keeps talking through several heartbeats (each ping proves one passed): it answers every ping
    // and sends a message with each answer, while the pool stays over its budget.
    let mut sent = 0;
    let mut pings = 0;
    while pings < 4 {
        match ws.next().await {
            Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t))) if t == "2" => {
                pings += 1;
                send_text(&mut ws, "3").await;
                send_text(&mut ws, "4hello").await;
                sent += 1;
            }
            Some(Ok(_)) => {}
            other => panic!("the innocent connection ended: {other:?}"),
        }
    }
    assert!(innocent.close_reason().is_none(), "{:?}", innocent.close_reason());
    eventually("every message to arrive", || received.load(std::sync::atomic::Ordering::SeqCst) == sent).await;
    drop(held);
}

#[tokio::test]
async fn forwarded_address_forms_are_counted_as_one_client() {
    let mut fx = start_with(EngineOptions::new().max_sessions_per_ip(1), |app| {
        app.set_trust_proxy(corvis::TrustProxy::parse("127.0.0.1").unwrap());
    })
    .await;
    // The first form opens the session; every other spelling of the same client is over its cap.
    assert_eq!(http(fx.addr, "GET", POLLING, &[("X-Forwarded-For", "203.0.113.7:40000")], b"").await.status, 200);
    let _ = fx.socket().await;
    for form in ["203.0.113.7", "203.0.113.7:1", "::ffff:203.0.113.7"] {
        assert_eq!(http(fx.addr, "GET", POLLING, &[("X-Forwarded-For", form)], b"").await.status, 503, "{form}");
    }
    assert_eq!(http(fx.addr, "GET", POLLING, &[("X-Forwarded-For", "[2001:db8:1:2::1]:40000")], b"").await.status, 200);
    let _ = fx.socket().await;
    for form in ["2001:db8:1:2::1", "[2001:db8:1:2::5]", "2001:db8:1:2::9%eth0"] {
        assert_eq!(http(fx.addr, "GET", POLLING, &[("X-Forwarded-For", form)], b"").await.status, 503, "{form}");
    }
    // A value that is no address counts against the socket's peer, not against a key of its own.
    assert_eq!(http(fx.addr, "GET", POLLING, &[("X-Forwarded-For", "junk-1")], b"").await.status, 200);
    let _ = fx.socket().await;
    assert_eq!(http(fx.addr, "GET", POLLING, &[("X-Forwarded-For", "junk-2")], b"").await.status, 503);
}

#[tokio::test]
async fn allow_request_is_not_invoked_for_a_client_over_a_cap() {
    use corvis::socket_io::Admission;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let mut fx = start(EngineOptions::new().max_sessions_per_ip(1).allow_request(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        async { Admission::Allow }
    }))
    .await;
    assert_eq!(http(fx.addr, "GET", POLLING, &[], b"").await.status, 200);
    let _ = fx.socket().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // Over the per-address cap: 503 and no hook call, polling and WebSocket-first.
    assert_eq!(http(fx.addr, "GET", POLLING, &[], b"").await.status, 503);
    let err = tokio_tungstenite::connect_async(format!("ws://{}/socket.io/?EIO=4&transport=websocket", fx.addr)).await.unwrap_err();
    assert!(matches!(err, tokio_tungstenite::tungstenite::Error::Http(ref r) if r.status() == 503), "{err:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn allow_request_that_takes_too_long_refuses_the_request() {
    use corvis::socket_io::Admission;
    // A hook that never answers.
    let fx = start(EngineOptions::new().allow_request_timeout(Duration::from_millis(300)).allow_request(|_| async {
        futures::future::pending::<()>().await;
        Admission::Allow
    }))
    .await;
    let started = std::time::Instant::now();
    let resp = http(fx.addr, "GET", POLLING, &[], b"").await;
    assert_eq!(resp.status, 403);
    assert_eq!(resp.json(), serde_json::json!({"code": 4, "message": "Forbidden"}));
    assert!(started.elapsed() >= Duration::from_millis(300), "not before the timeout");
    assert_eq!(fx.engine.session_count(), 0);
}

#[tokio::test]
async fn blind_pongs_do_not_keep_a_polling_session_that_never_polls() {
    let options = EngineOptions::new().ping_interval(Duration::from_millis(300)).ping_timeout(Duration::from_millis(200));
    let mut fx = start(options).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    // Queued data the client never collects: it must not be retained for as long as the pongs keep coming.
    for _ in 0..8 {
        socket.send("y".repeat(1024)).unwrap();
    }
    // The client posts pongs back to back without ever polling, until the session ends (the heartbeat, 300 + 200 ms, runs
    // on real sockets).
    let deadline = tokio::time::Instant::now() + EVENTUALLY;
    while socket.close_reason().is_none() {
        assert!(tokio::time::Instant::now() < deadline, "blind pongs kept the session alive for {EVENTUALLY:?}");
        let _ = post(fx.addr, &sid, "3").await;
    }
    assert_eq!(socket.close_reason(), Some(CloseReason::PingTimeout), "closed by the ping timeout, not kept by blind pongs");
    eventually("the session to be removed", || fx.engine.session_count() == 0).await;
}

#[tokio::test]
async fn a_polling_client_that_receives_the_ping_and_answers_stays_alive() {
    let options = EngineOptions::new().ping_interval(Duration::from_millis(300)).ping_timeout(Duration::from_millis(200));
    let mut fx = start(options).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    // The GET is parked when each ping is queued; the client answers every ping it receives.
    let mut pings = 0;
    while pings < 3 {
        let resp = poll(fx.addr, &sid).await;
        assert_eq!(resp.status, 200);
        if packets(&resp.text()).iter().any(|p| p == "2") {
            pings += 1;
            assert_eq!(post(fx.addr, &sid, "3").await.status, 200);
        }
    }
    assert_eq!(socket.close_reason(), None);
}

#[tokio::test]
async fn a_ping_delivered_over_polling_is_answered_over_the_upgraded_transport() {
    let options = EngineOptions::new().ping_interval(Duration::from_millis(300)).ping_timeout(Duration::from_millis(400));
    let mut fx = start(options).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    let resp = poll(fx.addr, &sid).await;
    assert!(packets(&resp.text()).iter().any(|p| p == "2"), "{}", resp.text());
    let mut ws = ws_connect(fx.addr, Some(&sid)).await;
    probe(&mut ws).await;
    send_text(&mut ws, "5").await;
    send_text(&mut ws, "3").await;
    // The next pings arrive over the WebSocket and are answered there.
    for _ in 0..3 {
        while next_text(&mut ws).await != "2" {}
        send_text(&mut ws, "3").await;
    }
    assert_eq!(socket.close_reason(), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_handshakes_from_an_address_at_its_cap_do_not_all_run_the_hook() {
    use corvis::socket_io::Admission;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let fx = start(EngineOptions::new().max_sessions_per_ip(2).allow_request(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        async {
            // A slow hook, so the handshakes overlap while it runs: the load, not a synchronisation.
            tokio::time::sleep(Duration::from_millis(300)).await;
            Admission::Allow
        }
    }))
    .await;
    let addr = fx.addr;
    let tasks: Vec<_> = (0..50).map(|_| tokio::spawn(async move { http(addr, "GET", POLLING, &[], b"").await.status })).collect();
    let mut ok = 0;
    let mut refused = 0;
    for t in tasks {
        match t.await.unwrap() {
            200 => ok += 1,
            503 => refused += 1,
            other => panic!("unexpected status {other}"),
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2, "the hook runs at most cap - current times");
    assert_eq!((ok, refused), (2, 48));
    assert_eq!(fx.engine.session_count(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_or_failed_hook_gives_its_slot_back() {
    use corvis::socket_io::Admission;
    let fx = start(EngineOptions::new().max_sessions_per_ip(1).allow_request_timeout(Duration::from_millis(200)).allow_request(|req| {
        let mode = req.headers().get("x-mode").and_then(|v| v.to_str().ok()).unwrap_or("allow").to_string();
        async move {
            match mode.as_str() {
                "hang" => futures::future::pending::<Admission>().await,
                "panic" => panic!("hook failure"),
                "reject" => Admission::Reject,
                _ => Admission::Allow,
            }
        }
    }))
    .await;
    for mode in ["hang", "panic", "reject", "hang", "panic", "reject"] {
        assert_eq!(http(fx.addr, "GET", POLLING, &[("X-Mode", mode)], b"").await.status, 403, "{mode}");
    }
    // The slot is free: the one allowed session fits, a second does not.
    assert_eq!(http(fx.addr, "GET", POLLING, &[], b"").await.status, 200);
    assert_eq!(http(fx.addr, "GET", POLLING, &[], b"").await.status, 503);
}
