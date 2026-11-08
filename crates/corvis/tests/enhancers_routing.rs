//! Global enhancers registered as providers, their overrides in testing modules, and routing edge
//! cases: prefix exclusions per route, middleware under URI versioning and optional parameters.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use corvis::prelude::*;
use corvis::testing::{Test, request};
use corvis::{DiError, Module, ResolveCtx};

// ------------------------------------------------------------------ helpers

macro_rules! filter {
    ($name:ident) => {
        #[injectable]
        pub struct $name;

        #[async_trait]
        impl ExceptionFilter for $name {
            async fn catch(&self, _exception: HttpException, _host: &ArgumentsHost) -> Response {
                Response::json(json!({ "by": stringify!($name) }))
            }
        }
    };
}

struct AllowAll;

#[async_trait]
impl CanActivate for AllowAll {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Ok(true)
    }
}

// ------------------------------------------------------------------ global filter order

filter!(FilterA);
filter!(FilterB);
filter!(FilterC);
filter!(FilterD);

#[controller("boom")]
pub struct BoomController;

#[routes]
impl BoomController {
    #[get]
    fn boom(&self) -> HttpResult<()> {
        Err(NotFoundException.into())
    }
}

#[module(controllers = [BoomController], providers = [Provider::app_filter::<FilterA>(), Provider::app_filter::<FilterB>()])]
pub struct FiltersModule;

async fn filters_app(bootstrap: impl FnOnce(&mut CorvisApplication)) -> CorvisApplication {
    let mut app = CorvisFactory::create::<FiltersModule>().await.unwrap();
    app.silence_route_logs();
    bootstrap(&mut app);
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn provider_filters_are_tried_last_registered_first() {
    let app = filters_app(|_| {}).await;
    request(&app).get("/boom").send().await.expect_json(json!({ "by": "FilterB" }));
}

#[corvis::test]
async fn bootstrap_filters_come_before_provider_filters_and_last_call_first() {
    let app = filters_app(|app| {
        app.use_global_filters(FilterC);
    })
    .await;
    request(&app).get("/boom").send().await.expect_json(json!({ "by": "FilterC" }));

    let app = filters_app(|app| {
        app.use_global_filters(FilterC);
        app.use_global_filters(FilterD);
    })
    .await;
    request(&app).get("/boom").send().await.expect_json(json!({ "by": "FilterD" }));
}

// ------------------------------------------------------------------ guards: registration order

static GUARD_LOG: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

macro_rules! recording_guard {
    ($name:ident) => {
        #[injectable]
        pub struct $name;

        #[async_trait]
        impl CanActivate for $name {
            async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
                GUARD_LOG.lock().unwrap().push(stringify!($name));
                Ok(true)
            }
        }
    };
}

recording_guard!(GuardA);
recording_guard!(GuardB);

#[controller("ordered")]
pub struct OrderedController;

#[routes]
impl OrderedController {
    #[get]
    fn get(&self) -> Value {
        json!({ "ok": true })
    }
}

#[module(controllers = [OrderedController], providers = [Provider::app_guard::<GuardA>(), Provider::app_guard::<GuardB>()])]
pub struct GuardsModule;

#[corvis::test]
async fn provider_guards_run_in_registration_order() {
    let mut app = CorvisFactory::create::<GuardsModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    request(&app).get("/ordered").send().await.expect_status(200);
    assert_eq!(*GUARD_LOG.lock().unwrap(), ["GuardA", "GuardB"]);
}

// ------------------------------------------------------------------ overrides of global enhancers

static DENY_CALLS: AtomicUsize = AtomicUsize::new(0);
static TRAIL: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

#[injectable]
pub struct RolesGuard;

#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        DENY_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(false)
    }
}

#[injectable]
pub struct TrailGuard;

#[async_trait]
impl CanActivate for TrailGuard {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        TRAIL.lock().unwrap().push("after");
        Ok(true)
    }
}

struct TrailMock;

#[async_trait]
impl CanActivate for TrailMock {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        TRAIL.lock().unwrap().push("mock");
        Ok(true)
    }
}

