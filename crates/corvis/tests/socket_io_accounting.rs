#![cfg(feature = "socket-io")]
//! One accounting path for the Socket.IO layer, measured with a counting allocator and raw clients that send without
//! copying (so what is counted is the server's memory).
//!
//! Everything an address holds on the shared unread budget counts against its share (`max_address_share`), in every
//! state: wire messages above the guaranteed share, messages parked for room, packets being reassembled, decoded
//! packets and binary attachments, before and after they reach a handler. These tests send what a hostile client
//! sends (binary events, large text events, fragmented messages) from one address and check, with the heap of the
//! process, that the address stays within its share plus the stated per-message slack and the server within its
//! invariant, and that a session of another address is served.
//!
//! It also checks the waiting rules: a session that waits because the server-wide budget is exhausted by others is
//! not closed with `ping timeout`, its packet is admitted as soon as room appears, and the wait is bounded by
//! `admission_timeout` with its own disconnect reason.

mod engine_support;
mod socket_io_raw;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use corvis::socket_io::testing::probe_auth;
use corvis::socket_io::{DisconnectReason, EngineOptions, IoOptions, IoServer};
use engine_support::io_client::start;
use socket_io_raw::{Raw, ip};
use tokio::sync::Notify;

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

const MIB: usize = 1024 * 1024;

/// The server-wide budget of the flooding tests.
const TOTAL: usize = 64 * MIB;

/// Registers `hold` (counts, keeps its arguments until `gate` opens, or for `delay` when there is one) and `echo`
/// (acknowledges), on the main namespace.
struct Handlers {
    entered: Arc<AtomicUsize>,
    gate: Arc<Notify>,
    delay: Option<Duration>,
    first_return: Arc<Mutex<Option<Instant>>>,
}

impl Handlers {
    fn new(delay: Option<Duration>) -> Handlers {
        Handlers { entered: Arc::new(AtomicUsize::new(0)), gate: Arc::new(Notify::new()), delay, first_return: Arc::new(Mutex::new(None)) }
    }

    fn register(&self, io: &IoServer) {
        let (entered, gate, delay, first) = (self.entered.clone(), self.gate.clone(), self.delay, self.first_return.clone());
        io.on_connection(move |socket| {
            let (entered, gate, first) = (entered.clone(), gate.clone(), first.clone());
            socket.on("hold", move |_socket, args, _ack| {
                let (entered, gate, first) = (entered.clone(), gate.clone(), first.clone());
                async move {
                    entered.fetch_add(1, Ordering::SeqCst);
                    match delay {
                        Some(delay) => {
                            tokio::time::sleep(delay).await;
                            first.lock().unwrap().get_or_insert_with(Instant::now);
                        }
                        None => gate.notified().await,
                    }
                    drop(args);
                }
            });
            socket.on("echo", |_socket, _args, ack| async move {
                if let Some(ack) = ack {
                    let _ = ack.send(());
                }
            });
        });
    }

    fn entered(&self) -> usize {
        self.entered.load(Ordering::SeqCst)
    }

    /// Waits until no handler has started for a second and a half: everything the budget admits is in.
    async fn settle(&self) {
        let mut last = usize::MAX;
        loop {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let now = self.entered();
            if now == last {
                return;
            }
            last = now;
        }
    }
}

/// The bound on what one address holds: its share, plus per open session the guaranteed share and one message in the
/// hand (what the transport had read when it stopped), plus what the process itself keeps for the connections (read
/// buffers, tasks, the writers' frames).
fn address_bound(options: &IoOptions, sessions: usize, message: usize) -> usize {
    // The fair share is the 16 KiB floor for any budget below 160 MiB with the default 10,000 sessions.
    let fair = 16 * 1024;
    options.address_share() + sessions * (fair + 3 * message) + 2 * MIB
}

/// An honest session on another address: a 3 KB JSON event with an acknowledgement, answered promptly, and the session
/// is not closed.
async fn honest_is_served(fx_addr: std::net::SocketAddr) {
    let mut honest = Raw::open(fx_addr, ip(200)).await.unwrap();
    assert!(honest.connect("").await.expect("joined").starts_with("40"));
    let payload = format!("[{}]", vec![r#"{"a":1}"#; 400].join(","));
    honest.msg_str(&format!(r#"21["echo",{payload}]"#)).await.unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            match honest.recv_text(Duration::from_secs(8)).await {
                Some(t) if t.starts_with("431") => return Some(t),
                Some(_) => {}
                None => return None,
            }
        }
    })
    .await;
    assert!(matches!(answer, Ok(Some(_))), "the honest session got no answer");
    assert!(!honest.is_closed(), "the honest session was closed");
}

