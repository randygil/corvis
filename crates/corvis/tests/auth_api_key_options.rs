//! The API key strategy's options and its behavior around the validator: the startup checks, the limits, the keys
//! the validator never sees, uniform answers, slowness, failures and panics. What the strategy shares with
//! `passport-headerapikey` is replayed from recorded fixtures by `tests/auth_api_key.rs`.
#![cfg(feature = "auth-api-key")]

use std::time::Duration;

use corvis::auth::api_key::{ApiKey, ApiKeySet, ApiKeySource, ApiKeyStrategy, ApiKeyStrategyOptions, ApiKeyValidator, ExpectedApiKey};
use corvis::auth::{AuthGuard, User};
use corvis::http::ServiceUnavailableException;
use corvis::prelude::*;
use corvis::testing::request;
use serde::Serialize;

const GOOD: &str = "test-key-000000000000000000000000000001";

#[derive(Debug, Clone, Serialize)]
pub struct Service {
    name: String,
}

/// The behavior of the validator is chosen by the text of the key.
async fn behave(key: &ApiKey) -> Result<Option<Service>, HttpException> {
    let text = key.expose();
    if text.starts_with("test-key-slow") {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
    if text.starts_with("test-key-panic") {
        panic!("validator panicked on {text}");
    }
    if text.starts_with("test-key-db-down") {
        return Err(ServiceUnavailableException::new(format!("database down while checking {text}")));
    }
    if text.starts_with("test-key-db-500") {
        return Err(InternalServerErrorException::new(format!("query failed for {text}")));
    }
    if text.starts_with("test-key-teapot-200") {
        return Err(HttpException::new(format!("redirect {text}"), 200u16));
    }
    if text.starts_with("test-key-forbidden") {
        return Err(ForbiddenException::new("Key suspended"));
    }
    if text.starts_with("test-key-revoked") {
        return Ok(None);
    }
    Ok(text.starts_with("test-key-").then(|| Service { name: "billing".into() }))
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
            async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<Service>, HttpException> {
                behave(key).await
            }
        }
    };
}

validator!(Hardened, "hardened", ApiKeyStrategyOptions::default().validator_timeout(Duration::from_millis(50)));
validator!(Short, "short", ApiKeyStrategyOptions::default().max_key_bytes(10));

#[controller("")]
pub struct Routes;

#[routes]
impl Routes {
    #[get("hardened")]
    #[use_guards(AuthGuard::new("hardened"))]
    fn hardened(&self, service: User<Service>) -> Value {
        json!({ "name": service.name })
    }

    #[get("optional")]
    #[use_guards(AuthGuard::new("hardened").optional())]
    fn optional(&self, req: Request) -> Value {
        json!({ "user": req.user() })
    }

    #[get("short")]
    #[use_guards(AuthGuard::new("short"))]
    fn short(&self) -> Value {
        json!({ "ok": true })
    }
}

#[module(
    providers = [Provider::strategy::<ApiKeyStrategy<Hardened>>(), Provider::strategy::<ApiKeyStrategy<Short>>()],
    controllers = [Routes],
)]
pub struct AppModule;

