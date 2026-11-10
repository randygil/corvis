//! The test application of the cross-cutting review of the authentication module (`e3-authentication-security-review`):
//! every built-in strategy registered together (JWT with header and cookie extractors, local with a small bcrypt
//! hasher, API key with a header and `bearer()`, the session over a recording store, two OAuth2 strategies against a
//! mock provider on `127.0.0.1`), and one route per guard shape. It records which handler ran with which user and how
//! many times each validator, the session user lookup and the provider ran, so a test can tell what a request cost
//! without measuring time.
//!
//! Shared by `auth_invariants.rs`, `auth_login_throttle.rs` and `fuzz_auth.rs`. The binaries that use it declare
//! `mod session_common;` and `mod oauth2_common;` next to `mod auth_common;`, because this module reuses the recording
//! store and the mock provider of those two.
#![allow(dead_code, unused_imports)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use corvis::auth::api_key::{ApiKey, ApiKeySource, ApiKeyStrategy, ApiKeyStrategyOptions, ApiKeyValidator};
use corvis::auth::jwt::{ExtractJwt, JwtKey, JwtKeyProvider, JwtStrategy, JwtStrategyOptions, JwtValidator};
use corvis::auth::local::{LocalStrategy, LocalStrategyOptions, LocalValidator, PasswordError, PasswordHasher, PasswordHasherOptions};
use corvis::auth::oauth2::{OAuth2Grant, OAuth2Options, OAuth2Strategy, OAuth2Validator};
use corvis::auth::session::{SessionModule, SessionOptions, SessionSerializer, SessionStrategy};
use corvis::auth::{AuthGuard, Authenticator, PassportModule, PassportModuleOptions};
use corvis::{ThrottlerGuard, ThrottlerModule, ThrottlerOptions, ThrottlerStorage};
use corvis::prelude::*;
use corvis::testing::{Test, TestResponse, request};
use serde::{Deserialize, Serialize};

use crate::oauth2_common::{self, Mock};
use crate::session_common::RecordingStore;

pub const JWT_SECRET: &str = "auth-common-jwt-secret-0123456789abcdef";
pub const OTHER_SECRET: &str = "auth-common-other-secret-0123456789abc";
pub const SESSION_SECRET: &str = "auth-common-session-secret-0123456789abcdef";
/// The API key of account 3.
pub const API_KEY: &str = "ak-test-0000000000000000000000000000000001";
/// The password of every account.
pub const PASSWORD: &str = "secret";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub role: String,
}

/// Accounts 1 to 4 and the ones a login through the provider creates (`gh:<provider id>`).
pub fn account(id: &str) -> Option<Account> {
    let (name, role) = match id {
        "1" => ("ana", "admin"),
        "2" => ("bob", "user"),
        "3" => ("cy", "user"),
        "4" => ("dee", "user"),
        other if other.starts_with("gh:") => ("social", "user"),
        _ => return None,
    };
    Some(Account { id: id.to_string(), name: name.to_string(), role: role.to_string() })
}

pub fn account_by_name(name: &str) -> Option<Account> {
    ["1", "2", "3", "4"].iter().filter_map(|id| account(id)).find(|a| a.name == name)
}

/// One handler run: which route and the user the request had.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub route: String,
    pub user: Option<Value>,
}

/// Counters and logs of one application.
#[derive(Default)]
pub struct Probe {
    pub jwt: AtomicUsize,
    pub local: AtomicUsize,
    pub key: AtomicUsize,
    pub oauth: AtomicUsize,
    /// Session user lookups (`deserialize_user`).
    pub lookups: AtomicUsize,
    /// The raw tokens the JWT key provider was asked about (it runs for every well-formed token before the signature).
    pub jwt_raw: Mutex<Vec<String>>,
    /// The keys the API key validator received.
    pub key_raw: Mutex<Vec<String>>,
    /// `{:?}` of every OAuth2 grant a validator received.
    pub grants: Mutex<Vec<String>>,
    hits: Mutex<Vec<Hit>>,
}

