//! The scenarios of `auth/sessions` that the recorded fixture does not cover: expiry, limits, store failures, the
//! lifecycle races, cookie handling, startup checks and how the session user meets the guards. Time moves with
//! tokio's paused clock, so no test sleeps.
#![cfg(feature = "auth-session")]

mod session_common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use corvis::auth::session::{SameSite, SessionCookie, SessionModule, SessionSerializer, SessionStore, SessionStoreError, SessionStrategy};
use corvis::auth::{AuthGuard, Authenticator};
use corvis::http::CookieKeys;
use corvis::prelude::*;
use corvis::testing::{Test, TestResponse, request};
use session_common::*;

async fn send(app: &CorvisApplication, method: &str, path: &str, cookie: Option<&str>, headers: &[(&str, &str)]) -> TestResponse {
    let client = request(app);
    let mut call = match method {
        "POST" => client.post(path),
        _ => client.get(path),
    };
    if let Some(cookie) = cookie {
        call = call.header("cookie", cookie);
    }
    for (k, v) in headers {
        call = call.header(k, v);
    }
    call.send().await
}

async fn get(app: &CorvisApplication, path: &str, cookie: Option<&str>) -> TestResponse {
    send(app, "GET", path, cookie, &[]).await
}

async fn post(app: &CorvisApplication, path: &str, cookie: Option<&str>) -> TestResponse {
    send(app, "POST", path, cookie, &[]).await
}

/// The `name=value` pair of the first session cookie a response sets.
fn cookie_of(r: &TestResponse) -> String {
    pair(&set_cookies(r)[0])
}

/// Logs in as account `id` and returns the session cookie.
async fn login(app: &CorvisApplication, id: &str) -> String {
    let r = send(app, "POST", "/login", None, &[("x-user", id)]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    cookie_of(&r)
}

fn data_of(r: &TestResponse) -> Value {
    r.json_value()["data"].clone()
}

// ---- the cookie and what it carries ----

#[tokio::test]
async fn a_value_is_kept_between_requests() {
    let app = app(options()).await;
    let r = post(&app, "/set?k=views&v=1", None).await;
    let cookie = cookie_of(&r);
    assert_eq!(data_of(&get(&app, "/sess", Some(&cookie)).await), json!({ "views": "1" }));
}

#[tokio::test]
async fn an_anonymous_request_creates_nothing() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let r = get(&app, "/sess", None).await;
    assert_eq!(r.status, 200);
    assert!(set_cookies(&r).is_empty());
    assert_eq!(store.len(), 0);
    assert!(store.take_ops().is_empty());
}

#[tokio::test]
async fn invalid_cookies_are_ignored_without_touching_the_store() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let cookie = cookie_of(&post(&app, "/set?k=a&v=1", None).await);
    store.take_ops();
    let id = id_of(&cookie);
    let keys = CookieKeys::new([NEW]).unwrap();
    let other = CookieKeys::new(["a-completely-different-32-byte-secret!!"]).unwrap();
    let encode = corvis::http::res::encode_uri_component;
    let tampered = format!("{}{}", &cookie[..cookie.len() - 1], if cookie.ends_with('A') { 'B' } else { 'A' });
    let cases = [
        ("tampered signature", tampered),
        ("unsigned identifier", format!("connect.sid={id}")),
        ("signed with another secret", format!("connect.sid={}", encode(&other.sign(&id)))),
        ("empty value", "connect.sid=".to_string()),
        ("only the prefix", "connect.sid=s%3A".to_string()),
        ("identifier with a space", format!("connect.sid={}", encode(&keys.sign("a b")))),
        ("identifier with a control character", format!("connect.sid={}", encode(&keys.sign("a\u{1}b")))),
        ("identifier with non-ASCII", format!("connect.sid={}", encode(&keys.sign("caf\u{e9}")))),
        ("identifier over 128 bytes", format!("connect.sid={}", encode(&keys.sign(&"a".repeat(129))))),
        ("a 5000-byte value", format!("connect.sid={}", encode(&format!("s:{}", "a".repeat(5000))))),
        ("a 20000-byte value", format!("connect.sid={}", "a".repeat(20000))),
    ];
    for (name, cookie) in cases {
        let r = get(&app, "/sess", Some(&cookie)).await;
        assert_eq!(r.status, 200, "{name}");
        assert_eq!(data_of(&r), json!({}), "{name}");
        assert!(set_cookies(&r).is_empty(), "{name}");
        assert!(store.take_ops().is_empty(), "{name}: the store must not be read");
    }
}

#[tokio::test]
async fn a_signed_but_unknown_identifier_is_never_adopted() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let keys = CookieKeys::new([NEW]).unwrap();
    let planted = "plantedPLANTEDplantedPLANTED0001";
    let cookie = format!("connect.sid={}", corvis::http::res::encode_uri_component(&keys.sign(planted)));
    let r = post(&app, "/set?k=a&v=1", Some(&cookie)).await;
    let fresh = id_of(&cookie_of(&r));
    assert_ne!(fresh, planted);
    assert!(store.inner.load(planted).await.unwrap().is_none());
    assert!(store.inner.load(&fresh).await.unwrap().is_some());
}