#[injectable]
pub struct RealFilter;

#[async_trait]
impl ExceptionFilter for RealFilter {
    async fn catch(&self, _e: HttpException, _h: &ArgumentsHost) -> Response {
        Response::json(json!({ "by": "real" }))
    }
}

struct MockFilter;

#[async_trait]
impl ExceptionFilter for MockFilter {
    async fn catch(&self, _e: HttpException, _h: &ArgumentsHost) -> Response {
        Response::json(json!({ "by": "mock" }))
    }
}

#[injectable]
pub struct RealInterceptor;

#[async_trait]
impl CorvisInterceptor for RealInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, _next: CallHandler) -> HandlerResult {
        Ok(Response::json(json!({ "by": "real" })))
    }
}

struct MockInterceptor;

#[async_trait]
impl CorvisInterceptor for MockInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, _next: CallHandler) -> HandlerResult {
        Ok(Response::json(json!({ "by": "mock" })))
    }
}

#[injectable]
pub struct RealPipe;

#[async_trait]
impl PipeTransform for RealPipe {
    async fn transform(&self, value: Value, _metadata: &ArgumentMetadata) -> Result<Value, HttpException> {
        Ok(value.as_str().map_or(value.clone(), |s| json!(format!("{s}-real"))))
    }
}

struct MockPipe;

#[async_trait]
impl PipeTransform for MockPipe {
    async fn transform(&self, value: Value, _metadata: &ArgumentMetadata) -> Result<Value, HttpException> {
        Ok(value.as_str().map_or(value.clone(), |s| json!(format!("{s}-mock"))))
    }
}

#[controller("guarded")]
pub struct GuardedController;

#[routes]
impl GuardedController {
    #[get]
    fn get(&self) -> Value {
        json!({ "ok": true })
    }

    #[get("fail")]
    fn fail(&self) -> HttpResult<()> {
        Err(NotFoundException.into())
    }

    #[get("echo")]
    fn echo(&self, #[query("q")] q: String) -> Value {
        json!({ "q": q })
    }
}

#[module(controllers = [GuardedController], providers = [Provider::app_guard::<RolesGuard>(), Provider::app_guard::<TrailGuard>()])]
pub struct DenyModule;

#[module(controllers = [GuardedController], providers = [Provider::app_filter::<RealFilter>()])]
pub struct FilterModule;

#[module(controllers = [GuardedController], providers = [Provider::app_interceptor::<RealInterceptor>()])]
pub struct InterceptorModule;

#[module(controllers = [GuardedController], providers = [Provider::app_pipe::<RealPipe>()])]
pub struct PipeModule;

#[corvis::test]
async fn override_guard_replaces_a_global_guard_and_keeps_its_position() {
    // Without the override the global guard rejects.
    let mut app = CorvisFactory::create::<DenyModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    request(&app).get("/guarded").send().await.expect_status(403);
    let rejected = DENY_CALLS.load(Ordering::SeqCst);
    assert_eq!(rejected, 1);

    TRAIL.lock().unwrap().clear();
    let module = Test::create_testing_module().import::<DenyModule>().override_guard::<RolesGuard>(TrailMock).compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    request(&app).get("/guarded").send().await.expect_status(200);
    assert_eq!(DENY_CALLS.load(Ordering::SeqCst), rejected, "the original guard must never be called");
    // The mock took the original's place: first, ahead of the second global guard.
    assert_eq!(*TRAIL.lock().unwrap(), ["mock", "after"]);
}

#[corvis::test]
async fn override_filter_replaces_a_global_filter() {
    let mut app = CorvisFactory::create::<FilterModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    request(&app).get("/guarded/fail").send().await.expect_json(json!({ "by": "real" }));

    let module = Test::create_testing_module().import::<FilterModule>().override_filter::<RealFilter>(MockFilter).compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    request(&app).get("/guarded/fail").send().await.expect_json(json!({ "by": "mock" }));
}