impl Probe {
    pub fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }

    pub fn runs(&self, route: &str) -> usize {
        self.hits.lock().unwrap().iter().filter(|h| h.route == route).count()
    }

    pub fn last_user(&self, route: &str) -> Option<Value> {
        self.hits.lock().unwrap().iter().rev().find(|h| h.route == route).and_then(|h| h.user.clone())
    }

    /// Validator and lookup calls, summed: what a request cost besides the guards themselves.
    pub fn strategy_calls(&self) -> usize {
        [&self.jwt, &self.local, &self.key, &self.oauth, &self.lookups].iter().map(|c| c.load(Ordering::SeqCst)).sum()
    }

    pub fn count(counter: &AtomicUsize) -> usize {
        counter.load(Ordering::SeqCst)
    }
}

pub struct ProbeRef(pub Arc<Probe>);

pub struct OauthOptions(pub OAuth2Options);
pub struct OauthSessionOptions(pub OAuth2Options);

// ---- strategies ----

struct RecordingKey(Arc<Probe>);

#[async_trait]
impl JwtKeyProvider for RecordingKey {
    async fn key(&self, _req: &Request, raw_token: &str) -> anyhow::Result<JwtKey> {
        self.0.jwt_raw.lock().unwrap().push(raw_token.to_string());
        Ok(JwtKey::secret(JWT_SECRET))
    }
}

#[injectable]
pub struct JwtAuth {
    probe: Arc<ProbeRef>,
}

#[async_trait]
impl JwtValidator for JwtAuth {
    const NAME: &'static str = "jwt";
    type User = Account;

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_extractors([ExtractJwt::from_auth_header_as_bearer_token(), ExtractJwt::from_cookie("jwt")]))
            .secret_or_key_provider(RecordingKey(self.probe.0.clone()))
    }

    async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Account>, HttpException> {
        self.probe.0.jwt.fetch_add(1, Ordering::SeqCst);
        Ok(payload.get("sub").and_then(Value::as_str).and_then(account))
    }
}

#[injectable]
pub struct KeyAuth {
    probe: Arc<ProbeRef>,
}

#[async_trait]
impl ApiKeyValidator for KeyAuth {
    const NAME: &'static str = "api-key";
    type User = Account;

    fn options(&self) -> ApiKeyStrategyOptions {
        ApiKeyStrategyOptions::new().source(ApiKeySource::header("x-api-key")).source(ApiKeySource::bearer())
    }

    async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<Account>, HttpException> {
        self.probe.0.key.fetch_add(1, Ordering::SeqCst);
        self.probe.0.key_raw.lock().unwrap().push(key.expose().to_string());
        Ok(key.ct_eq(API_KEY).then(|| account("3")).flatten())
    }
}

pub fn small_hasher() -> PasswordHasher {
    PasswordHasher::new(PasswordHasherOptions::bcrypt(4).allow_weak_parameters()).unwrap()
}

/// The bcrypt hash of [`PASSWORD`], made once on a thread of its own (the blocking variants refuse a runtime).
fn password_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| std::thread::spawn(|| small_hasher().hash_blocking(PASSWORD).unwrap()).join().unwrap())
}

#[injectable]
pub struct LocalAuth {
    probe: Arc<ProbeRef>,
    hasher: Arc<PasswordHasher>,
    flags: Arc<LocalFlags>,
}

/// The options of the local strategy of the application.
pub struct LocalFlags(pub LocalStrategyOptions);

#[async_trait]
impl LocalValidator for LocalAuth {
    const NAME: &'static str = "local";
    type User = Account;

    fn options(&self) -> LocalStrategyOptions {
        self.flags.0.clone()
    }

    async fn validate(&self, _req: &Request, username: &str, password: &str) -> Result<Option<Account>, HttpException> {
        // Each call derives one key (`verify_or_dummy` verifies against a dummy hash for an unknown user), so this
        // counter is the key derivation counter of the hasher.
        self.probe.0.local.fetch_add(1, Ordering::SeqCst);
        let user = account_by_name(username);
        let hash = user.as_ref().map(|_| password_hash());
        let ok = self.hasher.verify_or_dummy(password, hash).await.map_err(PasswordError::into_http_exception)?;
        Ok(user.filter(|_| ok))
    }
}

#[injectable]
pub struct Accounts {
    probe: Arc<ProbeRef>,
}

#[async_trait]
impl SessionSerializer for Accounts {
    type User = Account;

    async fn serialize_user(&self, user: &Account) -> Result<Value, HttpException> {
        Ok(json!(user.id))
    }

    async fn deserialize_user(&self, _req: &Request, stored: Value) -> Result<Option<Account>, HttpException> {
        self.probe.0.lookups.fetch_add(1, Ordering::SeqCst);
        Ok(stored.as_str().and_then(account))
    }
}