#[tokio::test]
async fn two_session_cookies_are_not_believed() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let mine = cookie_of(&post(&app, "/set?k=owner&v=me", None).await);
    let theirs = cookie_of(&post(&app, "/set?k=owner&v=attacker", None).await);
    for header in [format!("{mine}; {theirs}"), format!("{theirs}; {mine}"), format!("{mine}; {mine}")] {
        store.take_ops();
        let r = get(&app, "/sess", Some(&header)).await;
        assert_eq!(data_of(&r), json!({}), "{header}");
        assert!(store.take_ops().is_empty(), "{header}: nothing is loaded");
    }
    // One cookie still works, next to cookies of other names.
    let r = get(&app, "/sess", Some(&format!("other=1; {mine}; x=2"))).await;
    assert_eq!(data_of(&r), json!({ "owner": "me" }));
}

#[tokio::test]
async fn the_default_cookie_attributes_and_the_configured_ones() {
    let app1 = app(options()).await;
    let header = set_cookies(&post(&app1, "/set?k=a&v=1", None).await).remove(0);
    assert_eq!(header.split("; ").skip(1).collect::<Vec<_>>(), ["Path=/", "HttpOnly", "Secure", "SameSite=Lax"]);
    assert!(header.starts_with("connect.sid=s%3A"));

    let configured =
        options().name("__Host-sid").cookie(SessionCookie::default().same_site(SameSite::Strict).max_age(Duration::from_secs(3600)));
    let app2 = app(configured).await;
    let header = set_cookies(&post(&app2, "/set?k=a&v=1", None).await).remove(0);
    let attrs: Vec<&str> = header.split("; ").skip(1).collect();
    assert_eq!(attrs.len(), 5, "{header}");
    assert_eq!(attrs[0], "Path=/");
    assert!(attrs[1].starts_with("Expires="), "{header}");
    assert_eq!(&attrs[2..], ["HttpOnly", "Secure", "SameSite=Strict"]);
    assert!(header.starts_with("__Host-sid="));

    let domain = options().cookie(SessionCookie::default().domain("example.com").http_only(false));
    let header = set_cookies(&post(&app(domain).await, "/set?k=a&v=1", None).await).remove(0);
    assert!(header.contains("Domain=example.com") && !header.contains("HttpOnly"), "{header}");
}

#[tokio::test]
async fn the_secure_cookie_is_sent_over_plain_http() {
    // The test client is plain HTTP and the default cookie is Secure.
    let r = post(&app(options()).await, "/set?k=a&v=1", None).await;
    assert!(set_cookies(&r)[0].contains("; Secure"));
}

#[tokio::test]
async fn a_request_outside_the_cookie_path_has_no_session() {
    let app = app(options().cookie(SessionCookie::default().path("/app"))).await;
    assert_eq!(get(&app, "/has-session", None).await.json_value(), json!({ "has": false }));
}

#[tokio::test]
async fn request_session_is_none_without_the_module() {
    #[controller("")]
    pub struct Plain;
    #[routes]
    impl Plain {
        #[get("has-session")]
        fn has_session(&self, req: Request) -> Value {
            json!({ "has": req.session().is_some() })
        }
    }
    let module = Test::create_testing_module().with_logger(false).controller::<Plain>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    let r = get(&app, "/has-session", None).await;
    assert_eq!(r.json_value(), json!({ "has": false }));
}

#[tokio::test]
async fn a_cors_preflight_creates_no_session() {
    let store = RecordingStore::new();
    let module = Test::create_testing_module()
        .with_logger(false)
        .import_dynamic(SessionModule::for_root(options().store_arc(store.clone()).save_uninitialized(true)))
        .providers(vec![Provider::strategy::<HeaderStrategy>(), Provider::strategy::<SessionStrategy<Accounts>>()])
        .controller::<Routes>()
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    app.enable_cors(corvis::CorsOptions::new().reflect_origin());
    app.init().await.unwrap();
    let r =
        request(&app).options("/sess").header("origin", "https://example.com").header("access-control-request-method", "GET").send().await;
    assert!(set_cookies(&r).is_empty());
    assert_eq!(store.len(), 0);
    // With save_uninitialized the same app does create sessions for a plain request: the preflight was the exception.
    get(&app, "/sess", None).await;
    assert_eq!(store.len(), 1);
}

#[tokio::test]
async fn a_failed_route_still_updates_an_existing_session_and_a_new_one_is_not_stored() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let cookie = cookie_of(&post(&app, "/set?k=a&v=1", None).await);
    store.take_ops();
    // No such route: the chain returns an error, no cookie can be attached, the session is touched.
    let r = get(&app, "/nowhere", Some(&cookie)).await;
    assert_eq!(r.status, 404);
    assert!(set_cookies(&r).is_empty());
    assert_eq!(store.take_ops().iter().map(|o| o.op).collect::<Vec<_>>(), ["get", "touch"]);
    // A session that would be created is not: the client would never get its cookie.
    let app2 = app_save_uninitialized(&store).await;
    let before = store.len();
    assert_eq!(get(&app2, "/nowhere", None).await.status, 404);
    assert_eq!(store.len(), before);
}

async fn app_save_uninitialized(store: &Arc<RecordingStore>) -> CorvisApplication {
    app(options().store_arc(store.clone()).save_uninitialized(true)).await
}

