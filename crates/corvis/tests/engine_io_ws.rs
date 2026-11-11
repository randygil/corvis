#![cfg(feature = "socket-io")]
//! Engine.IO v4 over WebSocket, the upgrade, CORS and origin checks, the heartbeat, and close and shutdown: one test
//! per scenario of the `ws/engine-io` spec that does not need Node. Polling and the handshake are in `engine_io.rs`.
//!
//! Timing rule: "not before" bounds use the configured value, "within" bounds are generous.

mod engine_support;

use std::time::Duration;

use corvis::prelude::*;
use corvis::router::cors::CorsOptions;
use corvis::socket_io::EngineOptions;
use corvis::socket_io::engine::{CloseReason, EngineMessage, EngineServer, TransportKind};
use engine_support::*;
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

// ------------------------------------------------------------------------- CORS

#[tokio::test]
async fn no_cors_by_default() {
    let fx = start(EngineOptions::new()).await;
    let resp = http(fx.addr, "GET", POLLING, &[("Origin", "https://evil.example")], b"").await;
    assert_eq!(resp.status, 200);
    assert!(resp.header("access-control-allow-origin").is_none());
    // `enable_cors` of the application does not apply to the engine path either.
    let fx = start_with(EngineOptions::new(), |app| {
        app.enable_cors(CorsOptions::new().origin("https://app.example"));
    })
    .await;
    let resp = http(fx.addr, "GET", POLLING, &[("Origin", "https://app.example")], b"").await;
    assert_eq!(resp.status, 200);
    assert!(resp.header("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn configured_origin() {
    let fx = start(EngineOptions::new().cors(CorsOptions::new().origin("https://app.example"))).await;
    let resp = http(fx.addr, "GET", POLLING, &[("Origin", "https://app.example")], b"").await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.header("access-control-allow-origin"), Some("https://app.example"));
    // A single configured origin is sent as is, whatever the request says (the `cors` package does the same): the
    // browser, not the server, rejects the mismatch.
    let resp = http(fx.addr, "GET", POLLING, &[("Origin", "https://evil.example")], b"").await;
    assert_eq!(resp.header("access-control-allow-origin"), Some("https://app.example"));
    let preflight =
        http(fx.addr, "OPTIONS", POLLING, &[("Origin", "https://app.example"), ("Access-Control-Request-Method", "POST")], b"").await;
    assert!(preflight.status == 204 || preflight.status == 200, "{}", preflight.status);
    assert_eq!(preflight.header("access-control-allow-origin"), Some("https://app.example"));
}

#[tokio::test]
async fn origin_allowlist_on_websocket() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let fx = start(EngineOptions::new().allowed_origins(["https://app.example"])).await;
    let connect = |origin: &'static str| {
        let addr = fx.addr;
        async move {
            let mut req = format!("ws://{addr}/socket.io/?EIO=4&transport=websocket").into_client_request().unwrap();
            req.headers_mut().insert("origin", origin.parse().unwrap());
            tokio_tungstenite::connect_async(req).await
        }
    };
    assert!(connect("https://evil.example").await.is_err());
    assert_eq!(fx.engine.session_count(), 0);
    let (mut ws, _) = connect("https://app.example").await.expect("an allowed origin connects");
    assert!(next_text(&mut ws).await.starts_with("0{"));
    // The polling handshake is checked too: code 4, status 403.
    let resp = http(fx.addr, "GET", POLLING, &[("Origin", "https://evil.example")], b"").await;
    assert_eq!(resp.status, 403);
    assert_eq!(resp.json()["code"], 4);
    let resp = http(fx.addr, "GET", POLLING, &[("Origin", "https://app.example")], b"").await;
    assert_eq!(resp.status, 200);
}

// ------------------------------------------------------------- WebSocket transport

