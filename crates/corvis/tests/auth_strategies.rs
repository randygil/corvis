//! `AuthGuard` against the original: every case recorded from the original passport integration (12.0.1) over `passport` 0.7.0 with
//! custom `passport-strategy` 1.0.0 strategies (`tests/fixtures/auth/strategies.mjs`) is replayed against the same
//! routes with equivalent Corvis strategies. No Node is needed at test time.
//!
//! The apps and routes mirror the generator: `main` (one guard per route), `filter` (a global filter that catches
//! the 401) and `public` (a global guard that skips public routes). A case with a `divergence` is asserted against
//! the specification instead of the recorded response.

use corvis::auth::{AuthGuard, AuthenticateOptions, AuthenticatedUser, Strategy, StrategyOutcome};
use corvis::prelude::*;
use corvis::testing::request;
use serde::{Deserialize, Serialize};

const FIXTURE: &str = include_str!("fixtures/auth/strategies.json");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Claims {
    id: u32,
    role: String,
}

/// The strategy of the generator: the header `x-outcome-<name>` (else `x-outcome`) picks the outcome.
fn driven(name: &str, ctx: &ExecutionContext) -> StrategyOutcome {
    let req = ctx.request();
    let outcome = req.header(&format!("x-outcome-{name}")).or_else(|| req.header("x-outcome")).unwrap_or("fail");
    let user = || AuthenticatedUser::new(Claims { id: 7, role: "admin".into() }).unwrap();
    match outcome {
        "success" => StrategyOutcome::Success { user: user(), info: Some(json!({ "scope": "read" })) },
        "success-noinfo" => StrategyOutcome::Success { user: user(), info: None },
        "fail" => StrategyOutcome::Fail { challenge: None, status: None },
        "fail-challenge" => StrategyOutcome::Fail { challenge: Some(json!("no token")), status: None },
        "fail-400" => StrategyOutcome::Fail { challenge: Some(json!({ "message": "Missing credentials" })), status: Some(400) },
        "error-forbidden" => StrategyOutcome::Error(ForbiddenException::new("Account locked")),
        "error" => StrategyOutcome::Error(HttpException::internal(std::io::Error::other("db down"))),
        "redirect" => StrategyOutcome::Redirect { url: "https://idp.example/authorize?x=1".into(), status: None },
        "redirect-301" => StrategyOutcome::Redirect { url: "/moved".into(), status: Some(301) },
        "redirect-crlf" => StrategyOutcome::Redirect { url: "/a\r\nSet-Cookie: x=1".into(), status: None },
        "pass" => StrategyOutcome::Pass,
        other => panic!("unknown outcome {other}"),
    }
}

#[injectable]
pub struct StrategyA;

#[async_trait]
impl Strategy for StrategyA {
    fn name(&self) -> &str {
        "a"
    }
    async fn authenticate(&self, ctx: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
        driven("a", ctx)
    }
}

#[injectable]
pub struct StrategyB;

#[async_trait]
impl Strategy for StrategyB {
    fn name(&self) -> &str {
        "b"
    }
    async fn authenticate(&self, ctx: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
        driven("b", ctx)
    }
}

/// What the handlers of the generator answer: the user and the auth info the guard stored.
fn show(req: &Request) -> Value {
    json!({ "user": req.user().unwrap_or(Value::Null), "authInfo": req.auth_info().unwrap_or(Value::Null) })
}

#[controller("")]
pub struct MainController;

#[routes]
impl MainController {
    #[get("single")]
    #[use_guards(AuthGuard::new("a"))]
    fn single(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("chain")]
    #[use_guards(AuthGuard::chain(["a", "b"]))]
    fn chain(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("optional")]
    #[use_guards(AuthGuard::new("a").optional())]
    fn optional(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("optional-chain")]
    #[use_guards(AuthGuard::chain(["a", "b"]).optional())]
    fn optional_chain(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("custom")]
    #[use_guards(AuthGuard::new("a").on_failure(|_| UnauthorizedException::new("Token required")))]
    fn custom(&self, req: Request) -> Value {
        show(&req)
    }
}

#[controller("")]
pub struct UnknownController;

#[routes]
impl UnknownController {
    #[get("unknown")]
    #[use_guards(AuthGuard::new("typo"))]
    fn unknown(&self, req: Request) -> Value {
        show(&req)
    }
}

#[controller("")]
pub struct PublicController;

