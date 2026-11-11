//! The local strategy against the original: every case recorded from `passport-local` 1.0.0 behind the original
//! passport integration (`tests/fixtures/auth/local.mjs`) is replayed against the same routes with equivalent Corvis
//! validators. No Node is needed at test time. A case with a `divergence` is asserted against the specification.
#![cfg(feature = "auth-local")]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use corvis::auth::local::{LocalStrategy, LocalStrategyOptions, LocalValidator, PasswordError, PasswordHasher, PasswordHasherOptions};
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
validator!(Querying, "query", LocalStrategyOptions::default().allow_query_credentials());
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

    #[post("query")]
    #[use_guards(AuthGuard::new("query"))]
    fn query(&self, req: Request) -> Value {
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
        Provider::strategy::<LocalStrategy<Querying>>(),
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
    assert_eq!(diverged, 8, "four credential limits and four reads of the query string");
}

fn headers_of(case: &Value) -> Vec<(String, String)> {
    case["request"]["headers"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect()
}

#[corvis::test]
async fn divergences_follow_the_specification() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let app = app().await;
    for case in fixture["cases"].as_array().unwrap().iter().filter(|c| !c["divergence"].is_null()) {
        let name = case["name"].as_str().unwrap();
        // The original answered with a user: it called validate() with a non-string or an oversized credential, or
        // read the credential from the query string.
        assert_eq!(case["response"]["status"], 201, "{name}");
        let id = format!("divergence-{name}");
        let req = &case["request"];
        let mut headers = headers_of(case);
        headers.push(("x-test-id".to_string(), id.clone()));
        let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let r = post(&app, req["path"].as_str().unwrap(), &headers, req["body"].as_str().unwrap_or("")).await;
        assert_eq!(r.status, 401, "{name}");
        assert_eq!(r.json_value(), json!({ "message": "Unauthorized", "statusCode": 401 }), "{name}");
        assert_eq!(calls_of(&id), 0, "{name}: the validator must not run");
        if case["divergence"].as_str().unwrap().contains("allow_query_credentials") {
            // With the option, the strategy behaves as the original did.
            let path = req["path"].as_str().unwrap().replacen("/login", "/query", 1);
            let r = post(&app, &path, &headers, req["body"].as_str().unwrap_or("")).await;
            assert_eq!(r.status as u64, case["response"]["status"].as_u64().unwrap(), "{name}: with allow_query_credentials");
            assert_eq!(r.json_value(), case["response"]["body"], "{name}: with allow_query_credentials");
        }
    }
}

