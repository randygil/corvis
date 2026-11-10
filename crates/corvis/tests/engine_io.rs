#![cfg(feature = "socket-io")]
//! Engine.IO v4 behavior that does not need Node: one test per scenario of the `ws/engine-io` spec. The scenarios
//! that compare with Node run in the differential harness (`parity/socket-io/`, `socket_io_diff.rs`).
//!
//! Timing rule: "not before" bounds use the configured value, "within" bounds are generous.

mod engine_support;

use std::time::Duration;

use corvis::prelude::*;
use corvis::socket_io::EngineOptions;
use corvis::socket_io::engine::{CloseReason, EngineServer, TransportKind};
use engine_support::*;
use futures::StreamExt;

// ------------------------------------------------------------------ entry and paths

#[tokio::test]
async fn polling_request_is_not_routed() {
    let fx = start(EngineOptions::new()).await;
    let resp = http(fx.addr, "GET", POLLING, &[], b"").await;
    assert_eq!(resp.status, 200);
    assert!(resp.text().starts_with("0{"), "{}", resp.text());
    // The same application without an engine has no such route.
    let mut plain = CorvisFactory::create::<AppModule>().await.unwrap();
    let addr = plain.listen_in_background("127.0.0.1:0").await.unwrap();
    assert_eq!(http(addr, "GET", POLLING, &[], b"").await.status, 404);
}

#[tokio::test]
async fn other_paths_are_unchanged() {
    let fx = start(EngineOptions::new()).await;
    let mut plain = CorvisFactory::create::<AppModule>().await.unwrap();
    let plain_addr = plain.listen_in_background("127.0.0.1:0").await.unwrap();
    // The prefix rule is Node's: `/socket.io` without the slash is not the engine's.
    for path in ["/", "/other", "/socket.io", "/socket.iox/", "/api/socket.io/"] {
        let with = http(fx.addr, "GET", path, &[], b"").await;
        let without = http(plain_addr, "GET", path, &[], b"").await;
        assert_eq!(with.status, without.status, "{path}");
        assert_eq!(with.body, without.body, "{path}");
    }
    // Everything under the path is the engine's, whatever the method.
    let resp = http(fx.addr, "DELETE", "/socket.io/anything?EIO=4&transport=polling", &[], b"").await;
    assert_eq!(resp.status, 400);
    assert_eq!(resp.json()["code"], 2);
}

#[tokio::test]
async fn a_second_server_on_an_overlapping_path_is_refused_at_attach() {
    let fx = start(EngineOptions::new()).await;
    let other = EngineServer::new(EngineOptions::new().path("/socket.io/nested")).unwrap();
    let err = other.attach(&fx.app.get_http_server()).unwrap_err();
    assert!(err.to_string().contains("overlaps"), "{err}");
    // A disjoint path is fine, and both are served.
    let disjoint = EngineServer::new(EngineOptions::new().path("/chat")).unwrap();
    disjoint.attach(&fx.app.get_http_server()).unwrap();
    let resp = http(fx.addr, "GET", "/chat/?EIO=4&transport=polling", &[], b"").await;
    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn zero_ping_timeout_is_refused_naming_the_option() {
    let err = EngineServer::new(EngineOptions::new().ping_timeout(Duration::ZERO)).err().expect("refused");
    assert!(err.to_string().contains("ping_timeout"), "{err}");
}

// ----------------------------------------------------------------------- handshake

#[tokio::test]
async fn polling_handshake_answers_the_open_packet() {
    let fx = start(EngineOptions::new()).await;
    let resp = http(fx.addr, "GET", POLLING, &[], b"").await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.header("content-type"), Some("text/plain; charset=UTF-8"));
    assert_eq!(resp.header("cache-control"), Some("no-store"));
    let text = resp.text();
    let open: serde_json::Value = serde_json::from_str(text.strip_prefix('0').unwrap()).unwrap();
    let sid = open["sid"].as_str().unwrap();
    assert_eq!(sid.len(), 20);
    assert_eq!(open["upgrades"], serde_json::json!(["websocket"]));
    assert_eq!(open["pingInterval"], 25000);
    assert_eq!(open["pingTimeout"], 20000);
    assert_eq!(open["maxPayload"], 1_000_000);
    assert_eq!(fx.engine.session_count(), 1);
}