#[tokio::test]
async fn reserved_keys_are_refused_and_hidden() {
    let session = {
        // A session handle can be built by the middleware only; read it through a request.
        let app = app(options()).await;
        let r = post(&app, "/set?k=a&v=1", None).await;
        assert_eq!(r.status, 200);
        app
    };
    // `/set` with a reserved key panics in the handler (unwrap of the error): the response is a 500, nothing stored.
    let r = post(&session, "/set?k=cookie&v=x", None).await;
    assert_eq!(r.status, 500);
    let r = post(&session, "/set?k=__corvis&v=x", None).await;
    assert_eq!(r.status, 500);
    assert!(set_cookies(&r).is_empty());
}

// ---- time ----

#[tokio::test(start_paused = true)]
async fn an_idle_session_expires_and_its_record_is_destroyed() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let cookie = cookie_of(&post(&app, "/set?k=a&v=1", None).await);
    let id = id_of(&cookie);
    tokio::time::advance(Duration::from_secs(24 * 3600) + Duration::from_secs(1)).await;
    let r = get(&app, "/sess", Some(&cookie)).await;
    assert_eq!(data_of(&r), json!({}));
    assert!(store.inner.load(&id).await.unwrap().is_none(), "the old record is gone");

    // A store that keeps the record: the middleware destroys it and tells the browser to drop the cookie.
    let store = Arc::new(ForeverStore::default());
    let app = app_forever(&store).await;
    let cookie = cookie_of(&post(&app, "/set?k=a&v=1", None).await);
    tokio::time::advance(Duration::from_secs(24 * 3600) + Duration::from_secs(1)).await;
    let r = get(&app, "/sess", Some(&cookie)).await;
    assert_eq!(data_of(&r), json!({}));
    assert!(store.records.lock().unwrap().is_empty());
    let cleared = set_cookies(&r);
    assert_eq!(cleared.len(), 1);
    assert!(cleared[0].starts_with("connect.sid=;"), "{}", cleared[0]);
}

async fn app_forever(store: &Arc<ForeverStore>) -> CorvisApplication {
    app(options().store_arc(store.clone())).await
}

#[tokio::test(start_paused = true)]
async fn activity_extends_the_idle_time_only() {
    let app = app(options()).await;
    let cookie = cookie_of(&post(&app, "/set?k=a&v=1", None).await);
    for _ in 0..2 {
        tokio::time::advance(Duration::from_secs(23 * 3600)).await;
        assert_eq!(data_of(&get(&app, "/sess", Some(&cookie)).await), json!({ "a": "1" }));
    }
}

#[tokio::test(start_paused = true)]
async fn an_active_session_reaches_the_absolute_timeout() {
    // Rolling and used every hour: the idle timeout never fires, the absolute one does.
    let app = app(options().rolling(true)).await;
    let cookie = cookie_of(&post(&app, "/set?k=a&v=1", None).await);
    for hour in 1..=168 {
        tokio::time::advance(Duration::from_secs(3600)).await;
        assert_eq!(data_of(&get(&app, "/sess", Some(&cookie)).await), json!({ "a": "1" }), "hour {hour}");
    }
    tokio::time::advance(Duration::from_secs(3600)).await;
    assert_eq!(data_of(&get(&app, "/sess", Some(&cookie)).await), json!({}), "7 days and one hour");
}

#[tokio::test(start_paused = true)]
async fn rolling_renewal_does_not_extend_the_cookie_past_the_absolute_limit() {
    let app = app(options().rolling(true).cookie(SessionCookie::default().max_age(Duration::from_secs(7 * 24 * 3600)))).await;
    let first = set_cookies(&post(&app, "/set?k=a&v=1", None).await).remove(0);
    let cookie = pair(&first);
    for _ in 0..6 {
        tokio::time::advance(Duration::from_secs(23 * 3600)).await;
        get(&app, "/sess", Some(&cookie)).await;
    }
    let r = get(&app, "/sess", Some(&cookie)).await;
    let renewed = set_cookies(&r).remove(0);
    let expires = |h: &str| {
        h.split("; ").find_map(|a| a.strip_prefix("Expires=")).map(|d| chrono::DateTime::parse_from_rfc2822(d).unwrap().timestamp())
    };
    // Seven days after creation, not seven days after the renewal.
    let created = expires(&first).unwrap();
    assert!((expires(&renewed).unwrap() - created).abs() <= 1, "{first} then {renewed}");
}

/// A store that keeps every record forever and ignores the time to live.
#[derive(Default)]
struct ForeverStore {
    records: Mutex<HashMap<String, Value>>,
}

