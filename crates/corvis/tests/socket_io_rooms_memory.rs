#![cfg(feature = "socket-io")]
//! What a broadcast holds, measured with a counting allocator (the figures are the heap of the process).
//!
//! The recipients are long-polling sessions that never poll again, so what is queued for them stays queued and no
//! transport copies it into a frame: the measurement is the memory of the queues. Formula, for a broadcast of an
//! encoding of `E` bytes to `N` recipients: the queues hold `E` once (one reference counted buffer) plus a bounded
//! amount per recipient (a reference and a queue entry), not `N x E`.

mod engine_support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Duration;

use corvis::socket_io::{IoOptions, testing};
use engine_support::io_client::start;
use engine_support::{open_polling, poll, post};

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

fn live() -> isize {
    LIVE.load(Ordering::SeqCst)
}

const PAYLOAD: usize = 1 << 20;
/// What one more recipient may cost the queues: a reference to the encoding, a queue entry and the bookkeeping of
/// the broadcast's snapshot (it is a few dozen bytes; the bound is generous so the test does not depend on layout).
const PER_RECIPIENT: isize = 8 * 1024;

#[tokio::test]
async fn a_broadcast_holds_one_copy_of_its_encoding_however_many_sockets_it_reaches() {
    let fx = start(IoOptions::new(), |_| {}).await;
    let mut ids = Vec::new();
    for _ in 0..100 {
        let (sid, _) = open_polling(fx.addr).await;
        assert_eq!(post(fx.addr, &sid, "40").await.status, 200);
        let connect = poll(fx.addr, &sid).await.text();
        let id = connect.split("\"sid\":\"").nth(1).and_then(|s| s.split('"').next()).unwrap().to_string();
        ids.push(id);
    }
    for (n, id) in ids.iter().enumerate() {
        let socket = fx.io.main().socket(id).unwrap();
        socket.join("all").unwrap();
        if n < 10 {
            socket.join("few").unwrap();
        }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let payload = "x".repeat(PAYLOAD);

    let before = live();
    fx.io.to("few").emit("big", (payload.as_str(),)).await.unwrap();
    let few = live() - before;

    let before = live();
    fx.io.to("all").emit("big", (payload.as_str(),)).await.unwrap();
    let all = live() - before;

    println!("held by a broadcast to 10 sockets: {few} bytes; to 100 sockets: {all} bytes (payload {PAYLOAD})");
    for id in ids.iter().take(10) {
        assert!(testing::buffered_amount(&fx.io.main().socket(id).unwrap()) > PAYLOAD, "the packet is queued on every recipient");
    }
    assert!(few < (PAYLOAD as isize) * 3 / 2 + 10 * PER_RECIPIENT, "one copy for 10 recipients, held {few}");
    assert!(all < few + 90 * PER_RECIPIENT, "ninety more recipients cost {} bytes: a copy each would be {}", all - few, 90 * PAYLOAD);
    assert!(all < (PAYLOAD as isize) * 3 / 2 + 100 * PER_RECIPIENT, "one copy for 100 recipients, held {all}");
}
