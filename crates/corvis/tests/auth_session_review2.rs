//! Regression tests of the second security review of sessions: whether a guard login stands depends on what the
//! pipeline did (never on the status a filter or an interceptor gave the response), a flood that evicts the previous
//! session cannot make a login fail, and records the store cannot read back. Races are held with gates, never with a
//! sleep.
#![cfg(feature = "auth-session")]

mod session_common;

use std::sync::Arc;

use async_trait::async_trait;
use corvis::auth::session::{MemorySessionStore, SessionModule, SessionStore, SessionStrategy};
use corvis::auth::{AuthGuard, AuthenticatedUser, Authenticator};
use corvis::prelude::*;
use corvis::testing::{Test, TestResponse, request};
use session_common::*;

// ---- the filters and the interceptor of the reviewer's cases ----

/// The pattern of the documentation: a denial becomes a redirect to the second step.
#[injectable]
pub struct RedirectFilter;

#[async_trait]
impl ExceptionFilter for RedirectFilter {
    async fn catch(&self, _e: HttpException, _h: &ArgumentsHost) -> Response {
        Response::redirect("/mfa", 302u16)
    }
}

/// An "access denied" page answered with status 200.
#[injectable]
pub struct PageFilter;

#[async_trait]
impl ExceptionFilter for PageFilter {
    async fn catch(&self, _e: HttpException, _h: &ArgumentsHost) -> Response {
        Response::json(json!({ "page": "access denied" })).with_status(200u16)
    }
}

/// Turns the error of the handler into a normal answer.
#[injectable]
pub struct MapErrorInterceptor;

#[async_trait]
impl CorvisInterceptor for MapErrorInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        match next.handle().await {
            Err(_) => Ok(Response::json(json!({ "mapped": true }))),
            ok => ok,
        }
    }
}

#[controller("r2")]
pub struct R2Routes {
    authenticator: Arc<Authenticator>,
}

#[routes]
impl R2Routes {
    #[post("deny")]
    #[http_code(200)]
    #[use_guards(AuthGuard::new("header").log_in(), DenyGuard)]
    fn deny(&self) -> Value {
        json!({ "ok": true })
    }

    #[post("err")]
    #[http_code(200)]
    #[use_guards(AuthGuard::new("header").log_in())]
    fn err(&self) -> HttpResult<Value> {
        Err(ForbiddenException::new("second factor needed"))
    }

    #[post("panic")]
    #[http_code(200)]
    #[use_guards(AuthGuard::new("header").log_in())]
    fn panics(&self) -> Value {
        panic!("the handler panics");
    }

    #[post("dto")]
    #[http_code(200)]
    #[use_guards(AuthGuard::new("header").log_in())]
    fn dto(&self, #[query("n", ParseIntPipe)] n: i64) -> Value {
        json!({ "n": n })
    }

    #[post("rf-deny")]
    #[http_code(200)]
    #[use_guards(AuthGuard::new("header").log_in(), DenyGuard)]
    #[use_filters(RedirectFilter)]
    fn rf_deny(&self) -> Value {
        json!({ "ok": true })
    }

    #[post("rf-err")]
    #[http_code(200)]
    #[use_guards(AuthGuard::new("header").log_in())]
    #[use_filters(PageFilter)]
    fn rf_err(&self) -> HttpResult<Value> {
        Err(ForbiddenException::new("second factor needed"))
    }

    #[post("mapped")]
    #[http_code(200)]
    #[use_guards(AuthGuard::new("header").log_in())]
    #[use_interceptors(MapErrorInterceptor)]
    fn mapped(&self) -> HttpResult<Value> {
        Err(ForbiddenException::new("second factor needed"))
    }

    /// The handler ends the exchange with a redirect of its own: it returned normally, so the login stands.
    #[post("handler-redirect")]
    #[use_guards(AuthGuard::new("header").log_in())]
    fn handler_redirect(&self) -> Response {
        Response::redirect("/mfa", 302u16)
    }

    /// A login guard and then a handler that holds the request.
    #[post("login-held")]
    #[http_code(200)]
    #[use_guards(AuthGuard::new("header").log_in())]
    async fn login_held(&self) -> Value {
        cancel_gate().started.notify_one();
        cancel_gate().release.notified().await;
        json!({ "ok": true })
    }

