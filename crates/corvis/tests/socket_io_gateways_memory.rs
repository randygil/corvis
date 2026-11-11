#![cfg(feature = "socket-io")]
//! The memory a gateway pipeline holds for events in flight, and the memory pending disconnect hooks hold, measured with a
//! counting allocator. Everything a gateway handler holds of a message (the body as a `Value` shared with the context,
//! one copy for each `#[message_body]` parameter, or the copy an error's `cause` echoes) is charged to the connection's
//! unread budget at its real size by a try-charge that never waits, so the heap of the handlers in flight stays within
//! that budget times a small constant. When there is no room the event is answered with `exception` and the handler
//! does not run. The sockets whose disconnect hooks wait pin at most the application's byte cap. Servers bind to
//! 127.0.0.1; the tests wait for conditions, never for a fixed time.

mod engine_support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::time::Duration;

use corvis::Module;
use corvis::prelude::*;
use corvis::socket_io::{EngineOptions, IoAdapter, IoOptions};
use corvis::websocket_gateway;
use engine_support::io_client::Raw;
use tokio::sync::Notify;

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

fn grew(by: isize) {
    let now = LIVE.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size() as isize);
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
            grew(layout.size() as isize);
        }
        p
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            grew(new_size as isize - layout.size() as isize);
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// One test at a time: the counter is the process's.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const MIB: usize = 1024 * 1024;