#[async_trait]
impl SessionStore for ForeverStore {
    async fn load(&self, id: &str) -> Result<Option<Value>, SessionStoreError> {
        Ok(self.records.lock().unwrap().get(id).cloned())
    }
    async fn create(&self, id: &str, record: &Value, _ttl: Duration) -> Result<(), SessionStoreError> {
        let mut records = self.records.lock().unwrap();
        if records.contains_key(id) {
            return Err(SessionStoreError::Exists);
        }
        records.insert(id.to_string(), record.clone());
        Ok(())
    }
    async fn update(&self, id: &str, record: &Value, _ttl: Duration) -> Result<bool, SessionStoreError> {
        let mut records = self.records.lock().unwrap();
        Ok(records.get_mut(id).map(|slot| *slot = record.clone()).is_some())
    }
    async fn destroy(&self, id: &str) -> Result<(), SessionStoreError> {
        self.records.lock().unwrap().remove(id);
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn expiry_does_not_depend_on_the_store_honoring_the_ttl() {
    let store = Arc::new(ForeverStore::default());
    let app = app(options().store_arc(store.clone())).await;
    let cookie = cookie_of(&post(&app, "/set?k=a&v=1", None).await);
    tokio::time::advance(Duration::from_secs(24 * 3600) + Duration::from_secs(1)).await;
    assert_eq!(data_of(&get(&app, "/sess", Some(&cookie)).await), json!({}));
    assert!(store.records.lock().unwrap().is_empty(), "the stale record was destroyed");
}

#[tokio::test(start_paused = true)]
async fn a_record_without_timestamps_is_accepted_and_given_them() {
    let store = RecordingStore::new();
    let record = json!({ "cookie": { "originalMaxAge": null, "expires": null, "httpOnly": true, "path": "/" }, "views": "3" });
    store.inner.create("writtenByTheOriginal000000000001", &record, Duration::from_secs(3600)).await.unwrap();
    let app = app(options().store_arc(store.clone())).await;
    let keys = CookieKeys::new([NEW]).unwrap();
    let cookie = format!("connect.sid={}", corvis::http::res::encode_uri_component(&keys.sign("writtenByTheOriginal000000000001")));
    assert_eq!(data_of(&get(&app, "/sess", Some(&cookie)).await), json!({ "views": "3" }));
    let held = store.inner.load("writtenByTheOriginal000000000001").await.unwrap().unwrap();
    let created = held["__corvis"]["createdAt"].as_i64().unwrap();
    // The absolute period starts at the first load.
    tokio::time::advance(Duration::from_secs(8 * 24 * 3600)).await;
    assert_eq!(data_of(&get(&app, "/sess", Some(&cookie)).await), json!({}));
    assert!(created > 0);
}

#[tokio::test(start_paused = true)]
async fn records_with_unusable_timestamps_or_an_expired_cookie_are_refused() {
    let store = RecordingStore::new();
    let keys = CookieKeys::new([NEW]).unwrap();
    let cookie_for = |id: &str| format!("connect.sid={}", corvis::http::res::encode_uri_component(&keys.sign(id)));
    let cookie = json!({ "originalMaxAge": null, "expires": null, "httpOnly": true, "path": "/" });
    let records = [
        ("badTimestampsBadTimestamps000001", json!({ "cookie": cookie, "a": 1, "__corvis": { "createdAt": "x", "lastAccess": 1 } })),
        ("metaNotAnObjectMetaNotAnObject01", json!({ "cookie": cookie, "a": 1, "__corvis": 5 })),
        (
            "expiredCookieExpiredCookie000001",
            json!({ "cookie": { "originalMaxAge": 1000, "expires": "2001-01-01T00:00:00.000Z", "httpOnly": true, "path": "/" }, "a": 1 }),
        ),
        (
            "unparseableExpiresUnparseable001",
            json!({ "cookie": { "originalMaxAge": 1000, "expires": "soon", "httpOnly": true, "path": "/" }, "a": 1 }),
        ),
    ];
    let app = app(options().store_arc(store.clone())).await;
    for (id, record) in records {
        store.inner.create(id, &record, Duration::from_secs(3600)).await.unwrap();
        assert_eq!(data_of(&get(&app, "/sess", Some(&cookie_for(id))).await), json!({}), "{id}");
        assert!(store.inner.load(id).await.unwrap().is_none(), "{id} was destroyed");
    }
    // A timestamp from the future counts as now.
    let now = chrono::Utc::now().timestamp_millis();
    let id = "futureTimestampsFutureTimestamp1";
    let record = json!({ "cookie": cookie, "a": 1, "__corvis": { "createdAt": now + 10 * 24 * 3600 * 1000, "lastAccess": now + 10 * 24 * 3600 * 1000 } });
    store.inner.create(id, &record, Duration::from_secs(3600)).await.unwrap();
    assert_eq!(data_of(&get(&app, "/sess", Some(&cookie_for(id))).await), json!({ "a": 1 }));
    tokio::time::advance(Duration::from_secs(8 * 24 * 3600)).await;
    assert_eq!(data_of(&get(&app, "/sess", Some(&cookie_for(id))).await), json!({}), "it cannot outlive the absolute timeout");
}

// ---- limits and store failures ----

#[tokio::test]
async fn an_oversized_session_is_a_500_and_is_not_stored() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let r = post(&app, "/big", None).await;
    assert_eq!(r.status, 500);
    assert_eq!(r.json_value(), json!({ "statusCode": 500, "message": "Internal server error" }));
    assert!(set_cookies(&r).is_empty());
    assert_eq!(store.len(), 0);
    // A larger limit lets it through.
    let store = RecordingStore::new();
    let app = app_big(&store).await;
    assert_eq!(post(&app, "/big", None).await.status, 200);
    assert_eq!(store.len(), 1);
}

async fn app_big(store: &Arc<RecordingStore>) -> CorvisApplication {
    app(options().store_arc(store.clone()).max_session_bytes(64 * 1024)).await
}

#[tokio::test]
async fn a_store_failure_is_the_same_500_for_every_session() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let a = cookie_of(&post(&app, "/set?k=a&v=1", None).await);
    let b = cookie_of(&post(&app, "/set?k=b&v=2", None).await);
    *store.fail_load.lock().unwrap() = true;
    let ra = get(&app, "/sess", Some(&a)).await;
    let rb = get(&app, "/sess", Some(&b)).await;
    assert_eq!((ra.status, rb.status), (500, 500));
    assert_eq!(ra.body, rb.body);
    assert_eq!(ra.json_value(), json!({ "statusCode": 500, "message": "Internal server error" }));
    let text = ra.text();
    assert!(!text.contains(&id_of(&a)) && !text.contains("SECRET-ID") && !text.contains("connect"), "{text}");
    // A request without a cookie never reads the store and is not an error.
    assert_eq!(get(&app, "/sess", None).await.status, 200);

