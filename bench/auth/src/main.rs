//! The server of the authentication benchmark (`bench/auth/run.sh`): one route per scenario, setup routes that issue
//! the tokens and seed the session, and `GET /__bench/stats` with the request and allocation counters.
//!
//! It listens on `127.0.0.1` only (`PORT`, 3290 by default). The session module's cookie path is `/session`, so the JWT
//! routes do not run the session middleware.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use corvis::auth::jwt::{ExtractJwt, JwtKey, JwtKeyProvider, JwtStrategy, JwtStrategyOptions, JwtValidator};
use corvis::auth::session::{MemorySessionStore, SessionCookie, SessionModule, SessionOptions};
use corvis::auth::{AuthGuard, User};
use corvis::jwt::JwtAlgorithm;
use corvis::prelude::*;
use corvis::{JwtModuleOptions, JwtService, extract_bearer_token};
use serde::{Deserialize, Serialize};

// ---------- Counters ----------

static REQUESTS: AtomicU64 = AtomicU64::new(0);

/// The allocation counters are spread over shards, one per thread (modulo the number of shards), each on its own cache
/// lines: a single pair of counters shared by every worker thread becomes the bottleneck of the scenarios that
/// allocate a lot, and the throughput would measure the counter instead of the code.
const SHARDS: usize = 64;

#[repr(align(128))]
struct Shard {
    allocations: AtomicU64,
    bytes: AtomicU64,
}

static COUNTERS: [Shard; SHARDS] = [const { Shard { allocations: AtomicU64::new(0), bytes: AtomicU64::new(0) } }; SHARDS];
static NEXT_SHARD: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static SHARD: Cell<usize> = const { Cell::new(usize::MAX) };
}

fn count(bytes: usize) {
    let shard = SHARD
        .try_with(|s| {
            if s.get() == usize::MAX {
                s.set(NEXT_SHARD.fetch_add(1, Ordering::Relaxed) % SHARDS);
            }
            s.get()
        })
        .unwrap_or(0);
    COUNTERS[shard].allocations.fetch_add(1, Ordering::Relaxed);
    COUNTERS[shard].bytes.fetch_add(bytes as u64, Ordering::Relaxed);
}

/// The allocations and bytes counted so far, over every shard.
fn totals() -> (u64, u64) {
    COUNTERS.iter().fold((0, 0), |(a, b), s| (a + s.allocations.load(Ordering::Relaxed), b + s.bytes.load(Ordering::Relaxed)))
}

/// Counts every allocation of the process and the bytes it asked for (a `realloc` counts as one allocation of its new
/// size).
struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

// ---------- Keys and services ----------

/// The HMAC secret of the `jwt-hs256` strategy and of `JwtService` with HS256 (at least 32 bytes).
const SECRET: &str = "corvis-auth-bench-secret-0123456789abcdef";
const RSA_PRIVATE: &str = include_str!("../../../crates/corvis/tests/fixtures/jwt_rsa_private.pem");
const RSA_PUBLIC: &str = include_str!("../../../crates/corvis/tests/fixtures/jwt_rsa_public.pem");

struct Services {
    hs256: JwtService,
    rs256: JwtService,
    /// Signs the setup tokens: a day of validity, so a long run never sees one expire.
    hs256_setup: JwtService,
    rs256_setup: JwtService,
}

static SERVICES: LazyLock<Services> = LazyLock::new(|| {
    let rs = |expires: &str| {
        JwtService::new(
            JwtModuleOptions::new("").algorithm(JwtAlgorithm::RS256).private_key(RSA_PRIVATE).public_key(RSA_PUBLIC).expires_in(expires),
        )
    };
    Services {
        hs256: JwtService::new(JwtModuleOptions::new(SECRET).expires_in("1h")),
        rs256: rs("1h"),
        hs256_setup: JwtService::new(JwtModuleOptions::new(SECRET).expires_in("1d")),
        rs256_setup: rs("1d"),
    }
});

/// What every scenario's token carries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    sub: String,
}

fn claims_of(payload: &Value) -> Option<Claims> {
    payload["sub"].as_str().map(|sub| Claims { sub: sub.to_string() })
}

// ---------- JWT strategies ----------

/// `jwt-hs256`: a static HMAC key.
#[injectable]
pub struct Hs256Auth;

#[async_trait]
impl JwtValidator for Hs256Auth {
    const NAME: &'static str = "jwt-hs256";
    type User = Claims;

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key(JwtKey::secret(SECRET))
    }

    async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Claims>, HttpException> {
        Ok(claims_of(&payload))
    }
}

/// A key provider that hands back clones of one cached RSA public key, as its rustdoc asks a provider to do.
struct CachedKey {
    key: JwtKey,
}

#[async_trait]
impl JwtKeyProvider for CachedKey {
    async fn key(&self, _req: &Request, _raw_token: &str) -> corvis::anyhow::Result<JwtKey> {
        Ok(self.key.clone())
    }
}

/// `jwt-provider`: RS256 tokens verified with the key a provider returns.
#[injectable]
pub struct ProviderAuth;

#[async_trait]
impl JwtValidator for ProviderAuth {
    const NAME: &'static str = "jwt-provider";
    type User = Claims;

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
            .secret_or_key_provider(CachedKey { key: JwtKey::rsa_pem(RSA_PUBLIC) })
    }

    async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Claims>, HttpException> {
        Ok(claims_of(&payload))
    }
}

