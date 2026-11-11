//! The application shared by the session test binaries: a user store, a serializer, a header strategy, the routes of
//! the recorded fixture (`tests/fixtures/auth/session.mjs`) and a store that records every call.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use corvis::auth::session::{
    MemorySessionStore, SessionCookie, SessionModule, SessionOptions, SessionSerializer, SessionStore, SessionStoreError, SessionStrategy,
};
use corvis::auth::{AuthGuard, AuthenticateOptions, AuthenticatedUser, Authenticator, Strategy, StrategyOutcome, User};
use corvis::prelude::*;
use corvis::testing::{Test, TestResponse};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

pub const NEW: &str = "corvis-session-secret-NEW-0123456789abcdef";
pub const OLD: &str = "corvis-session-secret-OLD-fedcba9876543210";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub id: String,
    pub name: String,
}

pub fn account(id: &str) -> Option<Account> {
    let name = match id {
        "1" => "ana",
        "2" => "bob",
        "3" => "cy",
        "4" => "dee",
        _ => return None,
    };
    Some(Account { id: id.to_string(), name: name.to_string() })
}

pub fn deleted() -> &'static Mutex<HashSet<String>> {
    static DELETED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    DELETED.get_or_init(Default::default)
}

/// Signals of the route that holds a request in its handler: `STARTED` fires once the handler runs, `RELEASE` lets it go.
pub struct Gate {
    pub started: Notify,
    pub release: Notify,
}

pub fn gate() -> &'static Gate {
    static GATE: OnceLock<Gate> = OnceLock::new();
    GATE.get_or_init(|| Gate { started: Notify::new(), release: Notify::new() })
}

/// A gate by name, so that tests running at the same time do not share one.
pub fn named_gate(name: &str) -> Arc<Gate> {
    static GATES: OnceLock<Mutex<HashMap<String, Arc<Gate>>>> = OnceLock::new();
    let mut gates = GATES.get_or_init(Default::default).lock().unwrap();
    gates.entry(name.to_string()).or_insert_with(|| Arc::new(Gate { started: Notify::new(), release: Notify::new() })).clone()
}

pub fn cancel_gate() -> &'static Gate {
    static GATE: OnceLock<Gate> = OnceLock::new();
    GATE.get_or_init(|| Gate { started: Notify::new(), release: Notify::new() })
}

#[injectable]
pub struct Accounts;

#[async_trait]
impl SessionSerializer for Accounts {
    type User = Account;

    async fn serialize_user(&self, user: &Account) -> Result<Value, HttpException> {
        Ok(json!(user.id))
    }

    async fn deserialize_user(&self, req: &Request, stored: Value) -> Result<Option<Account>, HttpException> {
        if req.header("x-lookup-fails").is_some() {
            return Err(corvis::http::ServiceUnavailableException::new("lookup failed"));
        }
        let Some(id) = stored.as_str() else { return Ok(None) };
        if deleted().lock().unwrap().contains(id) {
            return Ok(None);
        }
        Ok(account(id))
    }
}

/// `x-user: <id>` authenticates as that account.
#[injectable]
pub struct HeaderStrategy;

#[async_trait]
impl Strategy for HeaderStrategy {
    fn name(&self) -> &str {
        "header"
    }

    async fn authenticate(&self, ctx: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
        match ctx.request().header("x-user").and_then(account).map(AuthenticatedUser::new) {
            Some(Ok(user)) => StrategyOutcome::Success { user, info: None },
            _ => StrategyOutcome::Fail { challenge: None, status: None },
        }
    }
}

/// What [`TypedUserGuard`] found, for the handler to echo.
#[derive(Clone)]
pub struct SeenByGuard(pub Option<Account>);

/// Reads `Request::typed_user::<Account>()` and lets the request through.
#[injectable]
pub struct TypedUserGuard;

#[async_trait]
impl corvis::CanActivate for TypedUserGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let req = ctx.switch_to_http().get_request();
        req.insert(SeenByGuard(req.typed_user::<Account>()));
        Ok(true)
    }

    fn controls_access(&self) -> bool {
        false
    }
}

/// Inserts an `Account` as a plain request extension: it is not a user.
#[injectable]
pub struct PlantingGuard;

#[async_trait]
impl corvis::CanActivate for PlantingGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        ctx.switch_to_http().get_request().insert(account("1").unwrap());
        Ok(true)
    }

    fn controls_access(&self) -> bool {
        false
    }
}

