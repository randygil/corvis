//! `core/injection-scopes` and `core/module-ref`: the request scope. One test per scenario of the spec deltas of
//! `e13-request-scoped-providers`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, Weak};
use std::time::Duration;

use corvis::DiError;
use corvis::prelude::*;
use corvis::testing::{Test, request};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::{assert_cycle, compile_err, compile_ok, next_id};

fn bump(counter: &AtomicUsize) -> usize {
    counter.fetch_add(1, Ordering::SeqCst)
}

/// Sends one request over a real socket on 127.0.0.1 and returns the raw response.
async fn raw_get(addr: std::net::SocketAddr, path: &str) -> String {
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), s.read_to_end(&mut out)).await.expect("response timed out").unwrap();
    String::from_utf8_lossy(&out).into_owned()
}

/// The JSON object of a raw response, whatever the framing of its body.
fn json_body(raw: &str) -> Value {
    let (start, end) = (raw.find('{').expect("no body"), raw.rfind('}').expect("no body"));
    serde_json::from_str(&raw[start..=end]).unwrap_or_else(|e| panic!("bad body {raw:?}: {e}"))
}

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..500 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} was not reached");
}

// ------------------------------------------------------------------ one instance per request

mod basic {
    use super::*;

    #[injectable(scope = "request")]
    pub struct RequestContext {
        #[init(next_id())]
        pub _id: usize,
        pub value: Mutex<String>,
    }

    #[injectable(scope = "request")]
    pub struct AuditLog {
        pub ctx: Arc<RequestContext>,
    }

    #[injectable(scope = "request")]
    pub struct Billing {
        pub ctx: Arc<RequestContext>,
    }

    /// Declares no scope: the module gives it one.
    #[injectable]
    pub struct PlainCtx {
        #[init(next_id())]
        pub id: usize,
    }

    #[injectable]
    pub struct Clock;

    #[injectable(scope = "request")]
    pub struct ClockUser {
        pub clock: Arc<Clock>,
    }

    #[controller("ctx")]
    pub struct CtxController {
        module_ref: ModuleRef,
    }

    #[routes]
    impl CtxController {
        #[get("twice")]
        async fn twice(&self, #[req] req: Request) -> String {
            let a = self.module_ref.resolve_for::<RequestContext>(&req).await.unwrap();
            let b = self.module_ref.resolve_for::<RequestContext>(&req).await.unwrap();
            Arc::ptr_eq(&a, &b).to_string()
        }

        #[get("shared")]
        async fn shared(&self, #[req] req: Request) -> String {
            let audit = self.module_ref.resolve_for::<AuditLog>(&req).await.unwrap();
            let billing = self.module_ref.resolve_for::<Billing>(&req).await.unwrap();
            Arc::ptr_eq(&audit.ctx, &billing.ctx).to_string()
        }

        #[get("alias")]
        async fn alias(&self, #[req] req: Request) -> String {
            let direct = self.module_ref.resolve_for::<RequestContext>(&req).await.unwrap();
            let alias = self.module_ref.resolve_token_for::<RequestContext>("CTX", &req).await.unwrap();
            Arc::ptr_eq(&direct, &alias).to_string()
        }

        #[get("plain")]
        async fn plain(&self, #[req] req: Request) -> Value {
            let a = self.module_ref.resolve_token_for::<PlainCtx>("PLAIN", &req).await.unwrap();
            let b = self.module_ref.resolve_token_for::<PlainCtx>("PLAIN", &req).await.unwrap();
            json!([a.id, b.id])
        }

        #[get("clock")]
        async fn clock(&self, #[req] req: Request) -> String {
            let user = self.module_ref.resolve_for::<ClockUser>(&req).await.unwrap();
            (Arc::as_ptr(&user.clock) as usize).to_string()
        }

        /// Writes a value derived from the request, waits so that other requests interleave, reads it back.
        #[get("concurrent/:n")]
        async fn concurrent(&self, #[param("n")] n: String, #[req] req: Request) -> Value {
            let ctx = self.module_ref.resolve_for::<RequestContext>(&req).await.unwrap();
            *ctx.value.lock().unwrap() = format!("value-{n}");
            tokio::time::sleep(Duration::from_millis(150)).await;
            let read = ctx.value.lock().unwrap().clone();
            json!({ "n": n, "read": read, "addr": Arc::as_ptr(&ctx) as usize })
        }
    }

    #[module(
        controllers = [CtxController],
        providers = [
            RequestContext, AuditLog, Billing, Clock, ClockUser,
            Provider::use_existing("CTX", Token::of::<RequestContext>()),
            Provider::use_class::<PlainCtx>("PLAIN").scope(Scope::Request),
        ]
    )]
    pub struct BasicModule;
}

