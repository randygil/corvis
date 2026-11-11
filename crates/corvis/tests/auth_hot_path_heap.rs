//! What the session middleware allocates for a session it does not change, and the record-sized buffers of one update,
//! measured with a counting allocator (a clock would make the test depend on the machine). The store's own work (the
//! parse of a record on load, the text it keeps on update) is measured inside the store and taken out, and so is
//! everything a request costs without the session middleware (a route outside the cookie path). Its own test binary,
//! because the allocator counts the whole process.
#![cfg(feature = "auth-session")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use corvis::auth::session::{MemorySessionStore, SessionCookie, SessionModule, SessionOptions, SessionStore, SessionStoreError};
use corvis::prelude::*;
use corvis::testing::{Test, request};

thread_local! {
    /// Bytes requested from the allocator by this thread.
    static BYTES: Cell<usize> = const { Cell::new(0) };
    /// Allocations of this thread of at least `LARGE.get()` bytes (0: not counted).
    static LARGE: Cell<usize> = const { Cell::new(0) };
    static LARGE_COUNT: Cell<usize> = const { Cell::new(0) };
}

fn note(size: usize) {
    let _ = BYTES.try_with(|b| b.set(b.get() + size));
    let _ = LARGE.try_with(|l| {
        if l.get() > 0 && size >= l.get() {
            let _ = LARGE_COUNT.try_with(|c| c.set(c.get() + 1));
        }
    });
}

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn bytes() -> usize {
    BYTES.with(Cell::get)
}

fn large_count() -> usize {
    LARGE_COUNT.with(Cell::get)
}

/// The memory store, measuring what its own `load` and `update` allocate on this thread.
struct MeasuredStore {
    inner: MemorySessionStore,
    load_bytes: AtomicUsize,
    touch_bytes: AtomicUsize,
    update_large: AtomicUsize,
}