#[tokio::test]
async fn the_open_packet_follows_the_options() {
    let fx = start(
        EngineOptions::new()
            .ping_interval(Duration::from_millis(300))
            .ping_timeout(Duration::from_millis(200))
            .max_http_buffer_size(4096),
    )
    .await;
    let (_, open) = open_polling(fx.addr).await;
    assert_eq!(open["pingInterval"], 300);
    assert_eq!(open["pingTimeout"], 200);
    assert_eq!(open["maxPayload"], 4096);
}

#[tokio::test]
async fn websocket_first_handshake_answers_the_open_packet_with_no_upgrades() {
    let fx = start(EngineOptions::new()).await;
    let mut ws = ws_connect(fx.addr, None).await;
    let first = next_text(&mut ws).await;
    let open: serde_json::Value = serde_json::from_str(first.strip_prefix('0').expect("an open packet")).unwrap();
    assert_eq!(open["upgrades"], serde_json::json!([]));
    assert_eq!(open["sid"].as_str().unwrap().len(), 20);
    assert_eq!(open["maxPayload"], 1_000_000);
}

#[tokio::test]
async fn the_connection_callback_gets_the_session_with_its_handshake() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    assert_eq!(socket.id(), sid);
    assert_eq!(socket.transport(), TransportKind::Polling);
    assert_eq!(socket.handshake().path(), "/socket.io/");
    assert_eq!(socket.handshake().query_param("EIO"), Some("4"));
}

#[tokio::test]
async fn address_behind_a_trusted_proxy() {
    let mut fx = start_with(EngineOptions::new(), |app| {
        app.set_trust_proxy(corvis::TrustProxy::parse("127.0.0.1").unwrap());
    })
    .await;
    http(fx.addr, "GET", POLLING, &[("X-Forwarded-For", "203.0.113.7")], b"").await;
    let socket = fx.socket().await;
    assert_eq!(socket.ip(), "203.0.113.7");
    // Without trusting the proxy the header is ignored.
    let mut fx2 = start(EngineOptions::new()).await;
    http(fx2.addr, "GET", POLLING, &[("X-Forwarded-For", "203.0.113.7")], b"").await;
    assert_eq!(fx2.socket().await.ip(), "127.0.0.1");
}

// ----------------------------------------------------------------------- errors