#[corvis::test]
async fn override_interceptor_replaces_a_global_interceptor() {
    let module = Test::create_testing_module()
        .import::<InterceptorModule>()
        .override_interceptor::<RealInterceptor>(MockInterceptor)
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    request(&app).get("/guarded").send().await.expect_json(json!({ "by": "mock" }));
}

#[corvis::test]
async fn override_pipe_replaces_a_global_pipe() {
    let mut app = CorvisFactory::create::<PipeModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    request(&app).get("/guarded/echo?q=a").send().await.expect_json(json!({ "q": "a-real" }));

    let module = Test::create_testing_module().import::<PipeModule>().override_pipe::<RealPipe>(MockPipe).compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    request(&app).get("/guarded/echo?q=a").send().await.expect_json(json!({ "q": "a-mock" }));
}

// ------------------------------------------------------------------ overriding the provider behind app_*

pub struct JwtGuard {
    allow: bool,
}

impl Injectable for JwtGuard {
    fn construct(_ctx: ResolveCtx) -> futures::future::BoxFuture<'static, Result<Self, DiError>> {
        Box::pin(async { Ok(JwtGuard { allow: false }) })
    }
}

#[async_trait]
impl CanActivate for JwtGuard {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Ok(self.allow)
    }
}

#[module(controllers = [GuardedController], providers = [Provider::app_guard::<JwtGuard>()])]
pub struct JwtModule;

fn jwt_guard_token() -> Token {
    Token::named(format!("APP_GUARD:{}", std::any::type_name::<JwtGuard>()))
}

#[corvis::test]
async fn overriding_the_provider_of_a_global_guard_keeps_it_global() {
    let module = Test::create_testing_module()
        .import::<JwtModule>()
        .override_provider_token(jwt_guard_token())
        .use_value(JwtGuard { allow: false })
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    // The overriding value is still a global guard: it rejects.
    request(&app).get("/guarded").send().await.expect_status(403);

    let module = Test::create_testing_module()
        .import::<JwtModule>()
        .override_provider_token(jwt_guard_token())
        .use_value(JwtGuard { allow: true })
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    request(&app).get("/guarded").send().await.expect_status(200);
}

#[corvis::test]
async fn overriding_the_provider_of_a_global_guard_with_a_non_guard_fails() {
    let module = Test::create_testing_module()
        .import::<JwtModule>()
        .override_provider_token(jwt_guard_token())
        .use_value("not a guard".to_string())
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    let err = app.init().await.unwrap_err().to_string();
    assert!(err.contains("JwtGuard") && err.contains("not a guard"), "{err}");
}

#[corvis::test]
async fn an_override_through_trait_object_is_accepted_for_a_global_guard() {
    let module = Test::create_testing_module()
        .import::<JwtModule>()
        .override_provider_token(jwt_guard_token())
        .use_value_arc(std::sync::Arc::new(AllowAll) as std::sync::Arc<dyn CanActivate>)
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    request(&app).get("/guarded").send().await.expect_status(200);
}

// ------------------------------------------------------------------ global prefix exclusions

#[controller("")]
pub struct StatusController;

#[routes]
impl StatusController {
    #[get("status")]
    fn status(&self) -> Value {
        json!({ "route": "status" })
    }

    #[get("users")]
    fn users(&self) -> Value {
        json!({ "route": "users" })
    }
}

#[controller("health")]
pub struct HealthController;

#[routes]
impl HealthController {
    #[get]
    fn health(&self) -> Value {
        json!({ "route": "health" })
    }

    #[get("live")]
    fn live(&self) -> Value {
        json!({ "route": "live" })
    }
}

#[module(controllers = [StatusController])]
pub struct StatusModule;

#[module(controllers = [HealthController])]
pub struct HealthModule;