    #[post("deep")]
    #[http_code(200)]
    fn deep(&self, req: Request) -> HttpResult<Value> {
        let depth: usize = req.query_param("n").unwrap().parse().unwrap();
        let mut value = json!(1);
        for _ in 0..depth {
            value = json!([value]);
        }
        req.session().unwrap().set("deep", value).unwrap();
        Ok(json!({ "ok": true }))
    }

    #[post("login")]
    #[http_code(200)]
    async fn login(&self, req: Request) -> HttpResult<Value> {
        let user = AuthenticatedUser::new(account("2").unwrap()).unwrap();
        self.authenticator.log_in(&req, &user).await?;
        Ok(json!({ "ok": true }))
    }
}

async fn app_r2(options: corvis::auth::session::SessionOptions, global: Option<&str>) -> corvis::CorvisApplication {
    let module = Test::create_testing_module()
        .with_logger(false)
        .import_dynamic(SessionModule::for_root(options))
        .providers(vec![
            Provider::strategy::<HeaderStrategy>(),
            Provider::strategy::<SessionStrategy<Accounts>>(),
            Provider::class::<DenyGuard>(),
        ])
        .controller::<Routes>()
        .controller::<R2Routes>()
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    match global {
        Some("redirect") => {
            app.use_global_filters(RedirectFilter);
        }
        Some("page") => {
            app.use_global_filters(PageFilter);
        }
        _ => {}
    }
    app.init().await.unwrap();
    app
}

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

/// `true` when the response sets no cookie, or only ones that clear the session cookie.
fn only_clears(r: &TestResponse) -> bool {
    set_cookies(r).iter().all(|c| c.starts_with("connect.sid=;"))
}

/// A login attempt on `path` with a pre-login anonymous session: whatever the answer is, no cookie that logs in is issued,
/// the pre-login session still works with its data, and no record of the store holds a user.
async fn assert_no_login(app: &CorvisApplication, store: &RecordingStore, path: &str, status: u16, label: &str) {
    let anon = pair(&set_cookies(&post(app, "/set?k=cart&v=milk", None).await)[0]);
    let r = send(app, "POST", path, Some(&anon), &[("x-user", "2")]).await;
    assert_eq!(r.status, status, "{label}: {}", r.text());
    assert!(set_cookies(&r).is_empty(), "{label}: {:?}", set_cookies(&r));
    for (id, record) in store.records().await {
        assert!(record["passport"]["user"].is_null(), "{label}: {id} holds a user");
    }
    assert_eq!(get(app, "/sess", Some(&anon)).await.json_value()["data"], json!({ "cart": "milk" }), "{label}: the old session is intact");
    assert!(get(app, "/who", Some(&anon)).await.json_value()["user"].is_null(), "{label}: the old session is not logged in");
    // And without a presented session nothing is created that logs in.
    let r = send(app, "POST", path, None, &[("x-user", "2")]).await;
    assert_eq!(r.status, status, "{label}");
    assert!(set_cookies(&r).is_empty(), "{label}: {:?}", set_cookies(&r));
}

// ---- item 1: the decision does not depend on the final status ----

#[tokio::test]
async fn a_filter_that_redirects_a_denial_leaves_no_login() {
    let store = RecordingStore::new();
    let app = app_r2(options().store_arc(store.clone()), None).await;
    assert_no_login(&app, &store, "/r2/rf-deny", 302, "route filter, denying guard").await;
}

#[tokio::test]
async fn a_filter_that_answers_200_to_an_error_leaves_no_login() {
    let store = RecordingStore::new();
    let app = app_r2(options().store_arc(store.clone()), None).await;
    assert_no_login(&app, &store, "/r2/rf-err", 200, "route filter, handler error").await;
}

#[tokio::test]
async fn a_global_filter_that_redirects_leaves_no_login() {
    let store = RecordingStore::new();
    let app = app_r2(options().store_arc(store.clone()), Some("redirect")).await;
    for path in ["/r2/deny", "/r2/err", "/r2/dto?n=abc", "/r2/panic"] {
        assert_no_login(&app, &store, path, 302, &format!("global redirect {path}")).await;
    }
}

#[tokio::test]
async fn a_global_filter_that_answers_200_leaves_no_login() {
    let store = RecordingStore::new();
    let app = app_r2(options().store_arc(store.clone()), Some("page")).await;
    for path in ["/r2/deny", "/r2/err", "/r2/dto?n=abc", "/r2/panic"] {
        assert_no_login(&app, &store, path, 200, &format!("global page {path}")).await;
    }
}

