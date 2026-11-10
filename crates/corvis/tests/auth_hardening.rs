//! Regressions from the security review of the strategy core: a response stored outside the guard phase, the typed
//! user kept apart from other request extensions, and the 3xx rule for redirects.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use corvis::auth::{AuthGuard, AuthenticateOptions, AuthenticatedUser, Strategy, StrategyOutcome, User};
use corvis::prelude::*;
use corvis::testing::request;
use serde::{Deserialize, Serialize};

static HANDLER_CALLS: AtomicUsize = AtomicUsize::new(0);
static INTERCEPTOR_RESPOND_WITH: AtomicBool = AtomicBool::new(true);
static FILTER_RESPOND_WITH: AtomicBool = AtomicBool::new(true);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Payload {
    id: u32,
}

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

fixed_strategy!(OneStrategy, "one", StrategyOutcome::Success { user: AuthenticatedUser::new(Payload { id: 1 }).unwrap(), info: None });
fixed_strategy!(
    InfoStrategy,
    "info",
    StrategyOutcome::Success { user: AuthenticatedUser::new(Payload { id: 1 }).unwrap(), info: Some(json!("earlier guard")) }
);
fixed_strategy!(FailsStrategy, "fails", StrategyOutcome::Fail { challenge: None, status: None });
fixed_strategy!(PassStrategy, "pass", StrategyOutcome::Pass);
fixed_strategy!(RedirStrategy, "redir", StrategyOutcome::Redirect { url: "/login".into(), status: None });
fixed_strategy!(BadStatusStrategy, "badstatus", StrategyOutcome::Redirect { url: "/login".into(), status: Some(200) });

/// A guard that has nothing to do with authentication and leaves a `String` on the request.
#[injectable]
pub struct ForeignGuard;

#[async_trait]
impl CanActivate for ForeignGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        ctx.request().insert("not-a-user".to_string());
        Ok(true)
    }
}

/// Runs an `AuthGuard` from an interceptor, after the guard phase, the way a careless wrapper might.
#[injectable]
pub struct GuardInInterceptor;

#[async_trait]
impl CorvisInterceptor for GuardInInterceptor {
    async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        AuthGuard::new("redir").can_activate(ctx).await?;
        next.handle().await
    }
}

#[injectable]
pub struct RespondWithInInterceptor;

#[async_trait]
impl CorvisInterceptor for RespondWithInInterceptor {
    async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        let stored = ctx.switch_to_http().respond_with(Response::new().with_status(302u16).with_header("Location", "/x"));
        INTERCEPTOR_RESPOND_WITH.store(stored, Ordering::SeqCst);
        next.handle().await
    }
}

/// Replaces the user the way a hand-written guard does.
#[injectable]
pub struct SetUserGuard;

#[async_trait]
impl CanActivate for SetUserGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        ctx.request().set_user(json!({ "id": 99 }));
        Ok(true)
    }
}

#[injectable]
pub struct PanicGuard;

#[async_trait]
impl CanActivate for PanicGuard {
    async fn can_activate(&self, _: &ExecutionContext) -> Result<bool, HttpException> {
        panic!("guard panic");
    }
}

/// Tries to end the request from an exception filter after a guard panicked.
#[injectable]
pub struct RespondingFilter;

#[async_trait]
impl ExceptionFilter for RespondingFilter {
    async fn catch(&self, _e: HttpException, host: &ArgumentsHost) -> Response {
        let stored = host.switch_to_http().respond_with(Response::new().with_status(302u16).with_header("Location", "/x"));
        FILTER_RESPOND_WITH.store(stored, Ordering::SeqCst);
        Response::json(json!({ "by": "filter" })).with_status(418)
    }
}

#[controller("h")]
pub struct HardeningController;

#[routes]
impl HardeningController {
    #[get("foreign")]
    #[use_guards(ForeignGuard)]
    fn foreign(&self, user: User<String>) -> Value {
        json!({ "user": user.0 })
    }