#[tokio::test]
async fn oversized_sid() {
    let fx = start(EngineOptions::new()).await;
    // The largest sid the server's header limit lets through; it is answered as unknown without a lookup.
    let sid = "a".repeat(8 * 1024);
    let resp = http(fx.addr, "GET", &format!("{POLLING}&sid={sid}"), &[], b"").await;
    assert_eq!(resp.status, 400);
    assert_eq!(resp.text(), r#"{"code":1,"message":"Session ID unknown"}"#);
    // A sid of 1 MiB never reaches the engine: the HTTP layer refuses the request line (Node's parser does too).
    let huge = "a".repeat(1 << 20);
    let resp = http(fx.addr, "GET", &format!("{POLLING}&sid={huge}"), &[], b"").await;
    assert_eq!(resp.status, 431);
    assert_eq!(fx.engine.session_count(), 0);
}

#[tokio::test]
async fn error_codes() {
    let fx = start(EngineOptions::new()).await;
    let cases: Vec<(&str, &str, &str, u16, u8, &str)> = vec![
        ("GET", "/socket.io/?EIO=3&transport=polling", "", 400, 5, "Unsupported protocol version"),
        ("GET", "/socket.io/?transport=polling", "", 400, 5, "Unsupported protocol version"),
        ("GET", "/socket.io/?EIO=4&transport=flashsocket", "", 400, 0, "Transport unknown"),
        ("GET", "/socket.io/?EIO=4", "", 400, 0, "Transport unknown"),
        ("GET", "/socket.io/?EIO=4&transport=polling&sid=aaaaaaaaaaaaaaaaaaaa", "", 400, 1, "Session ID unknown"),
        ("POST", "/socket.io/?EIO=4&transport=polling", "x", 400, 2, "Bad handshake method"),
        ("GET", "/socket.io/?EIO=4&transport=websocket", "", 400, 3, "Bad request"),
    ];
    for (method, target, body, status, code, message) in cases {
        let resp = http(fx.addr, method, target, &[], body.as_bytes()).await;
        assert_eq!(resp.status, status, "{method} {target}");
        assert_eq!(resp.header("content-type"), Some("application/json"), "{target}");
        let json = resp.json();
        assert_eq!(json["code"], code, "{target}");
        assert_eq!(json["message"], message, "{target}");
    }
    assert_eq!(fx.engine.session_count(), 0, "no error created a session");
}

#[tokio::test]
async fn duplicate_parameters_keep_the_last_value_and_a_mismatched_transport_is_refused() {
    let fx = start(EngineOptions::new()).await;
    // Node's query parsing keeps the last value of a repeated parameter.
    let ok = http(fx.addr, "GET", "/socket.io/?EIO=3&EIO=4&transport=polling", &[], b"").await;
    assert_eq!(ok.status, 200);
    let bad = http(fx.addr, "GET", "/socket.io/?EIO=4&EIO=3&transport=polling", &[], b"").await;
    assert_eq!(bad.status, 400);
    assert_eq!(bad.json()["code"], 5);
    // A websocket request on a polling session, outside an upgrade, is a bad request.
    let (sid, _) = open_polling(fx.addr).await;
    let mismatch = http(fx.addr, "GET", &format!("/socket.io/?EIO=4&transport=websocket&sid={sid}"), &[], b"").await;
    assert_eq!(mismatch.status, 400);
    assert_eq!(mismatch.json()["code"], 3);
    // A session request with another protocol version is a bad request too, not code 5.
    let proto = http(fx.addr, "GET", &format!("/socket.io/?EIO=3&transport=polling&sid={sid}"), &[], b"").await;
    assert_eq!(proto.json()["code"], 3);
}

#[tokio::test]
async fn rejected_upgrade_answers_the_status_line_of_node_and_closes() {
    let fx = start(EngineOptions::new()).await;
    let resp = http(
        fx.addr,
        "GET",
        "/socket.io/?EIO=4&transport=websocket&sid=aaaaaaaaaaaaaaaaaaaa",
        &[
            ("Upgrade", "websocket"),
            ("Connection", "Upgrade"),
            ("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="),
            ("Sec-WebSocket-Version", "13"),
        ],
        b"",
    )
    .await;
    assert_eq!(resp.status, 400);
    assert_eq!(resp.header("connection"), Some("close"));
    assert_eq!(resp.text(), "Session ID unknown");
    assert_eq!(fx.engine.session_count(), 0);
}

// ----------------------------------------------------------------------- polling

#[tokio::test]
async fn several_packets_in_one_poll() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    socket.send("one").unwrap();
    socket.send(vec![1u8, 2, 3]).unwrap();
    socket.send("héllo").unwrap();
    let resp = poll(fx.addr, &sid).await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.header("content-type"), Some("text/plain; charset=UTF-8"));
    assert_eq!(resp.header("cache-control"), Some("no-store"));
    assert_eq!(resp.text(), "4one\x1ebAQID\x1e4héllo");
}

#[tokio::test]
async fn a_pending_poll_is_answered_when_a_message_is_queued() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    let addr = fx.addr;
    let pending = tokio::spawn(async move { poll(addr, &sid).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!pending.is_finished(), "nothing to say yet");
    socket.send("now").unwrap();
    let resp = tokio::time::timeout(Duration::from_secs(10), pending).await.unwrap().unwrap();
    assert_eq!(resp.text(), "4now");
}

