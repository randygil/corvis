//! The deadlines of the application's callbacks (`auth/security-invariants`, "Callbacks the application supplies have a
//! deadline"): the JWT key provider, the JWT validator and the local validator answer a `503` that is the same for every
//! caller when they run too long, the dropped future is cancelled, and the warning is written once a minute. The tests
//! run on a paused clock, so the slow callbacks sleep for an hour and nothing waits: no timing is measured. Its own test
//! binary: the logger is process-wide, so the tests serialize on one mutex.
#![cfg(all(feature = "jwt", feature = "auth-local"))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvis::auth::jwt::{ExtractJwt, JwtKey, JwtKeyProvider, JwtStrategy, JwtStrategyOptions, JwtValidator};
use corvis::auth::local::{LocalStrategy, LocalStrategyOptions, LocalValidator};
use corvis::auth::{AuthGuard, User};
use corvis::prelude::*;
use corvis::testing::request;
use corvis::{LogLevel, LoggerService};
use serde::Serialize;

const SECRET: &str = "corvis-jwt-secret-one-0123456789abcdef";
const OTHER: &str = "corvis-jwt-secret-two-0123456789abcdef";

/// Counts the futures that were dropped before they finished.
static CANCELLED: AtomicUsize = AtomicUsize::new(0);

struct OnDrop;

impl Drop for OnDrop {
    fn drop(&mut self) {
        CANCELLED.fetch_add(1, Ordering::SeqCst);
    }
}

async fn hang() {
    let _guard = OnDrop;
    tokio::time::sleep(Duration::from_secs(3600)).await;
}

#[derive(Debug, Clone, Serialize)]
pub struct Who {
    name: String,
}

struct HangingKeys;

#[async_trait]
impl JwtKeyProvider for HangingKeys {
    async fn key(&self, _req: &Request, _raw_token: &str) -> anyhow::Result<JwtKey> {
        hang().await;
        Ok(JwtKey::secret(SECRET))
    }
}

#[injectable]
pub struct SlowKeyAuth;

#[async_trait]
impl JwtValidator for SlowKeyAuth {
    const NAME: &'static str = "slow-keys";
    type User = Who;

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key_provider(HangingKeys)
    }

    async fn validate(&self, _req: &Request, _payload: Value) -> Result<Option<Who>, HttpException> {
        Ok(Some(Who { name: "a".into() }))
    }
}

#[injectable]
pub struct SlowJwtAuth;

#[async_trait]
impl JwtValidator for SlowJwtAuth {
    const NAME: &'static str = "slow-jwt";
    type User = Who;

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
            .secret_or_key(JwtKey::secret(SECRET))
            .validator_timeout(Duration::from_secs(2))
    }

    async fn validate(&self, _req: &Request, _payload: Value) -> Result<Option<Who>, HttpException> {
        hang().await;
        Ok(Some(Who { name: "a".into() }))
    }
}

#[injectable]
pub struct SlowLocalAuth;

#[async_trait]
impl LocalValidator for SlowLocalAuth {
    const NAME: &'static str = "slow-local";
    type User = Who;

    fn options(&self) -> LocalStrategyOptions {
        LocalStrategyOptions::default()
    }

    async fn validate(&self, _req: &Request, username: &str, _password: &str) -> Result<Option<Who>, HttpException> {
        if username == "quick" {
            return Ok(Some(Who { name: "quick".into() }));
        }
        hang().await;
        Ok(None)
    }
}

#[controller("")]
pub struct Routes;

#[routes]
impl Routes {
    #[get("keys")]
    #[use_guards(AuthGuard::new("slow-keys"))]
    fn keys(&self, who: User<Who>) -> Value {
        json!({ "name": who.name })
    }

    #[get("keys-chain")]
    #[use_guards(AuthGuard::chain(["slow-keys", "slow-jwt"]).optional())]
    fn keys_chain(&self, req: Request) -> Value {
        json!({ "user": req.user() })
    }

    #[get("jwt")]
    #[use_guards(AuthGuard::new("slow-jwt"))]
    fn jwt(&self, who: User<Who>) -> Value {
        json!({ "name": who.name })
    }

    #[post("login")]
    #[use_guards(AuthGuard::new("slow-local"))]
    fn login(&self, who: User<Who>) -> Value {
        json!({ "name": who.name })
    }
}

#[module(
    providers = [
        Provider::strategy::<JwtStrategy<SlowKeyAuth>>(),
        Provider::strategy::<JwtStrategy<SlowJwtAuth>>(),
        Provider::strategy::<LocalStrategy<SlowLocalAuth>>(),
    ],
    controllers = [Routes],
)]
pub struct AppModule;

#[injectable]
pub struct ZeroKeyTimeout;

#[async_trait]
impl JwtValidator for ZeroKeyTimeout {
    const NAME: &'static str = "zero-key";
    type User = Who;

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
            .secret_or_key_provider(HangingKeys)
            .key_provider_timeout(Duration::ZERO)
    }

    async fn validate(&self, _req: &Request, _payload: Value) -> Result<Option<Who>, HttpException> {
        Ok(None)
    }
}

#[module(providers = [Provider::strategy::<JwtStrategy<ZeroKeyTimeout>>()])]
pub struct ZeroKeyModule;

#[injectable]
pub struct HugeValidatorTimeout;

#[async_trait]
impl LocalValidator for HugeValidatorTimeout {
    const NAME: &'static str = "huge-local";
    type User = Who;