fn provider_user(grant: &OAuth2Grant) -> Option<Account> {
    let id = grant.profile.as_ref().map(|p| p.id.clone()).unwrap_or_else(|| "anon".to_string());
    account(&format!("gh:{id}"))
}

macro_rules! oauth_validator {
    ($name:ident, $cfg:ident, $label:literal) => {
        #[injectable]
        pub struct $name {
            probe: Arc<ProbeRef>,
            cfg: Arc<$cfg>,
        }

        #[async_trait]
        impl OAuth2Validator for $name {
            const NAME: &'static str = $label;
            type User = Account;

            fn options(&self) -> OAuth2Options {
                self.cfg.0.clone()
            }

            async fn validate(&self, _req: &Request, grant: OAuth2Grant) -> Result<Option<Account>, HttpException> {
                self.probe.0.oauth.fetch_add(1, Ordering::SeqCst);
                self.probe.0.grants.lock().unwrap().push(format!("{grant:?}"));
                Ok(provider_user(&grant))
            }
        }
    };
}

oauth_validator!(OauthAuth, OauthOptions, "oauth");
oauth_validator!(OauthSessionAuth, OauthSessionOptions, "oauth-s");

// ---- guards and routes ----

/// A hand-written authorization guard that trusts `Request::user()` and runs no `AuthGuard`.
#[injectable]
pub struct RolesGuard;

#[async_trait]
impl corvis::CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let user = ctx.request().user_as::<Account>();
        Ok(user.is_some_and(|u| u.role == "admin"))
    }
}

#[controller("")]
pub struct Routes {
    probe: Arc<ProbeRef>,
    authenticator: Arc<Authenticator>,
}

impl Routes {
    fn hit(&self, route: &str, req: &Request) -> Value {
        let user = req.user();
        self.probe.0.hits.lock().unwrap().push(Hit { route: route.to_string(), user: user.clone() });
        json!({ "route": route, "user": user, "return_to": corvis::auth::oauth2::return_to(req) })
    }
}

#[routes]
impl Routes {
    #[get("jwt")]
    #[use_guards(AuthGuard::new("jwt"))]
    fn jwt(&self, req: Request) -> Value {
        self.hit("jwt", &req)
    }

    /// The same guard as `jwt`, read through the typed extractor.
    #[get("jwt-typed")]
    #[use_guards(AuthGuard::new("jwt"))]
    fn jwt_typed(&self, user: corvis::auth::User<Account>) -> Value {
        json!({ "typed": user.0.id })
    }

    #[get("jwt-opt")]
    #[use_guards(AuthGuard::new("jwt").optional())]
    fn jwt_opt(&self, req: Request) -> Value {
        self.hit("jwt-opt", &req)
    }

    #[get("key")]
    #[use_guards(AuthGuard::new("api-key"))]
    fn key(&self, req: Request) -> Value {
        self.hit("key", &req)
    }

    #[get("session")]
    #[use_guards(AuthGuard::new("session"))]
    fn session(&self, req: Request) -> Value {
        self.hit("session", &req)
    }

    #[get("chain-jwt-session")]
    #[use_guards(AuthGuard::chain(["jwt", "session"]))]
    fn chain_jwt_session(&self, req: Request) -> Value {
        self.hit("chain-jwt-session", &req)
    }

    #[get("chain-jwt-key")]
    #[use_guards(AuthGuard::chain(["jwt", "api-key"]))]
    fn chain_jwt_key(&self, req: Request) -> Value {
        self.hit("chain-jwt-key", &req)
    }

    #[get("chain-key-jwt")]
    #[use_guards(AuthGuard::chain(["api-key", "jwt"]))]
    fn chain_key_jwt(&self, req: Request) -> Value {
        self.hit("chain-key-jwt", &req)
    }

    #[get("chain-jwt-key-opt")]
    #[use_guards(AuthGuard::chain(["jwt", "api-key"]).optional())]
    fn chain_jwt_key_opt(&self, req: Request) -> Value {
        self.hit("chain-jwt-key-opt", &req)
    }

    #[get("chain-oauth-key")]
    #[use_guards(AuthGuard::chain(["oauth", "api-key"]))]
    fn chain_oauth_key(&self, req: Request) -> Value {
        self.hit("chain-oauth-key", &req)
    }