#[tokio::test]
async fn a_websocket_first_session_exchanges_messages() {
    let mut fx = start(EngineOptions::new()).await;
    let mut ws = ws_connect(fx.addr, None).await;
    let open = next_text(&mut ws).await;
    assert!(open.starts_with("0{"), "{open}");
    let socket = fx.socket().await;
    assert_eq!(socket.transport(), TransportKind::WebSocket);
    let mut messages = socket.messages();
    send_text(&mut ws, "4hello").await;
    assert_eq!(messages.next().await, Some(EngineMessage::Text("hello".into())));
    socket.send("world").unwrap();
    assert_eq!(next_text(&mut ws).await, "4world");
    // A client close frame ends the session.
    ws.send(Message::Close(None)).await.unwrap();
    assert_eq!(socket.closed().await, CloseReason::TransportClose);
}

/// The writer may be the first to see the peer's close frame (the library refuses its next frame), or the reader may:
/// either way the session ends as a peer close.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_close_frame_while_the_server_writes_ends_the_session_as_a_transport_close() {
    let mut fx = start(EngineOptions::new().max_buffered_amount(4 * 1024 * 1024)).await;
    let chunk = "x".repeat(64 * 1024);
    for round in 0..200 {
        let mut ws = ws_connect(fx.addr, None).await;
        let _open = next_text(&mut ws).await;
        let socket = fx.socket().await;
        // 1 MiB queued for a client that sends its close frame instead of reading it.
        for _ in 0..16 {
            socket.send(chunk.as_str()).unwrap();
        }
        ws.send(Message::Close(None)).await.unwrap();
        assert_eq!(socket.closed().await, CloseReason::TransportClose, "round {round}");
    }
}

#[tokio::test]
async fn a_websocket_message_over_the_limit_closes_with_1009() {
    let mut fx = start(EngineOptions::new().max_http_buffer_size(1000)).await;
    let mut ws = ws_connect(fx.addr, None).await;
    let _ = next_text(&mut ws).await;
    let socket = fx.socket().await;
    send_text(&mut ws, &format!("4{}", "x".repeat(1200))).await;
    assert_eq!(socket.closed().await, CloseReason::TransportError);
    let code = loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next()).await.expect("the close within 10 s") {
            Some(Ok(Message::Close(Some(frame)))) => break u16::from(frame.code),
            Some(Ok(_)) => continue,
            other => panic!("expected a close frame, got {other:?}"),
        }
    };
    assert_eq!(code, 1009);
}

#[tokio::test]
async fn a_client_ping_closes_the_session_with_transport_error() {
    let mut fx = start(EngineOptions::new()).await;
    let mut ws = ws_connect(fx.addr, None).await;
    let _ = next_text(&mut ws).await;
    let socket = fx.socket().await;
    send_text(&mut ws, "2").await;
    assert_eq!(socket.closed().await, CloseReason::TransportError);
}

/// A WebSocket client on a raw socket with a 4 KiB receive buffer that completed the handshake and reads nothing
/// else.
async fn raw_ws_client(addr: std::net::SocketAddr, target: &str) -> tokio::net::TcpStream {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let mut stream = socket.connect(addr).await.unwrap();
    let request = format!(
        "GET {target} HTTP/1.1\r\nHost: {addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 101"), "{}", String::from_utf8_lossy(&head));
    stream
}

#[tokio::test]
async fn a_peer_that_never_reads_is_dropped_after_the_send_timeout() {
    let mut fx = start_with(EngineOptions::new(), |app| {
        app.set_server_options(corvis::ServerOptions::new().send_timeout(Duration::from_secs(1)));
    })
    .await;
    let _client = raw_ws_client(fx.addr, "/socket.io/?EIO=4&transport=websocket").await;
    let socket = fx.socket().await;
    let started = std::time::Instant::now();
    let chunk = "x".repeat(100_000);
    let feeder = {
        let socket = socket.clone();
        tokio::spawn(async move {
            // 1 MB/s: slow enough that `max_buffered_amount` (8 MiB) cannot fill before the send timeout fires,
            // whatever the timer resolution of the platform.
            // The application's pace of sending (a producer), not a synchronisation.
            while socket.send(chunk.clone()).is_ok() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
    };
    let reason = tokio::time::timeout(Duration::from_secs(30), socket.closed()).await.expect("closed within 30 s");
    feeder.abort();
    assert_eq!(reason, CloseReason::TransportError);
    assert!(started.elapsed() >= Duration::from_secs(1), "not before the send timeout");
    assert_eq!(fx.engine.session_count(), 0);
}

// ----------------------------------------------------------------------- upgrade

async fn recv_close_or_end(ws: &mut WsClient) -> bool {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next()).await.expect("the socket to end within 10 s") {
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return true,
            Some(Ok(_)) => continue,
        }
    }
}

