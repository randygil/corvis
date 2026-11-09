//! Peak heap use of one compressed response per encoding. This file holds a single test because it
//! installs a counting allocator for the whole test binary; run it with `--nocapture` to see the
//! figures that `docs/http/compression.md` quotes.
#![cfg(feature = "compression")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use corvis::http::compression::CompressionOptions;
use corvis::prelude::*;
use corvis::serve_static;
use corvis::testing::{Test, request};

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(now, Ordering::SeqCst);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::SeqCst);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            if new_size >= layout.size() {
                let now = CURRENT.fetch_add(new_size - layout.size(), Ordering::SeqCst) + new_size - layout.size();
                PEAK.fetch_max(now, Ordering::SeqCst);
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Ordering::SeqCst);
            }
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

#[controller]
pub struct Api;

#[routes]
impl Api {
    #[get("small")]
    fn small(&self) -> Value {
        json!({ "d": "The quick brown fox jumps over the lazy dog. ".repeat(24) })
    }

    /// 5 MiB of text in 64 KiB chunks, without a length.
    #[get("stream")]
    fn stream(&self) -> Response {
        let chunk = bytes::Bytes::from("The quick brown fox jumps over the lazy dog. ".repeat(1489));
        let items: Vec<Result<bytes::Bytes, std::io::Error>> = (0..80).map(|_| Ok(chunk.clone())).collect();
        Response::stream(Box::pin(futures::stream::iter(items)), "text/plain")
    }
}

#[module(controllers = [Api])]
pub struct ApiModule;

/// Bytes allocated at the highest point of one request, above what was allocated before it.
async fn peak_of(app: &CorvisApplication, path: &str, accept: Option<&str>) -> (usize, usize) {
    let c = request(app);
    let mut req = c.get(path);
    if let Some(a) = accept {
        req = req.header("accept-encoding", a);
    }
    let base = CURRENT.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let r = req.send().await;
    let peak = PEAK.load(Ordering::SeqCst).saturating_sub(base);
    (peak, r.body.len())
}

#[tokio::test(flavor = "current_thread")]
async fn peak_memory_of_one_compressed_response() {
    let dir = std::env::temp_dir().join(format!("corvis-compression-mem-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut rows = String::from("[");
    let mut i: usize = 0;
    while rows.len() < 5 * 1024 * 1024 {
        if i > 0 {
            rows.push(',');
        }
        rows.push_str(&format!("{{\"id\":{i},\"name\":\"row {i}\",\"note\":\"{}\"}}", i.wrapping_mul(2_654_435_761u32 as usize)));
        i += 1;
    }
    rows.push(']');
    std::fs::write(dir.join("big.json"), &rows).unwrap();

    let module = Test::create_testing_module().controller::<Api>().with_logger(false).compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.silence_route_logs();
    app.enable_compression(CompressionOptions::new().threshold(0));
    app.use_middleware(serve_static(&dir).mount("/static"));
    app.init().await.unwrap();

    // Warm up lazily initialized state, then measure.
    for accept in [None, Some("gzip"), Some("br")] {
        peak_of(&app, "/small", accept).await;
    }
    println!("peak heap above the idle server, per response:");
    let mut figures = Vec::new();
    for (label, path) in [("1 KiB body", "/small"), ("5 MiB static file", "/static/big.json"), ("5 MiB stream", "/stream")] {
        let (plain, _) = peak_of(&app, path, None).await;
        for accept in ["gzip", "deflate", "br"] {
            let (peak, sent) = peak_of(&app, path, Some(accept)).await;
            println!(
                "  {label:18} {accept:8} peak {:>9} bytes ({:>7.1} KiB), sent {sent} bytes; uncompressed peak {plain}",
                peak,
                peak as f64 / 1024.0
            );
            figures.push((path, accept, peak));
        }
    }
    let brotli_small = figures.iter().find(|(p, a, _)| *p == "/small" && *a == "br").unwrap().2;
    assert!(brotli_small < 1024 * 1024, "brotli on a small body needs {brotli_small} bytes; Node's fixed 4 MiB window is not used");
    // A 5 MiB body is never held as a whole: gzip and deflate stay far below its size, and brotli
    // needs its 4 MiB window plus tables (a stream of unknown length uses a 1 MiB window).
    for (path, accept, peak) in &figures {
        if *path != "/small" {
            let limit = if *accept == "br" { 16 * 1024 * 1024 } else { 5 * 1024 * 1024 };
            assert!(*peak < limit, "{path} {accept}: {peak}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