    fn options(&self) -> LocalStrategyOptions {
        LocalStrategyOptions::default().validator_timeout(Duration::from_secs(301))
    }

    async fn validate(&self, _req: &Request, _username: &str, _password: &str) -> Result<Option<Who>, HttpException> {
        Ok(None)
    }
}

#[module(providers = [Provider::strategy::<LocalStrategy<HugeValidatorTimeout>>()])]
pub struct HugeLocalModule;

type Captured = Arc<Mutex<Vec<(LogLevel, String, Option<String>)>>>;

struct Capture(Captured);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn capture() -> Captured {
    let captured: Captured = Arc::default();
    corvis::logger::set_logger(Capture(captured.clone()));
    corvis::logger::set_log_levels(&LogLevel::all());
    captured
}

fn warnings(captured: &Captured, what: &str) -> Vec<String> {
    captured
        .lock()
        .unwrap()
        .iter()
        .filter(|(level, message, _)| *level == LogLevel::Warn && message.contains(what))
        .map(|(_, m, _)| m.clone())
        .collect()
}

async fn start<M: corvis::Module>() -> CorvisApplication {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

fn token(secret: &str) -> String {
    let exp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600;
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &json!({ "sub": "1", "exp": exp }),
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap()
}

fn unavailable() -> Value {
    json!({ "statusCode": 503, "message": "Service Unavailable" })
}

#[tokio::test(start_paused = true)]
async fn a_key_provider_that_hangs_is_a_503_for_every_token_and_is_cancelled() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let app = start::<AppModule>().await;
    let before = CANCELLED.load(Ordering::SeqCst);
    // A forged token reaches the provider as well as a genuine one: the answer does not tell them apart.
    let mut answers = Vec::new();
    for secret in [SECRET, OTHER, SECRET] {
        let r = request(&app).get("/keys").bearer(&token(secret)).send().await;
        answers.push((r.status, r.json_value()));
    }
    assert!(answers.iter().all(|a| *a == (503, unavailable())), "{answers:?}");
    assert_eq!(CANCELLED.load(Ordering::SeqCst) - before, 3, "each dropped future was cancelled");
    // An optional guard and a chain do not hide it either: the 503 is an error, not a failure.
    let r = request(&app).get("/keys-chain").bearer(&token(OTHER)).send().await;
    assert_eq!((r.status, r.json_value()), (503, unavailable()));
    // A request with no token never reaches the provider.
    let r = request(&app).get("/keys").send().await;
    assert_eq!(r.status, 401);
    // One warning for the burst, naming the strategy and no token.
    let lines = warnings(&captured, "key_provider_timeout");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("\"slow-keys\""), "{lines:?}");
    assert!(!lines[0].contains(&token(SECRET)));
}

#[tokio::test(start_paused = true)]
async fn a_jwt_validator_that_hangs_is_a_503_and_is_cancelled() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let app = start::<AppModule>().await;
    let before = CANCELLED.load(Ordering::SeqCst);
    for _ in 0..3 {
        let r = request(&app).get("/jwt").bearer(&token(SECRET)).send().await;
        assert_eq!((r.status, r.json_value()), (503, unavailable()));
    }
    assert_eq!(CANCELLED.load(Ordering::SeqCst) - before, 3);
    // A token the key refuses never reaches the validator.
    let r = request(&app).get("/jwt").bearer(&token(OTHER)).send().await;
    assert_eq!(r.status, 401);
    assert_eq!(CANCELLED.load(Ordering::SeqCst) - before, 3);
    let lines = warnings(&captured, "validator_timeout");
    assert_eq!(lines.len(), 1, "once a minute, not once a request: {lines:?}");
    assert!(lines[0].contains("\"slow-jwt\"") && lines[0].contains("answered 503"), "{lines:?}");
}

#[tokio::test(start_paused = true)]
async fn a_local_validator_that_hangs_is_a_503_and_is_cancelled() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let app = start::<AppModule>().await;
    let before = CANCELLED.load(Ordering::SeqCst);
    for user in ["ana", "bob"] {
        let r = request(&app).post("/login").json(&json!({ "username": user, "password": "secret" })).send().await;
        assert_eq!((r.status, r.json_value()), (503, unavailable()), "{user}");
    }
    assert_eq!(CANCELLED.load(Ordering::SeqCst) - before, 2);
    // A validator that answers in time is untouched.
    let r = request(&app).post("/login").json(&json!({ "username": "quick", "password": "secret" })).send().await;
    assert_eq!(r.status, 201, "{}", r.text());
    let lines = warnings(&captured, "validator_timeout");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("\"slow-local\""), "{lines:?}");
    assert!(!lines[0].contains("ana") && !lines[0].contains("bob") && !lines[0].contains("secret"), "{lines:?}");
}

#[tokio::test]
async fn the_deadlines_are_checked_at_startup() {
    let _g = SERIAL.lock().await;
    for (error, name) in
        [(start_error::<ZeroKeyModule>().await, "key_provider_timeout"), (start_error::<HugeLocalModule>().await, "validator_timeout")]
    {
        assert!(error.contains(name) && error.contains("between 1 ms and 300 s"), "{error}");
    }
}

async fn start_error<M: corvis::Module>() -> String {
    match CorvisFactory::create::<M>().await {
        Ok(mut app) => match app.init().await {
            Ok(()) => panic!("the application started"),
            Err(e) => e.to_string(),
        },
        Err(e) => e.to_string(),
    }
}