async fn basic_app() -> corvis::CorvisApplication {
    let mut app = compile_ok(Test::create_testing_module().import::<basic::BasicModule>()).await.create_corvis_application();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn two_resolutions_in_one_request_share_the_instance() {
    let app = basic_app().await;
    request(&app).get("/ctx/twice").send().await.expect_status(200).expect_text("true");
}

#[corvis::test]
async fn the_instance_is_shared_across_consumers() {
    let app = basic_app().await;
    request(&app).get("/ctx/shared").send().await.expect_status(200).expect_text("true");
}

#[corvis::test]
async fn an_alias_of_a_request_scoped_provider_resolves_to_the_instance_of_the_request() {
    let app = basic_app().await;
    request(&app).get("/ctx/alias").send().await.expect_status(200).expect_text("true");
}

#[corvis::test]
async fn the_scope_can_be_set_on_the_provider() {
    let app = basic_app().await;
    let first: Vec<usize> = request(&app).get("/ctx/plain").send().await.json();
    let second: Vec<usize> = request(&app).get("/ctx/plain").send().await.json();
    assert_eq!(first[0], first[1]);
    assert_eq!(second[0], second[1]);
    assert_ne!(first[0], second[0]);
}

#[corvis::test]
async fn a_request_scoped_provider_can_depend_on_a_singleton() {
    let app = basic_app().await;
    let a = request(&app).get("/ctx/clock").send().await.text();
    let b = request(&app).get("/ctx/clock").send().await.text();
    assert_eq!(a, b, "every request-scoped instance holds the same singleton");
}

#[corvis::test]
async fn concurrent_requests_never_share_an_instance() {
    let mut app = basic_app().await;
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let tasks: Vec<_> =
        (0..50).map(|n| tokio::spawn(async move { json_body(&raw_get(addr, &format!("/ctx/concurrent/{n}")).await) })).collect();
    let mut addresses = HashSet::new();
    for task in tasks {
        let body = task.await.unwrap();
        let n = body["n"].as_str().unwrap();
        assert_eq!(body["read"], format!("value-{n}"), "request {n} read the value of another request");
        addresses.insert(body["addr"].as_u64().unwrap());
    }
    assert_eq!(addresses.len(), 50, "instances alive at the same time must be distinct");
    app.close().await.unwrap();
}

// ------------------------------------------------------------------ never stored, released with the request

mod lifetime {
    use super::*;

    pub static CONSTRUCTED: AtomicUsize = AtomicUsize::new(0);
    pub static HOOK_CALLS: AtomicUsize = AtomicUsize::new(0);
    pub static WEAKS: Mutex<Option<HashMap<String, Weak<HeldRequest>>>> = Mutex::new(None);

    pub fn weak_of(key: &str) -> Option<Weak<HeldRequest>> {
        WEAKS.lock().unwrap().as_ref().and_then(|m| m.get(key).cloned())
    }

    /// Keeps the request it was built for, which is what would leak it without the removal of the store.
    #[injectable(scope = "request")]
    pub struct HeldRequest {
        pub _req: Request,
    }

    #[injectable(scope = "request")]
    pub struct Counted {
        #[init(bump(&CONSTRUCTED))]
        _n: usize,
    }

    #[injectable(scope = "request")]
    pub struct Hooked;

    #[async_trait]
    impl OnModuleInit for Hooked {
        async fn on_module_init(&self) -> corvis::Result<()> {
            HOOK_CALLS.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[controller("life")]
    pub struct LifeController {
        module_ref: ModuleRef,
    }

    impl LifeController {
        async fn hold(&self, key: &str, req: &Request) {
            let held = self.module_ref.resolve_for::<HeldRequest>(req).await.unwrap();
            WEAKS.lock().unwrap().get_or_insert_with(HashMap::new).insert(key.to_string(), Arc::downgrade(&held));
        }
    }

    #[routes]
    impl LifeController {
        #[get("ok/:key")]
        async fn ok(&self, #[param("key")] key: String, #[req] req: Request) -> &'static str {
            self.hold(&key, &req).await;
            "ok"
        }

        #[get("panic/:key")]
        async fn boom(&self, #[param("key")] key: String, #[req] req: Request) -> &'static str {
            self.hold(&key, &req).await;
            panic!("handler panicked");
        }

        #[get("slow/:key")]
        async fn slow(&self, #[param("key")] key: String, #[req] req: Request) -> &'static str {
            self.hold(&key, &req).await;
            tokio::time::sleep(Duration::from_secs(30)).await;
            "late"
        }

        #[get("counted")]
        async fn counted(&self, #[req] req: Request) -> &'static str {
            self.module_ref.resolve_for::<Counted>(&req).await.unwrap();
            "ok"
        }

        #[get("hooked")]
        async fn hooked(&self, #[req] req: Request) -> &'static str {
            self.module_ref.resolve_for::<Hooked>(&req).await.unwrap();
            "ok"
        }
    }

    #[module(controllers = [LifeController], providers = [HeldRequest, Counted, Hooked])]
    pub struct LifeModule;
}

async fn life_app() -> corvis::CorvisApplication {
    let mut app = compile_ok(Test::create_testing_module().import::<lifetime::LifeModule>()).await.create_corvis_application();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn a_request_scoped_provider_is_not_created_at_startup() {
    let before = lifetime::CONSTRUCTED.load(Ordering::SeqCst);
    let app = life_app().await;
    assert_eq!(lifetime::CONSTRUCTED.load(Ordering::SeqCst), before);
    request(&app).get("/life/counted").send().await.expect_status(200);
    assert_eq!(lifetime::CONSTRUCTED.load(Ordering::SeqCst), before + 1);
}

#[corvis::test]
async fn instances_are_released_after_the_response() {
    let app = life_app().await;
    request(&app).get("/life/ok/after-response").send().await.expect_status(200);
    let weak = lifetime::weak_of("after-response").expect("the handler ran");
    assert!(weak.upgrade().is_none(), "the instance outlived its request");
}

#[corvis::test]
async fn instances_are_released_when_the_handler_panics() {
    let app = life_app().await;
    request(&app).get("/life/panic/after-panic").send().await.expect_status(500);
    let weak = lifetime::weak_of("after-panic").expect("the handler ran");
    assert!(weak.upgrade().is_none(), "the instance outlived a panicking request");
}

#[corvis::test]
async fn instances_are_released_when_the_client_disconnects() {
    let mut app = life_app().await;
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let mut socket = TcpStream::connect(addr).await.unwrap();
    socket.write_all(b"GET /life/slow/after-disconnect HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
    wait_until("the handler to start", || lifetime::weak_of("after-disconnect").is_some_and(|w| w.upgrade().is_some())).await;
    drop(socket);
    let weak = lifetime::weak_of("after-disconnect").unwrap();
    wait_until("the instance to be released", || weak.upgrade().is_none()).await;
    app.close().await.unwrap();
}

#[corvis::test]
async fn app_get_does_not_see_request_scoped_providers() {
    let module = compile_ok(Test::create_testing_module().import::<lifetime::LifeModule>()).await;
    assert!(matches!(module.get::<lifetime::HeldRequest>(), Err(DiError::NotFound { .. })));
    let app = module.create_corvis_application();
    assert!(matches!(app.get::<lifetime::HeldRequest>(), Err(DiError::NotFound { .. })));
    assert!(matches!(app.module_ref().get_strict::<lifetime::HeldRequest>(), Err(DiError::NotFound { .. })));
}

#[corvis::test]
async fn lifecycle_hooks_are_not_called_on_request_scoped_instances() {
    let before = lifetime::HOOK_CALLS.load(Ordering::SeqCst);
    let app = life_app().await;
    request(&app).get("/life/hooked").send().await.expect_status(200);
    assert_eq!(lifetime::HOOK_CALLS.load(Ordering::SeqCst), before);
}

// ------------------------------------------------------------------ the current request

mod current {
    use super::*;

    #[derive(Clone)]
    pub struct User(pub &'static str);

    #[injectable(scope = "request")]
    pub struct RequestInfo {
        pub req: Request,
    }

    pub struct Tenant(pub String);

    #[injectable]
    pub struct InsertUserGuard;

    #[async_trait]
    impl CanActivate for InsertUserGuard {
        async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
            ctx.request().insert(User("alice"));
            Ok(true)
        }
    }

    /// Resolves the request-scoped instance before the guard runs.
    #[injectable]
    pub struct EarlyMiddleware {
        module_ref: ModuleRef,
    }

    #[async_trait]
    impl CorvisMiddleware for EarlyMiddleware {
        async fn use_(&self, req: Request, next: Next) -> HandlerResult {
            self.module_ref.resolve_for::<RequestInfo>(&req).await.unwrap();
            next.run(req).await
        }
    }

    #[controller("current")]
    #[use_guards(InsertUserGuard)]
    pub struct CurrentController {
        module_ref: ModuleRef,
    }

    #[routes]
    impl CurrentController {
        #[get("url")]
        async fn url(&self, #[req] req: Request) -> String {
            self.module_ref.resolve_for::<RequestInfo>(&req).await.unwrap().req.url()
        }

        #[get("user")]
        async fn user(&self, #[req] req: Request) -> String {
            let info = self.module_ref.resolve_for::<RequestInfo>(&req).await.unwrap();
            info.req.get::<User>().map(|u| u.0.to_string()).unwrap_or_default()
        }

        #[get("tenant")]
        async fn tenant(&self, #[req] req: Request) -> String {
            let a = self.module_ref.resolve_token_for::<Tenant>("TENANT", &req).await.unwrap();
            let b = self.module_ref.resolve_token_for::<Tenant>("TENANT", &req).await.unwrap();
            format!("{}:{}", a.0, Arc::ptr_eq(&a, &b))
        }
    }

    #[module(
        controllers = [CurrentController],
        providers = [
            RequestInfo,
            Provider::use_factory("TENANT", |ctx| async move {
                let tenant = ctx.request()?.header("x-tenant").unwrap_or("none").to_string();
                Ok(Tenant(tenant))
            })
            .scope(Scope::Request),
        ]
    )]
    pub struct CurrentModule;

    impl CorvisModule for CurrentModule {
        fn configure(&self, consumer: &mut MiddlewareConsumer) {
            consumer.apply::<EarlyMiddleware>().for_routes("current");
        }
    }
}

async fn current_app() -> corvis::CorvisApplication {
    let mut app = compile_ok(Test::create_testing_module().import::<current::CurrentModule>()).await.create_corvis_application();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn a_request_field_receives_the_request_being_handled() {
    let app = current_app().await;
    request(&app).get("/current/url?x=1").send().await.expect_status(200).expect_text("/current/url?x=1");
}

#[corvis::test]
async fn extensions_inserted_after_the_first_resolution_stay_visible() {
    let app = current_app().await;
    // The middleware resolved `RequestInfo`; the guard inserted the user afterwards.
    request(&app).get("/current/user").send().await.expect_status(200).expect_text("alice");
}

#[corvis::test]
async fn a_factory_provider_reads_the_request_with_ctx_request() {
    let app = current_app().await;
    request(&app).get("/current/tenant").set("x-tenant", "acme").send().await.expect_status(200).expect_text("acme:true");
    request(&app).get("/current/tenant").set("x-tenant", "globex").send().await.expect_status(200).expect_text("globex:true");
}

// ------------------------------------------------------------------ startup errors

mod startup {
    use super::*;

    #[injectable(scope = "request")]
    pub struct RequestContext;

    #[injectable]
    pub struct Clock;

    #[injectable(scope = "request")]
    pub struct UsesClock {
        pub _clock: Arc<Clock>,
    }

    #[controller("bad")]
    pub struct BadController {
        _ctx: Arc<RequestContext>,
    }

    #[routes]
    impl BadController {
        #[get]
        fn index(&self) -> &'static str {
            "never"
        }
    }

    #[injectable]
    pub struct Report {
        _formatter: Arc<Formatter>,
    }

    #[injectable(scope = "transient")]
    pub struct Formatter {
        _ctx: Arc<RequestContext>,
    }

    #[injectable]
    pub struct Holder {
        _req: Request,
    }

    #[injectable(scope = "request")]
    pub struct TenantGuard;

    #[async_trait]
    impl CanActivate for TenantGuard {
        async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
            Ok(true)
        }
    }

    #[controller("typed")]
    #[use_guards(TenantGuard)]
    pub struct TypedGuardController;

    #[routes]
    impl TypedGuardController {
        #[get]
        fn index(&self) -> &'static str {
            "never"
        }
    }
}

#[corvis::test]
async fn a_controller_cannot_depend_on_a_request_scoped_provider() {
    let err = compile_err(Test::create_testing_module().provider::<startup::RequestContext>().controller::<startup::BadController>()).await;
    assert!(matches!(err, DiError::RequestScopedDependency { .. }), "{err:?}");
    let text = err.to_string();
    assert!(text.contains("BadController") && text.contains("RequestContext") && text.contains("resolve_for"), "{text}");
}

#[corvis::test]
async fn a_singleton_cannot_reach_a_request_scoped_provider_through_a_transient() {
    let err = compile_err(
        Test::create_testing_module().provider::<startup::RequestContext>().provider::<startup::Formatter>().provider::<startup::Report>(),
    )
    .await;
    match &err {
        DiError::RequestScopedDependency { consumer, dependency, chain } => {
            assert_eq!(consumer, "Report");
            assert_eq!(dependency, "RequestContext");
            assert_eq!(chain, "Report -> Formatter -> RequestContext");
        }
        other => panic!("{other:?}"),
    }
}

#[corvis::test]
async fn a_singleton_cannot_have_a_request_field() {
    let err = compile_err(Test::create_testing_module().provider::<startup::Holder>()).await;
    match &err {
        DiError::RequestScopedDependency { consumer, dependency, .. } => {
            assert_eq!(consumer, "Holder");
            assert_eq!(dependency, "Request");
        }
        other => panic!("{other:?}"),
    }
}

#[corvis::test]
async fn a_request_scoped_provider_may_depend_on_a_singleton_and_the_app_starts() {
    compile_ok(Test::create_testing_module().provider::<startup::Clock>().provider::<startup::UsesClock>()).await;
}

#[corvis::test]
async fn a_singleton_factory_cannot_resolve_a_request_scoped_provider() {
    let err = compile_err(
        Test::create_testing_module()
            .provider::<startup::RequestContext>()
            .providers(vec![Provider::use_factory("REPORT", |ctx| async move {
                ctx.get::<startup::RequestContext>().await.map_err(anyhow::Error::from)
            })]),
    )
    .await;
    match &err {
        DiError::OutsideRequest { token, chain } => {
            assert_eq!(token, "RequestContext");
            assert!(chain.contains("REPORT"), "{chain}");
        }
        other => panic!("{other:?}"),
    }
    assert!(err.to_string().contains("RequestContext"));
}

#[corvis::test]
async fn a_global_enhancer_cannot_be_request_scoped() {
    let err = compile_err(Test::create_testing_module().providers(vec![Provider::app_guard::<startup::TenantGuard>()])).await;
    let text = err.to_string();
    assert!(text.contains("TenantGuard") && text.contains("cannot be request-scoped"), "{text}");
}

#[corvis::test]
async fn an_enhancer_referenced_by_type_cannot_be_request_scoped() {
    let mut module = compile_ok(Test::create_testing_module().controller::<startup::TypedGuardController>()).await;
    let err = module.init().await.unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("TenantGuard") && text.contains("cannot be request-scoped"), "{text}");
}

