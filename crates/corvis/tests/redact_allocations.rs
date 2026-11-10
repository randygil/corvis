//! The work of `Request::redacted_url` is bounded, measured in allocations (a clock would make the test depend on the
//! machine): a query string full of pairs costs a handful of allocations, not a few per pair as a parse of every pair
//! would. Its own test binary, because it installs a counting allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use corvis::http::body_parser::QueryParser;
use corvis::prelude::*;
use corvis::testing::request;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Allocations of the calls `f` makes on this thread.
fn count<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = ALLOCATIONS.with(Cell::get);
    let result = f();
    (result, ALLOCATIONS.with(Cell::get) - before)
}

#[controller("")]
pub struct Routes;

#[routes]
impl Routes {
    /// Marks the parameter the way a guard does, then redacts the URL and reports what that cost.
    #[get("probe")]
    fn probe(&self, req: Request) -> Value {
        req.redact_query_param("api_key");
        let (url, allocations) = count(|| req.redacted_url());
        json!({ "allocations": allocations, "length": url.len(), "hidden": !url.contains("SECRET") })
    }
}

#[module(controllers = [Routes])]
pub struct AppModule;

async fn probe(mode: QueryParser, query: &str) -> (usize, usize, bool) {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.silence_route_logs();
    app.set_query_parser(mode);
    app.init().await.unwrap();
    let r = request(&app).get(&format!("/probe?{query}")).send().await;
    assert_eq!(r.status, 200);
    let v = r.json_value();
    (v["allocations"].as_u64().unwrap() as usize, v["length"].as_u64().unwrap() as usize, v["hidden"].as_bool().unwrap())
}

#[tokio::test]
async fn redacting_a_query_string_full_of_pairs_allocates_a_handful_of_times() {
    // Few pairs, 900 pairs (below the parser limit) and 20 000 pairs (above it: everything is hidden without parsing),
    // matching and not, plain names, encoded names and a long value.
    let shapes = [
        ("a=1&".repeat(900), "900 plain pairs"),
        ("api_key=SECRET&".repeat(900), "900 matching pairs"),
        ("%61pi_key=SECRET&b=2&".repeat(450), "900 encoded and plain pairs"),
        ("a=&".repeat(20_000), "20 000 empty values"),
        ("api_key=SECRET&".repeat(4_000), "4 000 matching pairs"),
        (format!("api_key={}", "S".repeat(60_000)), "one pair, 60 KB value"),
        ("a&".repeat(30_000), "30 000 pairs without a value"),
    ];
    for mode in [QueryParser::Simple, QueryParser::Extended] {
        let (small, _, _) = probe(mode, "a=1&api_key=SECRET").await;
        for (query, label) in &shapes {
            let (allocations, length, hidden) = probe(mode, query).await;
            assert!(allocations >= 1, "{mode:?}, {label}: the counting allocator saw nothing");
            assert!(allocations <= 20, "{mode:?}, {label}: {allocations} allocations (a short query took {small})");
            assert!(length > 0);
            if label.contains("matching") || label.contains("encoded") || label.contains("one pair") {
                assert!(hidden, "{mode:?}, {label}: a value was left");
            }
        }
    }
}

#[tokio::test]
async fn the_allocations_do_not_grow_with_the_number_of_pairs() {
    for mode in [QueryParser::Simple, QueryParser::Extended] {
        let (ten, _, _) = probe(mode, &"api_key=SECRET&a=1&".repeat(5)).await;
        let (nine_hundred, _, _) = probe(mode, &"api_key=SECRET&a=1&".repeat(450)).await;
        assert!(nine_hundred <= ten + 12, "{mode:?}: {ten} allocations for 10 pairs, {nine_hundred} for 900");
    }
}