async fn start<M: Module>(options: IoOptions) -> (CorvisApplication, std::net::SocketAddr) {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.use_websocket_adapter(IoAdapter::new().options(options));
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_heap_of_events_in_flight_stays_within_the_unread_budget_times_a_constant() {
    let _one = ONE_AT_A_TIME.lock().await;
    const TOTAL: usize = 16 * MIB;
    // All the events come from one address, whose quota is the binding bound.
    const ADDRESS: usize = 4 * MIB;
    // Each held event costs two copies of 450 KiB: four fill most of the quota, and the fifth finds room for its packet
    // but not for its copies, so it is refused rather than parked.
    const EVENT: usize = 450 * 1024;
    const EVENTS: usize = 30;
    let engine = EngineOptions::new().max_total_unread_bytes(TOTAL).max_unread_bytes(4 * MIB);
    let (mut app, addr) = start::<ArraysModule>(
        IoOptions::new().max_decoded_bytes(2 * MIB).max_attachments(1).max_address_share(ADDRESS).engine(engine.max_http_buffer_size(MIB)),
    )
    .await;
    let arrays = app.get::<Arrays>().unwrap();
    let mut client = Raw::connect(addr).await;
    client.join("/arrays", None).await;
    let packet = format!(r#"2/arrays,["hold1","{}"]"#, "x".repeat(EVENT));
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    // One at a time until the gateway refuses one: each event either enters its handler or is answered with exception.
    let mut refused = None;
    for sent in 1..=EVENTS {
        client.send(&packet).await;
        let entered = arrays.entered.load(Ordering::SeqCst);
        tokio::select! {
            () = arrays.wait_entered(entered + 1) => {}
            message = client.recv_within(Duration::from_secs(30)) => {
                assert!(
                    matches!(&message, Some(engine_support::io_client::Msg::Text(t)) if t.contains("Too many messages in flight")),
                    "{message:?}"
                );
                refused = Some(sent);
                break;
            }
        }
    }
    let entered = arrays.entered.load(Ordering::SeqCst);
    assert!(refused.is_some() && entered >= 1, "{entered} handlers entered, refused at {refused:?}");
    let peak = (PEAK.load(Ordering::Relaxed) - baseline).max(0) as usize;
    assert!(
        peak <= 2 * ADDRESS,
        "{entered} handlers in flight hold {peak} bytes at their peak; the bound is 2 x the address quota ({})",
        2 * ADDRESS
    );
    arrays.release.notify_waiters();
    drop(client);
    app.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_event_that_finds_no_room_for_its_copies_is_answered_with_exception_and_its_handler_does_not_run() {
    let _one = ONE_AT_A_TIME.lock().await;
    // A budget that fits the arguments of one event but not the copies of the pipeline.
    let engine = EngineOptions::new().max_total_unread_bytes(2 * MIB).max_unread_bytes(2 * MIB);
    let (mut app, addr) =
        start::<ArraysModule>(IoOptions::new().max_decoded_bytes(2 * MIB).max_attachments(1).engine(engine.max_http_buffer_size(MIB)))
            .await;
    let arrays = app.get::<Arrays>().unwrap();
    let mut client = Raw::connect(addr).await;
    client.join("/arrays", None).await;
    // Two held events (two copies each: 1.6 MiB) leave room for the packet of a third (0.4 MiB) but not for its copies
    // (0.8 MiB, its packet's charge counting as room): the third is refused with `exception` by the gateway, not parked
    // by the Socket.IO layer. One at a time, so that no other packet waits in the budget meanwhile.
    let packet = format!(r#"2/arrays,["hold1","{}"]"#, "x".repeat(400 * 1024));
    for held in 1..=2 {
        client.send(&packet).await;
        arrays.wait_entered(held).await;
    }
    client.send(&packet).await;
    let answer = client.recv_within(Duration::from_secs(10)).await;
    assert!(
        matches!(&answer, Some(engine_support::io_client::Msg::Text(text)) if text.contains("Too many messages in flight")),
        "an event without room for its copies is answered with exception; got {answer:?}"
    );
    assert_eq!(arrays.entered.load(Ordering::SeqCst), 2, "and its handler did not run");
    arrays.release.notify_waiters();
    app.close().await.unwrap();
}

// ------------------------------------------------------------------------------- the charge bounds the copies (round 2)

/// Counts what entered and lets the test wait for it.
#[injectable]
pub struct Arrays {
    #[default]
    entered: Arc<AtomicUsize>,
    #[default]
    changed: Arc<Notify>,
    #[default]
    release: Arc<Notify>,
    #[default]
    finished: Arc<AtomicUsize>,
}

impl Arrays {
    fn enter(&self) -> tokio::sync::futures::Notified<'_> {
        let release = self.release.notified();
        self.entered.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_waiters();
        release
    }

    fn finish(&self) {
        self.finished.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_waiters();
    }

    async fn wait_for(&self, what: &AtomicUsize, count: usize) {
        let wait = async {
            loop {
                let notified = self.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if what.load(Ordering::SeqCst) >= count {
                    return;
                }
                notified.await;
            }
        };
        tokio::time::timeout(Duration::from_secs(30), wait).await.expect("the handlers to get there");
    }

    async fn wait_entered(&self, count: usize) {
        self.wait_for(&self.entered, count).await;
    }
}

#[websocket_gateway("/arrays")]
pub struct ArraysGateway {
    arrays: Arc<Arrays>,
}

#[handlers]
impl ArraysGateway {
    /// One body parameter (P = 1).
    #[subscribe_message("hold1")]
    async fn hold1(&self, #[message_body] body: Value) {
        let release = self.arrays.enter();
        tokio::pin!(release);
        release.as_mut().enable();
        release.await;
        drop(body);
        self.arrays.finish();
    }

    /// Two body parameters (P = 2).
    #[subscribe_message("hold2")]
    async fn hold2(&self, #[message_body] a: Value, #[message_body] b: Value) {
        let release = self.arrays.enter();
        tokio::pin!(release);
        release.as_mut().enable();
        release.await;
        drop((a, b));
        self.arrays.finish();
    }
}

#[module(providers = [Arrays, ArraysGateway])]
pub struct ArraysModule;

/// The reviewer's worst case for the copies: an array whose length is just over a power of two (2^17 + 1 zeros), which a
/// copy grown by doubling would hold at twice its size. The charge is `(1 + max(P, 1))` times the body's real cost
/// (`json_cost`: one `Value` slot per element plus the allocation's overhead, here 24 bytes); the heap the event holds,
/// at its peak and while the handler waits, stays within 1.25 times it. On one thread: when the test sees a handler
/// finish, the event's whole pipeline has ended (nothing awaits after the handler), so the next measurement starts clean.
#[tokio::test]
async fn the_copies_of_a_body_stay_within_their_charge_for_one_and_two_body_parameters() {
    let _one = ONE_AT_A_TIME.lock().await;
    const ELEMENTS: usize = (1 << 17) + 1;
    let (mut app, addr) = start::<ArraysModule>(IoOptions::new()).await;
    let arrays = app.get::<Arrays>().unwrap();
    let mut client = Raw::connect(addr).await;
    client.join("/arrays", None).await;
    let zeros = vec!["0"; ELEMENTS].join(",");
    let body_cost = 24 + ELEMENTS * std::mem::size_of::<Value>();
    for (round, (event, params)) in [("hold1", 1usize), ("hold2", 2)].into_iter().enumerate() {
        let packet = format!(r#"2/arrays,["{event}",[{zeros}]]"#);
        let charge = (1 + params.max(1)) * body_cost;
        let baseline = LIVE.load(Ordering::Relaxed);
        PEAK.store(baseline, Ordering::Relaxed);
        client.send(&packet).await;
        arrays.wait_entered(round + 1).await;
        let held = (LIVE.load(Ordering::Relaxed) - baseline).max(0) as usize;
        let peak = (PEAK.load(Ordering::Relaxed) - baseline).max(0) as usize;
        let bound = charge + charge / 4;
        println!(
            "{event}: charge {charge} B, held {held} B ({:.3} x), peak {peak} B ({:.3} x)",
            held as f64 / charge as f64,
            peak as f64 / charge as f64
        );
        assert!(held <= bound, "{event}: the waiting handler holds {held} bytes; its charge is {charge}, the bound {bound}");
        assert!(peak <= bound, "{event}: the event peaked at {peak} bytes; its charge is {charge}, the bound {bound}");
        arrays.release.notify_waiters();
        arrays.wait_for(&arrays.finished, round + 1).await;
    }
    drop(client);
    app.close().await.unwrap();
}

// ----------------------------------------------------------------------- what pending disconnect hooks pin (round 2)

static DROPPED: AtomicUsize = AtomicUsize::new(0);
static FIRST_DROP: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Counts the warnings of hooks dropped at the application's cap.
struct DropCounter;

impl corvis::LoggerService for DropCounter {
    fn log(&self, level: corvis::LogLevel, message: &str, _context: Option<&str>) {
        if level == corvis::LogLevel::Warn && message.contains("was dropped") {
            if DROPPED.fetch_add(1, Ordering::SeqCst) == 0 {
                *FIRST_DROP.lock().unwrap() = message.to_string();
            }
        }
    }
}

#[injectable]
pub struct Leaving {
    #[default]
    started: Arc<AtomicUsize>,
    #[default]
    changed: Arc<Notify>,
    #[default]
    gate: Arc<LeaveGate>,
}

pub struct LeaveGate(tokio::sync::Semaphore);

impl Default for LeaveGate {
    fn default() -> Self {
        LeaveGate(tokio::sync::Semaphore::new(0))
    }
}

#[websocket_gateway("/leaving")]
pub struct LeavingGateway {
    leaving: Arc<Leaving>,
}

#[handlers]
impl LeavingGateway {}

#[async_trait]
impl corvis::OnGatewayDisconnect<corvis::socket_io::IoSocket> for LeavingGateway {
    async fn handle_disconnect(&self, _socket: corvis::socket_io::IoSocket) {
        self.leaving.started.fetch_add(1, Ordering::SeqCst);
        self.leaving.changed.notify_waiters();
        self.leaving.gate.0.acquire().await.unwrap().forget();
    }
}

#[module(providers = [Leaving, LeavingGateway])]
pub struct LeavingModule;

/// Sockets that left wait for their hooks, from 240 addresses, until the application's byte cap is full and the next
/// hooks are dropped: the heap they hold stays within the cap (each one is charged what it pins at most, and the cap is
/// what is admitted), and none keeps its Engine.IO session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_sockets_of_pending_disconnect_hooks_pin_at_most_the_byte_cap() {
    let _one = ONE_AT_A_TIME.lock().await;
    const CAP: usize = MIB;
    let mut app = CorvisFactory::create::<LeavingModule>().await.unwrap();
    app.use_logger(DropCounter);
    app.use_websocket_adapter(IoAdapter::new().max_pending_disconnect_hook_bytes(CAP));
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let leaving = app.get::<Leaving>().unwrap();
    let visit = |last: u8| async move {
        let mut client = Raw::connect_from(addr, std::net::IpAddr::from([127, 0, 0, last])).await;
        client.join("/leaving", None).await;
        client.close().await;
    };
    // One round first, so that the tables the server keeps have their size.
    visit(9).await;
    leaving.gate.0.add_permits(1);
    let started = |n: usize| {
        let leaving = leaving.clone();
        async move {
            let wait = async {
                loop {
                    let notified = leaving.changed.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    if leaving.started.load(Ordering::SeqCst) >= n {
                        return;
                    }
                    notified.await;
                }
            };
            tokio::time::timeout(Duration::from_secs(30), wait).await.expect("the hooks to start");
        }
    };
    started(1).await;
    let baseline = LIVE.load(Ordering::Relaxed);
    // Two sockets per address stay below each address's share (a 64th of the cap) but fill the cap together.
    for _ in 0..2 {
        for last in 10..250u8 {
            visit(last).await;
        }
    }
    started(1 + 32).await;
    let held = (LIVE.load(Ordering::Relaxed) - baseline).max(0) as usize;
    // What may still be in flight: the Engine.IO sessions of the last few clients, closing.
    let slack = 256 * 1024;
    println!(
        "480 sockets left from 240 addresses: their pending hooks hold {held} B against a cap of {CAP} B; first drop: {}",
        FIRST_DROP.lock().unwrap()
    );
    assert!(DROPPED.load(Ordering::SeqCst) > 0, "the cap was reached and hooks were dropped");
    assert!(held <= CAP + slack, "the sockets of the pending hooks hold {held} bytes; the cap is {CAP}");
    leaving.gate.0.add_permits(1_000);
    app.close().await.unwrap();
}
