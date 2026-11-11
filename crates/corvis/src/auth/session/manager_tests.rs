//! The load and commit steps against the memory store: the size limit at its boundary, the identifier drawn only when
//! needed, and change detection. Identifier draws are counted, never timed.

use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;

use super::super::session::{FAIL_DRAWS, ID_DRAWS};
use super::super::store::SessionStore;
use super::super::{MemorySessionStore, SessionOptions};
use super::*;
use crate::http::res::encode_uri_component;

const SECRET: &str = "0123456789abcdef0123456789abcdef";

/// A memory store that remembers the operations it received and the records `touch` was given.
#[derive(Default)]
struct Recording {
    inner: MemorySessionStore,
    ops: Mutex<Vec<&'static str>>,
    touched: Mutex<Vec<Value>>,
}

#[async_trait]
impl SessionStore for Recording {
    async fn load(&self, id: &str) -> Result<Option<Value>, SessionStoreError> {
        self.inner.load(id).await
    }
    async fn create(&self, id: &str, record: &Value, ttl: Duration) -> Result<(), SessionStoreError> {
        self.ops.lock().unwrap().push("create");
        self.inner.create(id, record, ttl).await
    }
    async fn update(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError> {
        self.ops.lock().unwrap().push("update");
        self.inner.update(id, record, ttl).await
    }
    async fn touch(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError> {
        self.ops.lock().unwrap().push("touch");
        self.touched.lock().unwrap().push(record.clone());
        self.inner.touch(id, record, ttl).await
    }
    async fn destroy(&self, id: &str) -> Result<bool, SessionStoreError> {
        self.ops.lock().unwrap().push("destroy");
        self.inner.destroy(id).await
    }
}

fn manager(store: Arc<Recording>, max_bytes: usize) -> Arc<SessionManager> {
    let options = SessionOptions::with_secrets([SECRET]).unwrap().max_session_bytes(max_bytes).store_arc(store);
    Arc::new(SessionManager::new(options).unwrap())
}

fn request(cookie: Option<&str>) -> Request {
    let mut req = Request::empty();
    if let Some(cookie) = cookie {
        req.headers_mut().insert("cookie", cookie.parse().unwrap());
    }
    req
}

/// The `Cookie` header that names session `id`.
fn cookie_for(m: &SessionManager, id: &str) -> String {
    format!("{}={}", m.options.name, encode_uri_component(&m.options.keys.sign(id)))
}

/// The identifier in a `Set-Cookie` header of the session cookie.
fn id_in(set_cookie: &str) -> String {
    let value = set_cookie.split(';').next().unwrap().split_once('=').unwrap().1;
    let decoded = percent_encoding::percent_decode_str(value).decode_utf8().unwrap().into_owned();
    let rest = decoded.strip_prefix("s:").unwrap();
    rest[..rest.rfind('.').unwrap()].to_string()
}

fn draws() -> usize {
    ID_DRAWS.with(|c| c.get())
}

/// A stored record of exactly `len` serialized bytes, valid now.
fn record_of_len(len: usize) -> Value {
    let now = now_ms();
    let mut record =
        json!({ "cookie": { "originalMaxAge": null, "expires": null }, "pad": "", "__corvis": { "createdAt": now, "lastAccess": now } });
    let base = serde_json::to_vec(&record).unwrap().len();
    record["pad"] = json!("x".repeat(len - base));
    assert_eq!(serde_json::to_vec(&record).unwrap().len(), len);
    record
}

#[test]
fn the_counting_writer_measures_without_a_buffer() {
    let record = record_of_len(2000);
    assert!(fits(&record, 2000));
    assert!(!fits(&record, 1999));
    assert!(fits(&json!(null), 4));
    assert!(!fits(&json!(null), 3));
}

/// Spec "Size limit at the boundary": a stored record of exactly `max_session_bytes` loads, one byte more is no
/// session; a write of the same two sizes is saved and refused.
#[tokio::test]
async fn the_size_limit_holds_at_the_boundary() {
    let store = Arc::new(Recording::default());
    let m = manager(store.clone(), 1024);
    for (id, len, loads) in [("at-limit", 1024, true), ("one-more", 1025, false)] {
        store.inner.create(id, &record_of_len(len), Duration::from_secs(3600)).await.unwrap();
        let session = m.load(&request(Some(&cookie_for(&m, id)))).await.unwrap();
        assert_eq!(!session.is_new(), loads, "a stored record of {len} bytes");
    }
    assert!(m.check_writable(&record_of_len(1024)).is_ok());
    assert!(m.check_writable(&record_of_len(1025)).is_err());
}

/// Spec "Anonymous request": no identifier is drawn and no cookie is set for a request that saves nothing.
#[tokio::test]
async fn an_anonymous_request_draws_no_identifier() {
    let store = Arc::new(Recording::default());
    let m = manager(store.clone(), 16 * 1024);
    let before = draws();
    let session = m.load(&request(None)).await.unwrap();
    let _ = session.get_value("anything");
    assert_eq!(m.commit(&session, Some(200), false).await.unwrap(), None);
    assert_eq!(draws(), before);
    assert!(store.ops.lock().unwrap().is_empty());
}

/// Spec "First write": one identifier, of 32 base64url characters, and a cookie for it.
#[tokio::test]
async fn the_first_write_draws_one_identifier() {
    let store = Arc::new(Recording::default());
    let m = manager(store.clone(), 16 * 1024);
    let before = draws();
    let session = m.load(&request(None)).await.unwrap();
    session.set("k", 1).unwrap();
    let set_cookie = m.commit(&session, Some(200), false).await.unwrap().expect("a cookie is set");
    assert_eq!(draws() - before, 1);
    let id = id_in(&set_cookie);
    assert_eq!(id.len(), 32, "{id}");
    assert!(id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "{id}");
    assert!(store.inner.load(&id).await.unwrap().is_some());
    // The identifier a handler reads is the one that is saved.
    let session = m.load(&request(None)).await.unwrap();
    let read = session.id();
    session.set("k", 2).unwrap();
    let set_cookie = m.commit(&session, Some(200), false).await.unwrap().unwrap();
    assert_eq!(id_in(&set_cookie), read);
}

/// Spec "Failed draw": the 500 of a session failure, and nothing written.
#[tokio::test]
async fn a_failed_draw_at_commit_writes_nothing() {
    let store = Arc::new(Recording::default());
    let m = manager(store.clone(), 16 * 1024);
    let session = m.load(&request(None)).await.unwrap();
    session.set("k", 1).unwrap();
    FAIL_DRAWS.with(|f| f.set(true));
    let result = m.commit(&session, Some(200), false).await;
    let failed_id = session.id();
    FAIL_DRAWS.with(|f| f.set(false));
    let e = result.unwrap_err();
    assert_eq!(e.get_status(), 500);
    assert_eq!(failed_id, "", "a failed draw yields no identifier, never a weaker one");
    assert!(store.ops.lock().unwrap().is_empty(), "nothing was written");
    assert_eq!(store.inner.len(), 0);
    // A draw that failed when a handler asked for the identifier stops the write too.
    let session = m.load(&request(None)).await.unwrap();
    FAIL_DRAWS.with(|f| f.set(true));
    assert_eq!(session.id(), "");
    FAIL_DRAWS.with(|f| f.set(false));
    session.set("k", 1).unwrap();
    assert!(m.commit(&session, Some(200), false).await.is_err());
    assert_eq!(store.inner.len(), 0);
}

/// Spec "A value set to what it was": the session is touched, not updated, and `touch` gets the timestamps only.
#[tokio::test]
async fn a_value_set_to_what_it_was_is_a_touch() {
    let store = Arc::new(Recording::default());
    let m = manager(store.clone(), 16 * 1024);
    let session = m.load(&request(None)).await.unwrap();
    session.set("k", json!({ "a": [1, 2] })).unwrap();
    let id = id_in(&m.commit(&session, Some(200), false).await.unwrap().unwrap());
    store.ops.lock().unwrap().clear();

    let session = m.load(&request(Some(&cookie_for(&m, &id)))).await.unwrap();
    session.set("k", json!({ "a": [1, 2] })).unwrap();
    m.commit(&session, Some(200), false).await.unwrap();
    assert_eq!(*store.ops.lock().unwrap(), ["touch"]);
    let touched = store.touched.lock().unwrap().pop().unwrap();
    let keys: Vec<&str> = touched.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["cookie", "__corvis"]);

    // A real change is an update; a change undone in the same request is not.
    let session = m.load(&request(Some(&cookie_for(&m, &id)))).await.unwrap();
    session.set("k", 3).unwrap();
    m.commit(&session, Some(200), false).await.unwrap();
    let session = m.load(&request(Some(&cookie_for(&m, &id)))).await.unwrap();
    session.set("tmp", 1).unwrap();
    session.remove("tmp");
    m.commit(&session, Some(200), false).await.unwrap();
    assert_eq!(*store.ops.lock().unwrap(), ["touch", "update", "touch"]);
}