/// A second guard that refuses (a second factor that was not passed).
#[injectable]
pub struct DenyGuard;

#[async_trait]
impl corvis::CanActivate for DenyGuard {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Ok(false)
    }
}

#[controller("")]
pub struct Routes {
    authenticator: Arc<Authenticator>,
}

fn session_of(req: &Request) -> corvis::auth::session::Session {
    req.session().expect("the session module runs for every route")
}

#[routes]
impl Routes {
    #[get("sess")]
    fn sess(&self, req: Request) -> Value {
        json!({ "data": session_of(&req).data() })
    }

    #[post("set")]
    #[http_code(200)]
    fn set(&self, req: Request) -> Value {
        session_of(&req).set(req.query_param("k").unwrap(), req.query_param("v").unwrap()).unwrap();
        json!({ "ok": true })
    }

    #[post("set-same")]
    #[http_code(200)]
    fn set_same(&self, req: Request) -> Value {
        let session = session_of(&req);
        let key = req.query_param("k").unwrap();
        let value = session.get_value(key).unwrap();
        session.set(key, value).unwrap();
        json!({ "ok": true })
    }

    /// An object of `?n=` keys: about 8 bytes of record per key.
    #[post("fill")]
    #[http_code(200)]
    fn fill(&self, req: Request) -> Value {
        let n: usize = req.query_param("n").unwrap().parse().unwrap();
        let object: serde_json::Map<String, Value> = (0..n).map(|k| (format!("{k:x}"), json!(1))).collect();
        session_of(&req).set("blob", object).unwrap();
        json!({ "ok": true })
    }

    #[post("big")]
    #[http_code(200)]
    fn big(&self, req: Request) -> Value {
        session_of(&req).set("blob", "x".repeat(20 * 1024)).unwrap();
        json!({ "ok": true })
    }

    #[post("regenerate")]
    #[http_code(200)]
    fn regenerate(&self, req: Request) -> Value {
        session_of(&req).regenerate();
        json!({ "ok": true })
    }

    #[post("destroy")]
    #[http_code(200)]
    fn destroy(&self, req: Request) -> Value {
        session_of(&req).destroy();
        json!({ "ok": true })
    }

    #[post("login")]
    #[http_code(200)]
    #[use_guards(AuthGuard::new("header").log_in())]
    fn login(&self, req: Request) -> Value {
        json!({ "user": req.user() })
    }

    #[post("logout")]
    #[http_code(200)]
    async fn logout(&self, req: Request) -> Result<Value, HttpException> {
        self.authenticator.log_out(&req).await?;
        Ok(json!({ "ok": true }))
    }

    #[get("me")]
    #[use_guards(AuthGuard::new("session"))]
    fn me(&self, user: User<Account>) -> Value {
        json!({ "user": user.0 })
    }

    /// No guard: the session user is still on the request.
    #[get("who")]
    fn who(&self, req: Request) -> Value {
        json!({ "user": req.user() })
    }

    /// No `AuthGuard`: `User<T>` is the session user, typed.
    #[get("who-typed")]
    fn who_typed(&self, user: User<Account>) -> Value {
        json!({ "user": user.0 })
    }

    /// A guard after `AuthGuard` that reads the typed user.
    #[get("typed-guard")]
    #[use_guards(AuthGuard::new("header"), TypedUserGuard)]
    fn typed_guard(&self, req: Request) -> Value {
        json!({ "seen": req.get::<SeenByGuard>().map(|s| s.0) })
    }

    /// A guard that plants an `Account` as a plain extension, then one that reads the typed user.
    #[get("foreign-extension")]
    #[use_guards(PlantingGuard, TypedUserGuard)]
    fn foreign_extension(&self, req: Request) -> Value {
        json!({ "seen": req.get::<SeenByGuard>().map(|s| s.0) })
    }

    #[public]
    #[get("public-who")]
    #[use_guards(AuthGuard::new("header").skip_public())]
    fn public_who(&self, req: Request) -> Value {
        json!({ "user": req.user() })
    }

    /// An optional guard of another strategy.
    #[get("optional")]
    #[use_guards(AuthGuard::new("header").optional())]
    fn optional(&self, req: Request) -> Value {
        json!({ "user": req.user() })
    }

    #[get("chain")]
    #[use_guards(AuthGuard::chain(["session", "header"]).optional())]
    fn chain(&self, req: Request) -> Value {
        json!({ "user": req.user() })
    }

