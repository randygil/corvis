#![cfg(feature = "socket-io")]
//! The memory a gateway pipeline holds for events in flight, measured with a counting allocator. Everything a gateway
//! handler holds of a message (the arguments, the body as a `Value` shared with the context, one copy for each
//! `#[message_body]` parameter and the one an error's `cause` echoes) is charged to the connection's unread budget by a
//! try-charge that never waits, so the heap of N in-flight handlers stays within the server-wide budget `T` times a
//! constant (2 here), not within `T` times the number of copies. When there is no room the event is answered with
//! `exception` and the handler does not run. Servers bind to 127.0.0.1.

mod engine_support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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

#[injectable]
pub struct Held {
    #[default]
    entered: Arc<AtomicUsize>,
    #[default]
    release: Arc<Notify>,
    #[default]
    finished: Arc<AtomicUsize>,
    #[default]
    _unused: Arc<Mutex<()>>,
}

#[websocket_gateway("/hold")]
pub struct HoldGateway {
    held: Arc<Held>,
}

#[handlers]
impl HoldGateway {
    /// Keeps its body (and the copies the pipeline made) until the test releases it.
    #[subscribe_message("hold")]
    async fn hold(&self, #[message_body] body: Value) {
        self.held.entered.fetch_add(1, Ordering::SeqCst);
        let release = self.held.release.notified();
        tokio::pin!(release);
        release.as_mut().enable();
        release.await;
        let _keep = body;
        self.held.finished.fetch_add(1, Ordering::SeqCst);
    }
}

#[module(providers = [Held, HoldGateway])]
pub struct HoldModule;

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
    const EVENT: usize = 400 * 1024;
    const EVENTS: usize = 30;
    let engine = EngineOptions::new().max_total_unread_bytes(TOTAL).max_unread_bytes(4 * MIB);
    let (mut app, addr) = start::<HoldModule>(
        IoOptions::new().max_decoded_bytes(2 * MIB).max_attachments(1).max_address_share(ADDRESS).engine(engine.max_http_buffer_size(MIB)),
    )
    .await;
    let held = app.get::<Held>().unwrap();
    let mut client = Raw::connect(addr).await;
    client.join("/hold", None).await;
    let payload = "x".repeat(EVENT);
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    for _ in 0..EVENTS {
        client.send(&format!(r#"2/hold,["hold","{payload}"]"#)).await;
    }
    // Let the server take what it takes: some handlers are in flight, the rest wait parked or were refused.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let entered = held.entered.load(Ordering::SeqCst);
    let mut seen = Vec::new();
    while let Some(m) = client.recv_within(Duration::from_millis(200)).await {
        seen.push(format!("{m:?}").chars().take(120).collect::<String>());
    }
    assert!(entered >= 1, "some handlers are in flight; the client saw {seen:?}");
    let peak = (PEAK.load(Ordering::Relaxed) - baseline).max(0) as usize;
    assert!(
        peak <= 2 * ADDRESS,
        "{entered} handlers in flight hold {peak} bytes at their peak; the bound is 2 x the address quota ({})",
        2 * ADDRESS
    );
    held.release.notify_waiters();
    drop(client);
    app.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_event_that_finds_no_room_for_its_copies_is_answered_with_exception_and_its_handler_does_not_run() {
    let _one = ONE_AT_A_TIME.lock().await;
    // A budget that fits the arguments of one event but not the copies of the pipeline.
    let engine = EngineOptions::new().max_total_unread_bytes(2 * MIB).max_unread_bytes(2 * MIB);
    let (mut app, addr) =
        start::<HoldModule>(IoOptions::new().max_decoded_bytes(2 * MIB).max_attachments(1).engine(engine.max_http_buffer_size(MIB))).await;
    let held = app.get::<Held>().unwrap();
    let mut client = Raw::connect(addr).await;
    client.join("/hold", None).await;
    // Two held events (two copies each: 1.6 MiB) leave room for the packet of a third (0.4 MiB) but not for its copies
    // (0.8 MiB, its packet's charge counting as room): the third is refused with `exception` by the gateway, not parked
    // by the Socket.IO layer.
    let payload = "x".repeat(400 * 1024);
    for _ in 0..6 {
        client.send(&format!(r#"2/hold,["hold","{payload}"]"#)).await;
    }
    let mut refused = 0;
    while let Some(message) = client.recv_within(Duration::from_secs(2)).await {
        if let engine_support::io_client::Msg::Text(text) = message
            && text.contains("Too many messages in flight")
        {
            refused += 1;
        }
        if refused > 0 {
            break;
        }
    }
    assert!(refused > 0, "an event without room is answered with exception; {} handlers entered", held.entered.load(Ordering::SeqCst));
    assert!(held.entered.load(Ordering::SeqCst) < 6, "and its handler did not run");
    held.release.notify_waiters();
    app.close().await.unwrap();
}
