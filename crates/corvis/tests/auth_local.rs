//! The local strategy against the original: every case recorded from `passport-local` 1.0.0 behind the original
//! passport integration (`tests/fixtures/auth/local.mjs`) is replayed against the same routes with equivalent Corvis
//! validators. No Node is needed at test time. A case with a `divergence` is asserted against the specification.
#![cfg(feature = "auth-local")]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use corvis::auth::local::{LocalStrategy, LocalStrategyOptions, LocalValidator, PasswordHasher, PasswordHasherOptions};
use corvis::auth::{AuthGuard, User};
use corvis::prelude::*;
use corvis::testing::request;
use serde::Serialize;

const FIXTURE: &str = include_str!("fixtures/auth/local.json");

/// How many times a validator ran, per test (`x-test-id`), so tests can run side by side.
fn calls() -> &'static Mutex<HashMap<String, usize>> {
    static CALLS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    CALLS.get_or_init(Default::default)
}

fn calls_of(id: &str) -> usize {
    calls().lock().unwrap().get(id).copied().unwrap_or(0)
}

#[derive(Debug, Clone, Serialize)]
pub struct Echo {
    username: String,
    #[serde(rename = "usernameType")]
    username_type: &'static str,
    password: String,
}

/// What the validator of the generator does: the header `x-validate` picks `null` or `throw`.
fn echo(req: &Request, username: &str, password: &str) -> Result<Option<Echo>, HttpException> {
    if let Some(id) = req.header("x-test-id") {
        *calls().lock().unwrap().entry(id.to_string()).or_default() += 1;
    }
    match req.header("x-validate") {
        Some("null") => Ok(None),
        Some("throw") => Err(ForbiddenException::new("Account locked")),
        _ => Ok(Some(Echo { username: username.to_string(), username_type: "string", password: password.to_string() })),
    }
}

macro_rules! validator {
    ($name:ident, $strategy:literal, $options:expr) => {
        #[injectable]
        pub struct $name;

        #[async_trait]
        impl LocalValidator for $name {
            const NAME: &'static str = $strategy;
            type User = Echo;
            fn options(&self) -> LocalStrategyOptions {
                $options
            }
            async fn validate(&self, req: &Request, username: &str, password: &str) -> Result<Option<Echo>, HttpException> {
                echo(req, username, password)
            }
        }
    };
}

validator!(Plain, "local", LocalStrategyOptions::default());
validator!(Nested, "nested", LocalStrategyOptions::default().username_field("user[email]"));
validator!(UserObject, "userobj", LocalStrategyOptions::default().username_field("user"));
validator!(Indexed, "indexed", LocalStrategyOptions::default().username_field("names[1]"));
validator!(Raised, "raised", LocalStrategyOptions::default().max_password_bytes(4096));
validator!(Renamed, "renamed", LocalStrategyOptions::default().username_field("email").password_field("pass"));
validator!(NoUsernameField, "broken", LocalStrategyOptions::default().username_field(""));

#[controller("")]
pub struct Routes;

#[routes]
impl Routes {
    #[post("login")]
    #[use_guards(AuthGuard::new("local"))]
    fn login(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[post("nested")]
    #[use_guards(AuthGuard::new("nested"))]
    fn nested(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[post("userobj")]
    #[use_guards(AuthGuard::new("userobj"))]
    fn userobj(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[post("indexed")]
    #[use_guards(AuthGuard::new("indexed"))]
    fn indexed(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[post("raised")]
    #[use_guards(AuthGuard::new("raised"))]
    fn raised(&self, req: Request) -> Value {
        json!({ "user": req.user().unwrap_or(Value::Null) })
    }

    #[post("renamed")]
    #[use_guards(AuthGuard::new("renamed"))]
    fn renamed(&self, user: User<Echo>) -> Value {
        json!({ "username": user.username })
    }

    /// Shows the failure the strategy reported, which the default 401 hides.
    #[post("challenge")]
    #[use_guards(AuthGuard::new("local").on_failure(|failures| {
        let f = &failures[0];
        BadRequestException::new(json!({ "strategy": f.strategy, "challenge": f.challenge, "status": f.status }))
    }))]
    fn challenge(&self) -> Value {
        json!({})
    }
}

