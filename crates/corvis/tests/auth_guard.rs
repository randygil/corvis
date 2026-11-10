//! The strategy registry, `AuthGuard` and `PassportModule` with Corvis strategies: registration across modules,
//! startup checks, the user extractor, optional and failing guards. The behavior shared with the original is replayed
//! from recorded fixtures by `tests/auth_strategies.rs`.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use corvis::Module;
use corvis::auth::{
    AuthGuard, AuthResult, AuthenticateOptions, AuthenticatedUser, Authenticator, PassportModule, PassportModuleOptions, Strategy,
    StrategyOutcome, User,
};
use corvis::prelude::*;
use corvis::testing::request;
use serde::{Deserialize, Serialize};

static INIT_RAN: AtomicBool = AtomicBool::new(false);
static HANDLER_CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthPayload {
    id: u32,
    role: String,
}

/// A user whose `Serialize` fails.
#[derive(Clone)]
pub struct Broken;

impl Serialize for Broken {
    fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("no json"))
    }
}

// ---- a service injected into a strategy of another module ----

#[injectable]
pub struct UsersService;

impl UsersService {
    fn find(&self, id: &str) -> Option<AuthPayload> {
        (id == "7").then(|| AuthPayload { id: 7, role: "admin".into() })
    }
}

#[injectable]
pub struct HeaderStrategy {
    users: Arc<UsersService>,
}

#[async_trait]
impl Strategy for HeaderStrategy {
    fn name(&self) -> &str {
        "header"
    }
    async fn authenticate(&self, ctx: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
        let Some(user) = ctx.request().header("x-user").and_then(|id| self.users.find(id)) else {
            return StrategyOutcome::Fail { challenge: None, status: None };
        };
        match AuthenticatedUser::new(user) {
            Ok(user) => StrategyOutcome::Success { user, info: Some(json!({ "scope": "read" })) },
            Err(e) => StrategyOutcome::Error(e),
        }
    }
}

/// Always fails, answers with a fixed outcome, and counts how often it ran.
macro_rules! fixed_strategy {
    ($name:ident, $label:literal, $outcome:expr) => {
        #[injectable]
        pub struct $name;

        #[async_trait]
        impl Strategy for $name {
            fn name(&self) -> &str {
                $label
            }
            async fn authenticate(&self, _: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
                $outcome
            }
        }
    };
}

fixed_strategy!(
    StrategyOne,
    "one",
    StrategyOutcome::Success { user: AuthenticatedUser::new(AuthPayload { id: 1, role: "one".into() }).unwrap(), info: None }
);
fixed_strategy!(
    StrategyTwo,
    "two",
    StrategyOutcome::Success { user: AuthenticatedUser::new(AuthPayload { id: 2, role: "two".into() }).unwrap(), info: None }
);
fixed_strategy!(
    StrategyBroken,
    "broken",
    match AuthenticatedUser::new(Broken) {
        Ok(user) => StrategyOutcome::Success { user, info: None },
        Err(e) => StrategyOutcome::Error(e),
    }
);
fixed_strategy!(StrategyFails, "fails", StrategyOutcome::Fail { challenge: None, status: None });
// Two types that report the same name.
fixed_strategy!(JwtOne, "jwt", StrategyOutcome::Pass);
fixed_strategy!(JwtTwo, "jwt", StrategyOutcome::Pass);

// ---- controllers ----

#[controller("me")]
pub struct MeController;

#[routes]
impl MeController {
    #[get("typed")]
    #[use_guards(AuthGuard::new("header"))]
    fn typed(&self, user: User<AuthPayload>, req: Request) -> Value {
        json!({
            "typed": user.0,
            "json": req.user_as::<AuthPayload>(),
            "info": req.auth_info(),
            "viaGet": req.get::<AuthPayload>(),
        })
    }

    #[get("open")]
    fn open(&self, user: User<AuthPayload>) -> Value {
        json!({ "user": user.0 })
    }

