//! The memory session store holds no more than it accounts for: with a counting allocator, the live heap of a store
//! filled with adversarial records stays within the bytes it reports, which `max_total_bytes` bounds. This is the only
//! test of the binary because the allocator counts the whole process.
#![cfg(feature = "auth-session")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use corvis::auth::session::{MemorySessionStore, SessionStore};
use serde_json::{Value, json};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            LIVE.fetch_add(new_size, Ordering::Relaxed);
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

type Shape = Box<dyn Fn(usize) -> Value>;

/// The shapes that make a parsed `Value` much larger than its text.
fn shapes() -> Vec<(&'static str, Shape)> {
    vec![
        ("array of zeros", Box::new(|_| json!({ "a": vec![0; 7000] }))),
        ("array of empty objects", Box::new(|_| json!({ "a": (0..3500).map(|_| json!({})).collect::<Vec<_>>() }))),
        ("array of empty arrays", Box::new(|_| json!({ "a": (0..3500).map(|_| json!([])).collect::<Vec<_>>() }))),
        ("array of nulls", Box::new(|_| json!({ "a": vec![Value::Null; 3000] }))),
        (
            "object of many short keys",
            Box::new(|i| {
                let mut object = serde_json::Map::new();
                for k in 0..1500 {
                    object.insert(format!("{k:x}"), json!(i % 10));
                }
                json!({ "a": object })
            }),
        ),
        ("a long string", Box::new(|_| json!({ "a": "x".repeat(14_000) }))),
        ("records with a user", Box::new(|i| json!({ "passport": { "user": format!("user-{}", i % 40) }, "a": vec![0; 2000] }))),
        ("tiny records", Box::new(|_| json!({}))),
    ]
}

/// The cheapest records, one user each, with a count just past a growth of the hash tables (7/8 of a power of two),
/// where the tables hold the most spare room per entry.
fn the_overhead_covers_the_tables_just_after_they_grow() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    let ttl = Duration::from_secs(3600);
    for count in [449, 897, 1793, 3585, 7169] {
        for authenticated in [false, true] {
            let baseline = live();
            let store = MemorySessionStore::new();
            for i in 0..count {
                let record = if authenticated { json!({ "passport": { "user": i } }) } else { json!({}) };
                runtime.block_on(store.create(&format!("{i:032}"), &record, ttl)).unwrap();
            }
            let held = live().saturating_sub(baseline);
            let accounted = store.total_bytes();
            eprintln!("{count} sessions, authenticated {authenticated}: heap {held}, accounted {accounted}");
            assert!(held <= accounted, "{count} sessions: heap {held}, accounted {accounted}");
        }
    }
}

/// A flood of minimal sessions that are then destroyed leaves no room of the tables behind, and a refill with large
/// records stays within the bound: the tables shrink when they hold less than a quarter of their room.
fn the_tables_give_back_their_room() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    let ttl = Duration::from_secs(3600);
    let bound = 128usize << 20;
    let baseline = live();
    let store = MemorySessionStore::new().max_total_bytes(bound);
    for i in 0..100_000 {
        runtime.block_on(store.create(&format!("{i:032}"), &json!({}), ttl)).unwrap();
    }
    let full = live().saturating_sub(baseline);
    for i in 0..100_000 {
        assert!(runtime.block_on(store.destroy(&format!("{i:032}"))).unwrap());
    }
    assert_eq!(store.len(), 0);
    let emptied = live().saturating_sub(baseline);
    eprintln!("100000 minimal sessions: heap {full}; destroyed: heap {emptied}, accounted {}", store.total_bytes());
    assert!(emptied < 256 * 1024, "{emptied} bytes stay live after every session was destroyed");
    // The refill: records of 14 KB until the bound makes the store evict.
    for i in 0..10_000 {
        runtime.block_on(store.create(&format!("{i:032}"), &json!({ "a": "x".repeat(14_000) }), ttl)).unwrap();
    }
    let refilled = live().saturating_sub(baseline);
    let accounted = store.total_bytes();
    eprintln!("refilled: {} sessions, heap {refilled}, accounted {accounted}", store.len());
    assert!(accounted <= bound);
    assert!(refilled <= bound, "the refill holds {refilled} bytes against a bound of {bound}");
    drop(store);
    assert!(live().saturating_sub(baseline) < 64 * 1024, "the heap is given back");
}

/// One test function: the allocator counts the whole process, so nothing may run beside it.
#[test]
fn the_live_heap_of_the_store_stays_within_what_it_accounts_for() {
    the_overhead_covers_the_tables_just_after_they_grow();
    adversarial_shapes_stay_within_the_bound();
    the_tables_give_back_their_room();
}

fn adversarial_shapes_stay_within_the_bound() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    let ttl = Duration::from_secs(3600);
    for (name, shape) in shapes() {
        let baseline = live();
        // The bound is smaller than what is offered, so evictions happen too.
        let store = MemorySessionStore::new().max_total_bytes(8 << 20).max_sessions_per_user(1000);
        for i in 0..1500 {
            let record = shape(i);
            runtime.block_on(store.create(&format!("{i:032}"), &record, ttl)).unwrap();
        }
        let held = live().saturating_sub(baseline);
        let accounted = store.total_bytes();
        eprintln!("{name}: {} sessions, heap {held}, accounted {accounted}", store.len());
        assert!(accounted <= 8 << 20, "{name}: {accounted} is over the bound");
        assert!(store.len() > 100, "{name}: only {} sessions fit", store.len());
        // The allowance for everything that is not data is the per-entry overhead, which is counted in `accounted`.
        assert!(held <= accounted, "{name}: the store holds {held} bytes of heap but accounts for {accounted}");
        drop(store);
        assert!(live().saturating_sub(baseline) < 64 * 1024, "{name}: the heap is given back");
    }
}