#[module(
    providers = [
        Provider::strategy::<LocalStrategy<Plain>>(),
        Provider::strategy::<LocalStrategy<Nested>>(),
        Provider::strategy::<LocalStrategy<UserObject>>(),
        Provider::strategy::<LocalStrategy<Indexed>>(),
        Provider::strategy::<LocalStrategy<Raised>>(),
        Provider::strategy::<LocalStrategy<Renamed>>(),
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

async fn post(app: &CorvisApplication, path: &str, headers: &[(&str, &str)], body: &str) -> corvis::testing::TestResponse {
    let mut call = request(app).post(path);
    for (k, v) in headers {
        call = call.header(k, v);
    }
    if !body.is_empty() {
        call = call.body(body.to_string());
    }
    call.send().await
}

#[corvis::test]
async fn every_recorded_case_is_replayed() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(fixture["versions"]["passport-local"], "1.0.0");
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() >= 30, "{} cases", cases.len());
    let app = app().await;
    let mut diverged = 0;
    for case in cases {
        let name = case["name"].as_str().unwrap();
        if !case["divergence"].is_null() {
            diverged += 1;
            continue;
        }
        let req = &case["request"];
        let headers: Vec<(&str, &str)> =
            req["headers"].as_object().unwrap().iter().map(|(k, v)| (k.as_str(), v.as_str().unwrap())).collect();
        let r = post(&app, req["path"].as_str().unwrap(), &headers, req["body"].as_str().unwrap_or("")).await;
        let recorded = &case["response"];
        assert_eq!(r.status as u64, recorded["status"].as_u64().unwrap(), "{name}: status, body {}", r.text());
        assert_eq!(r.json_value(), recorded["body"], "{name}: body");
    }
    assert_eq!(diverged, 4);
}

#[corvis::test]
async fn divergences_follow_the_specification() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let app = app().await;
    let json = [("content-type", "application/json")];
    for case in fixture["cases"].as_array().unwrap().iter().filter(|c| !c["divergence"].is_null()) {
        let name = case["name"].as_str().unwrap();
        // The original answered with a user: it called validate() with a non-string or an oversized credential.
        assert_eq!(case["response"]["status"], 201, "{name}");
        let id = format!("divergence-{name}");
        let req = &case["request"];
        let r = post(&app, req["path"].as_str().unwrap(), &[json[0], ("x-test-id", &id)], req["body"].as_str().unwrap()).await;
        assert_eq!(r.status, 401, "{name}");
        assert_eq!(r.json_value(), json!({ "message": "Unauthorized", "statusCode": 401 }), "{name}");
        assert_eq!(calls_of(&id), 0, "{name}: the validator must not run");
    }
}