#[tokio::test]
async fn upgrade_with_queued_packets_loses_and_reorders_nothing() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, open) = open_polling(fx.addr).await;
    assert_eq!(open["upgrades"], serde_json::json!(["websocket"]));
    let socket = fx.socket().await;
    for i in 0..5 {
        socket.send(format!("before{i}")).unwrap();
    }
    let mut ws = ws_connect(fx.addr, Some(&sid)).await;
    probe(&mut ws).await;
    // The client's poll returns (a noop while upgrading, or the queued packets) and it sends `5`.
    let first = poll(fx.addr, &sid).await;
    let mut got: Vec<String> = packets(&first.text()).into_iter().filter(|p| p != "6").collect();
    send_text(&mut ws, "5").await;
    for i in 5..10 {
        socket.send(format!("after{i}")).unwrap();
    }
    // Everything that is not on the polling response arrives on the WebSocket, in order, once.
    let expected: Vec<String> = (0..5).map(|i| format!("4before{i}")).chain((5..10).map(|i| format!("4after{i}"))).collect();
    while got.len() < expected.len() {
        got.push(next_text(&mut ws).await);
    }
    assert_eq!(got, expected);
    assert_eq!(socket.transport(), TransportKind::WebSocket);
    socket.send("last").unwrap();
    assert_eq!(next_text(&mut ws).await, "4last");
}

#[tokio::test]
async fn a_pending_poll_gets_a_noop_while_upgrading() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let _socket = fx.socket().await;
    let mut ws = ws_connect(fx.addr, Some(&sid)).await;
    probe(&mut ws).await;
    let resp = poll(fx.addr, &sid).await;
    assert_eq!(resp.text(), "6");
    send_text(&mut ws, "5").await;
}

#[tokio::test]
async fn upgrade_never_completed() {
    let mut fx = start(EngineOptions::new().upgrade_timeout(Duration::from_millis(400))).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    let mut ws = ws_connect(fx.addr, Some(&sid)).await;
    let started = std::time::Instant::now();
    assert!(recv_close_or_end(&mut ws).await);
    assert!(started.elapsed() >= Duration::from_millis(400), "not before upgrade_timeout");
    // The session stays on polling and works.
    assert!(socket.close_reason().is_none());
    assert_eq!(socket.transport(), TransportKind::Polling);
    socket.send("still polling").unwrap();
    assert_eq!(poll(fx.addr, &sid).await.text(), "4still polling");
    // A new upgrade is possible afterwards.
    let mut again = ws_connect(fx.addr, Some(&sid)).await;
    probe(&mut again).await;
    let _ = poll(fx.addr, &sid).await;
    send_text(&mut again, "5").await;
    socket.send("upgraded").unwrap();
    assert_eq!(next_text(&mut again).await, "4upgraded");
}

#[tokio::test]
async fn second_upgrade() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    let mut first = ws_connect(fx.addr, Some(&sid)).await;
    probe(&mut first).await;
    // A second WebSocket while one is probing is closed; the first still upgrades.
    let mut second = ws_connect(fx.addr, Some(&sid)).await;
    assert!(recv_close_or_end(&mut second).await);
    let _ = poll(fx.addr, &sid).await;
    send_text(&mut first, "5").await;
    socket.send("one").unwrap();
    assert_eq!(next_text(&mut first).await, "4one");
    // After the upgrade another one is closed too, and the session is untouched.
    let mut third = ws_connect(fx.addr, Some(&sid)).await;
    assert!(recv_close_or_end(&mut third).await);
    assert!(socket.close_reason().is_none());
    socket.send("two").unwrap();
    assert_eq!(next_text(&mut first).await, "4two");
}