    #[get("cleared-json")]
    #[use_guards(AuthGuard::new("one"), AuthGuard::new("fails").optional())]
    fn cleared_json(&self, req: Request) -> Value {
        json!({ "user": req.user() })
    }

    #[get("cleared-typed")]
    #[use_guards(AuthGuard::new("one"), AuthGuard::new("fails").optional())]
    fn cleared_typed(&self, user: User<Payload>) -> Value {
        json!({ "user": user.0 })
    }

    #[get("passed-json")]
    #[use_guards(AuthGuard::new("one"), AuthGuard::new("pass"))]
    fn passed_json(&self, req: Request) -> Value {
        json!({ "user": req.user() })
    }

    /// The auth info of an earlier guard does not outlive a guard that passed: both the user and the info are cleared.
    #[get("passed-info")]
    #[use_guards(AuthGuard::new("info"), AuthGuard::new("pass"))]
    fn passed_info(&self, req: Request) -> Value {
        json!({ "user": req.user(), "info": req.auth_info() })
    }

    #[get("kept-info")]
    #[use_guards(AuthGuard::new("info"))]
    fn kept_info(&self, req: Request) -> Value {
        json!({ "user": req.user(), "info": req.auth_info() })
    }

    #[get("passed-typed")]
    #[use_guards(AuthGuard::new("one"), AuthGuard::new("pass"))]
    fn passed_typed(&self, user: User<Payload>) -> Value {
        json!({ "user": user.0 })
    }

    #[get("replaced-json")]
    #[use_guards(AuthGuard::new("one"), SetUserGuard)]
    fn replaced_json(&self, req: Request) -> Value {
        json!({ "user": req.user() })
    }

    #[get("replaced-typed")]
    #[use_guards(AuthGuard::new("one"), SetUserGuard)]
    fn replaced_typed(&self, user: User<Payload>) -> Value {
        json!({ "user": user.0 })
    }

    #[get("panic")]
    #[use_guards(PanicGuard)]
    #[use_filters(RespondingFilter)]
    fn panics(&self) -> Value {
        json!({ "ok": true })
    }

    #[get("authenticated")]
    #[use_guards(AuthGuard::new("one"))]
    fn authenticated(&self, user: User<Payload>) -> Value {
        json!({ "user": user.0 })
    }

    #[get("redirect")]
    #[use_guards(AuthGuard::new("redir"))]
    fn redirect(&self) -> Value {
        HANDLER_CALLS.fetch_add(1, Ordering::SeqCst);
        json!({ "ok": true })
    }

    #[get("bad-status")]
    #[use_guards(AuthGuard::new("badstatus"))]
    fn bad_status(&self) -> Value {
        HANDLER_CALLS.fetch_add(1, Ordering::SeqCst);
        json!({ "ok": true })
    }

    #[get("guard-in-interceptor")]
    #[use_interceptors(GuardInInterceptor)]
    fn guard_in_interceptor(&self) -> Value {
        HANDLER_CALLS.fetch_add(1, Ordering::SeqCst);
        json!({ "ok": true })
    }

    #[get("respond-in-interceptor")]
    #[use_interceptors(RespondWithInInterceptor)]
    fn respond_in_interceptor(&self) -> Value {
        json!({ "ok": true })
    }
}

#[module(
    providers = [
        Provider::strategy::<OneStrategy>(), Provider::strategy::<InfoStrategy>(), Provider::strategy::<FailsStrategy>(), Provider::strategy::<PassStrategy>(),
        Provider::strategy::<RedirStrategy>(), Provider::strategy::<BadStatusStrategy>(),
    ],
    controllers = [HardeningController]
)]
pub struct AppModule;