    #[get("optional")]
    #[use_guards(AuthGuard::new("header").optional())]
    fn optional(&self, req: Request) -> Value {
        json!({ "user": req.user(), "info": req.auth_info() })
    }

    #[get("custom")]
    #[use_guards(AuthGuard::new("header").on_failure(|failures| UnauthorizedException::new(format!("Token required ({})", failures.len()))))]
    fn custom(&self) -> Value {
        json!({ "ok": true })
    }

    #[get("pass")]
    #[use_guards(PreloadGuard, AuthGuard::new("pass"))]
    fn pass(&self, req: Request) -> Value {
        json!({ "user": req.user() })
    }

    #[get("broken")]
    #[use_guards(AuthGuard::new("broken"))]
    fn broken(&self) -> Value {
        HANDLER_CALLS.fetch_add(1, Ordering::SeqCst);
        json!({ "ok": true })
    }
}

/// Stores a JSON user, as an earlier guard would.
#[injectable]
pub struct PreloadGuard;

#[async_trait]
impl CanActivate for PreloadGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        ctx.request().set_user(json!({ "id": 99 }));
        Ok(true)
    }
}

fixed_strategy!(PassStrategy, "pass", StrategyOutcome::Pass);

#[controller("two")]
pub struct TwoController;

#[routes]
impl TwoController {
    #[get("one")]
    #[use_guards(AuthGuard::new("one"))]
    fn one(&self, user: User<AuthPayload>) -> Value {
        json!({ "role": user.role })
    }

    #[get("two")]
    #[use_guards(AuthGuard::new("two"))]
    fn two(&self, user: User<AuthPayload>) -> Value {
        json!({ "role": user.role })
    }

    #[get("default")]
    #[use_guards(AuthGuard::default())]
    fn default(&self, user: User<AuthPayload>) -> Value {
        json!({ "role": user.role })
    }
}

#[controller("late")]
pub struct LateController;

/// A user-written guard that calls the `Authenticator` directly with a name nobody registered.
#[injectable]
pub struct DirectGuard;

#[async_trait]
impl CanActivate for DirectGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let authenticator = ctx.module_ref().unwrap().get::<Authenticator>().unwrap();
        match authenticator.authenticate(ctx, &["nobody".to_string()], &AuthenticateOptions::new()).await? {
            AuthResult::Success { .. } | AuthResult::Pass | AuthResult::Redirected | AuthResult::Failed { .. } => Ok(true),
        }
    }
}

#[routes]
impl LateController {
    #[get("x")]
    #[use_guards(DirectGuard)]
    fn x(&self) -> Value {
        json!({ "ok": true })
    }
}

/// A composite guard around an `AuthGuard` that lists it in `inner_guards`.
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

#[controller("wrapped")]
pub struct WrappedController;

#[routes]
impl WrappedController {
    #[get("x")]
    #[use_guards(Wrapper { inner: AuthGuard::new("headr") })]
    fn x(&self) -> Value {
        json!({ "ok": true })
    }
}

/// A guard that lists itself as its inner guard.
#[injectable]
pub struct Loop;

#[async_trait]
impl CanActivate for Loop {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Ok(true)
    }

    fn inner_guards(&self) -> Vec<&dyn CanActivate> {
        vec![self]
    }
}

#[controller("looping")]
pub struct LoopingController;

#[routes]
impl LoopingController {
    #[get("x")]
    #[use_guards(Loop)]
    fn x(&self) -> Value {
        json!({ "ok": true })
    }
}

#[controller("wrapped-ok")]
pub struct WrappedOkController;

#[routes]
impl WrappedOkController {
    #[get("x")]
    #[use_guards(Wrapper { inner: AuthGuard::new("header") })]
    fn x(&self) -> Value {
        json!({ "ok": true })
    }
}

#[injectable]
pub struct InitProbe;