#[corvis::test]
async fn the_query_string_is_ignored_unless_allowed() {
    let app = app().await;
    let r = post(&app, "/login?username=ana&password=secret", &[], "").await;
    assert_eq!(r.status, 401);
    let r = post(&app, "/query?username=ana&password=secret", &[], "").await;
    assert_eq!(r.status, 201);
    // The body still wins over the query when it is read.
    let r = post(&app, "/query?username=eve", &[("content-type", "application/json")], r#"{"username":"ana","password":"secret"}"#).await;
    assert_eq!(r.json_value()["user"]["username"], "ana");
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
    // The blocking methods fail inside a runtime, so the hash is made on a thread of its own.
    let hash = HASH.get_or_init(|| std::thread::spawn(|| small_hasher().hash_blocking("secret").unwrap()).join().unwrap());
    let password_hash = match username {
        "ana" => hash.clone(),
        // An account created through social login, with no password set.
        "social" => String::new(),
        // A legacy hash with a cost above the verification limit.
        "legacy" => "$2b$15$EGdrhbKUv8Oc9vGiXX0HQOxSg445d458Muh7DAHskb6QbtCvdxcie".to_string(),
        // Imports from other systems.
        "php" => "$2x$04$EGdrhbKUv8Oc9vGiXX0HQOxSg445d458Muh7DAHskb6QbtCvdxcie".to_string(),
        "scrypt" => "$scrypt$ln=16,r=8,p=1$aM15713r3Xsvxbi31lqr1Q$nFNh2CVHVjNldFVKDHDlm4CbdRSCdEBsjjJxD+iCs5E".to_string(),
        "short" => "$2b$04$".to_string(),
        _ => return None,
    };
    Some(Account { password_hash })
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
            .map_err(PasswordError::into_http_exception)?;
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

#[corvis::test]
async fn a_refused_stored_hash_answers_like_an_unknown_user() {
    let mut app = CorvisFactory::create::<HashedModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let json = [("content-type", "application/json")];
    let unknown = post(&app, "/hashed", &json, r#"{"username":"nobody","password":"nope"}"#).await;
    let wrong = post(&app, "/hashed", &json, r#"{"username":"ana","password":"nope"}"#).await;
    assert_eq!(unknown.status, 401);
    assert_eq!((wrong.status, wrong.text()), (unknown.status, unknown.text()));
    for user in ["social", "legacy", "php", "scrypt", "short"] {
        let body = format!(r#"{{"username":"{user}","password":"nope"}}"#);
        let r = post(&app, "/hashed", &json, &body).await;
        assert_eq!((r.status, r.text()), (unknown.status, unknown.text()), "{user}");
    }
}

// A hasher that is busy answers 503, the same for every user, and not 401 or 500.

fn slow_hasher() -> &'static PasswordHasher {
    static H: OnceLock<PasswordHasher> = OnceLock::new();
    H.get_or_init(|| {
        let params = corvis::auth::local::Argon2Params { memory_kib: 8, iterations: 1, parallelism: 1 };
        let options = PasswordHasherOptions::argon2id(params)
            .allow_weak_parameters()
            .max_concurrent(1)
            .acquire_timeout(std::time::Duration::from_millis(1))
            .verify_limits(corvis::auth::local::VerifyLimits::default().max_bcrypt_cost(14));
        PasswordHasher::new(options).unwrap()
    })
}

#[injectable]
pub struct Slow;

#[async_trait]
impl LocalValidator for Slow {
    const NAME: &'static str = "slow";
    type User = Public;
    async fn validate(&self, _req: &Request, username: &str, password: &str) -> Result<Option<Public>, HttpException> {
        if username == "hang" {
            // A validator that never answers: the strategy's deadline answers for it.
            std::future::pending::<()>().await;
        }
        let stored = (username == "ana").then_some("$2b$04$iQJJhVONs/5UD7Hw1ADyBusLeIrz/tMsKqJxw7KFBLjqJm.Of4IyK");
        let ok = slow_hasher().verify_or_dummy(password, stored).await.map_err(PasswordError::into_http_exception)?;
        Ok(stored.filter(|_| ok).map(|_| Public { username: username.to_string() }))
    }
}

#[controller("")]
pub struct SlowRoutes;

#[routes]
impl SlowRoutes {
    #[post("slow")]
    #[use_guards(AuthGuard::new("slow"))]
    fn slow(&self, user: User<Public>) -> Value {
        json!({ "username": user.username })
    }
}

#[module(providers = [Provider::class::<Slow>(), Provider::strategy::<LocalStrategy<Slow>>()], controllers = [SlowRoutes])]
pub struct SlowModule;

/// The body of every `503` the authentication module builds.
const UNAVAILABLE: &str = r#"{"statusCode":503,"message":"Service Unavailable"}"#;

/// The two tests that hold the slow hasher's only slot take turns.
static SLOT_TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn slow_app() -> CorvisApplication {
    let mut app = CorvisFactory::create::<SlowModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

#[tokio::test]
async fn a_busy_hasher_answers_503_for_every_user() {
    let _turn = SLOT_TURN.lock().await;
    let app = slow_app().await;
    let json = [("content-type", "application/json")];
    // Free: a known user logs in.
    let ok = post(&app, "/slow", &json, r#"{"username":"ana","password":"secret"}"#).await;
    assert_eq!(ok.status, 201, "{}", ok.text());
    // The only slot is held: both logins are refused before any key derivation.
    let held = slow_hasher().hold_slot().expect("the slot is free");
    assert!(slow_hasher().hold_slot().is_err(), "a held slot is not free");
    let known = post(&app, "/slow", &json, r#"{"username":"ana","password":"secret"}"#).await;
    let unknown = post(&app, "/slow", &json, r#"{"username":"nobody","password":"secret"}"#).await;
    drop(held);
    assert_eq!((known.status, known.text().as_str()), (503, UNAVAILABLE));
    assert_eq!((unknown.status, unknown.text()), (known.status, known.text()), "the same answer for every user");
    // The slot is free again.
    let ok = post(&app, "/slow", &json, r#"{"username":"ana","password":"secret"}"#).await;
    assert_eq!(ok.status, 201, "{}", ok.text());
}

/// Spec "Busy hasher and slow validator on one route": both `503`s are the same bytes, as every other `503` of the
/// module (the JWT, local and API key deadlines, and the OAuth2 strategy at its limit, which `tests/auth_oauth2.rs`
/// compares with the same text).
#[tokio::test(start_paused = true)]
async fn every_framework_503_has_the_same_bytes() {
    let _turn = SLOT_TURN.lock().await;
    let app = slow_app().await;
    let json = [("content-type", "application/json")];
    let held = slow_hasher().hold_slot().expect("the slot is free");
    let busy = post(&app, "/slow", &json, r#"{"username":"ana","password":"secret"}"#).await;
    drop(held);
    // The paused clock runs past the validator's deadline of 5 seconds without waiting.
    let deadline = post(&app, "/slow", &json, r#"{"username":"hang","password":"secret"}"#).await;
    assert_eq!((busy.status, busy.text().as_str()), (503, UNAVAILABLE));
    assert_eq!((deadline.status, deadline.text().as_str()), (503, UNAVAILABLE));
    assert_eq!(PasswordError::Busy.into_http_exception().get_status(), 503);
}