#[corvis::test]
async fn the_validator_runs_for_valid_credentials_only() {
    let app = app().await;
    let json = |id: &'static str| [("content-type", "application/json"), ("x-test-id", id)];
    let ok = post(&app, "/login", &json("runs-ok"), r#"{"username":"ana","password":"secret"}"#).await;
    assert_eq!(ok.status, 201);
    assert_eq!(calls_of("runs-ok"), 1);
    for (id, body) in [
        ("runs-missing", r#"{"username":"ana"}"#),
        ("runs-empty", r#"{"username":"ana","password":""}"#),
        ("runs-number", r#"{"username":"ana","password":123}"#),
        ("runs-long", &format!(r#"{{"username":"ana","password":"{}"}}"#, "p".repeat(1025))),
    ] {
        let r = post(&app, "/login", &json(id), body).await;
        assert_eq!(r.status, 401, "{id}");
        assert_eq!(calls_of(id), 0, "{id}");
    }
}

#[corvis::test]
async fn missing_and_oversized_credentials_report_a_400_challenge_to_the_guard() {
    let app = app().await;
    let r = post(&app, "/challenge", &[("content-type", "application/json")], r#"{"username":"ana"}"#).await;
    assert_eq!(r.json_value(), json!({ "strategy": "local", "challenge": { "message": "Missing credentials" }, "status": 400 }));
    let body = format!(r#"{{"username":"ana","password":"{}"}}"#, "p".repeat(1025));
    let r = post(&app, "/challenge", &[("content-type", "application/json")], &body).await;
    assert_eq!(r.json_value(), json!({ "strategy": "local", "challenge": { "message": "Credentials too long" }, "status": 400 }));
    // The plain guard still answers the same 401 for both, so the response does not say which credential was missing.
    let r = post(&app, "/login", &[("content-type", "application/json")], &body).await;
    assert_eq!(r.json_value(), json!({ "message": "Unauthorized", "statusCode": 401 }));
}

#[corvis::test]
async fn the_limit_can_be_raised() {
    let app = app().await;
    let body = format!(r#"{{"username":"ana","password":"{}"}}"#, "p".repeat(2000));
    let id = "raised-limit";
    let r = post(&app, "/raised", &[("content-type", "application/json"), ("x-test-id", id)], &body).await;
    assert_eq!(r.status, 201);
    assert_eq!(calls_of(id), 1);
    let too_long = format!(r#"{{"username":"ana","password":"{}"}}"#, "p".repeat(4097));
    assert_eq!(post(&app, "/raised", &[("content-type", "application/json")], &too_long).await.status, 401);
}

#[corvis::test]
async fn field_names_are_configurable() {
    let app = app().await;
    let r = post(&app, "/renamed", &[("content-type", "application/json")], r#"{"email":"ana","pass":"secret"}"#).await;
    assert_eq!(r.json_value(), json!({ "username": "ana" }));
    let r = post(&app, "/renamed", &[("content-type", "application/json")], r#"{"username":"ana","password":"secret"}"#).await;
    assert_eq!(r.status, 401);
}

#[module(providers = [Provider::strategy::<LocalStrategy<NoUsernameField>>()], controllers = [Routes])]
pub struct BrokenModule;

#[corvis::test]
async fn an_empty_field_name_stops_the_application_at_startup() {
    let started = async {
        let mut app = CorvisFactory::create::<BrokenModule>().await?;
        app.silence_route_logs();
        app.init().await
    }
    .await;
    let error = started.unwrap_err().to_string();
    assert!(error.contains("LocalStrategy") && error.contains("empty"), "{error}");
}

// A validator written as the documentation shows it, with a real hasher and stored hashes.

#[derive(Clone)]
struct Account {
    password_hash: String,
}

#[injectable]
pub struct Accounts {
    hasher: std::sync::Arc<PasswordHasher>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Public {
    username: String,
}

#[injectable]
pub struct Hashed {
    accounts: std::sync::Arc<Accounts>,
}

fn small_hasher() -> PasswordHasher {
    PasswordHasher::new(PasswordHasherOptions::bcrypt(4).allow_weak_parameters()).unwrap()
}

fn account(username: &str) -> Option<Account> {
    static HASH: OnceLock<String> = OnceLock::new();
    let hash = HASH.get_or_init(|| small_hasher().hash_blocking("secret").unwrap());
    (username == "ana").then(|| Account { password_hash: hash.clone() })
}

#[async_trait]
impl LocalValidator for Hashed {
    const NAME: &'static str = "hashed";
    type User = Public;
    async fn validate(&self, _req: &Request, username: &str, password: &str) -> Result<Option<Public>, HttpException> {
        let user = account(username);
        let ok = self
            .accounts
            .hasher
            .verify_or_dummy(password, user.as_ref().map(|u| u.password_hash.as_str()))
            .await
            .map_err(|_| InternalServerErrorException::empty())?;
        Ok(user.filter(|_| ok).map(|_| Public { username: username.to_string() }))
    }
}

#[controller("")]
pub struct HashedRoutes;

#[routes]
impl HashedRoutes {
    #[post("hashed")]
    #[use_guards(AuthGuard::new("hashed"))]
    fn hashed(&self, user: User<Public>) -> Value {
        json!({ "username": user.username })
    }
}

#[module(
    providers = [
        Provider::use_value(Token::of::<PasswordHasher>(), small_hasher()),
        Provider::class::<Accounts>(),
        Provider::strategy::<LocalStrategy<Hashed>>(),
    ],
    controllers = [HashedRoutes],
)]
pub struct HashedModule;

#[corvis::test]
async fn a_validator_with_the_hasher_answers_the_same_for_unknown_users_and_wrong_passwords() {
    let mut app = CorvisFactory::create::<HashedModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let json = [("content-type", "application/json")];
    let ok = post(&app, "/hashed", &json, r#"{"username":"ana","password":"secret"}"#).await;
    assert_eq!((ok.status, ok.json_value()), (201, json!({ "username": "ana" })));
    let wrong = post(&app, "/hashed", &json, r#"{"username":"ana","password":"nope"}"#).await;
    let unknown = post(&app, "/hashed", &json, r#"{"username":"eve","password":"nope"}"#).await;
    assert_eq!((wrong.status, wrong.text()), (401, r#"{"message":"Unauthorized","statusCode":401}"#.to_string()));
    assert_eq!((unknown.status, unknown.text()), (wrong.status, wrong.text()));
}