    #[get("default")]
    #[use_guards(AuthGuard::default())]
    fn default_guard(&self, req: Request) -> Value {
        self.hit("default", &req)
    }

    /// No guard at all.
    #[get("open")]
    fn open(&self, req: Request) -> Value {
        self.hit("open", &req)
    }

    /// Only a hand-written authorization guard.
    #[get("roles")]
    #[use_guards(RolesGuard)]
    fn roles(&self, req: Request) -> Value {
        self.hit("roles", &req)
    }

    #[get("skip-public")]
    #[public]
    #[use_guards(AuthGuard::new("jwt").skip_public())]
    fn skip_public(&self, req: Request) -> Value {
        self.hit("skip-public", &req)
    }

    #[get("skip-private")]
    #[use_guards(AuthGuard::new("jwt").skip_public())]
    fn skip_private(&self, req: Request) -> Value {
        self.hit("skip-private", &req)
    }

    // The login recipe of the rate limiting page: a limit per client on the `default` throttler and one per account on
    // the `login` throttler (a no-op unless the application has those throttlers).
    #[post("login")]
    #[http_code(200)]
    #[apply_decorators(throttle("default").limit(10), throttle("login").limit(5).tracker("login-account"))]
    #[use_guards(AuthGuard::new("local"))]
    fn login(&self, req: Request) -> Value {
        self.hit("login", &req)
    }

    #[post("login-session")]
    #[http_code(200)]
    #[apply_decorators(throttle("default").limit(10), throttle("login").limit(5).tracker("login-account"))]
    #[use_guards(AuthGuard::new("local").log_in())]
    fn login_session(&self, req: Request) -> Value {
        self.hit("login-session", &req)
    }

    /// Gives the browser a session with some data, without logging anybody in.
    #[post("touch")]
    #[http_code(200)]
    fn touch(&self, req: Request) -> Value {
        req.session().expect("the session module runs for every route").set("seen", true).unwrap();
        self.hit("touch", &req)
    }

    #[post("logout")]
    #[http_code(200)]
    async fn logout(&self, req: Request) -> Result<Value, HttpException> {
        self.authenticator.log_out(&req).await?;
        Ok(self.hit("logout", &req))
    }

    // OAuth2 login and callback routes: the per-client throttler only.
    #[get("oauth")]
    #[apply_decorators(throttle("default").limit(5))]
    #[use_guards(AuthGuard::new("oauth"))]
    fn oauth(&self, req: Request) -> Value {
        self.hit("oauth", &req)
    }

    #[get("oauth-s")]
    #[apply_decorators(throttle("default").limit(5))]
    #[use_guards(AuthGuard::new("oauth-s"))]
    fn oauth_session(&self, req: Request) -> Value {
        self.hit("oauth-s", &req)
    }

    #[get("oauth-s-login")]
    #[apply_decorators(throttle("default").limit(5))]
    #[use_guards(AuthGuard::new("oauth-s").log_in())]
    fn oauth_session_login(&self, req: Request) -> Value {
        self.hit("oauth-s-login", &req)
    }
}

/// A gateway that says what its handshake request carries.
#[corvis::websocket_gateway(path = "/ws")]
pub struct Gateway {
    #[websocket_server]
    server: corvis::WsServer,
    probe: Arc<ProbeRef>,
}

#[corvis::handlers]
impl Gateway {
    #[subscribe_message("who")]
    fn who(&self, #[connected_socket] client: corvis::WsClient) -> Value {
        let handshake = client.handshake();
        self.probe.0.hits.lock().unwrap().push(Hit { route: "ws-who".into(), user: handshake.user() });
        json!({ "user": handshake.user(), "session": handshake.session().is_some() })
    }
}

/// A controller marked `#[public]`: its handlers have no guard unless they carry one.
#[controller("pub")]
#[public]
pub struct PublicRoutes {
    probe: Arc<ProbeRef>,
}

#[routes]
impl PublicRoutes {
    /// Not marked itself; the route-level guard was built without `.skip_public()`.
    #[get("own-guard")]
    #[use_guards(AuthGuard::new("jwt"))]
    fn own_guard(&self, req: Request) -> Value {
        self.probe.0.hits.lock().unwrap().push(Hit { route: "pub/own-guard".into(), user: req.user() });
        json!({ "ok": true })
    }