#[tokio::test]
async fn an_interceptor_that_maps_the_error_leaves_no_login() {
    let store = RecordingStore::new();
    let app = app_r2(options().store_arc(store.clone()), None).await;
    assert_no_login(&app, &store, "/r2/mapped", 200, "interceptor").await;
}

#[tokio::test]
async fn a_handler_error_a_panic_and_a_failing_pipe_leave_no_login() {
    let store = RecordingStore::new();
    let app = app_r2(options().store_arc(store.clone()), None).await;
    assert_no_login(&app, &store, "/r2/err", 403, "handler error").await;
    assert_no_login(&app, &store, "/r2/panic", 500, "panic").await;
    assert_no_login(&app, &store, "/r2/dto?n=abc", 400, "failing pipe").await;
    assert_no_login(&app, &store, "/r2/deny", 403, "denying guard").await;
}

#[tokio::test]
async fn a_handler_that_returns_normally_keeps_the_login_whatever_it_answers() {
    let store = RecordingStore::new();
    let app = app_r2(options().store_arc(store.clone()), None).await;
    let r = send(&app, "POST", "/r2/handler-redirect", None, &[("x-user", "2")]).await;
    assert_eq!(r.status, 302);
    let cookie = pair(&set_cookies(&r)[0]);
    assert_eq!(get(&app, "/me", Some(&cookie)).await.status, 200, "the login stands");
}

#[tokio::test]
async fn a_client_that_goes_away_during_a_guard_login_leaves_no_login() {
    let store = RecordingStore::new();
    let app = Arc::new(app_r2(options().store_arc(store.clone()), None).await);
    let anon = pair(&set_cookies(&post(&app, "/set?k=cart&v=milk", None).await)[0]);
    let held = {
        let (app, anon) = (app.clone(), anon.clone());
        tokio::spawn(async move { send(&app, "POST", "/r2/login-held", Some(&anon), &[("x-user", "2")]).await })
    };
    cancel_gate().started.notified().await;
    held.abort();
    let _ = held.await;
    // The destruction of the previous session is applied by a task the drop spawns.
    for _ in 0..200 {
        if store.inner.load(&id_of(&anon)).await.unwrap().is_none() {
            break;
        }
        tokio::task::yield_now().await;
    }
    for (id, record) in store.records().await {
        assert!(record["passport"]["user"].is_null(), "{id} holds a user");
    }
    // The previous session was destroyed, not restored (the safe direction).
    assert_eq!(get(&app, "/me", Some(&anon)).await.status, 401);
}

// ---- item 5: a flood cannot make a login fail ----

#[tokio::test]
async fn a_login_made_while_the_previous_session_is_evicted_logs_in() {
    let store = RecordingStore::new();
    let app = Arc::new(app_r2(options().store_arc(store.clone()), None).await);
    let anon = pair(&set_cookies(&post(&app, "/set?k=cart&v=milk", None).await)[0]);
    let gate = named_gate("eviction-login");
    let held = {
        let (app, anon) = (app.clone(), anon.clone());
        tokio::spawn(async move { post(&app, "/held?g=eviction-login&mode=login", Some(&anon)).await })
    };
    gate.started.notified().await;
    // The flood: the store drops the pre-login record while the login is in progress.
    assert!(store.inner.destroy(&id_of(&anon)).await.unwrap());
    gate.release.notify_one();
    let r = held.await.unwrap();
    assert_eq!(r.status, 200);
    let cookie = pair(&set_cookies(&r)[0]);
    assert_ne!(id_of(&cookie), id_of(&anon));
    assert_eq!(get(&app, "/me", Some(&cookie)).await.status, 200, "the login went through");
}