    *store.fail_load.lock().unwrap() = false;
    *store.fail_writes.lock().unwrap() = true;
    // A write that fails after the handler ran is a 500 and sends no cookie.
    let r = post(&app, "/set?k=c&v=3", None).await;
    assert_eq!(r.status, 500);
    assert!(set_cookies(&r).is_empty());
    // An existing session whose touch fails is a 500 as well, never an anonymous 200.
    assert_eq!(get(&app, "/sess", Some(&a)).await.status, 500);
}

#[tokio::test(start_paused = true)]
async fn every_store_call_is_bounded_in_time() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone()).store_timeout(Duration::from_millis(200))).await;
    let cookie = cookie_of(&post(&app, "/set?k=a&v=1", None).await);
    *store.delay.lock().unwrap() = Some(Duration::from_secs(30));
    let r = get(&app, "/sess", Some(&cookie)).await;
    assert_eq!(r.status, 500);
    assert_eq!(r.json_value(), json!({ "statusCode": 500, "message": "Internal server error" }));
}

// ---- lifecycle ----

#[tokio::test]
async fn a_destroyed_session_does_not_come_back() {
    let store = RecordingStore::new();
    let app = Arc::new(app(options().store_arc(store.clone())).await);
    let cookie = login(&app, "3").await;
    let old = id_of(&cookie);

    // Request A loads the logged-in session and waits inside its handler.
    let held = {
        let app = app.clone();
        let cookie = cookie.clone();
        tokio::spawn(async move { post(&app, "/held-set", Some(&cookie)).await })
    };
    gate().started.notified().await;
    // Request B logs out, which destroys the session.
    let out = post(&app, "/logout", Some(&cookie)).await;
    assert_eq!(out.status, 200);
    assert!(store.inner.load(&old).await.unwrap().is_none());
    // A goes on, modifies the session and finishes.
    gate().release.notify_one();
    let late = held.await.unwrap();
    assert_eq!(late.status, 200);
    assert!(set_cookies(&late).is_empty(), "no cookie for a session that is gone");
    assert!(store.inner.load(&old).await.unwrap().is_none(), "the old identifier is still absent");
    assert_eq!(get(&app, "/me", Some(&cookie)).await.status, 401);
}

#[tokio::test]
async fn a_logout_survives_a_client_that_goes_away() {
    let store = RecordingStore::new();
    let app = Arc::new(app(options().store_arc(store.clone())).await);
    let cookie = login(&app, "3").await;
    let old = id_of(&cookie);
    let held = {
        let app = app.clone();
        let cookie = cookie.clone();
        tokio::spawn(async move { post(&app, "/held-destroy", Some(&cookie)).await })
    };
    cancel_gate().started.notified().await;
    assert!(store.inner.load(&old).await.unwrap().is_some(), "not destroyed yet");
    // The connection drops: the request future is cancelled before it can commit.
    held.abort();
    let _ = held.await;
    for _ in 0..100 {
        if store.inner.load(&old).await.unwrap().is_none() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(store.inner.load(&old).await.unwrap().is_none(), "the destruction was still applied");
}

#[tokio::test]
async fn a_login_regenerates_the_identifier_and_destroys_the_old_session() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let anon = cookie_of(&post(&app, "/set?k=cart&v=milk", None).await);
    let old = id_of(&anon);
    let r = send(&app, "POST", "/login", Some(&anon), &[("x-user", "3")]).await;
    assert_eq!(r.status, 200);
    let fresh = id_of(&cookie_of(&r));
    assert_ne!(fresh, old);
    assert!(store.inner.load(&old).await.unwrap().is_none());
    assert_eq!(get(&app, "/me", Some(&anon)).await.status, 401);
    // The data of before the login is gone.
    let record = store.inner.load(&fresh).await.unwrap().unwrap();
    assert!(record.get("cart").is_none(), "{record}");
    assert_eq!(record["passport"], json!({ "user": "3" }));
}

#[tokio::test]
async fn a_failed_login_leaves_the_session_alone() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let anon = cookie_of(&post(&app, "/set?k=cart&v=milk", None).await);
    let r = send(&app, "POST", "/login", Some(&anon), &[("x-user", "9")]).await;
    assert_eq!((r.status, r.json_value()), (401, json!({ "message": "Unauthorized", "statusCode": 401 })));
    assert!(set_cookies(&r).is_empty());
    assert_eq!(data_of(&get(&app, "/sess", Some(&anon)).await), json!({ "cart": "milk" }));
}