#[async_trait]
impl OnModuleInit for InitProbe {
    async fn on_module_init(&self) -> corvis::Result<()> {
        INIT_RAN.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[controller("typo")]
pub struct TypoController;

#[routes]
impl TypoController {
    #[get("cats")]
    #[use_guards(AuthGuard::new("jtw"))]
    fn cats(&self) -> Value {
        json!({ "ok": true })
    }

    #[post("cats")]
    #[use_guards(AuthGuard::chain(["header", "nope"]))]
    fn create(&self) -> Value {
        json!({ "ok": true })
    }
}

#[controller("def")]
pub struct DefaultController;

#[routes]
impl DefaultController {
    #[get("x")]
    #[use_guards(AuthGuard::default())]
    fn x(&self) -> Value {
        json!({ "ok": true })
    }
}

#[controller("empty")]
pub struct EmptyChainController;

#[routes]
impl EmptyChainController {
    #[get("x")]
    #[use_guards(AuthGuard::chain(Vec::<String>::new()))]
    fn x(&self) -> Value {
        json!({ "ok": true })
    }
}

#[controller("opt")]
pub struct OptionalOnlyController;

#[routes]
impl OptionalOnlyController {
    #[get("x")]
    #[use_guards(AuthGuard::new("header").optional())]
    fn x(&self) -> Value {
        json!({ "ok": true })
    }
}

// ---- modules ----

#[module(providers = [UsersService], exports = [UsersService])]
pub struct UsersModule;

/// The strategy lives in one module, the route that uses it in another.
#[module(imports = [UsersModule], providers = [Provider::strategy::<HeaderStrategy>()])]
pub struct StrategiesModule;

#[module(imports = [StrategiesModule], controllers = [MeController])]
pub struct MeModule;

#[module(imports = [MeModule], providers = [
    Provider::strategy::<StrategyBroken>(), Provider::strategy::<PassStrategy>(),
])]
pub struct AppModule;

#[module(
    imports = [PassportModule::register(PassportModuleOptions::new().default_strategy("two"))],
    providers = [Provider::strategy::<StrategyOne>(), Provider::strategy::<StrategyTwo>()],
    controllers = [TwoController]
)]
pub struct TwoModule;

#[module(
    imports = [PassportModule::register(PassportModuleOptions::new().default_strategy("nobody"))],
    providers = [Provider::strategy::<StrategyOne>()],
    controllers = [DefaultController]
)]
pub struct DefaultMissingModule;

#[module(
    imports = [PassportModule::register(PassportModuleOptions::new().default_strategy("one"))],
    providers = [Provider::strategy::<StrategyOne>()],
    controllers = [DefaultController]
)]
pub struct DefaultOneModule;

#[module(
    imports = [PassportModule::register(PassportModuleOptions::new().default_strategies(["fails", "one"]))],
    providers = [Provider::strategy::<StrategyOne>(), Provider::strategy::<StrategyFails>()],
    controllers = [DefaultController]
)]
pub struct DefaultChainModule;

#[module(
    imports = [
        PassportModule::register(PassportModuleOptions::new().default_strategy("one")),
        PassportModule::register(PassportModuleOptions::new().default_strategy("two")),
    ],
    providers = [Provider::strategy::<StrategyOne>(), Provider::strategy::<StrategyTwo>()],
    controllers = [DefaultController]
)]
pub struct DefaultConflictModule;

#[module(
    imports = [
        PassportModule::register(PassportModuleOptions::new().default_strategy("one")),
        PassportModule::register(PassportModuleOptions::new().default_strategy("one")),
    ],
    providers = [Provider::strategy::<StrategyOne>()],
    controllers = [DefaultController]
)]
pub struct DefaultSameModule;

#[module(providers = [Provider::strategy::<JwtOne>(), Provider::strategy::<JwtTwo>()])]
pub struct DuplicateModule;

#[module(providers = [InitProbe, Provider::strategy::<StrategyOne>(), Provider::strategy::<HeaderStrategy>(), UsersService], controllers = [TypoController])]
pub struct TypoModule;

#[module(providers = [Provider::strategy::<StrategyOne>()], controllers = [DefaultController])]
pub struct NoDefaultModule;