#[tokio::test]
async fn a_login_that_races_a_logout_starts_a_session_of_its_own() {
    // Request A logs out the session C0 while request B, on C0, logs in afresh. B presented credentials, so its new
    // session is valid: nothing of C0 is brought back (the data is not kept across a login).
    let store = RecordingStore::new();
    let app = Arc::new(app_r2(options().store_arc(store.clone()), None).await);
    let c0 = {
        let r = send(&app, "POST", "/login", None, &[("x-user", "2")]).await;
        pair(&set_cookies(&r)[0])
    };
    let gate = named_gate("logout-login-default");
    let held = {
        let (app, c0) = (app.clone(), c0.clone());
        tokio::spawn(async move { post(&app, "/held?g=logout-login-default&mode=login", Some(&c0)).await })
    };
    gate.started.notified().await;
    assert_eq!(post(&app, "/logout", Some(&c0)).await.status, 200);
    gate.release.notify_one();
    let b = held.await.unwrap();
    let c1 = pair(&set_cookies(&b)[0]);
    assert_eq!(get(&app, "/me", Some(&c1)).await.status, 200);
    assert_eq!(get(&app, "/me", Some(&c0)).await.status, 401, "the logged-out identifier stays dead");
}

#[tokio::test]
async fn a_login_that_keeps_the_data_cannot_bring_back_a_session_that_was_ended() {
    let store = RecordingStore::new();
    let app = Arc::new(app_r2(options().keep_session_info_on_login(true).store_arc(store.clone()), None).await);
    let c0 = {
        let r = send(&app, "POST", "/login", None, &[("x-user", "2")]).await;
        pair(&set_cookies(&r)[0])
    };
    let gate = named_gate("logout-login-keep");
    let held = {
        let (app, c0) = (app.clone(), c0.clone());
        tokio::spawn(async move { post(&app, "/held?g=logout-login-keep&mode=login", Some(&c0)).await })
    };
    gate.started.notified().await;
    assert_eq!(post(&app, "/logout", Some(&c0)).await.status, 200);
    gate.release.notify_one();
    let b = held.await.unwrap();
    assert!(only_clears(&b), "{:?}", set_cookies(&b));
    for (id, record) in store.records().await {
        assert!(record["passport"]["user"].is_null(), "{id} holds a user");
    }
}

// ---- item 3: records the store could write and not read ----

#[tokio::test]
async fn a_session_nested_past_the_limit_is_refused_when_it_is_saved() {
    let store = RecordingStore::new();
    let app = app_r2(options().store_arc(store.clone()), None).await;
    let ok = post(&app, "/r2/deep?n=90", None).await;
    assert_eq!(ok.status, 200);
    let cookie = pair(&set_cookies(&ok)[0]);
    let loaded = get(&app, "/sess", Some(&cookie)).await;
    assert_eq!(loaded.status, 200);
    assert!(loaded.json_value()["data"]["deep"].is_array(), "a value inside the limit comes back");
    // 127 levels is what the body parser accepts: the save fails for the handler to see, nothing is stored.
    let before = store.len();
    let r = post(&app, "/r2/deep?n=127", None).await;
    assert_eq!(r.status, 500, "{}", r.text());
    assert!(set_cookies(&r).is_empty());
    assert_eq!(store.len(), before);
    // A logged-in session that tries to grow past the limit keeps its last good record and stays logged in.
    let c = pair(&set_cookies(&send(&app, "POST", "/login", None, &[("x-user", "2")]).await)[0]);
    let r = post(&app, "/r2/deep?n=120", Some(&c)).await;
    assert_eq!(r.status, 500);
    assert_eq!(get(&app, "/me", Some(&c)).await.status, 200, "the user is not logged out by the refused save");
}

#[tokio::test]
async fn a_record_that_does_not_parse_is_destroyed_and_is_no_session() {
    // A shared store could hand the middleware anything; the memory store's own text cannot be broken from outside,
    // so the case is driven through a record deeper than the parser's limit written by another writer.
    let store = Arc::new(MemorySessionStore::new());
    let mut deep = json!(1);
    for _ in 0..110 {
        deep = json!([deep]);
    }
    let app = app_r2(options().store_arc(store.clone()), None).await;
    let first = pair(&set_cookies(&post(&app, "/set?k=a&v=b", None).await)[0]);
    let id = id_of(&first);
    let record = json!({ "cookie": { "expires": null }, "deep": deep, "__corvis": { "createdAt": now(), "lastAccess": now() } });
    assert!(store.update(&id, &record, std::time::Duration::from_secs(600)).await.unwrap());
    // Nested 110 levels: readable by the parser but past the write limit, so it is no session and is left alone.
    let r = get(&app, "/sess", Some(&first)).await;
    assert_eq!(r.json_value()["data"], json!({}));
    assert!(store.load(&id).await.unwrap().is_some(), "a record that parses is not deleted");
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}