#[tokio::test]
async fn a_logout_destroys_the_session_and_gives_a_new_anonymous_one() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let cookie = login(&app, "4").await;
    let r = post(&app, "/logout", Some(&cookie)).await;
    let fresh = cookie_of(&r);
    assert_ne!(id_of(&fresh), id_of(&cookie));
    assert!(store.inner.load(&id_of(&cookie)).await.unwrap().is_none());
    assert_eq!(get(&app, "/me", Some(&cookie)).await.status, 401);
    assert_eq!(get(&app, "/me", Some(&fresh)).await.status, 401);
    // Logging out without a session creates nothing.
    let before = store.len();
    let r = post(&app, "/logout", None).await;
    assert_eq!(r.status, 200);
    assert!(set_cookies(&r).is_empty());
    assert_eq!(store.len(), before);
}

#[tokio::test]
async fn regenerate_and_destroy_from_a_handler() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let cookie = cookie_of(&post(&app, "/set?k=a&v=1", None).await);
    let r = post(&app, "/regenerate", Some(&cookie)).await;
    let fresh = cookie_of(&r);
    assert_ne!(id_of(&fresh), id_of(&cookie));
    assert!(store.inner.load(&id_of(&cookie)).await.unwrap().is_none());
    let r = post(&app, "/destroy", Some(&fresh)).await;
    assert!(set_cookies(&r)[0].starts_with("connect.sid=;"));
    assert!(store.inner.load(&id_of(&fresh)).await.unwrap().is_none());
}

#[tokio::test]
async fn the_cookie_moves_to_the_current_secret_after_a_rotation() {
    let store = RecordingStore::new();
    let old_app = app(SessionOptionsExt::with(OLD, store.clone())).await;
    let cookie = cookie_of(&post(&old_app, "/set?k=a&v=1", None).await);
    let rotated = app(corvis::auth::session::SessionOptions::with_secrets([NEW, OLD]).unwrap().store_arc(store.clone())).await;
    let r = get(&rotated, "/sess", Some(&cookie)).await;
    assert_eq!(data_of(&r), json!({ "a": "1" }));
    let renewed = cookie_of(&r);
    assert_ne!(renewed, cookie);
    assert_eq!(id_of(&renewed), id_of(&cookie));
    // Once the old secret is gone only the renewed cookie works.
    let only_new = app(options().store_arc(store.clone())).await;
    assert_eq!(data_of(&get(&only_new, "/sess", Some(&cookie)).await), json!({}));
    assert_eq!(data_of(&get(&only_new, "/sess", Some(&renewed)).await), json!({ "a": "1" }));
}

struct SessionOptionsExt;
impl SessionOptionsExt {
    fn with(secret: &str, store: Arc<RecordingStore>) -> corvis::auth::session::SessionOptions {
        corvis::auth::session::SessionOptions::with_secrets([secret]).unwrap().store_arc(store)
    }
}

// ---- the session user and the guards ----

#[tokio::test]
async fn the_session_user_reaches_a_guard_and_the_handler() {
    let app = app(options()).await;
    let cookie = login(&app, "3").await;
    let r = get(&app, "/me", Some(&cookie)).await;
    assert_eq!(r.json_value(), json!({ "user": { "id": "3", "name": "cy" } }));
    // A route without any guard still sees the user, as `passport.session()` provides `req.user` everywhere.
    assert_eq!(get(&app, "/who", Some(&cookie)).await.json_value(), json!({ "user": { "id": "3", "name": "cy" } }));
}

#[tokio::test]
async fn an_anonymous_request_to_a_protected_route_is_a_401() {
    let app = app(options()).await;
    let r = get(&app, "/me", None).await;
    assert_eq!((r.status, r.json_value()), (401, json!({ "message": "Unauthorized", "statusCode": 401 })));
}

#[tokio::test]
async fn a_deleted_user_is_anonymous_and_leaves_the_stored_session() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let cookie = login(&app, "2").await;
    deleted().lock().unwrap().insert("2".to_string());
    let r = get(&app, "/me", Some(&cookie)).await;
    deleted().lock().unwrap().remove("2");
    assert_eq!(r.status, 401);
    let record = store.inner.load(&id_of(&cookie)).await.unwrap().unwrap();
    assert_eq!(record["passport"], json!({}));
}

#[tokio::test]
async fn a_serializer_error_is_the_response() {
    let app = app(options()).await;
    let cookie = login(&app, "3").await;
    let r = send(&app, "GET", "/me", Some(&cookie), &[("x-lookup-fails", "1")]).await;
    assert_eq!(r.status, 503);
    // Without a user in the session the serializer is not asked.
    let r = send(&app, "GET", "/sess", None, &[("x-lookup-fails", "1")]).await;
    assert_eq!(r.status, 200);
}

#[tokio::test]
async fn an_optional_guard_of_another_strategy_clears_the_session_user_and_a_chain_keeps_it() {
    let app = app(options()).await;
    let cookie = login(&app, "3").await;
    // The optional header guard finds no credentials: like `req.user = null` in the original, the user is dropped.
    let r = get(&app, "/optional", Some(&cookie)).await;
    assert_eq!(r.json_value(), json!({ "user": null }));
    // Naming the session strategy first keeps it.
    let r = get(&app, "/chain", Some(&cookie)).await;
    assert_eq!(r.json_value(), json!({ "user": { "id": "3", "name": "cy" } }));
    // And a token still works for a request with no session.
    let r = send(&app, "GET", "/chain", None, &[("x-user", "1")]).await;
    assert_eq!(r.json_value(), json!({ "user": { "id": "1", "name": "ana" } }));
    // A public route that skips the guard keeps the session user.
    let r = get(&app, "/public-who", Some(&cookie)).await;
    assert_eq!(r.json_value(), json!({ "user": { "id": "3", "name": "cy" } }));
}