#[routes]
impl PublicController {
    #[get("open")]
    #[public]
    fn open(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("closed")]
    fn closed(&self, req: Request) -> Value {
        show(&req)
    }
}

#[controller("ctrl")]
#[public]
pub struct PublicAreaController;

#[routes]
impl PublicAreaController {
    #[get("open")]
    fn open(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("overridden")]
    #[set_metadata(PUBLIC_METADATA, false)]
    fn overridden(&self, req: Request) -> Value {
        show(&req)
    }
}

#[injectable]
pub struct LoginFilter;

#[async_trait]
impl ExceptionFilter for LoginFilter {
    fn catches(&self, e: &HttpException) -> bool {
        e.get_status() == 401
    }
    async fn catch(&self, _e: HttpException, _h: &ArgumentsHost) -> Response {
        Response::json(json!({ "error": "login" })).with_status(401u16)
    }
}

#[module(providers = [Provider::strategy::<StrategyA>(), Provider::strategy::<StrategyB>()], controllers = [MainController])]
pub struct MainModule;

#[module(providers = [Provider::strategy::<StrategyA>(), Provider::strategy::<StrategyB>()], controllers = [MainController])]
pub struct FilterModule;

#[module(providers = [Provider::strategy::<StrategyA>()], controllers = [PublicController, PublicAreaController])]
pub struct PublicModule;

#[module(providers = [Provider::strategy::<StrategyA>()], controllers = [UnknownController])]
pub struct UnknownModule;

async fn app_for(name: &str) -> CorvisApplication {
    let mut app = match name {
        "main" => CorvisFactory::create::<MainModule>().await.unwrap(),
        "filter" => {
            let mut app = CorvisFactory::create::<FilterModule>().await.unwrap();
            app.use_global_filters(LoginFilter);
            app
        }
        "public" => {
            let mut app = CorvisFactory::create::<PublicModule>().await.unwrap();
            app.use_global_guards(AuthGuard::new("a").skip_public());
            app
        }
        other => panic!("unknown app {other}"),
    };
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn every_recorded_case_is_replayed() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let versions = fixture["versions"].as_object().unwrap();
    assert_eq!(versions["passport"], "0.7.0");
    assert!(versions.values().any(|v| v == "12.0.1"), "{versions:?}");
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() >= 30, "{} cases", cases.len());

    let mut apps = std::collections::HashMap::new();
    let mut diverged = 0;
    for case in cases {
        let (name, app_name) = (case["name"].as_str().unwrap(), case["app"].as_str().unwrap());
        if !case["divergence"].is_null() {
            diverged += 1;
            continue;
        }
        if !apps.contains_key(app_name) {
            apps.insert(app_name.to_string(), app_for(app_name).await);
        }
        let app = &apps[app_name];
        let req = &case["request"];
        let mut call = request(app).get(req["path"].as_str().unwrap());
        for (k, v) in req["headers"].as_object().unwrap() {
            call = call.header(k, v.as_str().unwrap());
        }
        let r = call.send().await;
        let recorded = &case["response"];
        assert_eq!(r.status as u64, recorded["status"].as_u64().unwrap(), "{name}: status, body {}", r.text());
        for (header, expected) in recorded["headers"].as_object().unwrap() {
            let got = r.header(header);
            match expected.as_str() {
                Some(v) => assert_eq!(got, Some(v), "{name}: header {header}"),
                None => assert_eq!(got, None, "{name}: header {header} must be absent"),
            }
        }
        match &recorded["body"] {
            Value::String(text) => assert_eq!(&r.text(), text, "{name}: body"),
            expected => assert_eq!(&r.json_value(), expected, "{name}: body"),
        }
    }
    assert_eq!(diverged, 1);
}

#[corvis::test]
async fn the_recorded_unknown_strategy_case_is_a_startup_error_in_corvis() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let case = fixture["cases"].as_array().unwrap().iter().find(|c| !c["divergence"].is_null()).unwrap();
    assert_eq!(case["name"], "unknown strategy in a guard");
    assert_eq!(case["response"]["status"], 500, "the original answers 500 at request time");

    let mut app = CorvisFactory::create::<UnknownModule>().await.unwrap();
    app.silence_route_logs();
    let error = app.init().await.unwrap_err().to_string();
    assert!(error.contains("GET /unknown") && error.contains("\"typo\""), "{error}");
}