#[tokio::test]
async fn post_delivers_messages_in_order_and_answers_ok() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    let mut messages = socket.messages();
    let resp = post(fx.addr, &sid, "4a\x1e4b\x1ebAQID").await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.header("content-type"), Some("text/html"));
    assert_eq!(resp.header("content-length"), Some("2"));
    assert_eq!(resp.header("cache-control"), Some("no-store"));
    assert_eq!(resp.text(), "ok");
    use corvis::socket_io::engine::EngineMessage;
    assert_eq!(messages.next().await, Some(EngineMessage::Text("a".into())));
    assert_eq!(messages.next().await, Some(EngineMessage::Text("b".into())));
    assert_eq!(messages.next().await, Some(EngineMessage::Binary(vec![1, 2, 3].into())));
}

#[tokio::test]
async fn overlapping_polls() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    let addr = fx.addr;
    let sid2 = sid.clone();
    let first = tokio::spawn(async move { poll(addr, &sid2).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let second = poll(fx.addr, &sid).await;
    assert_eq!(second.status, 400);
    assert_eq!(socket.closed().await, CloseReason::TransportError);
    // The pending poll is released with a close packet.
    let first = tokio::time::timeout(Duration::from_secs(10), first).await.unwrap().unwrap();
    assert_eq!(first.text(), "1");
}

#[tokio::test]
async fn overlapping_posts_close_the_session() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    // A POST whose body never completes holds the slot.
    let mut slow = tokio::net::TcpStream::connect(fx.addr).await.unwrap();
    use tokio::io::AsyncWriteExt;
    slow.write_all(format!("POST {} HTTP/1.1\r\nHost: x\r\nContent-Length: 10\r\n\r\n4a", poll_url(&sid)).as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let second = post(fx.addr, &sid, "4b").await;
    assert_eq!(second.status, 400);
    assert_eq!(socket.closed().await, CloseReason::TransportError);
}

#[tokio::test]
async fn oversized_post_is_413_and_the_session_survives() {
    let mut fx = start(EngineOptions::new().max_http_buffer_size(1000)).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    let body = format!("4{}", "x".repeat(1000)); // 1001 bytes
    let resp = post(fx.addr, &sid, &body).await;
    assert_eq!(resp.status, 413);
    assert!(socket.close_reason().is_none());
    socket.send("still here").unwrap();
    assert_eq!(poll(fx.addr, &sid).await.text(), "4still here");
    // Exactly the limit is accepted.
    let body = format!("4{}", "x".repeat(999));
    assert_eq!(post(fx.addr, &sid, &body).await.status, 200);
}

#[tokio::test]
async fn octet_stream_posts_are_refused() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    let resp = http(fx.addr, "POST", &poll_url(&sid), &[("Content-Type", "application/octet-stream")], b"4a").await;
    assert_eq!(resp.status, 400);
    assert_eq!(socket.closed().await, CloseReason::TransportError);
}

#[tokio::test]
async fn an_invalid_packet_closes_the_session_with_parse_error() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    let mut messages = socket.messages();
    let resp = post(fx.addr, &sid, "4before\x1e9\x1e4after").await;
    assert_eq!(resp.status, 200, "Node answers ok even when the payload closes the session");
    assert_eq!(socket.closed().await, CloseReason::ParseError);
    use corvis::socket_io::engine::EngineMessage;
    assert_eq!(messages.next().await, Some(EngineMessage::Text("before".into())));
    assert_eq!(messages.next().await, None);
}

#[tokio::test]
async fn a_close_packet_ends_the_session_with_transport_close() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    assert_eq!(post(fx.addr, &sid, "1").await.status, 200);
    assert_eq!(socket.closed().await, CloseReason::TransportClose);
    assert_eq!(fx.engine.session_count(), 0);
    // The id is gone.
    assert_eq!(poll(fx.addr, &sid).await.json()["code"], 1);
}

#[tokio::test]
async fn a_method_other_than_get_or_post_on_a_session_is_a_500() {
    let fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let resp = http(fx.addr, "PUT", &poll_url(&sid), &[], b"").await;
    assert_eq!(resp.status, 500);
}