fn heavy(total: usize) -> IoOptions {
    IoOptions::new().max_decoded_bytes(4 * MIB).max_attachments(1).engine(EngineOptions::new().max_total_unread_bytes(total))
}

/// Eight sessions of 127.0.0.1 send `per` events each with `send`, to handlers that keep their arguments.
async fn one_address_floods<F>(options: IoOptions, message: usize, send: F) -> (usize, usize, usize)
where
    F: Fn(socket_io_raw::Tx) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
{
    const SESSIONS: usize = 8;
    let handlers = Handlers::new(None);
    let bound = address_bound(&options, SESSIONS, message);
    let total = TOTAL;
    let fx = start(options, |io| handlers.register(io)).await;
    let mut clients = Vec::new();
    for _ in 0..SESSIONS {
        let mut client = Raw::open(fx.addr, ip(1)).await.unwrap();
        assert!(client.connect("").await.expect("joined").starts_with("40"));
        clients.push(client);
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let base = live();
    let senders: Vec<_> = clients.iter().map(|c| tokio::spawn(send(c.tx()))).collect();
    handlers.settle().await;
    let held = (live() - base).max(0) as usize;
    let admitted = handlers.entered();
    println!(
        "one address, {SESSIONS} sessions: {admitted} handlers hold the packets, {held} B live (address bound {bound}, server budget {total})"
    );
    // The honest session of another address is served while the attacker holds all it may.
    honest_is_served(fx.addr).await;
    assert!(held <= bound, "one address holds {held} bytes, above its share plus slack ({bound})");
    handlers.gate.notify_waiters();
    for s in senders {
        s.abort();
    }
    drop(clients);
    fx.io.close();
    tokio::time::sleep(Duration::from_millis(500)).await;
    (held, bound, total)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn binary_attachments_of_one_address_stay_inside_its_share_and_do_not_starve_an_honest_session() {
    let _guard = ONE_AT_A_TIME.lock().await;
    // The reviewer's case: a placeholder header and a ~0.5 MB attachment, over and over, to a handler that keeps them.
    const ATTACHMENT: usize = 524_289;
    let data = Arc::new(vec![7u8; ATTACHMENT]);
    let header: &'static [u8] = br#"51-["hold",{"_placeholder":true,"num":0}]"#;
    let (held, bound, total) = one_address_floods(heavy(TOTAL), ATTACHMENT + 64, move |tx| {
        let data = data.clone();
        Box::pin(async move {
            for _ in 0..40 {
                if tx.msg(header).await.is_err() || tx.bin(&data).await.is_err() {
                    break;
                }
            }
        })
    })
    .await;
    assert!(held <= bound && bound < total / 2, "the address share ({bound}) is a small part of the budget ({total}), and held {held}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_text_events_of_one_address_stay_inside_its_share_and_do_not_starve_an_honest_session() {
    let _guard = ONE_AT_A_TIME.lock().await;
    // The reviewer's case: 0.9 MB strings, decoded packets under the quota, but messages in transit and parked outside it.
    let text = Arc::new(format!(r#"2["hold","{}"]"#, "x".repeat(900_000)).into_bytes());
    let (held, bound, total) = one_address_floods(heavy(TOTAL), 900_064, move |tx| {
        let text = text.clone();
        Box::pin(async move {
            for _ in 0..40 {
                if tx.msg(&text).await.is_err() {
                    break;
                }
            }
        })
    })
    .await;
    assert!(held <= bound && bound < total / 2, "the address share ({bound}) is a small part of the budget ({total}), and held {held}");
}

/// A fragmented message must not occupy more than it is charged: the library builds it by extending a buffer, and a
/// buffer keeps the capacity of its last doubling.
async fn fragmented(label: &str, send: impl Fn(socket_io_raw::Tx) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>) {
    const ATTACHMENT: usize = 524_289;
    let handlers = Handlers::new(None);
    let options = heavy(TOTAL);
    let share = options.address_share();
    let fx = start(options, |io| handlers.register(io)).await;
    let mut client = Raw::open(fx.addr, ip(1)).await.unwrap();
    assert!(client.connect("").await.expect("joined").starts_with("40"));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let base = live();
    let sender = tokio::spawn(send(client.tx()));
    handlers.settle().await;
    let held = (live() - base).max(0) as usize;
    let admitted = handlers.entered();
    // What is charged per kept attachment is its capacity (at most an eighth above its length) and 64 bytes; the
    // header's packet is a few hundred bytes. The message in the hand (not yet admitted) and the one being assembled
    // add at most two more, and a megabyte covers the connection's own buffers.
    let per_event = ATTACHMENT + ATTACHMENT / 8 + 64 + 1024;
    let bound = admitted * per_event + 2 * ATTACHMENT + MIB;
    println!(
        "{label}: {admitted} events kept, {held} B live ({:.2} per kept attachment), bound {bound}, share {share}",
        held as f64 / (admitted.max(1) * ATTACHMENT) as f64
    );
    assert!(admitted >= 4, "{label}: only {admitted} events admitted");
    assert!(held <= bound, "{label}: {held} bytes live for {admitted} kept attachments, above {bound}");
    handlers.gate.notify_waiters();
    sender.abort();
    drop(client);
    fx.io.close();
    tokio::time::sleep(Duration::from_millis(500)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fragmented_binary_messages_occupy_no_more_than_they_are_charged() {
    let _guard = ONE_AT_A_TIME.lock().await;
    let data = Arc::new(vec![7u8; 524_289]);
    let header: &'static [u8] = br#"51-["hold",{"_placeholder":true,"num":0}]"#;
    let d = data.clone();
    // A 524,288 byte frame and a one byte continuation: the buffer is a megabyte for half a megabyte of message.
    fragmented("524,288 + 1 B frames", move |tx| {
        let d = d.clone();
        Box::pin(async move {
            for _ in 0..30 {
                if tx.msg(header).await.is_err() || tx.bin_fragmented(&d, &[524_288, 1]).await.is_err() {
                    break;
                }
            }
        })
    })
    .await;
    // The same message in 1 KiB frames (513 of them).
    let d = data.clone();
    fragmented("1 KiB frames", move |tx| {
        let d = d.clone();
        Box::pin(async move {
            for _ in 0..30 {
                if tx.msg(header).await.is_err() || tx.bin_fragmented(&d, &[1024; 513]).await.is_err() {
                    break;
                }
            }
        })
    })
    .await;
    // A frame of 300,000 bytes and the rest: the slack of the last growth is not a half but still above an eighth.
    let d = data.clone();
    fragmented("300,000 + 224,289 B frames", move |tx| {
        let d = d.clone();
        Box::pin(async move {
            for _ in 0..30 {
                if tx.msg(header).await.is_err() || tx.bin_fragmented(&d, &[300_000, 224_289]).await.is_err() {
                    break;
                }
            }
        })
    })
    .await;
}

// ------------------------------------------------------------------------------------------------ waiting rules

/// Eleven attackers on eleven addresses fill the server-wide budget (their share is a sixth of it), then an honest session
/// on a ninth sends a packet bigger than anything left, which needs room in the budget.
struct Pressure {
    options: IoOptions,
    attack_packet: Vec<u8>,
    honest_packet: String,
}

fn pressure(admission: Duration) -> Pressure {
    let engine = EngineOptions::new()
        .max_http_buffer_size(200_000)
        .max_unread_bytes(2 * MIB)
        .max_total_unread_bytes(8 * MIB)
        .ping_interval(Duration::from_millis(500))
        .ping_timeout(Duration::from_secs(1));
    let options = IoOptions::new().engine(engine).max_decoded_bytes(MIB).max_attachments(1).admission_timeout(admission);
    // Forty-four packets of about 190 kB fill the 8 MiB budget to within a few kB: eleven addresses send four each (an
    // address could hold six), so the attackers leave no backlog behind and only their handlers hold the budget.
    let attack_packet = format!(r#"2["hold","{}"]"#, "x".repeat(190_000)).into_bytes();
    // Bigger than any packet of the attack, so it never fits in what the attackers leave free.
    let honest_packet = format!(r#"2/honest,5["echo","{}"]"#, "y".repeat(196_000));
    Pressure { options, attack_packet, honest_packet }
}

/// Handlers of the honest namespace that are running (`block` never returns).
static HONEST_RUNNING: AtomicIsize = AtomicIsize::new(0);

struct RunningGuard;

impl Drop for RunningGuard {
    fn drop(&mut self) {
        HONEST_RUNNING.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Siege {
    fx: engine_support::io_client::IoFixture,
    clients: Vec<Raw>,
    senders: Vec<tokio::task::JoinHandle<()>>,
    reasons: Arc<Mutex<Vec<DisconnectReason>>>,
}

async fn lay_siege(p: &Pressure, handlers: &Handlers) -> Siege {
    let reasons: Arc<Mutex<Vec<DisconnectReason>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = reasons.clone();
    let fx = start(p.options.clone(), |io| {
        handlers.register(io);
        io.of("/honest").unwrap().on_connection(move |socket| {
            let seen = seen.clone();
            socket.on("echo", |_socket, _args, ack| async move {
                if let Some(ack) = ack {
                    let _ = ack.send(());
                }
            });
            socket.on("block", |_socket, _args, _ack| async move {
                HONEST_RUNNING.fetch_add(1, Ordering::SeqCst);
                let _running = RunningGuard;
                futures::future::pending::<()>().await
            });
            socket.on_disconnect(move |_socket, reason| seen.lock().unwrap().push(reason));
        });
    })
    .await;
    let packet = Arc::new(p.attack_packet.clone());
    let mut clients = Vec::new();
    let mut senders = Vec::new();
    for address in 11..22u8 {
        let mut client = Raw::open(fx.addr, ip(address)).await.unwrap();
        assert!(client.connect("").await.expect("joined").starts_with("40"));
        let (tx, packet) = (client.tx(), packet.clone());
        senders.push(tokio::spawn(async move {
            for _ in 0..4 {
                if tx.msg(&packet).await.is_err() {
                    return;
                }
            }
        }));
        clients.push(client);
    }
    handlers.settle().await;
    Siege { fx, clients, senders, reasons }
}

impl Siege {
    fn end(self) {
        for s in self.senders {
            s.abort();
        }
        drop(self.clients);
        self.fx.io.close();
    }
}

async fn honest_joins(siege: &Siege) -> Raw {
    let mut honest = Raw::open(siege.fx.addr, ip(200)).await.unwrap();
    assert!(honest.connect("/honest").await.expect("joined").starts_with("40/honest"));
    honest
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_waiting_for_room_others_hold_is_not_closed_and_is_served_as_soon_as_a_handler_returns() {
    let _guard = ONE_AT_A_TIME.lock().await;
    // Heartbeat window 1.5 s; handlers return after 8 s; the wait for room is bounded by 20 s.
    let delay = Duration::from_secs(8);
    let p = pressure(Duration::from_secs(20));
    let handlers = Handlers::new(Some(delay));
    let siege = lay_siege(&p, &handlers).await;
    assert!(handlers.entered() >= 40, "the attackers hold the budget: {} handlers", handlers.entered());
    let mut honest = honest_joins(&siege).await;
    let sent = Instant::now();
    honest.msg_str(&p.honest_packet).await.unwrap();
    let answer = loop {
        match honest.recv_text(Duration::from_secs(15)).await {
            Some(t) if t.starts_with("43/honest,5") => break Some(t),
            Some(_) => {}
            None => break None,
        }
    };
    let waited = sent.elapsed();
    let first_return = *handlers.first_return.lock().unwrap();
    assert!(answer.is_some(), "the honest session was closed or never answered ({waited:?}); reasons {:?}", siege.reasons.lock().unwrap());
    assert!(waited > Duration::from_millis(2_000), "it waited beyond the heartbeat window without being closed: {waited:?}");
    let first_return = first_return.expect("a handler returned");
    let promptly = Instant::now().saturating_duration_since(first_return);
    assert!(promptly < Duration::from_secs(3), "the packet was admitted {promptly:?} after the first handler returned");
    assert!(siege.reasons.lock().unwrap().is_empty(), "no disconnect of the honest session: {:?}", siege.reasons.lock().unwrap());
    drop(honest);
    siege.end();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn when_handlers_never_return_the_waiting_session_is_closed_at_the_admission_timeout_not_by_the_heartbeat() {
    let _guard = ONE_AT_A_TIME.lock().await;
    let admission = Duration::from_secs(5);
    let p = pressure(admission);
    let handlers = Handlers::new(None);
    let siege = lay_siege(&p, &handlers).await;
    let mut honest = honest_joins(&siege).await;
    let sent = Instant::now();
    honest.msg_str(&p.honest_packet).await.unwrap();
    // Closed, but not by the heartbeat: that would be about 1.5 s after the packet.
    let mut closed = false;
    while let Some(m) = honest.recv(Duration::from_secs(30)).await {
        assert!(
            !matches!(m, socket_io_raw::In::Text(ref t) if t.starts_with("43/honest,5")),
            "the honest packet was answered while the budget is held"
        );
        if matches!(m, socket_io_raw::In::Close) {
            closed = true;
            break;
        }
    }
    let waited = sent.elapsed();
    assert!(closed, "the honest session was never closed");
    // The server reports the reason when it closes: give it a moment to run the handlers.
    for _ in 0..50 {
        if !siege.reasons.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(*siege.reasons.lock().unwrap(), [DisconnectReason::AdmissionTimeout], "closed after {waited:?}");
    assert!(waited >= admission, "closed after {waited:?}, before the admission timeout of {admission:?}");
    assert!(waited < admission + Duration::from_secs(5), "closed after {waited:?}");
    handlers.gate.notify_waiters();
    drop(honest);
    siege.end();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_that_goes_away_while_its_packet_waits_for_room_loses_its_handlers_at_the_admission_timeout() {
    let _guard = ONE_AT_A_TIME.lock().await;
    HONEST_RUNNING.store(0, Ordering::SeqCst);
    let admission = Duration::from_secs(3);
    let p = pressure(admission);
    let handlers = Handlers::new(None);
    let siege = lay_siege(&p, &handlers).await;
    let honest = honest_joins(&siege).await;
    // Two handlers of the honest session are in flight, then a packet that needs room the attackers hold parks.
    honest.msg_str(r#"2/honest,["block"]"#).await.unwrap();
    honest.msg_str(r#"2/honest,["block"]"#).await.unwrap();
    for _ in 0..100 {
        if HONEST_RUNNING.load(Ordering::SeqCst) == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(HONEST_RUNNING.load(Ordering::SeqCst), 2);
    honest.msg_str(&p.honest_packet).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    // The client goes away: its reader is paused, so the close is not read.
    let gone = Instant::now();
    drop(honest);
    while HONEST_RUNNING.load(Ordering::SeqCst) != 0 && gone.elapsed() < Duration::from_secs(25) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let cancelled_after = gone.elapsed();
    // Bound: the admission timeout (from the parked packet) plus slack, and far from `close_timeout` (30 s).
    assert_eq!(HONEST_RUNNING.load(Ordering::SeqCst), 0, "handlers still running {cancelled_after:?} after the client left");
    assert!(cancelled_after < admission + Duration::from_secs(4), "cancelled {cancelled_after:?} after the client left");
    assert_eq!(*siege.reasons.lock().unwrap(), [DisconnectReason::AdmissionTimeout]);
    handlers.gate.notify_waiters();
    siege.end();
}

// ------------------------------------------------------------------------------------------------------- auth

#[test]
fn the_stored_auth_is_charged_at_its_real_size_across_many_sockets_and_namespaces() {
    let _guard = ONE_AT_A_TIME.blocking_lock();
    let one_key_objects = format!(r#"0/n,{{"a":[{}]}}"#, vec![r#"{"":0}"#; 40].join(","));
    let nested = format!(r#"0/n,{}1{}"#, r#"{"a":"#.repeat(45), "}".repeat(45));
    let small_objects = format!(r#"0/n,{{"a":[{}]}}"#, vec![r#"{"x":1,"y":2,"z":3}"#; 20].join(","));
    let strings = format!(r#"0/n,{{"a":[{}]}}"#, vec![r#""abcdefgh""#; 200].join(","));
    let arrays = format!(r#"0/n,{{"a":[{}]}}"#, vec!["[1]"; 120].join(","));
    println!("{:<22}{:>12}{:>14}{:>14}", "shape", "per socket", "charged", "live heap");
    // The test before this one may still be freeing what its runtime held (its threads end after it gave the lock
    // back), which would be subtracted from the first measurement: wait until the heap is quiet.
    let mut quiet = live();
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let now = live();
        if now == quiet {
            break;
        }
        quiet = now;
    }
    let mut kept = Vec::new();
    let (mut total_charged, mut total_live) = (0usize, 0isize);
    for (name, text) in [
        ("one-key objects", one_key_objects),
        ("nested objects", nested),
        ("small objects", small_objects),
        ("strings", strings),
        ("one-element arrays", arrays),
    ] {
        // 20 sessions x 10 namespaces of the same auth, all kept alive: 200 sockets.
        let before = live();
        let mut charged = 0usize;
        for _ in 0..200 {
            let probe = probe_auth(&text, 1 << 20).unwrap_or_else(|e| panic!("{name}: {e}"));
            charged += probe.cost;
            kept.push(probe);
        }
        let held = live() - before;
        println!("{name:<22}{:>12}{charged:>14}{held:>14}", charged / 200);
        assert!(held >= 0 && held as usize <= charged, "{name}: {held} bytes live for 200 sockets, charged {charged}");
        assert!(held as usize * 3 >= charged, "{name}: charged {charged} for {held} live: the model is too loose to bound anything");
        total_charged += charged;
        total_live += held;
    }
    assert!(total_live as usize <= total_charged);
    drop(kept);
}