#[module(providers = [Provider::strategy::<HeaderStrategy>(), UsersService], controllers = [WrappedController])]
pub struct WrappedTypoModule;

#[module(providers = [Provider::strategy::<HeaderStrategy>(), UsersService], controllers = [WrappedOkController])]
pub struct WrappedOkModule;

#[module(providers = [Provider::strategy::<StrategyOne>()], controllers = [LoopingController])]
pub struct LoopingModule;

#[module(providers = [Provider::strategy::<StrategyOne>()], controllers = [EmptyChainController])]
pub struct EmptyChainModule;

#[module(providers = [Provider::strategy::<StrategyOne>()], controllers = [LateController])]
pub struct LateModule;

#[module(providers = [Provider::strategy::<HeaderStrategy>(), UsersService], controllers = [OptionalOnlyController])]
pub struct OptionalOnlyModule;

async fn app<M: Module>() -> CorvisApplication {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.silence_route_logs();
    app
}

async fn started<M: Module>() -> CorvisApplication {
    let mut app = app::<M>().await;
    app.init().await.unwrap();
    app
}

async fn init_error<M: Module>() -> String {
    let mut app = app::<M>().await;
    app.init().await.unwrap_err().to_string()
}

// ---- registry ----

#[corvis::test]
async fn a_strategy_is_found_by_name_from_another_module_with_its_injected_service() {
    let app = started::<AppModule>().await;
    let r = request(&app).get("/me/typed").header("x-user", "7").send().await;
    r.expect_status(200);
    let user = json!({ "id": 7, "role": "admin" });
    r.expect_json(json!({ "typed": user, "json": user, "info": { "scope": "read" }, "viaGet": null }));
}

#[corvis::test]
async fn two_strategies_with_different_names_serve_their_own_routes() {
    let app = started::<TwoModule>().await;
    request(&app).get("/two/one").send().await.expect_status(200).expect_json(json!({ "role": "one" }));
    request(&app).get("/two/two").send().await.expect_status(200).expect_json(json!({ "role": "two" }));
}

#[corvis::test]
async fn a_duplicate_strategy_name_stops_startup_naming_both_types() {
    let error = init_error::<DuplicateModule>().await;
    assert!(error.contains("\"jwt\"") && error.contains("JwtOne") && error.contains("JwtTwo"), "{error}");
}

// ---- default strategy ----

#[corvis::test]
async fn the_default_strategy_is_used_by_a_default_guard() {
    let app = started::<TwoModule>().await;
    request(&app).get("/two/default").send().await.expect_status(200).expect_json(json!({ "role": "two" }));
}

#[corvis::test]
async fn a_default_chain_tries_its_strategies_in_order() {
    let app = started::<DefaultChainModule>().await;
    request(&app).get("/def/x").send().await.expect_status(200);
}

#[corvis::test]
async fn a_default_guard_without_a_default_strategy_stops_startup() {
    let error = init_error::<NoDefaultModule>().await;
    assert!(error.contains("GET /def/x") && error.contains("no default strategy is configured"), "{error}");
}

#[corvis::test]
async fn an_unregistered_default_strategy_stops_startup() {
    let error = init_error::<DefaultMissingModule>().await;
    assert!(error.contains("GET /def/x") && error.contains("the default strategy \"nobody\" is not registered"), "{error}");
}

#[corvis::test]
async fn two_different_defaults_stop_startup_and_two_equal_ones_do_not() {
    let error = init_error::<DefaultConflictModule>().await;
    assert!(error.contains("different default strategies"), "{error}");
    started::<DefaultSameModule>().await;
    started::<DefaultOneModule>().await;
}

// ---- startup checks ----

#[corvis::test]
async fn a_typo_in_a_guard_stops_startup_before_any_init_hook() {
    INIT_RAN.store(false, Ordering::SeqCst);
    let error = init_error::<TypoModule>().await;
    assert!(error.contains("GET /typo/cats") && error.contains("\"jtw\""), "{error}");
    assert!(error.contains("POST /typo/cats") && error.contains("\"nope\""), "every problem is listed: {error}");
    assert!(!INIT_RAN.load(Ordering::SeqCst), "no init hook ran");
}