// ------------------------------------------------------------------ validation of lazy providers

mod validation {
    use super::*;

    pub struct Missing;

    #[injectable(scope = "request")]
    pub struct NeedsMissing {
        _missing: Arc<Missing>,
    }

    #[injectable(scope = "request")]
    pub struct CycleA {
        _b: Arc<CycleB>,
    }

    #[injectable(scope = "request")]
    pub struct CycleB {
        _a: Arc<CycleA>,
    }
}

#[corvis::test]
async fn a_missing_dependency_of_a_request_scoped_provider_fails_startup() {
    let err = compile_err(Test::create_testing_module().provider::<validation::NeedsMissing>()).await;
    match &err {
        DiError::UnknownDependencies { requester, token, index, module, .. } => {
            assert_eq!(requester, "NeedsMissing");
            assert_eq!(token, "Missing");
            assert_eq!(*index, 0);
            assert!(!module.is_empty());
        }
        other => panic!("{other:?}"),
    }
}

#[corvis::test]
async fn a_cycle_of_request_scoped_providers_fails_startup() {
    let err = compile_err(Test::create_testing_module().provider::<validation::CycleA>().provider::<validation::CycleB>()).await;
    assert_cycle(&err, &["CycleA", "CycleB"]);
}

// ------------------------------------------------------------------ resolve_for