#[tokio::test]
async fn a_guard_without_log_in_never_writes_the_session() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let r = send(&app, "GET", "/header-only", None, &[("x-user", "1")]).await;
    assert_eq!(r.status, 200);
    assert!(set_cookies(&r).is_empty());
    assert_eq!(store.len(), 0);
}

#[tokio::test]
async fn a_stored_empty_user_is_never_restored() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let keys = CookieKeys::new([NEW]).unwrap();
    for (n, empty) in [json!(null), json!(false), json!(0), json!(0.0), json!("")].into_iter().enumerate() {
        let id = format!("emptyUserEmptyUserEmptyUser{n:05}");
        let record =
            json!({ "cookie": { "originalMaxAge": null, "expires": null, "httpOnly": true, "path": "/" }, "passport": { "user": empty } });
        store.inner.create(&id, &record, Duration::from_secs(3600)).await.unwrap();
        let cookie = format!("connect.sid={}", corvis::http::res::encode_uri_component(&keys.sign(&id)));
        assert_eq!(get(&app, "/me", Some(&cookie)).await.status, 401, "{empty}");
        assert_eq!(get(&app, "/who", Some(&cookie)).await.json_value(), json!({ "user": null }), "{empty}");
        let held = store.inner.load(&id).await.unwrap().unwrap();
        assert_eq!(held["passport"], json!({}), "{empty} is removed from the session");
    }
}

/// A serializer that returns an empty value for the user (and one that restores an empty user).
#[injectable]
pub struct NullSerializer;

#[async_trait]
impl SessionSerializer for NullSerializer {
    type User = Account;
    async fn serialize_user(&self, _: &Account) -> Result<Value, HttpException> {
        Ok(Value::Null)
    }
    async fn deserialize_user(&self, _: &Request, _: Value) -> Result<Option<Account>, HttpException> {
        Ok(None)
    }
}

#[tokio::test]
async fn logging_in_with_an_empty_stored_value_is_refused_and_stores_nothing() {
    let store = RecordingStore::new();
    let module = Test::create_testing_module()
        .with_logger(false)
        .import_dynamic(SessionModule::for_root(options().store_arc(store.clone())))
        .providers(vec![Provider::strategy::<HeaderStrategy>(), Provider::strategy::<SessionStrategy<NullSerializer>>()])
        .controller::<Routes>()
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    let r = send(&app, "POST", "/login", None, &[("x-user", "1")]).await;
    assert_eq!(r.status, 500);
    assert!(set_cookies(&r).is_empty());
    assert_eq!(store.len(), 0);
}

#[tokio::test]
async fn log_in_and_log_out_without_session_support_are_a_fixed_500() {
    use corvis::auth::AuthenticatedUser;
    let module = Test::create_testing_module().with_logger(false).controller::<Routes>().compile().await.unwrap();
    let authenticator = module.get::<Authenticator>().unwrap();
    let req = corvis::http::Request::new(corvis::http::Method::GET, "/".parse().unwrap(), Default::default(), Default::default()).unwrap();
    let user = AuthenticatedUser::new(account("1").unwrap()).unwrap();
    let e = authenticator.log_in(&req, &user).await.unwrap_err();
    assert_eq!(e.status().as_u16(), 500);
    assert_eq!(e.get_response(), &json!({ "statusCode": 500, "message": "Internal server error" }));
    let e = authenticator.log_out(&req).await.unwrap_err();
    assert_eq!(e.status().as_u16(), 500);
    assert!(req.user().is_none());
}

// ---- startup ----

async fn init_error(options: corvis::auth::session::SessionOptions) -> String {
    let module = Test::create_testing_module()
        .with_logger(false)
        .import_dynamic(SessionModule::for_root(options))
        .providers(vec![Provider::strategy::<HeaderStrategy>(), Provider::strategy::<SessionStrategy<Accounts>>()])
        .controller::<Routes>()
        .compile()
        .await;
    match module {
        Err(e) => format!("{e:#}"),
        Ok(module) => {
            let mut app = module.create_corvis_application();
            app.init().await.err().map(|e| format!("{e:#}")).unwrap_or_default()
        }
    }
}

#[tokio::test]
async fn unsafe_cookie_configurations_stop_the_application() {
    let e = init_error(options().cookie(SessionCookie::default().same_site(SameSite::None).secure(false))).await;
    assert!(e.contains("same_site") && e.contains("secure"), "{e}");
    let e = init_error(options().name("__Host-sid").cookie(SessionCookie::default().domain("example.com"))).await;
    assert!(e.contains("__Host-"), "{e}");
    let e = init_error(options().name("__Secure-sid").cookie(SessionCookie::default().secure(false))).await;
    assert!(e.contains("__Secure-"), "{e}");
    let e = init_error(options().name("a;b")).await;
    assert!(e.contains("cookie name"), "{e}");
    let e = init_error(options().idle_timeout(Duration::from_secs(10))).await;
    assert!(e.contains("idle_timeout"), "{e}");
    let e = init_error(options().cookie(SessionCookie::default().max_age(Duration::from_secs(8 * 24 * 3600)))).await;
    assert!(e.contains("max_age"), "{e}");
    let e = init_error(options().max_session_bytes(10)).await;
    assert!(e.contains("max_session_bytes"), "{e}");
    let e = init_error(options().store(corvis::auth::session::MemorySessionStore::new().max_sessions(0))).await;
    assert!(e.contains("max_sessions"), "{e}");
}

