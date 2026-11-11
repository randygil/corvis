#![cfg(feature = "socket-io")]
//! The memory a client can make the Socket.IO layer hold, measured with a counting allocator.
//!
//! This binary installs a global allocator that counts the bytes live, and its tests take a lock so only one runs at
//! a time: the figures below are the heap of the process, not an estimate. They check that
//!
//! - decoding a packet never holds more than the connection reserved for it, and the decoded packet never holds more
//!   than it is charged (the cost model is an upper bound of real heap, for adversarial shapes too);
//! - one address, however many sessions it opens, holds a bounded amount while its handlers wait, and an honest
//!   address is still served while it does.
//!
//! The figures are the layout sizes the program requested; the model adds 16 bytes per allocation on top for the
//! allocator's own bookkeeping.

mod engine_support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::time::Duration;

use corvis::socket_io::testing::probe_decode;
use corvis::socket_io::{EngineOptions, IoOptions};
use engine_support::io_client::{Raw, start};
use tokio::sync::Notify;

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

fn grow(by: isize) {
    let now = LIVE.fetch_add(by, Ordering::SeqCst) + by;
    PEAK.fetch_max(now, Ordering::SeqCst);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grow(layout.size() as isize);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size() as isize, Ordering::SeqCst);
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grow(layout.size() as isize);
        }
        p
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            grow(new_size as isize - layout.size() as isize);
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// One test at a time: the counters are the process's.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn live() -> isize {
    LIVE.load(Ordering::SeqCst)
}

fn reset_peak() {
    PEAK.store(live(), Ordering::SeqCst);
}

const WIRE: usize = 990_000;

/// `shape` repeated, comma separated, up to about `wire` bytes.
fn repeated(shape: &str, wire: usize) -> String {
    vec![shape; wire / (shape.len() + 1)].join(",")
}