mod resolving {
    use super::*;

    #[injectable]
    pub struct CatsService {
        #[init(next_id())]
        pub _id: usize,
    }

    #[injectable(scope = "request")]
    pub struct RequestContext;

    /// Needs the request scope through a transient provider.
    #[injectable(scope = "transient")]
    pub struct TransientUser {
        _ctx: Arc<RequestContext>,
    }

    #[injectable(scope = "transient")]
    pub struct PlainTransient;

    /// A private request-scoped provider of a module that does not export it.
    #[injectable(scope = "request")]
    pub struct Secret;

    #[module(providers = [Secret])]
    pub struct VaultModule;

    pub static LATE: Mutex<Option<String>> = Mutex::new(None);
    pub static GATE: tokio::sync::Notify = tokio::sync::Notify::const_new();

    #[controller("resolving")]
    pub struct ResolvingController {
        module_ref: ModuleRef,
    }

    #[routes]
    impl ResolvingController {
        #[get("singleton")]
        async fn singleton(&self, #[req] req: Request) -> String {
            let through_request = self.module_ref.resolve_for::<CatsService>(&req).await.unwrap();
            let stored = self.module_ref.get::<CatsService>().unwrap();
            Arc::ptr_eq(&through_request, &stored).to_string()
        }

        #[get("hidden")]
        async fn hidden(&self, #[req] req: Request) -> String {
            match self.module_ref.resolve_for::<Secret>(&req).await {
                Err(DiError::NotFound { .. }) => "not-found".into(),
                other => format!("unexpected: {:?}", other.map(|_| ())),
            }
        }