#[tokio::test]
async fn a_non_probe_packet_closes_only_the_candidate() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    let mut ws = ws_connect(fx.addr, Some(&sid)).await;
    send_text(&mut ws, "4not a probe").await;
    assert!(recv_close_or_end(&mut ws).await);
    assert!(socket.close_reason().is_none());
    assert_eq!(socket.transport(), TransportKind::Polling);
}

// --------------------------------------------------------------------- heartbeat

#[tokio::test]
async fn client_stops_answering_pings() {
    let mut fx = start(EngineOptions::new().ping_interval(Duration::from_millis(300)).ping_timeout(Duration::from_millis(200))).await;
    let started = std::time::Instant::now();
    let mut ws = ws_connect(fx.addr, None).await;
    let _ = next_text(&mut ws).await;
    let socket = fx.socket().await;
    assert_eq!(next_text(&mut ws).await, "2");
    assert_eq!(socket.closed().await, CloseReason::PingTimeout);
    assert!(started.elapsed() >= Duration::from_millis(500), "not before interval plus timeout");
}

#[tokio::test]
async fn a_client_that_answers_pings_stays_connected() {
    let mut fx = start(EngineOptions::new().ping_interval(Duration::from_millis(200)).ping_timeout(Duration::from_millis(200))).await;
    let mut ws = ws_connect(fx.addr, None).await;
    let _ = next_text(&mut ws).await;
    let socket = fx.socket().await;
    for _ in 0..4 {
        assert_eq!(next_text(&mut ws).await, "2");
        send_text(&mut ws, "3").await;
    }
    assert!(socket.close_reason().is_none());
}

// ---------------------------------------------------------------- close and shutdown

#[tokio::test]
async fn client_close() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    assert_eq!(post(fx.addr, &sid, "1").await.status, 200);
    assert_eq!(socket.closed().await, CloseReason::TransportClose);
}

#[tokio::test]
async fn closing_a_socket_writes_what_is_queued_then_closes() {
    let mut fx = start(EngineOptions::new()).await;
    let (sid, _) = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    socket.send("bye").unwrap();
    socket.close();
    assert_eq!(poll(fx.addr, &sid).await.text(), "4bye\x1e1");
    assert_eq!(socket.closed().await, CloseReason::ForcedClose);
}

#[tokio::test]
async fn graceful_shutdown_with_a_pending_poll() {
    let app = CorvisFactory::create::<AppModule>().await.unwrap();
    let engine = EngineServer::new(EngineOptions::new()).unwrap();
    let (tx, mut sockets) = tokio::sync::mpsc::unbounded_channel();
    engine.on_connection(move |s| {
        let _ = tx.send(s);
    });
    engine.attach(&app.get_http_server()).unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (addr, serving) = app
        .listen_until_in_background("127.0.0.1:0", async move {
            let _ = stopped.await;
        })
        .await
        .unwrap();
    let (sid, _) = open_polling(addr).await;
    let socket = sockets.recv().await.unwrap();
    let pending = tokio::spawn(async move { poll(addr, &sid).await });
    eventually("the poll to wait", || corvis::socket_io::testing::poll_pending(&socket)).await;
    assert!(!pending.is_finished());
    stop.send(()).unwrap();
    let resp = tokio::time::timeout(Duration::from_secs(5), pending).await.expect("answered within 5 s").unwrap();
    assert_eq!(resp.text(), "1");
    assert_eq!(socket.closed().await, CloseReason::ForcedClose);
    tokio::time::timeout(Duration::from_secs(10), serving).await.expect("the server drains").unwrap().unwrap();
    assert_eq!(engine.session_count(), 0);
}

#[tokio::test]
async fn closing_the_server_refuses_new_handshakes_with_503() {
    let mut fx = start(EngineOptions::new()).await;
    let _ = open_polling(fx.addr).await;
    let socket = fx.socket().await;
    fx.engine.close();
    assert_eq!(socket.closed().await, CloseReason::ForcedClose);
    let resp = http(fx.addr, "GET", POLLING, &[], b"").await;
    assert_eq!(resp.status, 503);
}