fn event(arg: &str) -> String {
    format!(r#"2["hold",{arg}]"#)
}

/// The shapes that cost the most per byte of text, and the ordinary ones.
fn shapes() -> Vec<(&'static str, String)> {
    let long_key = format!(r#"{{"{}":1}}"#, "k".repeat(500));
    let nested_object: String = format!("{}1{}", r#"{"a":"#.repeat(100), "}".repeat(100));
    // The worst shape per byte of text: every level is an object (a box, a table) holding one empty key.
    let nested_empty_key: String = format!("{}0{}", r#"{"":"#.repeat(100), "}".repeat(100));
    let nested_array: String = format!("{}{}", "[".repeat(100), "]".repeat(100));
    let many_keys = (0..60_000).map(|i| format!(r#""k{i}":{i}"#)).collect::<Vec<_>>().join(",");
    let repeated_key = repeated(r#""a":1"#, WIRE);
    let escapes = repeated(r#""éééé""#, WIRE);
    let user = r#"{"id":12345,"name":"Ada Lovelace","email":"ada@example.com","active":true,"score":98.5,"tags":["a","b"]}"#;
    vec![
        ("empty objects", event(&format!("[{}]", repeated("{}", WIRE)))),
        ("empty arrays", event(&format!("[{}]", repeated("[]", WIRE)))),
        ("empty strings", event(&format!(r#"[{}]"#, repeated(r#""""#, WIRE)))),
        ("zeros", event(&format!("[{}]", repeated("0", WIRE)))),
        ("nulls", event(&format!("[{}]", repeated("null", WIRE)))),
        ("floats", event(&format!("[{}]", repeated("-1.25e-7", WIRE)))),
        ("one-byte strings", event(&format!(r#"[{}]"#, repeated(r#""a""#, WIRE)))),
        ("one-key objects", event(&format!("[{}]", repeated(r#"{"a":1}"#, WIRE)))),
        ("one-element arrays", event(&format!("[{}]", repeated("[1]", WIRE)))),
        ("two-element arrays", event(&format!("[{}]", repeated("[1,2]", WIRE)))),
        ("nested one-key objects", event(&format!("[{}]", repeated(&nested_object, WIRE)))),
        ("nested empty-key objects", event(&format!("[{}]", repeated(&nested_empty_key, WIRE)))),
        ("nested arrays", event(&format!("[{}]", repeated(&nested_array, WIRE)))),
        ("long keys", event(&format!("[{}]", repeated(&long_key, WIRE)))),
        ("many short keys in one object", event(&format!("[{{{many_keys}}}]"))),
        ("one key repeated in one object", event(&format!("[{{{repeated_key}}}]"))),
        ("escaped strings", event(&format!("[{escapes}]"))),
        ("one long string", event(&format!(r#"["{}"]"#, "x".repeat(WIRE)))),
        ("a hundred arguments", event(&vec![format!("[{}]", repeated("{}", WIRE / 100)); 100].join(","))),
        ("ordinary objects", event(&format!("[{}]", repeated(user, WIRE)))),
        ("an acknowledgement", format!("31[[{}]]", repeated("{}", WIRE))),
        ("a binary header", format!(r#"51-["hold",{{"_placeholder":true,"num":0}},[{}]]"#, repeated("[]", WIRE / 2))),
    ]
}

#[test]
fn the_decoded_packet_holds_no_more_than_it_is_charged_and_decoding_no_more_than_it_reserved() {
    let _guard = ONE_AT_A_TIME.blocking_lock();
    println!("{:<34}{:>10}{:>14}{:>14}{:>14}{:>8}", "shape", "wire", "live heap", "charged", "reserved", "x wire");
    for (name, text) in shapes() {
        let wire = text.len();
        reset_peak();
        let before = live();
        let probe = probe_decode(&text, 1 << 30).unwrap_or_else(|e| panic!("{name}: {e}"));
        let after = live();
        let held = (after - before) as usize;
        // The probe copied `text` (the wire message, accounted apart from the reservation) and has dropped the copy.
        let peak = (PEAK.load(Ordering::SeqCst) - before) as usize;
        println!("{name:<34}{wire:>10}{held:>14}{:>14}{:>14}{:>8.1}", probe.cost, probe.reservation, probe.cost as f64 / wire as f64);
        assert!(held <= probe.cost, "{name}: the decoded packet holds {held} bytes but is charged {}", probe.cost);
        assert!(probe.cost <= probe.reservation, "{name}: charged {} above the reservation {}", probe.cost, probe.reservation);
        // The peak includes the wire text itself, which the connection charges apart from the reservation.
        assert!(peak <= probe.reservation + wire + 64, "{name}: decoding peaked at {peak} bytes, reserved {}", probe.reservation);
        drop(probe);
        assert!(live() - before < 4096, "{name}: dropping the packet frees what it held");
    }
}

#[test]
fn a_packet_above_the_budget_is_refused_before_anything_is_built() {
    let _guard = ONE_AT_A_TIME.blocking_lock();
    // The reviewer's packet: 330,000 empty objects in 0.99 MB. It costs about 45 MB decoded.
    let text = event(&format!("[{}]", repeated("{}", WIRE)));
    reset_peak();
    let before = live();
    let refused = probe_decode(&text, 16 * 1024 * 1024);
    assert!(refused.is_err());
    let peak = PEAK.load(Ordering::SeqCst) - before;
    // Only the first pass ran: one `u32` per container (330,000 of them) and nothing else.
    assert!(peak < 3 * 1024 * 1024, "refused after allocating {peak} bytes");
}

/// A `max_decoded_bytes` and server-wide budget small enough to test with, over the minimum the options allow.
fn options(total: usize, packet: usize) -> IoOptions {
    IoOptions::new().max_decoded_bytes(packet).max_attachments(1).engine(EngineOptions::new().max_total_unread_bytes(total))
}

/// Registers `hold`: counts the call, then keeps its arguments until `gate` opens.
fn holding(entered: &Arc<AtomicUsize>, gate: &Arc<Notify>) -> impl Fn(&corvis::socket_io::IoServer) + use<> {
    let (entered, gate) = (entered.clone(), gate.clone());
    move |io| {
        let (entered, gate) = (entered.clone(), gate.clone());
        io.on_connection(move |socket| {
            let (entered, gate) = (entered.clone(), gate.clone());
            socket.on("hold", move |_socket, args, _ack| {
                let (entered, gate) = (entered.clone(), gate.clone());
                async move {
                    entered.fetch_add(1, Ordering::SeqCst);
                    gate.notified().await;
                    drop(args);
                }
            });
        });
    }
}

async fn wait_for(what: &str, done: impl FnMut() -> bool) {
    engine_support::eventually(what, done).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_sessions_of_one_address_hold_a_bounded_amount_while_their_handlers_wait() {
    let _guard = ONE_AT_A_TIME.lock().await;
    // Server-wide budget 16 MiB (the least the options allow), packets up to 8 MiB.
    const TOTAL: usize = 16 * 1024 * 1024;
    const SESSIONS: usize = 12;
    let (entered, gate) = (Arc::new(AtomicUsize::new(0)), Arc::new(Notify::new()));
    let fx = start(options(TOTAL, 8 * 1024 * 1024), holding(&entered, &gate)).await;
    // 55,000 empty objects: 165 kB of text, about 7 MB decoded.
    let text = event(&format!("[{}]", repeated("{}", 165_000)));
    let wire = text.len();
    let mut clients = Vec::new();
    for _ in 0..SESSIONS {
        let mut client = Raw::connect(fx.addr).await;
        client.join("/", None).await;
        clients.push(client);
    }
    // The sessions' own buffers are not what is measured: the baseline is taken once they are open.
    let before = live();
    for client in &mut clients {
        client.send(&text).await;
    }
    wait_for("the first handler", || entered.load(Ordering::SeqCst) >= 1).await;
    // The others cannot have room: the share of an address is one maximum packet (9.4 MB here), the packet is bigger than half of it, so one packet per address
    // beyond the guaranteed shares. Give them time to (wrongly) get in: the absence of an admission over real sockets,
    // a quiet period, not a condition (a shorter one would make the check weaker, never wrong).
    tokio::time::sleep(Duration::from_millis(500)).await;
    let admitted = entered.load(Ordering::SeqCst);
    let held = (live() - before).max(0) as usize;
    // The invariant: the server-wide budget plus, per open session, the fair share and one message in the hand. The
    // test's own clients share the process: each holds the frame it wrote (allowed here as two copies of the text).
    let bound = TOTAL + SESSIONS * (16 * 1024 + wire + 1024) + SESSIONS * 2 * wire;
    println!("{SESSIONS} sessions x {wire} B wire: {admitted} admitted, {held} B live (bound {bound})");
    assert_eq!(admitted, 1, "one address holds at most its quota while the handler waits");
    assert!(held <= bound, "{held} bytes live above the bound {bound}");
    // Nobody is dropped or deadlocked: releasing the handler lets the others through, one after the other.
    gate.notify_waiters();
    // Each handler that starts waits for the gate: it is opened again at every check, until all have run.
    wait_for("every packet to be handled", || {
        gate.notify_waiters();
        entered.load(Ordering::SeqCst) == SESSIONS
    })
    .await;
    drop(clients);
    fx.io.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_honest_address_is_served_while_another_holds_its_maximum() {
    let _guard = ONE_AT_A_TIME.lock().await;
    const TOTAL: usize = 64 * 1024 * 1024;
    let (entered, gate) = (Arc::new(AtomicUsize::new(0)), Arc::new(Notify::new()));
    let honest_seen = Arc::new(AtomicUsize::new(0));
    let seen = honest_seen.clone();
    let hold = holding(&entered, &gate);
    let fx = start(options(TOTAL, 16 * 1024 * 1024), move |io| {
        hold(io);
        let seen = seen.clone();
        io.of("/honest").unwrap().on_connection(move |socket| {
            let seen = seen.clone();
            socket.on("work", move |_socket, _args, _ack| {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                }
            });
        });
    })
    .await;
    // The attacker: sessions from 127.0.0.1, each with a packet of about 7 MB decoded, handlers that never finish.
    let big = event(&format!("[{}]", repeated("{}", 165_000)));
    let mut attackers = Vec::new();
    for _ in 0..8 {
        let mut client = Raw::connect(fx.addr).await;
        client.join("/", None).await;
        client.send(&big).await;
        attackers.push(client);
    }
    wait_for("an attacker handler", || entered.load(Ordering::SeqCst) >= 1).await;
    // The absence of a third admission over real sockets: a quiet period, not a condition (a shorter one weakens the check).
    tokio::time::sleep(Duration::from_millis(300)).await;
    // The address holds its quota (16 MiB: two of these packets) and no more.
    assert!(entered.load(Ordering::SeqCst) <= 2, "the attacking address holds {} packets", entered.load(Ordering::SeqCst));
    // The honest address: another source address, packets that need room in the server-wide budget (1,500 empty
    // objects: 4.5 kB of text, 190 kB decoded, far above the guaranteed share).
    let mut honest = Raw::connect_from(fx.addr, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2))).await;
    honest.join("/honest", None).await;
    let work = format!(r#"2/honest,["work",[{}]]"#, repeated("{}", 4_500));
    for _ in 0..5 {
        honest.send(&work).await;
    }
    wait_for("the honest address to be served", || honest_seen.load(Ordering::SeqCst) == 5).await;
    gate.notify_waiters();
    drop((attackers, honest));
    fx.io.close();
}

/// A cookie of `len` bytes.
fn cookie(len: usize) -> String {
    format!("c={}", "x".repeat(len - 2))
}

/// Joins namespace `nsp` on a raw WebSocket client and returns the socket id.
async fn join_ws(ws: &mut engine_support::WsClient, nsp: &str) -> String {
    engine_support::send_text(ws, &format!("40{nsp},")).await;
    loop {
        let text = engine_support::next_text(ws).await;
        if text.starts_with(&format!("40{nsp},")) {
            let json: serde_json::Value = serde_json::from_str(&text[text.find('{').unwrap()..]).unwrap();
            return json["sid"].as_str().unwrap().to_string();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_sockets_of_one_connection_share_its_handshake_request() {
    let _guard = ONE_AT_A_TIME.lock().await;
    const COOKIE: usize = 16 * 1024;
    let fx = start(IoOptions::new(), |io| {
        for n in 0..10 {
            io.of(&format!("/n{n}")).unwrap();
        }
    })
    .await;
    let url = format!("ws://{}/socket.io/?EIO=4&transport=websocket", fx.addr);
    let mut request = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
    request.headers_mut().insert("cookie", cookie(COOKIE).parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.expect("WebSocket handshake");
    let _open = engine_support::next_text(&mut ws).await;
    let mut ids = vec![join_ws(&mut ws, "/n0").await];
    let mut growth = Vec::new();
    for n in 1..10 {
        let before = live();
        ids.push(join_ws(&mut ws, &format!("/n{n}")).await);
        growth.push(live() - before);
    }
    println!("heap growth per extra socket with a {COOKIE}-byte cookie: {growth:?}");
    for (n, grew) in growth.iter().enumerate() {
        assert!(*grew < COOKIE as isize, "socket {} grew the heap by {grew} bytes", n + 1);
    }
    let sockets: Vec<_> =
        ids.iter().enumerate().map(|(n, id)| fx.io.of(&format!("/n{n}")).unwrap().socket(id).expect("connected")).collect();
    for socket in &sockets[1..] {
        assert!(std::ptr::eq(socket.handshake().request(), sockets[0].handshake().request()), "a socket holds a request of its own");
    }
    assert_eq!(sockets[0].handshake().headers().get("cookie").map(|v| v.len()), Some(COOKIE));
    drop(sockets);
    fx.io.close();
}

/// One request on a kept-alive connection, written as prepared bytes; returns the status line.
async fn exchange(stream: &mut tokio::net::TcpStream, request: &[u8]) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream.write_all(request).await.unwrap();
    let mut response = Vec::with_capacity(4096);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        assert!(n > 0, "the server closed the connection");
        response.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&response);
        if let Some(end) = text.find("\r\n\r\n") {
            let length: usize = text[..end]
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                .unwrap_or(0);
            if response.len() >= end + 4 + length {
                return text.lines().next().unwrap().to_string();
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_post_of_an_existing_session_copies_nothing_of_its_request() {
    let _guard = ONE_AT_A_TIME.lock().await;
    const COOKIE: usize = 16 * 1024;
    let fx = start(IoOptions::new(), |_| {}).await;
    let (sid, _) = engine_support::open_polling(fx.addr).await;
    let post = |target: &str| {
        format!(
            "POST {target} HTTP/1.1\r\nHost: x\r\nCookie: {}\r\nContent-Type: text/plain;charset=UTF-8\r\nContent-Length: 1\r\n\r\n6",
            cookie(COOKIE)
        )
        .into_bytes()
    };
    // The peak of one request on a kept-alive connection that already served one like it (its read buffer is grown).
    let peak_of = async |request: &[u8], status: &str| {
        let mut stream = tokio::net::TcpStream::connect(fx.addr).await.unwrap();
        assert!(exchange(&mut stream, request).await.contains(status));
        let before = live();
        reset_peak();
        assert!(exchange(&mut stream, request).await.contains(status));
        PEAK.load(Ordering::SeqCst) - before
    };
    // What the HTTP server itself allocates for these bytes (reading, parsing, answering a route that does not exist),
    // then what the engine adds to it for a `POST` of a session.
    let server = peak_of(&post("/nothing-here"), "404").await;
    let engine = peak_of(&post(&engine_support::poll_url(&sid)), "200").await;
    println!("a request with a {COOKIE}-byte cookie: {server} bytes at its peak for the HTTP server, {engine} as a POST of a session");
    assert!(engine - server < COOKIE as isize, "the POST allocated {} bytes more than the HTTP server needs for it", engine - server);
    fx.io.close();
}