    #[get("header-only")]
    #[use_guards(AuthGuard::new("header"))]
    fn header_only(&self, req: Request) -> Value {
        json!({ "user": req.user() })
    }

    #[post("admin/delete")]
    #[http_code(200)]
    fn admin_delete(&self, req: Request) -> Value {
        deleted().lock().unwrap().insert(req.query_param("id").unwrap().to_string());
        json!({ "ok": true })
    }

    #[post("admin/restore")]
    #[http_code(200)]
    fn admin_restore(&self, req: Request) -> Value {
        deleted().lock().unwrap().remove(req.query_param("id").unwrap());
        json!({ "ok": true })
    }

    /// Writes to the session after the gate opens, so another request can run first.
    #[post("held-set")]
    #[http_code(200)]
    async fn held_set(&self, req: Request) -> Value {
        gate().started.notify_one();
        gate().release.notified().await;
        session_of(&req).set("late", true).unwrap();
        json!({ "ok": true })
    }

    /// Holds the request at the gate named by `g`, then does what `mode` says to the session before it finishes:
    /// `regenerate_keeping_data`, `regenerate`, `set` (a data change) or `touch` (nothing, so the session is only
    /// touched).
    #[post("held")]
    #[http_code(200)]
    async fn held(&self, req: Request) -> HttpResult<Value> {
        let gate = named_gate(req.query_param("g").unwrap());
        gate.started.notify_one();
        gate.release.notified().await;
        let session = session_of(&req);
        match req.query_param("mode").unwrap() {
            "regenerate_keeping_data" => session.regenerate_keeping_data(),
            "regenerate" => session.regenerate(),
            "login" => {
                let user = AuthenticatedUser::new(account("2").unwrap()).unwrap();
                self.authenticator.log_in(&req, &user).await?;
            }
            "set" => session.set("late", true).unwrap(),
            _ => {}
        }
        Ok(json!({ "ok": true }))
    }

    /// A login that a second guard then refuses.
    #[post("login-then-deny")]
    #[http_code(200)]
    #[use_guards(AuthGuard::new("header").log_in(), DenyGuard)]
    fn login_then_deny(&self) -> Value {
        json!({ "ok": true })
    }

    /// A login followed by a handler that answers 403.
    #[post("login-then-403")]
    #[http_code(200)]
    #[use_guards(AuthGuard::new("header").log_in())]
    fn login_then_403(&self) -> HttpResult<Value> {
        Err(ForbiddenException::new("second factor needed"))
    }

    /// A login the handler makes itself, then a 403 answer: the handler decides, so the login stands.
    #[post("handler-login-then-403")]
    #[http_code(200)]
    async fn handler_login_then_403(&self, req: Request) -> HttpResult<Response> {
        let user = AuthenticatedUser::new(account("2").unwrap()).unwrap();
        self.authenticator.log_in(&req, &user).await?;
        let mut response = Response::json(json!({ "ok": false }));
        response.set_status(403u16);
        Ok(response)
    }

    /// Destroys the session, then holds the request: the client goes away before the response exists.
    #[post("held-destroy")]
    #[http_code(200)]
    async fn held_destroy(&self, req: Request) -> Value {
        session_of(&req).destroy();
        cancel_gate().started.notify_one();
        cancel_gate().release.notified().await;
        json!({ "ok": true })
    }

    #[get("has-session")]
    fn has_session(&self, req: Request) -> Value {
        json!({ "has": req.session().is_some() })
    }
}

/// One store call as the recording store saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    pub op: &'static str,
    pub id: String,
    /// For `destroy`: the record existed.
    pub existed: bool,
}

/// A [`MemorySessionStore`] that remembers every call, and can be told to fail.
pub struct RecordingStore {
    pub inner: MemorySessionStore,
    ops: Mutex<Vec<Op>>,
    /// The records `touch` received, oldest first.
    pub touched: Mutex<Vec<Value>>,
    seen: Mutex<Vec<String>>,
    pub fail_load: Mutex<bool>,
    pub fail_writes: Mutex<bool>,
    pub delay: Mutex<Option<Duration>>,
}

impl RecordingStore {
    pub fn new() -> Arc<Self> {
        Arc::new(RecordingStore {
            inner: MemorySessionStore::new(),
            ops: Mutex::new(Vec::new()),
            touched: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
            fail_load: Mutex::new(false),
            fail_writes: Mutex::new(false),
            delay: Mutex::new(None),
        })
    }