    #[get("skip")]
    #[use_guards(AuthGuard::new("jwt").skip_public())]
    fn skip(&self, req: Request) -> Value {
        self.probe.0.hits.lock().unwrap().push(Hit { route: "pub/skip".into(), user: req.user() });
        json!({ "ok": true })
    }
}

// ---- the application ----

/// An application and what a test looks at.
pub struct Harness {
    pub app: corvis::CorvisApplication,
    pub probe: Arc<Probe>,
    pub mock: Mock,
    pub store: Arc<RecordingStore>,
}

pub struct Config {
    /// Applies to `SessionOptions` before the application starts.
    pub session: Box<dyn FnOnce(SessionOptions) -> SessionOptions>,
    pub provider: Box<dyn Fn(&oauth2_common::Recorded) -> oauth2_common::Reply + Send + Sync>,
    pub trust_proxy: bool,
    /// `require_route_posture()`: the application does not start with an unprotected route.
    pub posture: bool,
    /// A global `AuthGuard::new("jwt").optional()`.
    pub global_guard: bool,
    /// The throttler storage; when set, `ThrottlerModule` is imported and `ThrottlerGuard` is a global guard, so it
    /// runs before every `AuthGuard` of the grid.
    pub throttle: Option<ThrottlerStorage>,
    pub local: LocalStrategyOptions,
    /// Applies to the options of the `oauth` strategy (the one that keeps its state in a cookie).
    pub oauth: Box<dyn FnOnce(OAuth2Options) -> OAuth2Options>,
    /// The application logs (the request line included, through `LoggingInterceptor`): a test installs a sink first.
    pub logging: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            session: Box::new(|o| o),
            provider: Box::new(oauth2_common::standard),
            trust_proxy: false,
            posture: false,
            global_guard: false,
            throttle: None,
            local: LocalStrategyOptions::default(),
            oauth: Box::new(|o| o),
            logging: false,
        }
    }
}

/// The storage of the login recipe: `default` counts by client (60 s), `login` counts by account (5 min) through the
/// tracker `login-account`, which reads the username the local strategy configured with `local` will read.
pub fn recipe_storage(local: LocalStrategyOptions) -> ThrottlerStorage {
    ThrottlerStorage::new(vec![
        ThrottlerOptions::new(60_000, 100),
        ThrottlerOptions { name: "login".into(), ttl: 300_000, limit: 100 },
    ])
    .with_named_tracker("login-account", corvis::auth::local::login_tracker(local))
}

pub fn oauth_options(mock: &Mock) -> OAuth2Options {
    oauth2_common::generic_options(mock)
}

pub fn oauth_session_options(mock: &Mock) -> OAuth2Options {
    oauth2_common::generic_options(mock).state_in_session()
}

pub fn session_options(store: &Arc<RecordingStore>) -> SessionOptions {
    SessionOptions::with_secrets([SESSION_SECRET]).unwrap().store_arc(store.clone())
}

impl Harness {
    pub async fn new() -> Harness {
        Self::with(Config::default()).await
    }

    pub async fn with(config: Config) -> Harness {
        Self::try_with(config).await.unwrap_or_else(|e| panic!("the application did not start: {e}"))
    }

