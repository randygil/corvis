//! Strategies written by the application: the pattern of the guide "Writing a strategy" (`DeviceStrategy` below is the
//! code of that page) against a recorded `passport-custom` 1.2.1 strategy with an injected service
//! (`tests/fixtures/auth/custom.mjs`), the testing override, and what the framework guarantees around a strategy of
//! your own: an error or a panic never lets a request through. No cargo feature is needed, and no Node at test time.

use std::sync::atomic::{AtomicUsize, Ordering};

use corvis::Module;
use corvis::auth::{AuthGuard, AuthenticateOptions, AuthenticatedUser, Strategy, StrategyOutcome, User};
use corvis::prelude::*;
use corvis::testing::{Test, request};
use serde::Serialize;

const FIXTURE: &str = include_str!("fixtures/auth/custom.json");

static HANDLER_RUNS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Serialize)]
pub struct Device {
    id: String,
    name: String,
}

#[derive(Debug)]
pub enum DevicesError {
    Blocked,
    Down,
}

impl std::fmt::Display for DevicesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("devices store error")
    }
}

impl std::error::Error for DevicesError {}

// ---- the strategy of the guide ----

#[injectable]
pub struct DevicesService;

impl DevicesService {
    async fn find(&self, id: &str) -> Result<Option<Device>, DevicesError> {
        match id {
            "dev-blocked" => Err(DevicesError::Blocked),
            "dev-broken" => Err(DevicesError::Down),
            "dev-1" | "dev-info" => Ok(Some(Device { id: id.to_string(), name: format!("device {id}") })),
            _ => Ok(None),
        }
    }
}

#[injectable]
pub struct DeviceStrategy {
    devices: Arc<DevicesService>,
}

#[async_trait]
impl Strategy for DeviceStrategy {
    fn name(&self) -> &str {
        "device"
    }

    async fn authenticate(&self, ctx: &ExecutionContext, _options: &AuthenticateOptions) -> StrategyOutcome {
        // Read the credential, bounded, from the places this strategy accepts and no other.
        let Some(id) = ctx.request().header("x-device-id").filter(|id| !id.is_empty() && id.len() <= 64) else {
            return StrategyOutcome::Fail { challenge: None, status: None };
        };
        match self.devices.find(id).await {
            Ok(Some(device)) => {
                let info = (id == "dev-info").then(|| json!({ "scope": "telemetry" }));
                match AuthenticatedUser::new(device) {
                    Ok(user) => StrategyOutcome::Success { user, info },
                    Err(e) => StrategyOutcome::Error(e),
                }
            }
            Ok(None) => StrategyOutcome::Fail { challenge: None, status: None },
            // An error ends the request: it is never a way in.
            Err(DevicesError::Blocked) => StrategyOutcome::Error(ForbiddenException::new("Device blocked")),
            Err(e) => StrategyOutcome::Error(HttpException::internal(e)),
        }
    }
}

/// The first strategy of the chain cases: accepts one bearer token.
#[injectable]
pub struct BearerUser;

#[async_trait]
impl Strategy for BearerUser {
    fn name(&self) -> &str {
        "bearer-user"
    }
    async fn authenticate(&self, ctx: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
        match ctx.request().header("authorization") {
            Some("Bearer user-token-0000000000000000000000000001") => {
                StrategyOutcome::Success { user: AuthenticatedUser::new(json!({ "id": "user-1" })).unwrap(), info: None }
            }
            _ => StrategyOutcome::Fail { challenge: None, status: None },
        }
    }
}

// ---- strategies that misbehave ----

/// Returns an error whatever the request.
#[injectable]
pub struct Erroring;

#[async_trait]
impl Strategy for Erroring {
    fn name(&self) -> &str {
        "erroring"
    }
    async fn authenticate(&self, _: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
        StrategyOutcome::Error(HttpException::internal(DevicesError::Down))
    }
}

/// Panics with the credential in the message.
#[injectable]
pub struct Panicking;

#[async_trait]
impl Strategy for Panicking {
    fn name(&self) -> &str {
        "panicking"
    }
    async fn authenticate(&self, ctx: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
        panic!("credential {:?} broke the strategy", ctx.request().header("x-device-id"));
    }
}

#[injectable]
pub struct Succeeding;

#[async_trait]
impl Strategy for Succeeding {
    fn name(&self) -> &str {
        "succeeding"
    }
    async fn authenticate(&self, _: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
        StrategyOutcome::Success { user: AuthenticatedUser::new(json!({ "id": "anyone" })).unwrap(), info: None }
    }
}