        #[get("spawn")]
        async fn spawn(&self, #[req] req: Request) -> &'static str {
            let module_ref = self.module_ref.clone();
            tokio::spawn(async move {
                GATE.notified().await;
                let outcome = match module_ref.resolve_for::<RequestContext>(&req).await {
                    Err(DiError::OutsideRequest { .. }) => "outside-request".to_string(),
                    other => format!("unexpected: {:?}", other.map(|_| ())),
                };
                *LATE.lock().unwrap() = Some(outcome);
            });
            "spawned"
        }
    }

    #[module(
        imports = [VaultModule],
        controllers = [ResolvingController],
        providers = [CatsService, RequestContext, TransientUser, PlainTransient]
    )]
    pub struct ResolvingModule;
}

/// The providers sit in the root module, so the application's own `ModuleRef` sees them.
async fn resolving_root_app() -> corvis::CorvisApplication {
    let builder = Test::create_testing_module()
        .provider::<resolving::CatsService>()
        .provider::<resolving::RequestContext>()
        .provider::<resolving::TransientUser>()
        .provider::<resolving::PlainTransient>();
    let mut app = compile_ok(builder).await.create_corvis_application();
    app.init().await.unwrap();
    app
}

async fn resolving_app() -> corvis::CorvisApplication {
    let mut app = compile_ok(Test::create_testing_module().import::<resolving::ResolvingModule>()).await.create_corvis_application();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn resolve_for_returns_the_stored_singleton() {
    let app = resolving_app().await;
    request(&app).get("/resolving/singleton").send().await.expect_status(200).expect_text("true");
}

#[corvis::test]
async fn resolve_for_does_not_see_providers_the_module_cannot_see() {
    let app = resolving_app().await;
    request(&app).get("/resolving/hidden").send().await.expect_status(200).expect_text("not-found");
}

#[corvis::test]
async fn resolve_for_fails_for_a_request_that_was_never_dispatched() {
    let app = resolving_root_app().await;
    let req = Request::new(http::Method::GET, "/".parse().unwrap(), http::HeaderMap::new(), bytes::Bytes::new()).unwrap();
    let module_ref = app.module_ref();
    let err = module_ref.resolve_for::<resolving::RequestContext>(&req).await.map(|_| ()).unwrap_err();
    assert!(matches!(err, DiError::OutsideRequest { .. }), "{err:?}");
    // Through a transient provider the request scope is still needed.
    let transient = module_ref.resolve_for::<resolving::TransientUser>(&req).await.map(|_| ()).unwrap_err();
    assert!(matches!(transient, DiError::OutsideRequest { .. }), "{transient:?}");
}

#[corvis::test]
async fn resolve_for_resolves_providers_that_are_not_request_scoped_without_a_store() {
    let app = resolving_root_app().await;
    let req = Request::new(http::Method::GET, "/".parse().unwrap(), http::HeaderMap::new(), bytes::Bytes::new()).unwrap();
    let module_ref = app.module_ref();
    let through_request = module_ref.resolve_for::<resolving::CatsService>(&req).await.unwrap();
    assert!(Arc::ptr_eq(&through_request, &module_ref.get::<resolving::CatsService>().unwrap()));
    let a = module_ref.resolve_for::<resolving::PlainTransient>(&req).await.unwrap();
    let b = module_ref.resolve_for::<resolving::PlainTransient>(&req).await.unwrap();
    assert!(!Arc::ptr_eq(&a, &b), "a transient provider is created for each call");
}

#[corvis::test]
async fn resolve_for_works_in_an_application_without_request_scoped_providers() {
    let mut module = compile_ok(Test::create_testing_module().provider::<startup::Clock>()).await;
    module.init().await.unwrap();
    let app = module.create_corvis_application();
    let req = Request::new(http::Method::GET, "/".parse().unwrap(), http::HeaderMap::new(), bytes::Bytes::new()).unwrap();
    let clock = app.module_ref().resolve_for::<startup::Clock>(&req).await.unwrap();
    assert!(Arc::ptr_eq(&clock, &app.get::<startup::Clock>().unwrap()));
}

#[corvis::test]
async fn a_request_kept_after_its_response_cannot_resolve() {
    let app = resolving_app().await;
    request(&app).get("/resolving/spawn").send().await.expect_status(200).expect_text("spawned");
    resolving::GATE.notify_one();
    wait_until("the spawned task", || resolving::LATE.lock().unwrap().is_some()).await;
    assert_eq!(resolving::LATE.lock().unwrap().as_deref(), Some("outside-request"));
}

#[corvis::test]
async fn a_standalone_context_cannot_resolve_a_request_scoped_provider() {
    let mut app = corvis::CorvisFactory::create_application_context::<resolving::ResolvingModule>().await.unwrap();
    let err = app.module_ref().resolve::<resolving::RequestContext>().await.map(|_| ()).unwrap_err();
    match err {
        DiError::OutsideRequest { token, .. } => assert_eq!(token, "RequestContext"),
        other => panic!("{other:?}"),
    }
    app.close().await.unwrap();
}

// ------------------------------------------------------------------ Scoped<T>

mod scoped {
    use corvis::Scoped;

    use super::*;

    pub static CONSTRUCTED: AtomicUsize = AtomicUsize::new(0);

    #[injectable(scope = "request")]
    pub struct ScopedCtx {
        #[init(next_id())]
        pub _id: usize,
    }

    #[injectable(scope = "transient")]
    pub struct Timer;

    #[injectable]
    pub struct CatsSvc;

    pub struct Tenant;

    #[injectable(scope = "request")]
    pub struct Counted {
        #[init(bump(&CONSTRUCTED))]
        _n: usize,
    }

    /// A request-scoped provider the controller's module cannot see.
    #[injectable(scope = "request")]
    pub struct Secret;

    #[derive(Clone)]
    pub struct GuardSaw(pub usize);

    #[injectable]
    pub struct SawGuard;

    #[async_trait]
    impl CanActivate for SawGuard {
        async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
            let scoped = Scoped::<ScopedCtx>::from_context(ctx).await?;
            ctx.request().insert(GuardSaw(Arc::as_ptr(&scoped.into_inner()) as usize));
            Ok(true)
        }
    }

    #[controller("scoped")]
    #[use_guards(SawGuard)]
    pub struct ScopedController;

    #[routes]
    impl ScopedController {
        #[get("same")]
        async fn same(&self, ctx: Scoped<ScopedCtx>, #[req] req: Request) -> String {
            let guard = req.get::<GuardSaw>().expect("the guard ran").0;
            ((&*ctx as *const ScopedCtx as usize) == guard).to_string()
        }

        #[get("addr")]
        async fn addr(&self, ctx: Scoped<ScopedCtx>) -> String {
            tokio::time::sleep(Duration::from_millis(150)).await;
            (&*ctx as *const ScopedCtx as usize).to_string()
        }

        #[get("transient")]
        async fn transient(&self, a: Scoped<Timer>, b: Scoped<Timer>) -> String {
            std::ptr::eq(&*a, &*b).to_string()
        }

        #[get("singleton")]
        async fn singleton(&self, cats: Scoped<CatsSvc>) -> String {
            (&*cats as *const CatsSvc as usize).to_string()
        }

        #[get("tenant")]
        async fn tenant(&self, _tenant: Scoped<Tenant>) -> &'static str {
            "ok"
        }

        #[get("secret")]
        async fn secret(&self, _secret: Scoped<Secret>) -> &'static str {
            "never"
        }

        #[get("counted")]
        async fn counted(&self, _c: Scoped<Counted>) -> &'static str {
            "ok"
        }
    }

    #[module(
        controllers = [ScopedController],
        providers = [
            ScopedCtx, Timer, CatsSvc, Counted,
            Provider::use_factory(Token::of::<Tenant>(), |ctx| async move {
                match ctx.request()?.header("x-tenant") {
                    Some(_) => Ok(Tenant),
                    None => Err(BadRequestException::new("missing tenant").into_anyhow()),
                }
            })
            .scope(Scope::Request),
        ]
    )]
    pub struct ScopedModule;
}

