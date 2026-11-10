//! The API key strategy against the original: every case recorded from `passport-headerapikey` 1.2.2 and from
//! `passport-custom` 1.2.1 strategies behind the original passport integration (`tests/fixtures/auth/api_key.mjs`) is
//! replayed against the same routes with equivalent Corvis validators. No Node is needed at test time. A case with a
//! `divergence` is asserted against the specification, not the recorded response.
#![cfg(feature = "auth-api-key")]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use corvis::auth::api_key::{ApiKey, ApiKeySource, ApiKeyStrategy, ApiKeyStrategyOptions, ApiKeyValidator};
use corvis::auth::{AuthGuard, AuthenticateOptions, AuthenticatedUser, Strategy, StrategyOutcome, User};
use corvis::prelude::*;
use corvis::testing::request;
use serde::Serialize;

const FIXTURE: &str = include_str!("fixtures/auth/api_key.json");

/// What the validator received, per test (`x-test-id`), so tests can run side by side.
fn received() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static RECEIVED: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    RECEIVED.get_or_init(Default::default)
}

fn keys_of(id: &str) -> Vec<String> {
    received().lock().unwrap().get(id).cloned().unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
pub struct Service {
    id: &'static str,
    #[serde(rename = "keyLength")]
    key_length: usize,
}

/// What `verifyKey` of the generator does, decided from the text of the key.
fn verify(req: &Request, key: &ApiKey) -> Result<Option<Service>, HttpException> {
    let text = key.expose();
    if let Some(id) = req.header("x-test-id") {
        received().lock().unwrap().entry(id.to_string()).or_default().push(text.to_string());
    }
    if text.starts_with("test-key-null") {
        Ok(None)
    } else if text.starts_with("test-key-throw") {
        Err(ForbiddenException::new("Key suspended"))
    } else if text.starts_with("test-key-boom") {
        Err(HttpException::internal(std::io::Error::other("db down")))
    } else if text.starts_with("test-key-") {
        Ok(Some(Service { id: "service-1", key_length: text.chars().count() }))
    } else {
        Ok(None)
    }
}

fn info_of(user: &Service) -> Option<Value> {
    (user.key_length == 37).then(|| json!({ "scope": "read" }))
}

macro_rules! validator {
    ($name:ident, $strategy:literal, $options:expr) => {
        #[injectable]
        pub struct $name;

        #[async_trait]
        impl ApiKeyValidator for $name {
            const NAME: &'static str = $strategy;
            type User = Service;
            fn options(&self) -> ApiKeyStrategyOptions {
                $options
            }
            async fn validate(&self, req: &Request, key: &ApiKey) -> Result<Option<Service>, HttpException> {
                verify(req, key)
            }
            fn info(&self, user: &Service) -> Option<Value> {
                info_of(user)
            }
        }
    };
}

validator!(HeaderKey, "header", ApiKeyStrategyOptions::default());
validator!(CustomHeader, "custom", ApiKeyStrategyOptions::new().source(ApiKeySource::header("X-Service-Key")));
validator!(Prefixed, "prefixed", ApiKeyStrategyOptions::new().source(ApiKeySource::header("Authorization").prefix("Api-Key ")));
validator!(Plus, "plus", ApiKeyStrategyOptions::new().source(ApiKeySource::header("X-Key").prefix("Key+")));
validator!(Query, "query", ApiKeyStrategyOptions::new().source(ApiKeySource::query("api_key")));
validator!(Bearer, "bearer", ApiKeyStrategyOptions::new().source(ApiKeySource::bearer()));
validator!(
    TwoPlaces,
    "two-places",
    ApiKeyStrategyOptions::new().source(ApiKeySource::header("X-Api-Key")).source(ApiKeySource::query("api_key"))
);
// Every source at once, read through an optional guard so the failure reason can be seen.
validator!(
    Reasons,
    "reasons",
    ApiKeyStrategyOptions::new()
        .source(ApiKeySource::header("X-Api-Key"))
        .source(ApiKeySource::header("Authorization").prefix("Api-Key "))
        .source(ApiKeySource::query("api_key"))
);

/// The first strategy of the chain cases: accepts one bearer token, as the generator's `bearer-user`.
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

#[controller("")]
pub struct Routes;

#[routes]
impl Routes {
    #[get("header")]
    #[use_guards(AuthGuard::new("header"))]
    fn header(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[get("custom")]
    #[use_guards(AuthGuard::new("custom"))]
    fn custom(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[get("prefixed")]
    #[use_guards(AuthGuard::new("prefixed"))]
    fn prefixed(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[get("plus")]
    #[use_guards(AuthGuard::new("plus"))]
    fn plus(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[get("query")]
    #[use_guards(AuthGuard::new("query"))]
    fn query(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[get("bearer")]
    #[use_guards(AuthGuard::new("bearer"))]
    fn bearer(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[get("two-places")]
    #[use_guards(AuthGuard::new("two-places"))]
    fn two_places(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[get("chain")]
    #[use_guards(AuthGuard::chain(["bearer-user", "header"]))]
    fn chain(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[post("submit")]
    #[use_guards(AuthGuard::new("header"))]
    fn submit(&self, user: User<Service>) -> Value {
        json!({ "id": user.id })
    }

    /// Shows the reason the strategy reported, which the default 401 hides.
    #[get("reasons")]
    #[use_guards(AuthGuard::new("reasons").optional())]
    fn reasons(&self, req: Request) -> Value {
        json!({ "authInfo": req.auth_info(), "user": req.user() })
    }
}

#[module(
    providers = [
        Provider::strategy::<ApiKeyStrategy<HeaderKey>>(),
        Provider::strategy::<ApiKeyStrategy<CustomHeader>>(),
        Provider::strategy::<ApiKeyStrategy<Prefixed>>(),
        Provider::strategy::<ApiKeyStrategy<Plus>>(),
        Provider::strategy::<ApiKeyStrategy<Query>>(),
        Provider::strategy::<ApiKeyStrategy<Bearer>>(),
        Provider::strategy::<ApiKeyStrategy<TwoPlaces>>(),
        Provider::strategy::<ApiKeyStrategy<Reasons>>(),
        Provider::strategy::<BearerUser>(),
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

fn headers_of(case: &Value) -> Vec<(String, String)> {
    case["request"]["headers"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect()
}

fn as_refs(headers: &[(String, String)]) -> Vec<(&str, &str)> {
    headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

const UNAUTHORIZED: fn() -> Value = || json!({ "message": "Unauthorized", "statusCode": 401 });

#[corvis::test]
async fn every_recorded_case_is_replayed() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(fixture["versions"]["passport-headerapikey"], "1.2.2");
    assert_eq!(fixture["versions"]["passport-custom"], "1.2.1");
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() >= 50, "{} cases", cases.len());
    let app = app().await;
    let mut diverged = 0;
    for case in cases {
        let name = case["name"].as_str().unwrap();
        if !case["divergence"].is_null() {
            diverged += 1;
            continue;
        }
        let headers = headers_of(case);
        let r = get(&app, case["request"]["path"].as_str().unwrap(), &as_refs(&headers)).await;
        let recorded = &case["response"];
        assert_eq!(r.status as u64, recorded["status"].as_u64().unwrap(), "{name}: status, body {}", r.text());
        assert_eq!(r.json_value(), recorded["body"], "{name}: body");
    }
    assert_eq!(diverged, 5, "an oversized key twice, a control character, a conflict, the literal prefix");
}

#[corvis::test]
async fn divergences_follow_the_specification() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let app = app().await;
    for case in fixture["cases"].as_array().unwrap().iter().filter(|c| !c["divergence"].is_null()) {
        let name = case["name"].as_str().unwrap();
        let divergence = &case["divergence"];
        let id = format!("divergence-{name}");
        let mut headers = headers_of(case);
        headers.push(("x-test-id".to_string(), id.clone()));
        let r = get(&app, case["request"]["path"].as_str().unwrap(), &as_refs(&headers)).await;
        let status = divergence["status"].as_u64().unwrap_or(401);
        assert_eq!(r.status as u64, status, "{name}: {}", r.text());
        match divergence["validatorKey"].as_str() {
            Some(key) => {
                // The literal prefix is removed and the validator runs with the rest.
                assert_eq!(keys_of(&id), [key], "{name}");
                assert_eq!(r.json_value()["user"]["keyLength"], key.chars().count(), "{name}");
            }
            None => {
                assert_eq!(r.json_value(), UNAUTHORIZED(), "{name}");
                assert!(keys_of(&id).is_empty(), "{name}: the validator must not run");
            }
        }
        // The original answered with a user for the cases Corvis refuses.
        assert_eq!(case["response"]["status"], if status == 200 { 401 } else { 200 }, "{name}");
    }
}

#[corvis::test]
async fn a_valid_key_reaches_the_handler_with_the_validators_user() {
    let app = app().await;
    let key = "test-key-000000000000000000000000000001";
    let r = get(&app, "/header", &[("x-api-key", key), ("x-test-id", "valid-key")]).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json_value(), json!({ "user": { "id": "service-1", "keyLength": 39 } }));
    assert_eq!(keys_of("valid-key"), [key]);
    // The typed user is readable with `User<T>`.
    let r = request(&app).post("/submit").header("x-api-key", key).send().await;
    assert_eq!((r.status, r.json_value()), (201, json!({ "id": "service-1" })));
}

#[corvis::test]
async fn the_validator_does_not_run_without_a_usable_key() {
    let app = app().await;
    let long = "k".repeat(513);
    let two_keys = ("x-api-key", "test-key-000000000000000000000000000001");
    type Case<'a> = (&'a str, String, Vec<(&'a str, &'a str)>);
    let cases: Vec<Case> = vec![
        ("no-key", "/header".into(), vec![]),
        ("empty", "/header".into(), vec![("x-api-key", "")]),
        ("long", "/header".into(), vec![("x-api-key", &long)]),
        ("query-not-a-source", "/header?api_key=test-key-000000000000000000000000000001".into(), vec![]),
        ("tab", "/header".into(), vec![("x-api-key", "test-key-0000\t0000000000000000000000001")]),
        ("conflict", "/two-places?api_key=test-key-000000000000000000000000000002".into(), vec![two_keys]),
        ("control", "/query?api_key=test-key-%00000000000000000000000000001".into(), vec![]),
        ("repeated", "/query?api_key=test-key-000000000000000000000000000001&api_key=x".into(), vec![]),
        ("prefix-absent", "/prefixed".into(), vec![("authorization", "test-key-000000000000000000000000000001")]),
    ];
    for (id, path, mut headers) in cases {
        headers.push(("x-test-id", id));
        let r = get(&app, &path, &headers).await;
        assert_eq!(r.status, 401, "{id}");
        assert_eq!(r.json_value(), UNAUTHORIZED(), "{id}");
        assert!(keys_of(id).is_empty(), "{id}: the validator ran");
    }
}

#[corvis::test]
async fn the_same_key_in_two_places_calls_the_validator_once() {
    let app = app().await;
    let key = "test-key-000000000000000000000000000001";
    let r = get(&app, &format!("/two-places?api_key={key}"), &[("x-api-key", key), ("x-test-id", "same-twice")]).await;
    assert_eq!(r.status, 200);
    assert_eq!(keys_of("same-twice"), [key]);
}

#[corvis::test]
async fn a_key_in_the_body_is_not_read() {
    let app = app().await;
    let r = request(&app)
        .post("/submit")
        .header("x-test-id", "body-key")
        .json(&json!({ "api_key": "test-key-000000000000000000000000000001" }))
        .send()
        .await;
    assert_eq!((r.status, r.json_value()), (401, UNAUTHORIZED()));
    assert!(keys_of("body-key").is_empty());
}

#[corvis::test]
async fn failure_reasons_are_fixed_strings() {
    let app = app().await;
    let key = "test-key-000000000000000000000000000001";
    let other = "test-key-000000000000000000000000000002";
    let long = "k".repeat(513);
    let reason = |r: corvis::testing::TestResponse| r.json_value()["authInfo"]["message"].as_str().map(str::to_string);
    assert_eq!(reason(get(&app, "/reasons", &[]).await).as_deref(), Some("Missing API Key"));
    assert_eq!(reason(get(&app, "/reasons", &[("authorization", key)]).await).as_deref(), Some("Missing API Key"), "a wrong prefix");
    let r = get(&app, &format!("/reasons?api_key={other}"), &[("x-api-key", key)]).await;
    assert_eq!(reason(r).as_deref(), Some("Conflicting API keys"));
    assert_eq!(reason(get(&app, "/reasons", &[("x-api-key", &long)]).await).as_deref(), Some("API key too long"));
    let r = get(&app, "/reasons?api_key=a%00b", &[]).await;
    assert_eq!(reason(r).as_deref(), Some("Invalid API key characters"));
    let r = get(&app, "/reasons?api_key=a&api_key=b", &[]).await;
    assert_eq!(reason(r).as_deref(), Some("Ambiguous API key"));
    // The reason of a wrong key that reached the validator is no reason at all, and nothing shows the key.
    let wrong = "wrong-key-000000000000000000000000000001";
    let r = get(&app, "/reasons", &[("x-api-key", wrong)]).await;
    assert_eq!(r.json_value(), json!({ "authInfo": null, "user": null }));
}

/// Collects every line the framework logger receives.
struct Capture(std::sync::Arc<Mutex<Vec<String>>>);

impl corvis::LoggerService for Capture {
    fn log(&self, _level: corvis::LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push(format!("[{}] {message}", context.unwrap_or("")));
    }
}

/// The logger is process-wide and this is the only test of the binary that installs one; the others only read
/// responses, so lines they write end up here too, which can only make the check stricter.
#[corvis::test]
async fn no_log_line_holds_a_key_while_every_fixture_case_runs() {
    let lines = std::sync::Arc::new(Mutex::new(Vec::new()));
    corvis::logger::set_logger(Capture(lines.clone()));
    corvis::logger::set_log_levels(&corvis::LogLevel::all());
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let app = app().await;
    for case in fixture["cases"].as_array().unwrap() {
        let headers = headers_of(case);
        let _ = get(&app, case["request"]["path"].as_str().unwrap(), &as_refs(&headers)).await;
    }
    let all = lines.lock().unwrap().join("\n");
    assert!(all.contains("accepts keys in the query parameter"), "the logger was not installed in time:\n{all}");
    for key_text in
        ["test-key-0", "test-key-null", "test-key-throw", "test-key-boom", "test-key-info", "wrong-key-", "kkkkkkkk", "user-token-"]
    {
        assert!(!all.contains(key_text), "a log line holds {key_text}:\n{all}");
    }
}