async fn prefixed<M: Module>(exclude: &[&str]) -> CorvisApplication {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.set_global_prefix_with_exclude("api", exclude);
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn excluding_a_single_handler_from_the_prefix() {
    let app = prefixed::<StatusModule>(&["status"]).await;
    let c = request(&app);
    c.get("/status").send().await.expect_status(200);
    c.get("/api/users").send().await.expect_status(200);
    c.get("/api/status").send().await.expect_status(404);
    c.get("/users").send().await.expect_status(404);
}

#[corvis::test]
async fn excluding_a_whole_controller_with_a_wildcard() {
    let app = prefixed::<HealthModule>(&["health/*"]).await;
    let c = request(&app);
    c.get("/health").send().await.expect_status(200);
    c.get("/health/live").send().await.expect_status(200);
    c.get("/api/health").send().await.expect_status(404);
    c.get("/api/health/live").send().await.expect_status(404);
}

#[corvis::test]
async fn an_exclusion_naming_only_the_controller_path_excludes_that_route_only() {
    let app = prefixed::<HealthModule>(&["health"]).await;
    let c = request(&app);
    c.get("/health").send().await.expect_status(200);
    c.get("/api/health/live").send().await.expect_status(200);
    c.get("/health/live").send().await.expect_status(404);
    c.get("/api/health").send().await.expect_status(404);
}

#[corvis::test]
async fn prefix_exclusions_ignore_the_version_segment() {
    let mut app = CorvisFactory::create::<HealthModule>().await.unwrap();
    app.set_global_prefix_with_exclude("api", &["health"]);
    app.enable_versioning(VersioningOptions::uri().default_version("1"));
    app.silence_route_logs();
    app.init().await.unwrap();
    let c = request(&app);
    c.get("/v1/health").send().await.expect_status(200);
    c.get("/api/v1/health/live").send().await.expect_status(200);
}

// ------------------------------------------------------------------ middleware under URI versioning

#[injectable]
pub struct MarkMiddleware;

#[async_trait]
impl CorvisMiddleware for MarkMiddleware {
    async fn use_(&self, req: Request, next: Next) -> HandlerResult {
        let mut res = next.run(req).await?;
        res.set_header("x-mw", "yes");
        Ok(res)
    }
}

#[controller("cats")]
pub struct CatsController;

#[routes]
impl CatsController {
    #[get]
    fn list(&self) -> Value {
        json!({ "cats": [] })
    }
}

#[controller(path = "cats", version = "2")]
pub struct CatsV2Controller;

#[routes]
impl CatsV2Controller {
    #[get]
    fn list(&self) -> Value {
        json!({ "cats": [], "v": 2 })
    }
}

#[controller(path = "health", version = VERSION_NEUTRAL)]
pub struct NeutralHealthController;

#[routes]
impl NeutralHealthController {
    #[get]
    fn health(&self) -> Value {
        json!({ "ok": true })
    }
}

#[module(controllers = [CatsController])]
pub struct MwIncludeModule;

impl CorvisModule for MwIncludeModule {
    fn configure(&self, consumer: &mut MiddlewareConsumer) {
        consumer.apply::<MarkMiddleware>().for_routes("cats");
    }
}

#[module(controllers = [CatsV2Controller])]
pub struct MwExcludeModule;

impl CorvisModule for MwExcludeModule {
    fn configure(&self, consumer: &mut MiddlewareConsumer) {
        consumer.apply::<MarkMiddleware>().exclude("cats").for_routes("*");
    }
}

#[module(controllers = [NeutralHealthController])]
pub struct MwNeutralModule;

impl CorvisModule for MwNeutralModule {
    fn configure(&self, consumer: &mut MiddlewareConsumer) {
        consumer.apply::<MarkMiddleware>().for_routes("health");
    }
}

async fn versioned<M: Module>() -> CorvisApplication {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.enable_versioning(VersioningOptions::uri().default_version("1"));
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn middleware_text_patterns_match_versioned_routes() {
    let app = versioned::<MwIncludeModule>().await;
    request(&app).get("/v1/cats").send().await.expect_status(200).expect_header("x-mw", "yes");
}

#[corvis::test]
async fn middleware_exclusions_cover_versioned_routes() {
    let app = versioned::<MwExcludeModule>().await;
    let res = request(&app).get("/v2/cats").send().await;
    res.expect_status(200);
    assert_eq!(res.header("x-mw"), None);
}

#[corvis::test]
async fn middleware_on_neutral_routes_still_matches() {
    let app = versioned::<MwNeutralModule>().await;
    request(&app).get("/health").send().await.expect_status(200).expect_header("x-mw", "yes");
}

#[corvis::test]
async fn middleware_patterns_without_versioning_are_unchanged() {
    let mut app = CorvisFactory::create::<MwIncludeModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    request(&app).get("/cats").send().await.expect_header("x-mw", "yes");
}

// ------------------------------------------------------------------ optional route parameters

#[controller("cats")]
pub struct OptionalCatsController;

#[routes]
impl OptionalCatsController {
    #[get("count")]
    fn count(&self) -> Value {
        json!({ "route": "count" })
    }

    #[get(":id?")]
    fn one(&self, #[param("id")] id: Option<String>) -> Value {
        json!({ "route": "one", "id": id })
    }
}

#[controller("files")]
pub struct FilesController;

#[routes]
impl FilesController {
    #[get(":dir?/list")]
    fn list(&self, #[param("dir")] dir: Option<String>) -> Value {
        json!({ "dir": dir })
    }

    #[get("pair/:a?/:b?")]
    fn pair(&self, #[param("a")] a: Option<String>, #[param("b")] b: Option<String>) -> Value {
        json!({ "a": a, "b": b })
    }
}

#[module(controllers = [OptionalCatsController, FilesController])]
pub struct OptionalModule;

impl CorvisModule for OptionalModule {
    fn configure(&self, consumer: &mut MiddlewareConsumer) {
        consumer.apply::<MarkMiddleware>().for_routes(RouteSpec::controller::<OptionalCatsController>());
    }
}

#[controller("deep")]
pub struct TooManyOptionalController;

#[routes]
impl TooManyOptionalController {
    #[get(":a?/:b?/:c?/:d?/:e?")]
    fn deep(&self) -> Value {
        json!({})
    }
}

#[module(controllers = [TooManyOptionalController])]
pub struct TooManyOptionalModule;

#[corvis::test]
async fn optional_segment_answers_with_and_without_it() {
    let mut app = CorvisFactory::create::<OptionalModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let c = request(&app);
    c.get("/cats").send().await.expect_status(200).expect_json(json!({ "route": "one", "id": null }));
    c.get("/cats/7").send().await.expect_status(200).expect_json(json!({ "route": "one", "id": "7" }));
    // The optional route does not shadow the static one.
    c.get("/cats/count").send().await.expect_json(json!({ "route": "count" }));
    // A second segment is not part of the route.
    c.get("/cats/7/8").send().await.expect_status(404);
}

#[corvis::test]
async fn optional_segment_in_the_middle() {
    let mut app = CorvisFactory::create::<OptionalModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let c = request(&app);
    c.get("/files/list").send().await.expect_json(json!({ "dir": null }));
    c.get("/files/docs/list").send().await.expect_json(json!({ "dir": "docs" }));
}

#[corvis::test]
async fn consecutive_optional_segments_bind_left_to_right() {
    let mut app = CorvisFactory::create::<OptionalModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let c = request(&app);
    c.get("/files/pair").send().await.expect_json(json!({ "a": null, "b": null }));
    c.get("/files/pair/1").send().await.expect_json(json!({ "a": "1", "b": null }));
    c.get("/files/pair/1/2").send().await.expect_json(json!({ "a": "1", "b": "2" }));
}

#[corvis::test]
async fn controller_middleware_covers_the_optional_variants() {
    let mut app = CorvisFactory::create::<OptionalModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let c = request(&app);
    c.get("/cats").send().await.expect_header("x-mw", "yes");
    c.get("/cats/7").send().await.expect_header("x-mw", "yes");
}

#[corvis::test]
async fn more_than_four_optional_segments_fail_at_startup() {
    let mut app = CorvisFactory::create::<TooManyOptionalModule>().await.unwrap();
    app.silence_route_logs();
    let err = app.init().await.unwrap_err().to_string();
    assert!(err.contains("TooManyOptionalController") && err.contains("5 optional segments"), "{err}");
}
