//! End-to-end tests: a real server per test, real WebSocket clients (tokio-tungstenite) and
//! the in-memory HTTP client of `corvis::testing` against the same application.

use std::net::SocketAddr;
use std::sync::Once;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use corvis::futures::{SinkExt, StreamExt};
use corvis::prelude::*;
use corvis::testing::{TestClient, request};
use corvis::tokio;
use corvis::tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{self, Message};

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

static ENV: Once = Once::new();
/// Test servers bind to 127.0.0.1:5170-5199.
static NEXT_PORT: AtomicU16 = AtomicU16::new(5170);

struct TestApp {
    app: CorvisApplication,
    addr: SocketAddr,
}

async fn start() -> TestApp {
    ENV.call_once(|| {
        // SAFETY: set once, before any application of this test binary reads the environment.
        unsafe {
            std::env::set_var("DATABASE_URL", "sqlite::memory:");
            std::env::set_var("STATS_INTERVAL_MS", "100");
        }
    });
    let mut app = tertulia::create_app().await.unwrap();
    loop {
        let port = NEXT_PORT.fetch_add(1, Ordering::SeqCst);
        assert!(port < 5200, "no free port left in 5170-5199");
        if let Ok(addr) = app.listen_in_background(format!("127.0.0.1:{port}")).await {
            return TestApp { app, addr };
        }
    }
}

impl TestApp {
    fn http(&self) -> TestClient {
        request(&self.app)
    }

    async fn token(&self, username: &str) -> String {
        let res = self.http().post("/api/auth/login").json(&json!({ "username": username })).send().await;
        assert_eq!(res.status(), 200, "{}", res.text());
        res.json_value()["accessToken"].as_str().unwrap().to_string()
    }

    async fn raw_socket(&self, query: &str) -> Ws {
        tokio_tungstenite::connect_async(format!("ws://{}/chat{query}", self.addr)).await.unwrap().0
    }

    /// Logs in, connects and consumes the `welcome` frame.
    async fn connect(&self, username: &str) -> Client {
        let token = self.token(username).await;
        let ws = self.raw_socket(&format!("?token={token}")).await;
        let mut client = Client { ws, next_id: 0 };
        let welcome = client.expect("welcome").await;
        assert_eq!(welcome["data"]["user"]["username"], username);
        client
    }
}

struct Client {
    ws: Ws,
    next_id: u64,
}

impl Client {
    async fn send(&mut self, event: &str, data: Value) {
        self.ws.send(Message::Text(json!({ "event": event, "data": data }).to_string())).await.unwrap();
    }

    async fn send_with_id(&mut self, event: &str, data: Value, id: Value) {
        self.ws.send(Message::Text(json!({ "event": event, "data": data, "id": id }).to_string())).await.unwrap();
    }

    async fn next(&mut self) -> Value {
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(5), self.ws.next())
                .await
                .expect("timed out waiting for a frame")
                .expect("socket closed")
                .unwrap();
            if let Message::Text(t) = msg {
                return serde_json::from_str(&t).unwrap();
            }
        }
    }

    /// The next frame, which must be `event`.
    async fn expect(&mut self, event: &str) -> Value {
        let frame = self.next().await;
        assert_eq!(frame["event"], event, "unexpected frame: {frame}");
        frame
    }

    /// Sends `event` and returns its reply or `exception` frame (matched through the `id`).
    async fn ask(&mut self, event: &str, data: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send_with_id(event, data, json!(id)).await;
        let frame = self.next().await;
        assert_eq!(frame["id"], id, "unexpected frame: {frame}");
        frame
    }

    async fn join(&mut self, room: &str) -> Value {
        let frame = self.ask("join", json!({ "room": room })).await;
        assert_eq!(frame["event"], "joined", "{frame}");
        frame["data"].clone()
    }

    /// Nothing else is queued for this client: the next frame is the answer to a `ping`.
    async fn assert_idle(&mut self) {
        let pong = self.ask("ping", Value::Null).await;
        assert_eq!(pong["event"], "ping", "expected no pending frames, got {pong}");
        assert_eq!(pong["data"], "pong");
    }
}

async fn next_text(ws: &mut Ws) -> String {
    match tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("timed out").expect("socket closed").unwrap() {
        Message::Text(t) => t,
        other => panic!("expected a text frame, got {other:?}"),
    }
}