#[async_trait]
impl SessionStore for MeasuredStore {
    async fn load(&self, id: &str) -> Result<Option<Value>, SessionStoreError> {
        let before = bytes();
        let record = self.inner.load(id).await;
        self.load_bytes.store(bytes() - before, Ordering::SeqCst);
        record
    }
    async fn create(&self, id: &str, record: &Value, ttl: Duration) -> Result<(), SessionStoreError> {
        self.inner.create(id, record, ttl).await
    }
    async fn update(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError> {
        let before = large_count();
        let present = self.inner.update(id, record, ttl).await;
        self.update_large.store(large_count() - before, Ordering::SeqCst);
        present
    }
    async fn touch(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError> {
        let before = bytes();
        let present = self.inner.touch(id, record, ttl).await;
        self.touch_bytes.store(bytes() - before, Ordering::SeqCst);
        present
    }
    async fn destroy(&self, id: &str) -> Result<bool, SessionStoreError> {
        self.inner.destroy(id).await
    }
}

/// About `kib` KiB of session data: small objects, as a real session holds.
fn items(kib: usize) -> Value {
    Value::Array(
        (0..kib * 8)
            .map(|i| json!({ "id": i, "name": format!("item-{i:05}"), "tags": ["alpha", "beta"], "note": "n".repeat(64) }))
            .collect(),
    )
}

#[controller("s")]
pub struct SessionRoutes;

#[routes]
impl SessionRoutes {
    #[post("seed/:kib")]
    #[http_code(200)]
    fn seed(&self, req: Request) -> Value {
        let kib: usize = req.param("kib").unwrap().parse().unwrap();
        let session = req.session().unwrap();
        session.set("items", items(kib)).unwrap();
        session.set("n", 0).unwrap();
        json!({ "ok": true })
    }

    #[get("read")]
    fn read(&self, req: Request) -> Value {
        json!({ "n": req.session().unwrap().get_value("n") })
    }

    #[post("write")]
    #[http_code(200)]
    fn write(&self, req: Request) -> Value {
        let session = req.session().unwrap();
        let n = session.get::<u64>("n").unwrap_or(0) + 1;
        session.set("n", n).unwrap();
        json!({ "ok": true })
    }
}

/// The same handler outside the cookie path: what a request costs without the session middleware.
#[controller("plain")]
pub struct PlainRoutes;

#[routes]
impl PlainRoutes {
    #[get("read")]
    fn read(&self) -> Value {
        json!({ "n": 0 })
    }
}

async fn app(store: Arc<MeasuredStore>) -> CorvisApplication {
    let options = SessionOptions::with_secrets(["0123456789abcdef0123456789abcdef"])
        .unwrap()
        .cookie(SessionCookie::default().path("/s"))
        .max_session_bytes(256 * 1024)
        .store_arc(store);
    let module = Test::create_testing_module()
        .with_logger(false)
        .import_dynamic(SessionModule::for_root(options))
        .controller::<SessionRoutes>()
        .controller::<PlainRoutes>()
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    app
}

fn store() -> Arc<MeasuredStore> {
    Arc::new(MeasuredStore {
        inner: MemorySessionStore::new(),
        load_bytes: AtomicUsize::new(0),
        touch_bytes: AtomicUsize::new(0),
        update_large: AtomicUsize::new(0),
    })
}

/// The session cookie of a session seeded with about `kib` KiB, and the length of its record.
async fn seeded(app: &CorvisApplication, store: &MeasuredStore, kib: usize) -> (String, usize) {
    let r = request(app).post(&format!("/s/seed/{kib}")).send().await;
    assert_eq!(r.status, 200, "{}", r.text());
    let cookie = r.headers.get("set-cookie").unwrap().to_str().unwrap().split(';').next().unwrap().to_string();
    let id = percent_encoding::percent_decode_str(cookie.split_once('=').unwrap().1).decode_utf8().unwrap().into_owned();
    let id = &id["s:".len()..id.rfind('.').unwrap()];
    let record = store.inner.load(id).await.unwrap().unwrap();
    (cookie, serde_json::to_vec(&record).unwrap().len())
}

/// The fewest bytes a GET of `path` with `cookie` allocated over a few runs (one-time allocations do not count), with
/// what the store's own load and touch allocated in the same run.
async fn least_bytes(app: &CorvisApplication, store: &MeasuredStore, path: &str, cookie: &str) -> (usize, usize) {
    let mut least = (usize::MAX, 0);
    for _ in 0..5 {
        store.load_bytes.store(0, Ordering::SeqCst);
        store.touch_bytes.store(0, Ordering::SeqCst);
        let before = bytes();
        let r = request(app).get(path).header("cookie", cookie).send().await;
        let total = bytes() - before;
        assert_eq!(r.status, 200, "{}", r.text());
        if total < least.0 {
            least = (total, store.load_bytes.load(Ordering::SeqCst) + store.touch_bytes.load(Ordering::SeqCst));
        }
    }
    least
}

/// Spec "Read-only request with a large session": beyond the store's parse of the record, the session middleware
/// allocates at most 4 KiB for a request that reads one value of a session of 16 KiB or 64 KiB and changes nothing.
#[tokio::test]
async fn an_unchanged_session_costs_the_middleware_at_most_4_kib() {
    for kib in [16, 64] {
        let store = store();
        let app = app(store.clone()).await;
        let (cookie, record_len) = seeded(&app, &store, kib).await;
        let (plain, _) = least_bytes(&app, &store, "/plain/read", &cookie).await;
        let (with_session, in_store) = least_bytes(&app, &store, "/s/read", &cookie).await;
        assert!(in_store > 0, "the store measured its load");
        let middleware = with_session.saturating_sub(plain).saturating_sub(in_store);
        println!(
            "{kib} KiB session ({record_len} bytes): request {with_session}, without the middleware {plain}, store load and touch {in_store}, middleware {middleware}"
        );
        assert!(middleware <= 4096, "{kib} KiB session ({record_len} bytes of record): the middleware allocated {middleware} bytes");
    }
}

/// One `update` serializes the record once, in the store: the middleware measures the record's size without a buffer
/// of that size.
#[tokio::test]
async fn an_update_allocates_no_record_sized_buffer_outside_the_store() {
    for kib in [16, 64] {
        let store = store();
        let app = app(store.clone()).await;
        let (cookie, record_len) = seeded(&app, &store, kib).await;
        for _ in 0..2 {
            LARGE.with(|l| l.set(record_len));
            let before = large_count();
            let r = request(&app).post("/s/write").header("cookie", &cookie).send().await;
            let total = large_count() - before;
            LARGE.with(|l| l.set(0));
            assert_eq!(r.status, 200, "{}", r.text());
            let in_store = store.update_large.load(Ordering::SeqCst);
            println!("{kib} KiB session ({record_len} bytes): record-sized buffers {total}, of which the store's {in_store}");
            assert!(in_store >= 1, "the store's own text of the record");
            assert_eq!(total - in_store, 0, "{kib} KiB session: {} record-sized buffer(s) outside the store", total - in_store);
        }
    }
}