#[controller("")]
pub struct Routes;

#[routes]
impl Routes {
    #[get("device")]
    #[use_guards(AuthGuard::new("device"))]
    fn device(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[get("device-info")]
    #[use_guards(AuthGuard::new("device"))]
    fn device_info(&self, req: Request) -> Value {
        json!({ "info": req.auth_info() })
    }

    #[get("device-typed")]
    #[use_guards(AuthGuard::new("device"))]
    fn device_typed(&self, device: User<Device>) -> Value {
        json!({ "name": device.name })
    }

    #[get("chain")]
    #[use_guards(AuthGuard::chain(["bearer-user", "device"]))]
    fn chain(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[get("erroring")]
    #[use_guards(AuthGuard::new("erroring"))]
    fn erroring(&self) -> Value {
        HANDLER_RUNS.fetch_add(1, Ordering::SeqCst);
        json!({ "ran": true })
    }

    #[get("erroring-optional")]
    #[use_guards(AuthGuard::new("erroring").optional())]
    fn erroring_optional(&self) -> Value {
        HANDLER_RUNS.fetch_add(1, Ordering::SeqCst);
        json!({ "ran": true })
    }

    #[get("panicking")]
    #[use_guards(AuthGuard::new("panicking"))]
    fn panicking(&self) -> Value {
        HANDLER_RUNS.fetch_add(1, Ordering::SeqCst);
        json!({ "ran": true })
    }

    #[get("panicking-optional")]
    #[use_guards(AuthGuard::new("panicking").optional())]
    fn panicking_optional(&self) -> Value {
        HANDLER_RUNS.fetch_add(1, Ordering::SeqCst);
        json!({ "ran": true })
    }

    #[get("panicking-chain")]
    #[use_guards(AuthGuard::chain(["panicking", "succeeding"]))]
    fn panicking_chain(&self) -> Value {
        HANDLER_RUNS.fetch_add(1, Ordering::SeqCst);
        json!({ "ran": true })
    }
}

#[module(
    providers = [
        Provider::class::<DevicesService>(),
        Provider::strategy::<DeviceStrategy>(),
        Provider::strategy::<BearerUser>(),
        Provider::strategy::<Erroring>(),
        Provider::strategy::<Panicking>(),
        Provider::strategy::<Succeeding>(),
    ],
    controllers = [Routes],
)]
pub struct AppModule;

async fn app() -> CorvisApplication {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

async fn get(app: &CorvisApplication, path: &str, headers: &[(&str, &str)]) -> corvis::testing::TestResponse {
    let mut call = request(app).get(path);
    for (k, v) in headers {
        call = call.header(k, v);
    }
    call.send().await
}

#[corvis::test]
async fn every_recorded_case_is_replayed() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(fixture["versions"]["passport-custom"], "1.2.1");
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() >= 15, "{} cases", cases.len());
    let app = app().await;
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let headers: Vec<(&str, &str)> =
            case["request"]["headers"].as_object().unwrap().iter().map(|(k, v)| (k.as_str(), v.as_str().unwrap())).collect();
        let r = get(&app, case["request"]["path"].as_str().unwrap(), &headers).await;
        let recorded = &case["response"];
        assert_eq!(r.status as u64, recorded["status"].as_u64().unwrap(), "{name}: status, body {}", r.text());
        assert_eq!(r.json_value(), recorded["body"], "{name}: body");
    }
}

#[corvis::test]
async fn a_strategy_with_an_injected_service_authenticates() {
    let app = app().await;
    let r = get(&app, "/device-typed", &[("x-device-id", "dev-1")]).await;
    assert_eq!((r.status, r.json_value()), (200, json!({ "name": "device dev-1" })));
}

#[corvis::test]
async fn the_info_of_a_success_is_the_auth_info() {
    let app = app().await;
    let r = get(&app, "/device-info", &[("x-device-id", "dev-info")]).await;
    assert_eq!(r.json_value(), json!({ "info": { "scope": "telemetry" } }));
    let r = get(&app, "/device-info", &[("x-device-id", "dev-1")]).await;
    assert_eq!(r.json_value(), json!({ "info": null }));
}

// ---- fail closed ----

#[corvis::test]
async fn an_error_of_a_strategy_ends_the_request_even_behind_an_optional_guard() {
    let app = app().await;
    let before = HANDLER_RUNS.load(Ordering::SeqCst);
    for path in ["/erroring", "/erroring-optional"] {
        let r = get(&app, path, &[]).await;
        assert_eq!(r.status, 500, "{path}");
        assert_eq!(r.json_value(), json!({ "statusCode": 500, "message": "Internal server error" }), "{path}");
    }
    assert_eq!(HANDLER_RUNS.load(Ordering::SeqCst), before, "no handler ran");
}