#[corvis::test]
async fn a_wrapper_that_lists_its_inner_guard_is_checked_at_startup() {
    let error = init_error::<WrappedTypoModule>().await;
    assert!(error.contains("GET /wrapped/x") && error.contains("\"headr\""), "{error}");
    started::<WrappedOkModule>().await;
}

#[corvis::test]
async fn a_guard_that_returns_itself_stops_startup_naming_the_route() {
    let error = init_error::<LoopingModule>().await;
    assert!(error.contains("GET /looping/x") && error.contains("32 levels"), "{error}");
}

#[corvis::test]
async fn an_empty_chain_stops_startup() {
    let error = init_error::<EmptyChainModule>().await;
    assert!(error.contains("GET /empty/x") && error.contains("no strategy names"), "{error}");
}

#[corvis::test]
async fn a_name_resolved_only_at_request_time_answers_500() {
    let app = started::<LateModule>().await;
    request(&app)
        .get("/late/x")
        .send()
        .await
        .expect_status(500)
        .expect_json(json!({ "statusCode": 500, "message": "Internal server error" }));
}

#[corvis::test]
async fn a_global_guard_with_a_typo_stops_startup() {
    let mut app = app::<TwoModule>().await;
    app.use_global_guards(AuthGuard::new("jtw"));
    let error = app.init().await.unwrap_err().to_string();
    assert!(error.contains("\"jtw\"") && error.contains("GET /two/one"), "{error}");
}

// ---- the user ----

#[corvis::test]
async fn an_extractor_without_a_user_answers_the_401() {
    let app = started::<AppModule>().await;
    request(&app).get("/me/open").send().await.expect_status(401).expect_json(json!({ "message": "Unauthorized", "statusCode": 401 }));
}

#[corvis::test]
async fn a_user_that_does_not_serialize_is_a_500_and_the_handler_does_not_run() {
    let app = started::<AppModule>().await;
    request(&app)
        .get("/me/broken")
        .send()
        .await
        .expect_status(500)
        .expect_json(json!({ "statusCode": 500, "message": "Internal server error" }));
    assert_eq!(HANDLER_CALLS.load(Ordering::SeqCst), 0);
}

#[corvis::test]
async fn an_optional_guard_without_credentials_runs_the_handler_without_a_user() {
    let app = started::<AppModule>().await;
    request(&app).get("/me/optional").send().await.expect_status(200).expect_json(json!({ "user": null, "info": null }));
    request(&app)
        .get("/me/optional")
        .header("x-user", "7")
        .send()
        .await
        .expect_json(json!({ "user": { "id": 7, "role": "admin" }, "info": { "scope": "read" } }));
}

#[corvis::test]
async fn a_failing_guard_answers_a_custom_exception() {
    let app = started::<AppModule>().await;
    request(&app)
        .get("/me/custom")
        .send()
        .await
        .expect_status(401)
        .expect_json(json!({ "message": "Token required (1)", "error": "Unauthorized", "statusCode": 401 }));
}

#[corvis::test]
async fn a_passing_strategy_removes_a_json_user_stored_earlier() {
    let app = started::<AppModule>().await;
    request(&app).get("/me/pass").send().await.expect_status(200).expect_json(json!({ "user": null }));
}

// ---- route posture ----

#[corvis::test]
async fn an_optional_guard_does_not_protect_a_route() {
    let mut app = app::<OptionalOnlyModule>().await;
    app.require_route_posture();
    let error = app.init().await.unwrap_err().to_string();
    assert!(error.contains("Route posture check failed") && error.contains("/opt/x"), "{error}");
    // The same route with a guard that controls access passes the check.
    assert!(AuthGuard::new("header").controls_access());
    assert!(!AuthGuard::new("header").optional().controls_access());
}