    /// The calls since the last time, oldest first.
    pub fn take_ops(&self) -> Vec<Op> {
        std::mem::take(&mut *self.ops.lock().unwrap())
    }

    /// Every record still held, by identifier.
    pub async fn records(&self) -> HashMap<String, Value> {
        let ids = self.seen.lock().unwrap().clone();
        let mut out = HashMap::new();
        for id in ids {
            if let Some(record) = self.inner.load(&id).await.unwrap() {
                out.insert(id, record);
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    fn note(&self, op: &'static str, id: &str, existed: bool) {
        self.ops.lock().unwrap().push(Op { op, id: id.to_string(), existed });
        let mut seen = self.seen.lock().unwrap();
        if !seen.iter().any(|s| s == id) {
            seen.push(id.to_string());
        }
    }

    async fn pause(&self) {
        let delay = *self.delay.lock().unwrap();
        if let Some(d) = delay {
            tokio::time::sleep(d).await;
        }
    }
}

#[async_trait]
impl SessionStore for RecordingStore {
    async fn load(&self, id: &str) -> Result<Option<Value>, SessionStoreError> {
        self.note("get", id, false);
        self.pause().await;
        if *self.fail_load.lock().unwrap() {
            return Err(SessionStoreError::backend("connection refused for sess:SECRET-ID"));
        }
        self.inner.load(id).await
    }

    async fn create(&self, id: &str, record: &Value, ttl: Duration) -> Result<(), SessionStoreError> {
        self.note("set", id, false);
        if *self.fail_writes.lock().unwrap() {
            return Err(SessionStoreError::backend("write failed"));
        }
        self.inner.create(id, record, ttl).await
    }

    async fn update(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError> {
        self.note("set", id, false);
        if *self.fail_writes.lock().unwrap() {
            return Err(SessionStoreError::backend("write failed"));
        }
        self.inner.update(id, record, ttl).await
    }

    async fn touch(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError> {
        self.note("touch", id, false);
        self.touched.lock().unwrap().push(record.clone());
        if *self.fail_writes.lock().unwrap() {
            return Err(SessionStoreError::backend("write failed"));
        }
        self.inner.touch(id, record, ttl).await
    }

    async fn destroy(&self, id: &str) -> Result<bool, SessionStoreError> {
        let existed = self.inner.load(id).await.unwrap().is_some();
        self.note("destroy", id, existed);
        self.inner.destroy(id).await
    }

    fn in_process(&self) -> bool {
        true
    }
}

/// The defaults of the specification, over plain HTTP in the test client: `Secure` stays on (the cookie is sent whatever
/// the transport).
pub fn options() -> SessionOptions {
    SessionOptions::with_secrets([NEW]).unwrap()
}

/// The cookie of the original with its defaults: no `Secure`, no `SameSite`.
pub fn original_cookie() -> SessionCookie {
    SessionCookie::default().secure(false).without_same_site()
}

/// An initialized application with sessions configured by `options` and the routes above.
pub async fn app(options: SessionOptions) -> corvis::CorvisApplication {
    app_with(options, |t| t).await
}

pub async fn app_with(
    options: SessionOptions,
    extra: impl FnOnce(corvis::testing::TestingModuleBuilder) -> corvis::testing::TestingModuleBuilder,
) -> corvis::CorvisApplication {
    let builder = Test::create_testing_module()
        .with_logger(false)
        .import_dynamic(SessionModule::for_root(options))
        .providers(vec![
            Provider::strategy::<HeaderStrategy>(),
            Provider::strategy::<SessionStrategy<Accounts>>(),
            Provider::class::<DenyGuard>(),
            Provider::class::<TypedUserGuard>(),
            Provider::class::<PlantingGuard>(),
        ])
        .controller::<Routes>();
    let module = extra(builder).compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    app
}

/// Every `Set-Cookie` header of a response.
pub fn set_cookies(response: &TestResponse) -> Vec<String> {
    response.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect()
}

/// The `name=value` pair of a `Set-Cookie` header, to send back as a `Cookie` header.
pub fn pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().to_string()
}

/// The session identifier inside a `connect.sid=s%3A<id>.<sig>` pair.
pub fn id_of(pair: &str) -> String {
    let value = pair.split_once('=').unwrap().1;
    let decoded = percent_encoding::percent_decode_str(value).decode_utf8().unwrap().into_owned();
    let rest = decoded.strip_prefix("s:").unwrap();
    rest[..rest.rfind('.').unwrap()].to_string()
}