#[corvis::test]
async fn a_panic_in_a_strategy_is_a_uniform_500_and_never_a_way_in() {
    let app = app().await;
    let before = HANDLER_RUNS.load(Ordering::SeqCst);
    for path in ["/panicking", "/panicking-optional", "/panicking-chain"] {
        let r = get(&app, path, &[("x-device-id", "secret-device-credential")]).await;
        assert_eq!(r.status, 500, "{path}");
        assert_eq!(r.json_value(), json!({ "statusCode": 500, "message": "Internal server error" }), "{path}");
        assert!(!r.text().contains("secret-device-credential"), "{path}");
    }
    assert_eq!(HANDLER_RUNS.load(Ordering::SeqCst), before, "no handler ran");
    // The application keeps serving.
    assert_eq!(get(&app, "/device", &[("x-device-id", "dev-1")]).await.status, 200);
}

// ---- startup checks ----

#[controller("")]
pub struct TypoRoutes;

/// A guard around an `AuthGuard` that lists it in `inner_guards`, as the example app's global guard does.
pub struct Wrapper {
    inner: AuthGuard,
}

#[async_trait]
impl CanActivate for Wrapper {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        self.inner.can_activate(ctx).await
    }

    fn inner_guards(&self) -> Vec<&dyn CanActivate> {
        vec![&self.inner]
    }
}

#[routes]
impl TypoRoutes {
    #[get("typo")]
    #[use_guards(Wrapper { inner: AuthGuard::new("devise") })]
    fn typo(&self) -> Value {
        json!({})
    }
}

#[module(providers = [Provider::class::<DevicesService>(), Provider::strategy::<DeviceStrategy>()], controllers = [TypoRoutes])]
pub struct TypoModule;

#[injectable]
pub struct Nameless;

#[async_trait]
impl Strategy for Nameless {
    fn name(&self) -> &str {
        ""
    }
    async fn authenticate(&self, _: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
        StrategyOutcome::Pass
    }
}

#[module(providers = [Provider::strategy::<Nameless>()])]
pub struct NamelessModule;

async fn startup_error<M: Module>() -> String {
    match CorvisFactory::create::<M>().await {
        Ok(mut app) => match app.init().await {
            Ok(()) => panic!("the application started"),
            Err(e) => e.to_string(),
        },
        Err(e) => e.to_string(),
    }
}

#[corvis::test]
async fn a_misspelled_strategy_name_inside_a_wrapper_guard_stops_startup() {
    let error = startup_error::<TypoModule>().await;
    assert!(error.contains("\"devise\"") && error.contains("not registered"), "{error}");
}

#[corvis::test]
async fn a_strategy_without_a_name_stops_startup() {
    let error = startup_error::<NamelessModule>().await;
    assert!(error.contains("invalid name") && error.contains("empty"), "{error}");
}

// ---- testing ----

/// Always succeeds, under the name of the strategy it replaces.
pub struct StubDevice;

#[async_trait]
impl Strategy for StubDevice {
    fn name(&self) -> &str {
        "device"
    }
    async fn authenticate(&self, _: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
        StrategyOutcome::Success {
            user: AuthenticatedUser::new(Device { id: "stub".into(), name: "stub device".into() }).unwrap(),
            info: None,
        }
    }
}

#[module(providers = [Provider::class::<DevicesService>(), Provider::strategy::<DeviceStrategy>()], controllers = [Routes2])]
pub struct DevicesModule;

#[controller("")]
pub struct Routes2;

#[routes]
impl Routes2 {
    #[get("device")]
    #[use_guards(AuthGuard::new("device"))]
    fn device(&self, device: User<Device>) -> Value {
        json!({ "name": device.name })
    }
}

#[corvis::test]
async fn a_testing_module_replaces_a_strategy() {
    let module =
        Test::create_testing_module().import::<DevicesModule>().override_strategy::<DeviceStrategy>(StubDevice).compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    // No device header and no real check: the stub answers.
    let r = request(&app).get("/device").send().await;
    assert_eq!((r.status, r.json_value()), (200, json!({ "name": "stub device" })));

    // Without the override the real strategy refuses the same request.
    let module = Test::create_testing_module().import::<DevicesModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    assert_eq!(request(&app).get("/device").send().await.status, 401);
}