async fn start<M: corvis::Module>() -> CorvisApplication {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

async fn startup_error<M: corvis::Module>() -> String {
    match CorvisFactory::create::<M>().await {
        Ok(mut app) => match app.init().await {
            Ok(()) => panic!("the application started"),
            Err(e) => e.to_string(),
        },
        Err(e) => e.to_string(),
    }
}

async fn with_key(app: &CorvisApplication, path: &str, key: &str) -> corvis::testing::TestResponse {
    request(app).get(path).header("x-api-key", key).send().await
}

// ---- startup checks ----

macro_rules! bad_options {
    ($validator:ident, $module:ident, $strategy:literal, $options:expr) => {
        validator!($validator, $strategy, $options);

        #[module(providers = [Provider::strategy::<ApiKeyStrategy<$validator>>()])]
        pub struct $module;
    };
}

bad_options!(NoSource, NoSourceModule, "no-source", ApiKeyStrategyOptions::new());
bad_options!(EmptyHeader, EmptyHeaderModule, "empty-header", ApiKeyStrategyOptions::new().source(ApiKeySource::header("")));
bad_options!(CookieHeader, CookieHeaderModule, "cookie-header", ApiKeyStrategyOptions::new().source(ApiKeySource::header("Cookie")));
bad_options!(EmptyPrefix, EmptyPrefixModule, "empty-prefix", ApiKeyStrategyOptions::new().source(ApiKeySource::header("X-Key").prefix("")));
bad_options!(EmptyQuery, EmptyQueryModule, "empty-query", ApiKeyStrategyOptions::new().source(ApiKeySource::query("")));
bad_options!(Zero, ZeroModule, "zero-limit", ApiKeyStrategyOptions::default().max_key_bytes(0));
bad_options!(Huge, HugeModule, "huge-limit", ApiKeyStrategyOptions::default().max_key_bytes(5000));
bad_options!(NoTime, NoTimeModule, "no-time", ApiKeyStrategyOptions::default().validator_timeout(Duration::ZERO));
bad_options!(
    Nine,
    NineModule,
    "nine-sources",
    (0..9).fold(ApiKeyStrategyOptions::new(), |o, i| o.source(ApiKeySource::header(format!("x-key-{i}"))))
);

#[corvis::test]
async fn invalid_options_stop_the_application_with_an_error_that_names_the_strategy() {
    let cases = [
        (startup_error::<NoSourceModule>().await, "no-source", "no source"),
        (startup_error::<EmptyHeaderModule>().await, "empty-header", "empty header name"),
        (startup_error::<CookieHeaderModule>().await, "cookie-header", "cookies are never a source"),
        (startup_error::<EmptyPrefixModule>().await, "empty-prefix", "empty prefix"),
        (startup_error::<EmptyQueryModule>().await, "empty-query", "empty parameter name"),
        (startup_error::<ZeroModule>().await, "zero-limit", "max_key_bytes"),
        (startup_error::<HugeModule>().await, "huge-limit", "max_key_bytes"),
        (startup_error::<NoTimeModule>().await, "no-time", "validator_timeout"),
        (startup_error::<NineModule>().await, "nine-sources", "at most 8"),
    ];
    for (error, strategy, what) in cases {
        assert!(error.contains(&format!("ApiKeyStrategy \"{strategy}\"")), "{error}");
        assert!(error.contains(what), "{error}");
    }
}

// ---- configured keys ----

/// A configured key read from the environment of the application, as the example app does.
pub struct Keys {
    set: ApiKeySet<&'static str>,
}

#[injectable]
pub struct FromConfig {
    keys: Arc<Keys>,
}

#[async_trait]
impl ApiKeyValidator for FromConfig {
    const NAME: &'static str = "configured";
    type User = Service;
    async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<Service>, HttpException> {
        Ok(self.keys.set.lookup(key).map(|name| Service { name: name.to_string() }))
    }
}

#[controller("")]
pub struct ConfiguredRoutes;

#[routes]
impl ConfiguredRoutes {
    #[get("configured")]
    #[use_guards(AuthGuard::new("configured"))]
    fn configured(&self, service: User<Service>) -> Value {
        json!({ "name": service.name })
    }
}

fn keys_provider(secrets: &'static [(&'static str, &'static str)]) -> Provider {
    Provider::use_factory(Token::of::<Keys>(), move |_ctx| async move {
        let mut set = ApiKeySet::new();
        for (secret, name) in secrets {
            set.insert(ExpectedApiKey::new(secret)?, *name);
        }
        Ok(Keys { set })
    })
}

#[module(
    providers = [keys_provider(&[(GOOD, "billing"), ("test-key-000000000000000000000000000002", "reports")]), Provider::strategy::<ApiKeyStrategy<FromConfig>>()],
    controllers = [ConfiguredRoutes],
)]
pub struct ConfiguredModule;

#[module(providers = [keys_provider(&[("short", "billing")]), Provider::strategy::<ApiKeyStrategy<FromConfig>>()])]
pub struct ShortKeyModule;

#[corvis::test]
async fn configured_keys_authenticate_their_owner_only() {
    let app = start::<ConfiguredModule>().await;
    let r = with_key(&app, "/configured", GOOD).await;
    assert_eq!((r.status, r.json_value()), (200, json!({ "name": "billing" })));
    let r = with_key(&app, "/configured", "test-key-000000000000000000000000000002").await;
    assert_eq!(r.json_value(), json!({ "name": "reports" }));
    for wrong in ["test-key-000000000000000000000000000003", "test-key-00000000000000000000000000000", "", "x"] {
        assert_eq!(with_key(&app, "/configured", wrong).await.status, 401, "{wrong}");
    }
}

#[corvis::test]
async fn a_short_configured_key_stops_the_application() {
    let error = startup_error::<ShortKeyModule>().await;
    assert!(error.contains("at least 32 bytes (got 5)"), "{error}");
    assert!(!error.contains("short\""), "the secret must not be in the message: {error}");
}

// ---- limits ----

#[corvis::test]
async fn max_key_bytes_is_configurable() {
    let app = start::<AppModule>().await;
    assert_eq!(with_key(&app, "/short", "test-key-1").await.status, 200, "10 bytes");
    assert_eq!(with_key(&app, "/short", "test-key-12").await.status, 401, "11 bytes");
}

// ---- uniform answers ----