fn usernames(members: &Value) -> Vec<String> {
    members.as_array().unwrap().iter().map(|m| m["username"].as_str().unwrap().to_string()).collect()
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

#[corvis::test]
async fn sockets_without_a_valid_token_are_rejected() {
    let t = start().await;
    for query in ["", "?token=not-a-jwt"] {
        let mut ws = t.raw_socket(query).await;
        let exception: Value = serde_json::from_str(&next_text(&mut ws).await).unwrap();
        assert_eq!(exception, json!({ "event": "exception", "data": { "status": "error", "message": "Unauthorized" } }));
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap().unwrap().unwrap() {
            Message::Close(Some(f)) => {
                assert_eq!(u16::from(f.code), tertulia::chat::CLOSE_UNAUTHORIZED);
                assert_eq!(f.reason, "Unauthorized");
            }
            other => panic!("expected a close frame, got {other:?}"),
        }
    }
    t.http().get("/api/auth/me").send().await.expect_status(401);
}

#[corvis::test]
async fn joining_returns_members_and_history_and_notifies_the_room() {
    let t = start().await;
    let mut ana = t.connect("ana").await;
    let mut bob = t.connect("bob").await;

    let joined = ana.join("general").await;
    assert_eq!(joined["room"]["name"], "general");
    assert_eq!(usernames(&joined["members"]), ["ana"]);
    assert_eq!(joined["history"], json!([]));

    let joined = bob.join("general").await;
    assert_eq!(usernames(&joined["members"]), ["ana", "bob"]);
    let notice = ana.expect("user_joined").await;
    assert_eq!(notice["data"]["user"]["username"], "bob");
    assert_eq!(usernames(&notice["data"]["members"]), ["ana", "bob"]);
    // Joining twice is idempotent and does not notify again.
    ana.join("general").await;
    bob.assert_idle().await;
}

#[corvis::test]
async fn messages_reach_the_room_but_not_the_sender_who_gets_an_ack() {
    let t = start().await;
    let mut ana = t.connect("ana").await;
    let mut bob = t.connect("bob").await;
    let mut carl = t.connect("carl").await;
    ana.join("general").await;
    bob.join("general").await;
    ana.expect("user_joined").await;
    carl.join("rust").await;

    ana.send_with_id("message", json!({ "room": "general", "text": "  hola a todos  " }), json!("m-1")).await;
    let ack = ana.expect("sent").await;
    assert_eq!(ack["id"], "m-1");
    assert_eq!(ack["data"]["text"], "hola a todos");
    assert_eq!(ack["data"]["username"], "ana");

    let msg = bob.expect("message").await;
    assert_eq!(msg["data"], ack["data"]);
    assert!(msg.get("id").is_none(), "broadcasts carry no ack id: {msg}");
    ana.assert_idle().await;
    carl.assert_idle().await;
}

#[corvis::test]
async fn invalid_payloads_answer_exception_frames() {
    let t = start().await;
    let mut ana = t.connect("ana").await;

    let err = ana.ask("join", json!({ "room": "Not Valid!" })).await;
    assert_eq!(err["event"], "exception");
    assert_eq!(err["data"]["message"], "Validation failed");
    assert_eq!(err["data"]["statusCode"], 400);
    assert_eq!(err["data"]["errors"][0]["code"], "invalid_format");
    assert_eq!(err["data"]["errors"][0]["path"], json!(["room"]));

    let err = ana.ask("join", json!({ "room": "general", "admin": true })).await;
    assert_eq!(err["data"]["errors"][0]["code"], "unrecognized_keys");

    let err = ana.ask("join", json!({ "room": "nope" })).await;
    assert_eq!(err["data"], json!({ "status": "error", "code": "ROOM_NOT_FOUND", "message": "Room #nope does not exist", "room": "nope" }));

    ana.join("general").await;
    let err = ana.ask("message", json!({ "room": "general", "text": "   " })).await;
    assert_eq!(err["data"]["errors"][0]["code"], "too_small");
    assert_eq!(err["data"]["errors"][0]["path"], json!(["text"]));
    let err = ana.ask("typing", json!({ "room": "general", "typing": "yes" })).await;
    assert_eq!(err["data"]["errors"][0]["code"], "invalid_type");

    // Unknown events and non-JSON frames are ignored, like the JSON WebSocket adapter of Node.js frameworks.
    ana.send("dance", json!({})).await;
    ana.ws.send(Message::Text("not json".into())).await.unwrap();
    ana.assert_idle().await;
}

#[corvis::test]
async fn guards_protect_rooms_and_moderator_actions() {
    let t = start().await;
    let mut ana = t.connect("ana").await;
    let mut admin = t.connect("admin").await;
    ana.join("general").await;

    let err = ana.ask("message", json!({ "room": "random", "text": "hi" })).await;
    assert_eq!(err["data"], json!({ "status": "error", "code": "NOT_IN_ROOM", "message": "Join #random first", "room": "random" }));

    ana.ask("message", json!({ "room": "general", "text": "first" })).await;
    let err = ana.ask("clear", json!({ "room": "general" })).await;
    assert_eq!(err["data"], json!({ "status": "error", "message": "Forbidden resource" }));

    admin.join("general").await;
    ana.expect("user_joined").await;
    admin.send("clear", json!({ "room": "general" })).await;
    let cleared = admin.expect("cleared").await;
    assert_eq!(cleared["data"], json!({ "room": "general", "by": "admin", "deleted": 1 }));
    assert_eq!(ana.expect("cleared").await["data"], cleared["data"]);
    let history = t.http().get("/api/rooms/general/messages").send().await;
    assert_eq!(history.json_value(), json!([]));
}

#[corvis::test]
async fn typing_indicators_go_to_the_others_only() {
    let t = start().await;
    let mut ana = t.connect("ana").await;
    let mut bob = t.connect("bob").await;
    ana.join("general").await;
    bob.join("general").await;
    ana.expect("user_joined").await;

    ana.send("typing", json!({ "room": "general", "typing": true })).await;
    assert_eq!(bob.expect("typing").await["data"], json!({ "room": "general", "user": "ana", "typing": true }));
    ana.send("typing", json!({ "room": "general", "typing": false })).await;
    assert_eq!(bob.expect("typing").await["data"]["typing"], false);
    ana.assert_idle().await;
}

#[corvis::test]
async fn leaving_and_disconnecting_clean_up_presence() {
    let t = start().await;
    let mut ana = t.connect("ana").await;
    let mut bob = t.connect("bob").await;
    let mut carl = t.connect("carl").await;
    for c in [&mut ana, &mut bob, &mut carl] {
        c.join("general").await;
    }
    ana.expect("user_joined").await;
    ana.expect("user_joined").await;
    bob.expect("user_joined").await;
    let members = t.http().get("/api/rooms/general/members").send().await.json_value();
    assert_eq!(usernames(&members), ["ana", "bob", "carl"]);

    // `leave`: the others are told, the leaver can no longer talk there.
    assert_eq!(carl.ask("leave", json!({ "room": "general" })).await["event"], "left");
    for c in [&mut ana, &mut bob] {
        let left = c.expect("user_left").await;
        assert_eq!(left["data"]["user"]["username"], "carl");
        assert_eq!(usernames(&left["data"]["members"]), ["ana", "bob"]);
    }
    assert_eq!(carl.ask("message", json!({ "room": "general", "text": "hi?" })).await["data"]["code"], "NOT_IN_ROOM");

    // Disconnect: `handle_disconnect` tells every room the socket was in.
    bob.ws.close(None).await.unwrap();
    let left = ana.expect("user_left").await;
    assert_eq!(left["data"]["user"]["username"], "bob");
    assert_eq!(usernames(&left["data"]["members"]), ["ana"]);
    let members = t.http().get("/api/rooms/general/members").send().await.json_value();
    assert_eq!(usernames(&members), ["ana"]);
    let stats = t.http().get("/api/stats").send().await.json_value();
    assert_eq!(stats["connections"], 2);
    assert_eq!(stats["onlineUsers"], json!(["ana", "carl"]));
}

#[corvis::test]
async fn rest_announcements_are_pushed_over_websocket() {
    let t = start().await;
    let mut ana = t.connect("ana").await;
    let mut bob = t.connect("bob").await;
    ana.join("general").await;
    bob.join("general").await;
    ana.expect("user_joined").await;
    let admin = t.token("admin").await;
    let member = t.token("dora").await;

    let res = t
        .http()
        .post("/api/rooms/general/announcements")
        .header("authorization", &bearer(&admin))
        .json(&json!({ "text": "Pizza at 7!" }))
        .send()
        .await;
    assert_eq!(res.status(), 201, "{}", res.text());
    assert_eq!(res.json_value(), json!({ "room": "general", "delivered": 2 }));
    for c in [&mut ana, &mut bob] {
        assert_eq!(c.expect("announcement").await["data"], json!({ "room": "general", "from": "admin", "text": "Pizza at 7!" }));
    }

    let post = |token: Option<&str>, room: &str| {
        let mut req = t.http().post(&format!("/api/rooms/{room}/announcements")).json(&json!({ "text": "x" }));
        if let Some(tk) = token {
            req = req.header("authorization", &bearer(tk));
        }
        req.send()
    };
    assert_eq!(post(Some(&member), "general").await.status(), 403);
    assert_eq!(post(None, "general").await.status(), 401);
    assert_eq!(post(Some(&admin), "nope").await.status(), 404);
    ana.assert_idle().await;
}

#[corvis::test]
async fn rooms_created_over_rest_are_broadcast_and_capacity_is_enforced() {
    let t = start().await;
    let mut ana = t.connect("ana").await;
    let mut bob = t.connect("bob").await;
    let token = t.token("ana").await;

    let res = t
        .http()
        .post("/api/rooms")
        .header("authorization", &bearer(&token))
        .json(&json!({ "name": "tiny", "topic": "Room for one", "maxMembers": 1 }))
        .send()
        .await;
    assert_eq!(res.status(), 201, "{}", res.text());
    assert_eq!(res.json_value()["createdBy"], "ana");
    // `RoomsService` emits `room.created`; the gateway's `#[on_event]` pushes it to everyone.
    for c in [&mut ana, &mut bob] {
        let created = c.expect("room_created").await;
        assert_eq!(created["data"]["room"]["name"], "tiny");
        assert_eq!(created["data"]["room"]["maxMembers"], 1);
    }

    ana.join("tiny").await;
    let err = bob.ask("join", json!({ "room": "tiny" })).await;
    assert_eq!(err["data"], json!({ "status": "error", "code": "ROOM_FULL", "message": "#tiny is full", "room": "tiny", "max": 1 }));

    let dup = t.http().post("/api/rooms").header("authorization", &bearer(&token)).json(&json!({ "name": "tiny" })).send().await;
    assert_eq!(dup.status(), 409);
    let invalid = t.http().post("/api/rooms").header("authorization", &bearer(&token)).json(&json!({ "name": "No Spaces" })).send().await;
    assert_eq!(invalid.status(), 400);
    let rooms = t.http().get("/api/rooms").send().await.json_value();
    let tiny = rooms.as_array().unwrap().iter().find(|r| r["name"] == "tiny").unwrap();
    assert_eq!(usernames(&tiny["members"]), ["ana"]);
}

#[corvis::test]
async fn history_is_persisted_and_served_over_rest_and_on_join() {
    let t = start().await;
    let mut ana = t.connect("ana").await;
    ana.join("rust").await;
    for text in ["one", "two", "three"] {
        assert_eq!(ana.ask("message", json!({ "room": "rust", "text": text })).await["event"], "sent");
    }
    let last_two = t.http().get("/api/rooms/rust/messages?limit=2").send().await.json_value();
    let texts: Vec<&str> = last_two.as_array().unwrap().iter().map(|m| m["text"].as_str().unwrap()).collect();
    assert_eq!(texts, ["two", "three"]);
    t.http().get("/api/rooms/nope/messages").send().await.expect_status(404);

    let mut bob = t.connect("bob").await;
    let joined = bob.join("rust").await;
    let texts: Vec<&str> = joined["history"].as_array().unwrap().iter().map(|m| m["text"].as_str().unwrap()).collect();
    assert_eq!(texts, ["one", "two", "three"]);
}

#[corvis::test]
async fn acknowledgements_match_pipelined_requests() {
    let t = start().await;
    let mut ana = t.connect("ana").await;
    ana.join("general").await;
    // Handlers start in frame order but run concurrently (an async handler may finish after
    // the next one): the `id` tells which request each reply answers.
    let mut expected = std::collections::HashMap::new();
    for i in 0..10 {
        let (event, data, reply) = match i % 3 {
            0 => ("message", json!({ "room": "general", "text": format!("t{i}") }), "sent"),
            1 => ("ping", Value::Null, "ping"),
            _ => ("message", json!({ "room": "nowhere", "text": "x" }), "exception"),
        };
        let id = format!("req-{i}");
        ana.ws.feed(Message::Text(json!({ "event": event, "data": data, "id": id }).to_string())).await.unwrap();
        expected.insert(id, (reply, i));
    }
    ana.ws.flush().await.unwrap();
    for _ in 0..10 {
        let frame = ana.next().await;
        let (reply, i) = expected.remove(frame["id"].as_str().unwrap()).expect("unknown or repeated id");
        assert_eq!(frame["event"], reply, "{frame}");
        if reply == "sent" {
            assert_eq!(frame["data"]["text"], format!("t{i}"));
        }
    }
    assert!(expected.is_empty());
    ana.assert_idle().await;
}

#[corvis::test]
async fn the_firehose_is_a_plain_websocket_for_moderators() {
    let t = start().await;
    let url = |token: &str| format!("ws://{}/ws/firehose?token={token}", t.addr);
    let member = t.token("ana").await;
    for token in ["", member.as_str()] {
        match tokio_tungstenite::connect_async(url(token)).await {
            Err(tungstenite::Error::Http(res)) => assert_eq!(res.status(), 401),
            other => panic!("expected a 401 handshake, got {:?}", other.map(|_| ())),
        }
    }

    let admin = t.token("admin").await;
    let (mut hose, _) = tokio_tungstenite::connect_async(url(&admin)).await.unwrap();
    assert_eq!(serde_json::from_str::<Value>(&next_text(&mut hose).await).unwrap(), json!({ "type": "hello", "user": "admin" }));

    let mut ana = t.connect("ana").await;
    ana.join("general").await;
    ana.ask("message", json!({ "room": "general", "text": "is anyone listening?" })).await;
    let event: Value = serde_json::from_str(&next_text(&mut hose).await).unwrap();
    assert_eq!(event["type"], "message");
    assert_eq!(event["message"]["text"], "is anyone listening?");

    // No `{ event, data }` envelope here: raw text both ways.
    hose.send(Message::Text("ping".into())).await.unwrap();
    assert_eq!(next_text(&mut hose).await, "pong");
}

#[corvis::test]
async fn stats_are_streamed_with_server_sent_events() {
    let t = start().await;
    let mut ana = t.connect("ana").await;
    ana.join("general").await;

    let mut tcp = tokio::net::TcpStream::connect(t.addr).await.unwrap();
    tcp.write_all(b"GET /api/stats/stream HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n").await.unwrap();
    let mut raw = String::new();
    let mut buf = [0u8; 4096];
    while raw.matches("event: stats").count() < 2 {
        let n = tokio::time::timeout(Duration::from_secs(5), tcp.read(&mut buf)).await.unwrap().unwrap();
        assert!(n > 0, "stream closed: {raw}");
        raw.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
    assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
    assert!(raw.to_ascii_lowercase().contains("content-type: text/event-stream"), "{raw}");
    let data = raw.lines().find_map(|l| l.strip_prefix("data: ")).unwrap();
    let stats: Value = serde_json::from_str(data).unwrap();
    assert_eq!(stats["connections"], 1);
    assert_eq!(stats["onlineUsers"], json!(["ana"]));
    let general = stats["rooms"].as_array().unwrap().iter().find(|r| r["name"] == "general").unwrap();
    assert_eq!(general["members"], 1);
}

#[corvis::test]
async fn the_browser_client_is_served() {
    let t = start().await;
    let page = t.http().get("/").send().await;
    assert_eq!(page.status(), 200);
    assert!(page.header("content-type").unwrap().starts_with("text/html"));
    assert!(page.text().contains("Tertulia"));
    t.http().get("/app.js").send().await.expect_status(200);
    t.http().get("/styles.css").send().await.expect_status(200);
}
