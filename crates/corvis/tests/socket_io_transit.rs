#![cfg(feature = "socket-io")]
//! What the WebSocket transport keeps per session outside the unread budget, measured with a counting allocator.
//!
//! Two things sit in the WebSocket library (tungstenite 0.24) and are charged to nobody: its read buffer, which grows to
//! hold the largest frame it ever received and is never shrunk, and the buffer of a fragmented message that is not
//! complete yet. These tests pin both with the formulas of the design (`design.md`, "Security review (round 3)"), per
//! session and at the default `max_http_buffer_size`, so a change of the library (or of how the transport uses it) that
//! makes them worse fails here, and the numbers that the limits pages state are the numbers measured.
//!
//! Raw clients write from the caller's buffers (no copy), so what the allocator counts is the server's memory.

mod engine_support;
mod socket_io_raw;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Duration;

use corvis::socket_io::{EngineOptions, IoOptions};
use engine_support::io_client::start;
use socket_io_raw::{Raw, Tx, frame_header, ip};

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        p
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_add(new_size as isize - layout.size() as isize, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// One test at a time: the counter is the process's.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn live() -> isize {
    LIVE.load(Ordering::Relaxed)
}

/// Waits until the live heap stops moving (three samples within 32 KiB of each other), and returns it. The heap of a
/// server on real sockets, with tasks that free on their own schedule, has no condition that says it is done: this is a
/// quiet period, used to read the worst transient and the idle retention, not to order anything.
async fn settled() -> isize {
    let mut last = live();
    let mut calm = 0;
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let now = live();
        if (now - last).abs() < 32 * 1024 {
            calm += 1;
            if calm == 3 {
                return now;
            }
        } else {
            calm = 0;
        }
        last = now;
    }
    live()
}

/// The size the read buffer of the library reaches for a frame of `frame` bytes: it grows by doubling from its first
/// 4,096 bytes, and holds the frame and the header (at most 14 bytes) and what the last read added (at most 4,096).
fn read_buffer(frame: usize) -> usize {
    (frame + 14 + 4096).next_power_of_two().max(4096)
}

/// What one session may hold per frame of up to `m` bytes, once it has received such a frame: the read buffer, plus the
/// slack the measurements show above its power of two (up to a quarter: the library's own growth is not exact, and
/// the same single library measured outside this server shows the same excess). At the default `m` of 1,000,000 the
/// measurement is 1.17 MB.
fn persistent_bound(m: usize) -> usize {
    read_buffer(m) / 4 * 5
}

/// What one session may hold while a fragmented message is incomplete: the message's buffer (a vector that doubles, so
/// up to twice the power of two above the message) and the read buffer. At the default `m` the measurement is 3.29 MB.
fn transient_bound(m: usize) -> usize {
    2 * m.next_power_of_two() + persistent_bound(m)
}

struct Measured {
    persistent: usize,
    transient: usize,
}

