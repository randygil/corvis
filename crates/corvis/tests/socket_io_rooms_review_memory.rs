#![cfg(feature = "socket-io")]
//! Memory regressions of the first security review of rooms and broadcast (#34), measured with a counting allocator
//! (the heap of the process; one test at a time).
//!
//! - The answers a broadcast with acknowledgements collects count against the connections that sent them, so a
//!   handler that starts many such calls cannot make the server hold more than the answering addresses' quotas and
//!   the server-wide budget.
//! - `fetch_sockets` shares the handshake of local sockets: what it allocates per socket does not depend on how big
//!   the handshake is.
//! - Chaining `to` calls costs what each call adds, not what the chain already holds.

mod engine_support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::time::Duration;

use corvis::socket_io::IoOptions;
use corvis::socket_io::engine::EngineOptions;
use engine_support::io_client::{Msg, Raw, start};

struct Counting;

/// Bytes live now.
static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);
/// Bytes requested since the start (freed ones included).
static REQUESTED: AtomicUsize = AtomicUsize::new(0);

fn grow(by: isize) {
    if by > 0 {
        REQUESTED.fetch_add(by as usize, Ordering::SeqCst);
    }
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

static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn live() -> isize {
    LIVE.load(Ordering::SeqCst)
}

fn requested() -> usize {
    REQUESTED.load(Ordering::SeqCst)
}

async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ------------------------------------------------------------------------------------------------------------ F2

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn answers_held_by_pending_broadcasts_stay_within_the_answer_budget() {
    let _guard = ONE_AT_A_TIME.lock().await;
    // The least server-wide budget the options allow, and an address share of one maximum packet.
    const TOTAL: usize = 16 * 1024 * 1024;
    const PACKET: usize = 2 * 1024 * 1024;
    const ANSWER: usize = 300_000;
    const ANSWERERS: usize = 6;
    const CALLS: usize = 32;
    let options = IoOptions::new().max_decoded_bytes(PACKET).max_attachments(1).engine(EngineOptions::new().max_total_unread_bytes(TOTAL));
    let fx = start(options, |io| {
        io.on_connection(|socket| {
            socket.join("room").unwrap();
            // The reviewer's handler: each `ask` makes a call that waits for every member, and one member is silent.
            socket.on("ask", |socket, _args, _ack| async move {
                let _ = socket.to("room").timeout(Duration::from_secs(6)).emit_with_ack("q", ()).await;
            });
        });
    })
    .await;
    let sent = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..ANSWERERS {
        let mut client = Raw::connect(fx.addr).await;
        client.join("/", None).await;
        let (sent, text) = (sent.clone(), "x".repeat(ANSWER));
        tasks.push(tokio::spawn(async move {
            // Answers every call it is asked, until the connection ends.
            while let Some(Msg::Text(t)) = client.recv_within(Duration::from_secs(20)).await {
                let Some(rest) = t.strip_prefix('2') else { continue };
                let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
                if id.is_empty() || !rest[id.len()..].starts_with("[\"q\"") {
                    continue;
                }
                client.send(&format!("3{id}[\"{text}\"]")).await;
                sent.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }
    let mut silent = Raw::connect(fx.addr).await;
    silent.join("/", None).await;
    let mut trigger = Raw::connect(fx.addr).await;
    trigger.join("/", None).await;
    let before = live();
    for _ in 0..CALLS {
        trigger.send(r#"2["ask"]"#).await;
    }
    wait_for("answers to flow", || sent.load(Ordering::SeqCst) >= 8).await;
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let held = (live() - before).max(0) as usize;
    // Held while the calls wait: at most the server-wide answer budget (a quarter of the unread budget, but never less
    // than one call may hold: 16 MiB here, the default of `max_broadcast_ack_bytes`), and per session the message being
    // read and the one parked; and the test's own clients, which keep each answer and build it again to send it.
    // Without a bound (measured): every answer of every call, 64 MB.
    const ANSWER_BUDGET: usize = 16 * 1024 * 1024;
    let bound = ANSWER_BUDGET + 9 * (16 * 1024 + 3 * ANSWER) + ANSWERERS * 4 * ANSWER;
    println!(
        "held by {CALLS} pending calls: {held} bytes after {} answers (bound {bound}; unbounded would be {})",
        sent.load(Ordering::SeqCst),
        CALLS * ANSWERERS * ANSWER
    );
    assert!(held <= bound, "{held} bytes held above the bound {bound}");
    // The calls end at their deadline and everything is given back.
    for task in tasks {
        task.abort();
    }
    drop((silent, trigger));
    tokio::time::sleep(Duration::from_secs(1)).await;
    wait_for("the server to give the answers back", || (live() - before) < 2 * 1024 * 1024).await;
    fx.io.close();
}

// ------------------------------------------------------------------------------------------------------------ B3

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fetch_sockets_allocates_a_bounded_amount_per_socket_whatever_the_handshake_holds() {
    let _guard = ONE_AT_A_TIME.lock().await;
    const SOCKETS: usize = 200;
    const AUTH: usize = 14 * 1024;
    const ROOMS: usize = 20;
    let fx = start(IoOptions::new(), |io| {
        io.on_connection(|socket| {
            socket.join((0..ROOMS).map(|n| format!("{}-{n}", "r".repeat(200))).collect::<Vec<_>>()).unwrap();
        });
    })
    .await;
    let mut clients = Vec::new();
    for n in 0..SOCKETS {
        // Many addresses: one address may hold 256 sessions, and this keeps the test far from it.
        let mut client = Raw::connect_from(fx.addr, IpAddr::V4(Ipv4Addr::new(127, 0, 1 + (n / 100) as u8, 1 + (n % 100) as u8))).await;
        client.join("/", Some(&format!(r#"{{"token":"{}"}}"#, "a".repeat(AUTH)))).await;
        clients.push(client);
    }
    wait_for("every socket", || fx.io.main().socket_count() == SOCKETS).await;
    let before = requested();
    let fetched = fx.io.fetch_sockets().await.unwrap();
    let allocated = requested() - before;
    assert_eq!(fetched.len(), SOCKETS);
    let per_socket = allocated / SOCKETS;
    // Per socket: its id, its info, a reference per room, the entry of the result: well under 3 KiB, where a copy
    // of the handshake alone is 14 KiB and a copy of the room names 4 KiB.
    println!("fetch_sockets of {SOCKETS} sockets with {AUTH} B of auth: {per_socket} B allocated per socket");
    assert!(per_socket < 3 * 1024, "{per_socket} bytes allocated per socket");
    // What is held is shared too: the 14 KiB of auth of each socket is not duplicated.
    assert_eq!(fetched[0].handshake().auth()["token"].as_str().map(str::len), Some(AUTH));
    drop(fetched);
    drop(clients);
    fx.io.close();
}

// ------------------------------------------------------------------------------------------------------ chaining

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chaining_to_calls_costs_what_each_call_adds() {
    let _guard = ONE_AT_A_TIME.lock().await;
    const CALLS: usize = 5_000;
    let fx = start(IoOptions::new().max_rooms_per_socket(CALLS + 1), |_| {}).await;
    let names: Vec<String> = (0..CALLS).map(|n| format!("room-{n}")).collect();
    let before = requested();
    let mut operator = fx.io.to("start");
    for name in &names {
        operator = operator.to(name.as_str());
    }
    fx.io.sockets_join("x").await.unwrap();
    operator.emit("e", ()).await.unwrap();
    let allocated = requested() - before;
    println!("{CALLS} chained calls, then an emit: {allocated} bytes allocated");
    // Linear: well under 1 KiB a call. Cloning two sets at every call would be hundreds of megabytes.
    assert!(allocated < CALLS * 1024, "{allocated} bytes allocated by {CALLS} chained calls");
    fx.io.close();
}