async fn scoped_app() -> corvis::CorvisApplication {
    let module = compile_ok(Test::create_testing_module().import::<scoped::ScopedModule>().provider::<scoped::Secret>()).await;
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn a_scoped_parameter_and_a_guard_share_the_instance() {
    let app = scoped_app().await;
    request(&app).get("/scoped/same").send().await.expect_status(200).expect_text("true");
}

#[corvis::test]
async fn scoped_gives_requests_in_flight_different_instances() {
    let app = scoped_app().await;
    let client = request(&app);
    let (a, b) = tokio::join!(client.get("/scoped/addr").send(), client.get("/scoped/addr").send());
    assert_ne!(a.text(), b.text());
}

#[corvis::test]
async fn scoped_resolves_transient_providers_anew_for_each_parameter() {
    let app = scoped_app().await;
    request(&app).get("/scoped/transient").send().await.expect_status(200).expect_text("false");
}

#[corvis::test]
async fn scoped_resolves_singletons() {
    let app = scoped_app().await;
    let body = request(&app).get("/scoped/singleton").send().await.text();
    assert_eq!(body, (Arc::as_ptr(&app.get::<scoped::CatsSvc>().unwrap()) as usize).to_string());
}

#[corvis::test]
async fn a_factory_http_exception_is_returned_unchanged() {
    let app = scoped_app().await;
    let response = request(&app).get("/scoped/tenant").send().await;
    response.expect_status(400);
    assert_eq!(response.json_value()["message"], "missing tenant");
    request(&app).get("/scoped/tenant").set("x-tenant", "acme").send().await.expect_status(200);
}

#[corvis::test]
async fn a_provider_that_is_not_visible_gives_a_500_that_does_not_name_it() {
    let app = scoped_app().await;
    let response = request(&app).get("/scoped/secret").send().await;
    response.expect_status(500);
    assert!(!response.text().contains("Secret"), "{}", response.text());
    assert_eq!(response.json_value()["message"], "Internal Server Error");
}

#[corvis::test]
async fn the_scope_error_recommends_scoped() {
    let err = compile_err(Test::create_testing_module().provider::<startup::RequestContext>().controller::<startup::BadController>()).await;
    let text = err.to_string();
    assert!(text.contains("Scoped<RequestContext>") && text.contains("resolve_for"), "{text}");
    assert!(text.find("Scoped<RequestContext>") < text.find("resolve_for"), "{text}");
}

#[cfg(feature = "websockets")]
#[corvis::test]
async fn scoped_fails_in_a_websocket_context_and_creates_nothing() {
    use corvis::{ContextType, Scoped, Target};

    let mut module = compile_ok(Test::create_testing_module().import::<scoped::ScopedModule>().provider::<scoped::Secret>()).await;
    module.init().await.unwrap();
    let app = module.create_corvis_application();
    let before = scoped::CONSTRUCTED.load(Ordering::SeqCst);
    let ctx = ExecutionContext::for_ws(
        Request::new(http::Method::GET, "/".parse().unwrap(), http::HeaderMap::new(), bytes::Bytes::new()).unwrap(),
        Arc::new(Target::new("Gateway")),
        Arc::new(Target::new("handler")),
        Some(app.module_ref()),
        Arc::new(()),
        json!(null),
    );
    assert_eq!(ctx.get_type(), ContextType::Ws);
    let err = Scoped::<scoped::Counted>::from_context(&ctx).await.map(|_| ()).unwrap_err();
    assert_eq!(err.status().as_u16(), 500);
    assert_eq!(scoped::CONSTRUCTED.load(Ordering::SeqCst), before);
}

// ------------------------------------------------------------------ nested dispatch

mod nested {
    use std::sync::OnceLock;
    use std::sync::atomic::AtomicBool;

    use super::*;

    pub static APP: OnceLock<Arc<tokio::sync::Mutex<corvis::CorvisApplication>>> = OnceLock::new();
    pub static ENTERED: AtomicBool = AtomicBool::new(false);

    #[injectable(scope = "request")]
    pub struct NestedCtx {
        pub id: usize,
    }

    #[controller("nested")]
    pub struct NestedController {
        module_ref: ModuleRef,
    }

    #[routes]
    impl NestedController {
        /// The first call dispatches a clone of its own request again; that inner call just reports its instance.
        #[get("run")]
        async fn run(&self, #[req] req: Request) -> Value {
            let outer = self.module_ref.resolve_for::<NestedCtx>(&req).await.unwrap();
            if ENTERED.swap(true, Ordering::SeqCst) {
                return json!({ "id": outer.id });
            }
            let inner = APP.get().unwrap().lock().await.handle(req.clone()).await.unwrap();
            let inner_id = inner.json_body().expect("json body")["id"].as_u64().unwrap() as usize;
            // The inner dispatch must not have removed the outer store.
            let again = self.module_ref.resolve_for::<NestedCtx>(&req).await.unwrap();
            json!({ "id": outer.id, "inner": inner_id, "same_after": Arc::ptr_eq(&outer, &again) })
        }
    }

    #[module(controllers = [NestedController], providers = [NestedCtx])]
    pub struct NestedModule;
}

#[corvis::test]
async fn a_nested_dispatch_shares_the_instances_of_the_outer_request() {
    let mut app = compile_ok(Test::create_testing_module().import::<nested::NestedModule>()).await.create_corvis_application();
    app.init().await.unwrap();
    let client = request(&app);
    assert!(nested::APP.set(Arc::new(tokio::sync::Mutex::new(app))).is_ok());
    let body = client.get("/nested/run").send().await.json_value();
    assert_eq!(body["id"], body["inner"], "the nested dispatch built its own instance");
    assert_eq!(body["same_after"], true);
}

#[cfg(feature = "websockets")]
mod gateway {
    use corvis::websocket_gateway;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    use super::*;

    #[injectable(scope = "request")]
    pub struct GatewayCtx;

    #[websocket_gateway(path = "/rs")]
    pub struct RsGateway {
        module_ref: ModuleRef,
    }

    #[handlers]
    impl RsGateway {
        #[subscribe_message("ctx")]
        async fn ctx(&self, #[connected_socket] client: corvis::WsClient) -> Value {
            let outcome = self.module_ref.resolve_for::<GatewayCtx>(client.handshake()).await;
            json!({ "outside": matches!(outcome, Err(DiError::OutsideRequest { .. })) })
        }
    }

    #[module(providers = [GatewayCtx, RsGateway])]
    pub struct GatewayModule;

    #[corvis::test]
    async fn a_gateway_message_handler_cannot_resolve_a_request_scoped_provider() {
        let mut app = corvis::CorvisFactory::create::<GatewayModule>().await.unwrap();
        let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/rs")).await.unwrap();
        ws.send(Message::Text(json!({ "event": "ctx", "data": null }).to_string())).await.unwrap();
        let reply = loop {
            let msg = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("timeout").unwrap().unwrap();
            if let Message::Text(t) = msg {
                break serde_json::from_str::<Value>(&t).unwrap();
            }
        };
        assert_eq!(reply, json!({ "event": "ctx", "data": { "outside": true } }));
        app.close().await.unwrap();
    }
}