/// A guard that verifies the bearer token with `JwtService` (RS256 module keys), the pattern of an application that
/// checks tokens without a strategy.
#[injectable]
pub struct ServiceGuard;

#[async_trait]
impl CanActivate for ServiceGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let req = ctx.switch_to_http().get_request();
        let Some(token) = extract_bearer_token(req) else { return Err(UnauthorizedException.into()) };
        let claims: Claims = SERVICES.rs256.verify_or_unauthorized(&token)?;
        req.insert(claims);
        Ok(true)
    }
}

// ---------- Routes ----------

#[controller("jwt")]
pub struct JwtController;

#[routes]
impl JwtController {
    #[get("hs256")]
    #[use_guards(AuthGuard::new("jwt-hs256"))]
    fn hs256(&self, user: User<Claims>) -> Value {
        json!({ "sub": user.sub })
    }

    #[get("provider")]
    #[use_guards(AuthGuard::new("jwt-provider"))]
    fn provider(&self, user: User<Claims>) -> Value {
        json!({ "sub": user.sub })
    }
}

#[controller("service")]
pub struct ServiceController;

#[routes]
impl ServiceController {
    #[get("verify")]
    #[use_guards(ServiceGuard)]
    fn verify(&self, req: Request) -> Value {
        json!({ "sub": req.get::<Claims>().map(|c| c.sub) })
    }

    #[post("sign")]
    fn sign(&self) -> HttpResult<Value> {
        let token = SERVICES.hs256.sign(&json!({ "sub": "7", "role": "admin" })).map_err(HttpException::from_anyhow)?;
        Ok(json!({ "ok": !token.is_empty() }))
    }

    #[post("sign-rs256")]
    fn sign_rs256(&self) -> HttpResult<Value> {
        let token = SERVICES.rs256.sign(&json!({ "sub": "7", "role": "admin" })).map_err(HttpException::from_anyhow)?;
        Ok(json!({ "ok": !token.is_empty() }))
    }
}

/// The data of the seeded session: 128 items, about 16 KiB of JSON.
fn seed_items() -> Value {
    let items: Vec<Value> =
        (0..128).map(|i| json!({ "id": i, "name": format!("item-{i:03}"), "tags": ["alpha", "beta"], "note": "n".repeat(64) })).collect();
    Value::Array(items)
}

#[controller("session")]
pub struct SessionController;

#[routes]
impl SessionController {
    /// A handler that does not touch the session.
    #[get("none")]
    fn none(&self) -> Value {
        json!({ "ok": true })
    }

    /// Reads one value of the session and changes nothing.
    #[get("read")]
    fn read(&self, req: Request) -> Value {
        let session = req.session().expect("the session module is imported");
        json!({ "n": session.get_value("n") })
    }

    /// Changes one key of the session.
    #[post("write")]
    fn write(&self, req: Request) -> Value {
        let session = req.session().expect("the session module is imported");
        let n = REQUESTS.load(Ordering::Relaxed);
        let _ = session.set("n", n);
        json!({ "ok": true })
    }

    /// Setup: fills a new session with about 16 KiB of data; the response sets its cookie.
    #[post("seed")]
    fn seed(&self, req: Request) -> Value {
        let session = req.session().expect("the session module is imported");
        session.set("items", seed_items()).expect("the items serialize");
        session.set("n", 0).expect("a number serializes");
        json!({ "ok": true })
    }
}

#[controller("__bench")]
pub struct BenchController;

#[routes]
impl BenchController {
    #[get("stats")]
    fn stats(&self) -> Value {
        let (allocations, bytes) = totals();
        json!({ "requests": REQUESTS.load(Ordering::Relaxed), "allocations": allocations, "bytes": bytes })
    }

    /// Setup: the bearer tokens of the scenarios.
    #[get("tokens")]
    fn tokens(&self) -> HttpResult<Value> {
        let payload = json!({ "sub": "7" });
        let hs256 = SERVICES.hs256_setup.sign(&payload).map_err(HttpException::from_anyhow)?;
        let rs256 = SERVICES.rs256_setup.sign(&payload).map_err(HttpException::from_anyhow)?;
        Ok(json!({ "hs256": hs256, "rs256": rs256 }))
    }
}

fn session_module() -> DynamicModule {
    SessionModule::for_root_async(|_ctx| async move {
        Ok(SessionOptions::with_secrets(["corvis-auth-bench-session-secret-0123456789"])?
            .name("sid")
            .cookie(SessionCookie::default().path("/session").secure(false))
            .max_session_bytes(64 * 1024)
            .store(MemorySessionStore::new()))
    })
}

#[module(
    imports = [session_module()],
    controllers = [JwtController, ServiceController, SessionController, BenchController],
    providers = [
        Hs256Auth,
        Provider::strategy::<JwtStrategy<Hs256Auth>>(),
        ProviderAuth,
        Provider::strategy::<JwtStrategy<ProviderAuth>>(),
        ServiceGuard,
    ],
)]
pub struct AppModule;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    corvis::logger::set_log_levels(&[corvis::logger::LogLevel::Error, corvis::logger::LogLevel::Fatal]);
    LazyLock::force(&SERVICES);

    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.silence_route_logs();
    // Every request but the benchmark's own counts.
    app.use_fn(|req: Request, next: Next| async move {
        if !req.path().starts_with("/__bench") {
            REQUESTS.fetch_add(1, Ordering::Relaxed);
        }
        next.run(req).await
    });
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3290);
    // Loopback only.
    app.listen(format!("127.0.0.1:{port}")).await
}