/// Opens `sessions` sessions from one address with `max_http_buffer_size` `m`, sends each one valid event of about `m`
/// bytes (to an event nobody listens to: it is decoded and dropped, so only the library's buffers remain), and then an
/// incomplete fragmented message of the worst shape. Returns the heap per session in both states.
async fn measure(m: usize, sessions: usize, source: u8) -> Measured {
    let options = IoOptions::new().engine(EngineOptions::new().max_http_buffer_size(m).ping_interval(Duration::from_secs(60)));
    let fx = start(options, |_| {}).await;
    let mut clients = Vec::new();
    for _ in 0..sessions {
        let mut c = Raw::open(fx.addr, ip(source)).await.unwrap();
        assert!(c.connect("").await.expect("joined").starts_with("40"));
        clients.push(c);
    }
    let base = settled().await;
    // One valid message of nearly the largest size: the text of an event with a long string argument.
    let event = String::from(r#"2["nobody",""#) + &"x".repeat(m - 64) + r#""]"#;
    assert!(event.len() < m);
    let event = std::sync::Arc::new(event.into_bytes());
    for c in &clients {
        c.tx().msg(&event).await.unwrap();
    }
    let idle = settled().await;
    let persistent = ((idle - base).max(0) as usize) / sessions;
    assert!(clients.iter().all(|c| !c.is_closed()), "the sessions stay open");

    // The worst transient: a binary message of `m - 1` bytes and a continuation of one byte, both non-final (the
    // message buffer doubles to about 2 m), and a continuation frame that claims `m` bytes and sends most of them.
    let first = vec![7u8; m - 1];
    let partial = vec![7u8; m - 1000];
    async fn incomplete(tx: &Tx, first: &[u8], partial: &[u8], m: usize) {
        tx.raw(&[&frame_header(2, false, first.len()), first]).await.unwrap();
        tx.raw(&[&frame_header(0, false, 1), &[7u8]]).await.unwrap();
        tx.raw(&[&frame_header(0, false, m), partial]).await.unwrap();
    }
    for c in &clients {
        incomplete(&c.tx(), &first, &partial, m).await;
    }
    let worst = settled().await;
    let transient = ((worst - base).max(0) as usize) / sessions;
    println!("M = {m}: {sessions} sessions: idle retention {persistent} B per session, worst transient {transient} B per session");
    assert!(clients.iter().all(|c| !c.is_closed()), "the sessions stay open");
    drop(clients);
    fx.io.close();
    engine_support::eventually("the sessions to close", || fx.io.engine().session_count() == 0).await;
    Measured { persistent, transient }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_transport_holds_at_most_the_documented_bytes_per_session_at_the_default_message_size() {
    let _guard = ONE_AT_A_TIME.lock().await;
    // The default `max_http_buffer_size`, which the numbers of the limits pages are stated at.
    let m = 1_000_000;
    let measured = measure(m, 16, 11).await;
    assert!(
        measured.persistent <= persistent_bound(m),
        "an idle session keeps {} bytes after one large message, above {}",
        measured.persistent,
        persistent_bound(m)
    );
    assert!(
        measured.transient <= transient_bound(m),
        "a session with an incomplete message holds {} bytes, above {}",
        measured.transient,
        transient_bound(m)
    );
    // The numbers the pages state (MiB, per session) are measured: about 1.17 MB and 3.29 MB.
    assert!(measured.persistent > m / 2, "the measurement saw the buffer ({} B)", measured.persistent);
    assert!(measured.transient > 2 * m, "the measurement saw the incomplete message ({} B)", measured.transient);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn max_http_buffer_size_lowers_what_a_session_holds_in_proportion() {
    let _guard = ONE_AT_A_TIME.lock().await;
    for (m, source) in [(100_000usize, 12u8), (250_000, 13)] {
        let measured = measure(m, 32, source).await;
        assert!(measured.persistent <= persistent_bound(m), "M = {m}: idle {} above {}", measured.persistent, persistent_bound(m));
        assert!(measured.transient <= transient_bound(m), "M = {m}: transient {} above {}", measured.transient, transient_bound(m));
        // Far below what the default holds.
        assert!(measured.transient < 1_000_000, "M = {m}: {} B per session", measured.transient);
    }
}

// ------------------------------------------------------------------------------------ a polling POST held for the budget

/// A polling `POST` whose records exceed the session's unread limit is answered only when the application takes some
/// of them. The records are charged; the request body must not stay alive, uncharged, while the answer waits.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_polling_post_body_is_released_while_its_answer_is_held_for_the_budget() {
    use tokio::io::AsyncWriteExt;
    let _guard = ONE_AT_A_TIME.lock().await;
    const SESSIONS: usize = 16;
    // 600 packets of 1,000 bytes: a body of about 600 KB of records, far above the session's 100 KB limit.
    let record = format!("4{}", "x".repeat(999));
    let body = std::sync::Arc::new(vec![record.as_str(); 600].join("\x1e").into_bytes());
    let options =
        EngineOptions::new().max_unread_bytes(100 * 1024).ping_interval(Duration::from_secs(60)).ping_timeout(Duration::from_secs(60));
    let mut fx = engine_support::start(options).await;
    let mut sids = Vec::new();
    let mut sockets = Vec::new();
    for _ in 0..SESSIONS {
        let (sid, _) = engine_support::open_polling(fx.addr).await;
        sockets.push(fx.socket().await);
        sids.push(sid);
    }
    let base = settled().await;
    let mut connections = Vec::new();
    for sid in &sids {
        let mut s = tokio::net::TcpStream::connect(fx.addr).await.unwrap();
        let head = format!(
            "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain;charset=UTF-8\r\nContent-Length: {}\r\n\r\n",
            engine_support::poll_url(sid),
            body.len()
        );
        s.write_all(head.as_bytes()).await.unwrap();
        s.write_all(&body).await.unwrap();
        connections.push(s);
    }
    // The answers are held: nobody takes the records.
    let held = settled().await;
    let per_session = ((held - base).max(0) as usize) / SESSIONS;
    // The records are what is charged: each is a copy of its own length (and a packet's bookkeeping).
    let charged = body.len() - 599;
    println!("polling POST of {} bytes held for the budget: {per_session} B live per session for {charged} B of records", body.len());
    assert!(per_session >= charged, "the records are held ({per_session} B per session)");
    // The records and nothing of the body: with the body kept alive the heap holds about twice the records.
    assert!(
        per_session <= charged + charged / 3 + 64 * 1024,
        "{per_session} B live per session for {charged} B of records: the body is still alive"
    );
    drop(connections);
    drop(sockets);
    fx.engine.close();
}