    /// The application, or the text of the error that stopped it.
    pub async fn try_with(config: Config) -> Result<Harness, String> {
        let mock = Mock::start(config.provider).await;
        let store = RecordingStore::new();
        let probe = Arc::new(Probe::default());
        let session = (config.session)(session_options(&store));
        let mut builder = Test::create_testing_module().with_logger(config.logging);
        if let Some(storage) = config.throttle {
            builder = builder.import_dynamic(ThrottlerModule::for_root_storage(storage)).providers(vec![Provider::app_guard::<ThrottlerGuard>()]);
        }
        let module = builder
            .import_dynamic(PassportModule::register(PassportModuleOptions::new().default_strategy("jwt")))
            .import_dynamic(SessionModule::for_root(session))
            .providers(vec![
                Provider::value(ProbeRef(probe.clone())),
                Provider::value(LocalFlags(config.local)),
                Provider::value(OauthOptions((config.oauth)(oauth_options(&mock)))),
                Provider::value(OauthSessionOptions(oauth_session_options(&mock))),
                Provider::use_value(Token::of::<PasswordHasher>(), small_hasher()),
                Provider::class::<JwtAuth>(),
                Provider::class::<KeyAuth>(),
                Provider::class::<LocalAuth>(),
                Provider::class::<OauthAuth>(),
                Provider::class::<OauthSessionAuth>(),
                Provider::class::<RolesGuard>(),
                Provider::class::<Gateway>(),
                Provider::strategy::<JwtStrategy<JwtAuth>>(),
                Provider::strategy::<ApiKeyStrategy<KeyAuth>>(),
                Provider::strategy::<LocalStrategy<LocalAuth>>(),
                Provider::strategy::<SessionStrategy<Accounts>>(),
                Provider::strategy::<OAuth2Strategy<OauthAuth>>(),
                Provider::strategy::<OAuth2Strategy<OauthSessionAuth>>(),
            ])
            .controller::<Routes>()
            .controller::<PublicRoutes>()
            .compile()
            .await
            .map_err(|e| format!("{e:#}"))?;
        let mut app = module.create_corvis_application();
        if config.trust_proxy {
            app.set_trust_proxy(true);
        }
        if config.posture {
            app.require_route_posture();
        }
        if config.global_guard {
            app.use_global_guards(AuthGuard::new("jwt").optional());
        }
        if config.logging {
            app.use_global_interceptors(corvis::LoggingInterceptor);
        }
        app.init().await.map_err(|e| format!("{e:#}"))?;
        Ok(Harness { app, probe, mock, store })
    }

    pub async fn get(&self, path: &str, headers: &[(&str, &str)]) -> TestResponse {
        let mut call = request(&self.app).get(path);
        for (k, v) in headers {
            call = call.header(k, v);
        }
        call.send().await
    }

    pub async fn post_json(&self, path: &str, headers: &[(&str, &str)], body: Value) -> TestResponse {
        let mut call = request(&self.app).post(path).header("content-type", "application/json");
        for (k, v) in headers {
            call = call.header(k, v);
        }
        call.body(body.to_string()).send().await
    }

    /// Logs in as `name` with the local strategy and returns the session cookie pair.
    pub async fn login(&self, name: &str) -> String {
        let r = self.post_json("/login-session", &[], json!({ "username": name, "password": PASSWORD })).await;
        assert_eq!(r.status, 200, "{}", r.text());
        session_pair(&r).expect("a session cookie")
    }

    pub fn calls(&self) -> usize {
        self.probe.strategy_calls()
    }
}

// ---- credentials and responses ----

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// An HS256 token for account `sub` signed with `secret`, valid for an hour (`exp_offset` seconds from now, negative
/// for an expired one).
pub fn token_with(sub: &str, secret: &str, exp_offset: i64) -> String {
    let exp = (now() as i64 + exp_offset) as u64;
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &json!({ "sub": sub, "exp": exp }),
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap()
}

/// A valid token of account `sub`.
pub fn token(sub: &str) -> String {
    token_with(sub, JWT_SECRET, 3600)
}

pub fn expired_token(sub: &str) -> String {
    token_with(sub, JWT_SECRET, -3600)
}

pub fn forged_token(sub: &str) -> String {
    token_with(sub, OTHER_SECRET, 3600)
}

pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// Every `Set-Cookie` of a response.
pub fn set_cookies(r: &TestResponse) -> Vec<String> {
    r.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect()
}

/// The `name=value` pair of the session cookie a response sets, if it does.
pub fn session_pair(r: &TestResponse) -> Option<String> {
    set_cookies(r).iter().find(|c| c.starts_with("connect.sid=")).map(|c| c.split(';').next().unwrap().to_string())
}

/// The header names of a response, sorted.
pub fn header_names(r: &TestResponse) -> Vec<String> {
    let mut names: Vec<String> = r.headers.keys().map(|k| k.as_str().to_string()).collect();
    names.sort();
    names.dedup();
    names
}

/// A response without the `date` header, to compare two for equality.
pub fn shape(r: &TestResponse) -> (u16, String, Vec<String>) {
    let names = header_names(r).into_iter().filter(|n| n != "date").collect();
    (r.status, r.text(), names)
}

pub fn unauthorized() -> Value {
    json!({ "message": "Unauthorized", "statusCode": 401 })
}

/// The id of the user a handler saw, from the response body of the grid.
pub fn user_id(r: &TestResponse) -> Option<String> {
    r.json_value()["user"]["id"].as_str().map(str::to_string)
}

/// The case count of the property tests: `PROPTEST_CASES`, else `default`.
pub fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