async fn started() -> CorvisApplication {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

const UNAUTHORIZED: fn() -> Value = || json!({ "message": "Unauthorized", "statusCode": 401 });

#[corvis::test]
async fn an_auth_guard_run_from_an_interceptor_with_a_redirecting_strategy_does_not_run_the_handler() {
    let app = started().await;
    request(&app)
        .get("/h/guard-in-interceptor")
        .send()
        .await
        .expect_status(500)
        .expect_json(json!({ "statusCode": 500, "message": "Internal server error" }));
    assert_eq!(HANDLER_CALLS.load(Ordering::SeqCst), 0);
}

#[corvis::test]
async fn respond_with_from_an_interceptor_returns_false_and_stores_nothing() {
    let app = started().await;
    request(&app).get("/h/respond-in-interceptor").send().await.expect_status(200).expect_json(json!({ "ok": true }));
    assert!(!INTERCEPTOR_RESPOND_WITH.load(Ordering::SeqCst));
}

#[corvis::test]
async fn a_redirecting_strategy_in_the_guard_phase_still_redirects() {
    let app = started().await;
    request(&app).get("/h/redirect").send().await.expect_status(302).expect_header("location", "/login");
    assert_eq!(HANDLER_CALLS.load(Ordering::SeqCst), 0);
}

#[corvis::test]
async fn a_redirect_with_a_status_outside_3xx_is_a_500() {
    let app = started().await;
    request(&app)
        .get("/h/bad-status")
        .send()
        .await
        .expect_status(500)
        .expect_json(json!({ "statusCode": 500, "message": "Internal server error" }));
    assert_eq!(HANDLER_CALLS.load(Ordering::SeqCst), 0);
}

#[corvis::test]
async fn a_foreign_extension_of_the_same_type_does_not_satisfy_the_user_extractor() {
    let app = started().await;
    request(&app).get("/h/foreign").send().await.expect_status(401).expect_json(UNAUTHORIZED());
}

#[corvis::test]
async fn the_typed_user_is_still_read_after_a_successful_authentication() {
    let app = started().await;
    request(&app).get("/h/authenticated").send().await.expect_status(200).expect_json(json!({ "user": { "id": 1 } }));
}

#[corvis::test]
async fn an_optional_failure_clears_the_json_and_the_typed_user_alike() {
    let app = started().await;
    request(&app).get("/h/cleared-json").send().await.expect_status(200).expect_json(json!({ "user": null }));
    request(&app).get("/h/cleared-typed").send().await.expect_status(401).expect_json(UNAUTHORIZED());
}

#[corvis::test]
async fn a_later_pass_clears_the_json_and_the_typed_user_alike() {
    let app = started().await;
    request(&app).get("/h/passed-json").send().await.expect_status(200).expect_json(json!({ "user": null }));
    request(&app).get("/h/passed-typed").send().await.expect_status(401).expect_json(UNAUTHORIZED());
}

#[corvis::test]
async fn a_later_pass_clears_the_auth_info_of_an_earlier_guard_too() {
    let app = started().await;
    request(&app).get("/h/kept-info").send().await.expect_status(200).expect_json(json!({ "user": { "id": 1 }, "info": "earlier guard" }));
    request(&app).get("/h/passed-info").send().await.expect_status(200).expect_json(json!({ "user": null, "info": null }));
}

#[corvis::test]
async fn set_user_drops_the_typed_user_so_both_users_agree() {
    let app = started().await;
    request(&app).get("/h/replaced-json").send().await.expect_status(200).expect_json(json!({ "user": { "id": 99 } }));
    request(&app).get("/h/replaced-typed").send().await.expect_status(401).expect_json(UNAUTHORIZED());
}

#[corvis::test]
async fn a_panicking_guard_does_not_leave_the_guard_phase_on_for_exception_filters() {
    let app = started().await;
    request(&app).get("/h/panic").send().await.expect_status(418).expect_json(json!({ "by": "filter" }));
    assert!(!FILTER_RESPOND_WITH.load(Ordering::SeqCst));
}