#[tokio::test]
async fn a_short_secret_stops_the_application_without_printing_it() {
    let module = Test::create_testing_module()
        .with_logger(false)
        .import_dynamic(SessionModule::for_root_async(|_| async {
            Ok(corvis::auth::session::SessionOptions::with_secrets(["twenty-bytes-secret!!"])?)
        }))
        .providers(vec![Provider::strategy::<SessionStrategy<Accounts>>()])
        .compile()
        .await;
    let e = format!("{:#}", module.err().expect("startup fails"));
    assert!(e.contains("session secret") && e.contains("32"), "{e}");
    assert!(!e.contains("twenty-bytes"), "{e}");
}

#[tokio::test]
async fn disabling_the_absolute_timeout_is_explicit() {
    assert_eq!(init_error(options().no_absolute_timeout()).await, "");
}

#[tokio::test]
async fn log_in_without_a_session_strategy_stops_the_application() {
    let module = Test::create_testing_module()
        .with_logger(false)
        .import_dynamic(SessionModule::for_root(options()))
        .providers(vec![Provider::strategy::<HeaderStrategy>()])
        .controller::<Routes>()
        .compile()
        .await;
    let e = match module {
        Err(e) => format!("{e:#}"),
        Ok(m) => format!("{:#}", m.create_corvis_application().init().await.unwrap_err()),
    };
    assert!(e.contains("POST /login") && e.contains("session strategy"), "{e}");
    // The `session` strategy itself is not registered either, so the protected route is named too.
    assert!(e.contains("\"session\""), "{e}");
}

#[injectable]
pub struct OtherSerializer;

#[async_trait]
impl SessionSerializer for OtherSerializer {
    type User = Account;
    async fn serialize_user(&self, user: &Account) -> Result<Value, HttpException> {
        Ok(json!(user.id))
    }
    async fn deserialize_user(&self, _: &Request, _: Value) -> Result<Option<Account>, HttpException> {
        Ok(None)
    }
}

#[tokio::test]
async fn two_session_strategies_stop_the_application() {
    let module = Test::create_testing_module()
        .with_logger(false)
        .import_dynamic(SessionModule::for_root(options()))
        .providers(vec![Provider::strategy::<SessionStrategy<Accounts>>(), Provider::strategy::<SessionStrategy<OtherSerializer>>()])
        .compile()
        .await;
    let e = match module {
        Err(e) => format!("{e:#}"),
        Ok(m) => format!("{:#}", m.create_corvis_application().init().await.unwrap_err()),
    };
    assert!(e.contains("session strategy") || e.contains("two strategies"), "{e}");
}

#[tokio::test]
async fn a_login_guard_cannot_name_the_session_strategy() {
    #[controller("")]
    pub struct Bad;
    #[routes]
    impl Bad {
        #[post("bad")]
        #[use_guards(AuthGuard::new("session").log_in())]
        fn bad(&self) -> Value {
            json!({})
        }
    }
    let module = Test::create_testing_module()
        .with_logger(false)
        .import_dynamic(SessionModule::for_root(options()))
        .providers(vec![Provider::strategy::<SessionStrategy<Accounts>>()])
        .controller::<Bad>()
        .compile()
        .await;
    let e = match module {
        Err(e) => format!("{e:#}"),
        Ok(m) => format!("{:#}", m.create_corvis_application().init().await.unwrap_err()),
    };
    assert!(e.contains("cannot be combined"), "{e}");
}

// ---- API ----

#[tokio::test]
async fn debug_output_holds_neither_identifier_nor_data() {
    #[controller("")]
    pub struct Dbg;
    #[routes]
    impl Dbg {
        #[get("dbg")]
        fn dbg(&self, req: Request) -> Value {
            let session = req.session().unwrap();
            session.set("secret-data", "hunter2").unwrap();
            json!({ "debug": format!("{session:?}"), "id": session.id() })
        }
    }
    let module = Test::create_testing_module()
        .with_logger(false)
        .import_dynamic(SessionModule::for_root(options()))
        .controller::<Dbg>()
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    let r = get(&app, "/dbg", None).await.json_value();
    let debug = r["debug"].as_str().unwrap();
    assert!(!debug.contains(r["id"].as_str().unwrap()) && !debug.contains("hunter2"), "{debug}");
    let manager = module_debug(&app);
    assert!(!manager.contains(NEW), "{manager}");
}

fn module_debug(app: &CorvisApplication) -> String {
    format!("{:?}", app.get::<corvis::auth::session::SessionManager>().unwrap())
}

#[tokio::test]
async fn a_cookie_over_the_limit_after_percent_decoding_is_ignored() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    // 4097 decoded bytes in 4097 * 3 raw ones.
    let value = format!("s%3A{}", "%61".repeat(4100));
    let r = get(&app, "/sess", Some(&format!("connect.sid={value}"))).await;
    assert_eq!(data_of(&r), json!({}));
    assert!(store.take_ops().is_empty());
    let _ = CookieKeys::new([NEW]);
}