#[corvis::test]
async fn missing_malformed_unknown_and_revoked_keys_get_the_same_401() {
    let app = start::<AppModule>().await;
    let long = "k".repeat(600);
    type Answer = (String, u16, Vec<u8>, Vec<(String, String)>);
    let mut answers: Vec<Answer> = Vec::new();
    let cases: Vec<(&str, Option<String>)> = vec![
        ("missing", None),
        ("empty", Some(String::new())),
        ("unknown", Some("unknown-key-0000000000000000000000000001".into())),
        ("revoked", Some("test-key-revoked-000000000000000000001".into())),
        ("oversized", Some(long)),
        ("control", Some("test-key-\t0000000000000000000000000001".into())),
    ];
    for (name, key) in cases {
        let mut call = request(&app).get("/hardened");
        if let Some(key) = &key {
            call = call.header("x-api-key", key);
        }
        let r = call.send().await;
        let mut headers: Vec<(String, String)> = r
            .headers
            .iter()
            .filter(|(k, _)| k.as_str() != "date")
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        headers.sort();
        answers.push((name.to_string(), r.status, r.body.to_vec(), headers));
    }
    let (_, status, body, headers) = &answers[0];
    assert_eq!(*status, 401);
    assert_eq!(body, br#"{"message":"Unauthorized","statusCode":401}"#);
    for (name, s, b, h) in &answers[1..] {
        assert_eq!((s, b, h), (status, body, headers), "{name} differs from the missing-key answer");
    }
}

// ---- the validator's failures ----

#[corvis::test]
async fn an_infrastructure_failure_is_a_5xx_that_does_not_depend_on_the_key() {
    let app = start::<AppModule>().await;
    let a = with_key(&app, "/hardened", "test-key-db-down-0000000000000000000001").await;
    let b = with_key(&app, "/hardened", "test-key-db-down-9999999999999999999999").await;
    assert_eq!(a.status, 503);
    assert_eq!((a.status, a.body.clone()), (b.status, b.body.clone()));
    assert_eq!(a.json_value(), json!({ "statusCode": 503, "message": "Service Unavailable" }));
    for key in ["test-key-db-500-0000000000000000000001", "test-key-db-500-9999999999999999999999"] {
        let r = with_key(&app, "/hardened", key).await;
        assert_eq!(r.status, 500);
        assert_eq!(r.json_value(), json!({ "statusCode": 500, "message": "Internal server error" }));
        assert!(!r.text().contains("db-500"));
    }
    // A status below 400 is not an answer either.
    let r = with_key(&app, "/hardened", "test-key-teapot-200-00000000000000000001").await;
    assert_eq!(r.status, 500);
    assert!(!r.text().contains("teapot"));
}

#[corvis::test]
async fn a_4xx_of_the_validator_is_passed_on() {
    let app = start::<AppModule>().await;
    let r = with_key(&app, "/hardened", "test-key-forbidden-0000000000000000001").await;
    assert_eq!(r.status, 403);
    assert_eq!(r.json_value(), json!({ "message": "Key suspended", "error": "Forbidden", "statusCode": 403 }));
}

#[corvis::test]
async fn a_slow_validator_is_a_503_even_behind_an_optional_guard() {
    let app = start::<AppModule>().await;
    for path in ["/hardened", "/optional"] {
        let r = with_key(&app, path, "test-key-slow-00000000000000000000001").await;
        assert_eq!(r.status, 503, "{path}");
        assert_eq!(r.json_value(), json!({ "statusCode": 503, "message": "Service Unavailable" }), "{path}");
    }
}

#[corvis::test]
async fn a_panicking_validator_is_a_uniform_500_even_behind_an_optional_guard() {
    let app = start::<AppModule>().await;
    for path in ["/hardened", "/optional"] {
        let r = with_key(&app, path, "test-key-panic-0000000000000000000001").await;
        assert_eq!(r.status, 500, "{path}");
        assert_eq!(r.json_value(), json!({ "statusCode": 500, "message": "Internal server error" }), "{path}");
        assert!(!r.text().contains("panic-000"), "{path}");
    }
    // The application keeps serving.
    assert_eq!(with_key(&app, "/hardened", GOOD).await.status, 200);
}

#[corvis::test]
async fn an_optional_guard_lets_a_request_without_a_valid_key_through_without_a_user() {
    let app = start::<AppModule>().await;
    let r = request(&app).get("/optional").send().await;
    assert_eq!((r.status, r.json_value()), (200, json!({ "user": null })));
    let r = with_key(&app, "/optional", "test-key-revoked-000000000000000000001").await;
    assert_eq!(r.json_value(), json!({ "user": null }));
    let r = with_key(&app, "/optional", GOOD).await;
    assert_eq!(r.json_value(), json!({ "user": { "name": "billing" } }));
}
