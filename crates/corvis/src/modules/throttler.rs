//! Rate limiting with `ThrottlerModule.forRoot()` and `ThrottlerGuard`.
//!
//! ```ignore
//! #[module(
//!     imports = [ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 10)])],
//!     providers = [Provider::app_guard::<ThrottlerGuard>()],
//! )]
//! pub struct AppModule;
//!
//! #[get]
//! #[set_metadata(SKIP_THROTTLE, true)]   // @SkipThrottle()
//! async fn health(&self) {}
//! ```

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::common::context::{ContextType, ExecutionContext};
use crate::common::guard::CanActivate;
use crate::common::reflector::Reflector;
use crate::di::{DynamicModule, Module, ModuleDefinition, Provider, Token};
use crate::http::HttpException;
use crate::prelude::Arc;

/// `@SkipThrottle()` – `#[set_metadata(SKIP_THROTTLE, true)]`.
pub const SKIP_THROTTLE: &str = "THROTTLER:SKIP";
/// `@Throttle({ default: { limit } })` – `#[set_metadata(THROTTLE_LIMIT, 3)]`.
pub const THROTTLE_LIMIT: &str = "THROTTLER:LIMIT";
/// TTL override in milliseconds – `#[set_metadata(THROTTLE_TTL, 60_000)]`.
pub const THROTTLE_TTL: &str = "THROTTLER:TTL";

#[derive(Debug, Clone)]
pub struct ThrottlerOptions {
    pub name: String,
    /// Window in milliseconds.
    pub ttl: u64,
    /// Max requests per window.
    pub limit: u32,
}

impl ThrottlerOptions {
    pub fn new(ttl: u64, limit: u32) -> Self {
        ThrottlerOptions { name: "default".into(), ttl, limit }
    }
}

/// Default maximum number of counters kept by [`ThrottlerStorage`].
pub const DEFAULT_THROTTLER_MAX_ENTRIES: usize = 100_000;

type KeyHash = [u8; 32];

#[derive(Default)]
struct Counters {
    /// `sha256(key)` -> (hits, expiration).
    hits: HashMap<KeyHash, (u32, Instant)>,
    /// The same counters ordered by expiration, for amortized purging and eviction.
    by_expiry: BTreeSet<(Instant, KeyHash)>,
}

/// In-memory storage of hit counters.
///
/// Keys are stored as `sha256(key)`, so their size does not depend
/// on the request. Expired counters are removed on every `increment` (each one once, in
/// expiration order) and the table never holds more than `max_entries` counters: when it is
/// full, the counter closest to expiring is dropped.
pub struct ThrottlerStorage {
    pub(crate) throttlers: Vec<ThrottlerOptions>,
    counters: Mutex<Counters>,
    max_entries: usize,
    headers: bool,
}

impl ThrottlerStorage {
    pub fn new(throttlers: Vec<ThrottlerOptions>) -> Self {
        ThrottlerStorage {
            throttlers,
            counters: Mutex::new(Counters::default()),
            max_entries: DEFAULT_THROTTLER_MAX_ENTRIES,
            headers: true,
        }
    }

    /// Whether [`ThrottlerGuard`] sends the `X-RateLimit-*` and `Retry-After` headers (default `true`, as in
    /// the original throttler library). They reveal each limit, its window and the throttler names to every client; pass `false`
    /// to hide them. Counting and blocking are not affected.
    pub fn with_headers(mut self, headers: bool) -> Self {
        self.headers = headers;
        self
    }

    /// Maximum number of counters kept in memory (default [`DEFAULT_THROTTLER_MAX_ENTRIES`]).
    pub fn with_max_entries(mut self, max_entries: usize) -> Self {
        self.max_entries = max_entries.max(1);
        self
    }

    /// Number of live counters.
    pub fn len(&self) -> usize {
        self.counters.lock().unwrap_or_else(|e| e.into_inner()).hits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Approximate memory used by the counters, in bytes.
    pub fn memory_usage(&self) -> usize {
        let per_entry = std::mem::size_of::<(KeyHash, (u32, Instant))>() + std::mem::size_of::<(Instant, KeyHash)>() + 16;
        self.len() * per_entry
    }

    /// Increments the counter for `key`; returns `(hits, time_to_expire)`.
    pub fn increment(&self, key: &str, ttl: Duration) -> (u32, Duration) {
        use sha2::Digest as _;
        let hash: KeyHash = sha2::Sha256::digest(key.as_bytes()).into();
        let now = Instant::now();
        let mut guard = self.counters.lock().unwrap_or_else(|e| e.into_inner());
        let c = &mut *guard;
        while let Some(&(exp, k)) = c.by_expiry.first() {
            if exp > now {
                break;
            }
            c.by_expiry.pop_first();
            c.hits.remove(&k);
        }
        if let Some(entry) = c.hits.get_mut(&hash) {
            entry.0 = entry.0.saturating_add(1);
            return (entry.0, entry.1.saturating_duration_since(now));
        }
        let exp = now + ttl;
        while c.hits.len() >= self.max_entries {
            let Some((_, k)) = c.by_expiry.pop_first() else { break };
            c.hits.remove(&k);
        }
        c.hits.insert(hash, (1, exp));
        c.by_expiry.insert((exp, hash));
        (1, ttl)
    }
}

/// `ThrottlerModule`.
#[derive(Default)]
pub struct ThrottlerModule;
crate::default_injectable!(ThrottlerModule);

impl Module for ThrottlerModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::new::<ThrottlerModule>("ThrottlerModule").module_provider(Provider::class::<ThrottlerModule>())
    }
}

impl ThrottlerModule {
    /// `ThrottlerModule.forRoot([{ ttl: 60000, limit: 10 }])` – global.
    pub fn for_root(throttlers: Vec<ThrottlerOptions>) -> DynamicModule {
        Self::for_root_storage(ThrottlerStorage::new(throttlers))
    }

    /// Like [`for_root`](Self::for_root) with a configured storage, e.g.
    /// `ThrottlerStorage::new(vec![...]).with_max_entries(500_000)`.
    pub fn for_root_storage(storage: ThrottlerStorage) -> DynamicModule {
        DynamicModule::new::<ThrottlerModule>()
            .provider(Provider::use_value(Token::of::<ThrottlerStorage>(), storage))
            .export(Token::of::<ThrottlerStorage>())
            .global(true)
    }

    /// `ThrottlerModule.forRootAsync({ useFactory })` – global. The factory can resolve other providers and
    /// returns a `Vec<ThrottlerOptions>` or a configured [`ThrottlerStorage`].
    pub fn for_root_async<F, Fut, S>(factory: F) -> DynamicModule
    where
        F: Fn(crate::di::ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = crate::Result<S>> + Send + 'static,
        S: Into<ThrottlerStorage> + 'static,
    {
        let factory = Arc::new(factory);
        DynamicModule::new::<ThrottlerModule>()
            .provider(Provider::use_factory(Token::of::<ThrottlerStorage>(), move |ctx| {
                let factory = factory.clone();
                async move { Ok(factory(ctx).await?.into()) }
            }))
            .export(Token::of::<ThrottlerStorage>())
            .global(true)
    }
}

impl From<Vec<ThrottlerOptions>> for ThrottlerStorage {
    fn from(throttlers: Vec<ThrottlerOptions>) -> Self {
        ThrottlerStorage::new(throttlers)
    }
}

/// Rate limiting guard (keyed by client IP + handler).
#[crate::injectable]
pub struct ThrottlerGuard {
    storage: Arc<ThrottlerStorage>,
    reflector: Arc<Reflector>,
}

impl ThrottlerGuard {
    /// Sets the rate limit headers of one throttler, as the original throttler library does: `Retry-After` when it blocks the
    /// request (`time_to_block_expire`), otherwise `X-RateLimit-Limit`, `-Remaining` and `-Reset`; names carry a
    /// `-<name>` suffix unless the throttler is `default`. Values are seconds, rounded up. Nothing is set outside
    /// HTTP or when the storage has headers turned off. A throttler name that is not a valid header token makes
    /// the header be dropped.
    fn set_headers(&self, ctx: &ExecutionContext, name: &str, limit: u32, hits: u32, time_to_block_expire: Duration, blocked: bool) {
        if !self.storage.headers || ctx.get_type() != ContextType::Http {
            return;
        }
        let suffix = if name == "default" { String::new() } else { format!("-{name}") };
        let seconds = time_to_block_expire.as_millis().div_ceil(1000);
        let http = ctx.switch_to_http();
        if blocked {
            http.set_response_header(&format!("Retry-After{suffix}"), &seconds.max(1).to_string());
        } else {
            http.set_response_header(&format!("X-RateLimit-Limit{suffix}"), &limit.to_string());
            http.set_response_header(&format!("X-RateLimit-Remaining{suffix}"), &limit.saturating_sub(hits).to_string());
            http.set_response_header(&format!("X-RateLimit-Reset{suffix}"), &seconds.to_string());
        }
    }
}

#[async_trait]
impl CanActivate for ThrottlerGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let targets = [ctx.get_handler(), ctx.get_class()];
        if self.reflector.get_all_and_override::<bool>(SKIP_THROTTLE, &targets).unwrap_or(false) {
            return Ok(true);
        }
        let req = ctx.request();
        for t in &self.storage.throttlers {
            let limit = self.reflector.get_all_and_override::<u32>(THROTTLE_LIMIT, &targets).unwrap_or(t.limit);
            let ttl = self.reflector.get_all_and_override::<u64>(THROTTLE_TTL, &targets).unwrap_or(t.ttl);
            let key = format!("{}:{}:{}:{}", t.name, req.ip(), ctx.get_class().name, ctx.get_handler().name);
            let (hits, time_to_expire) = self.storage.increment(&key, Duration::from_millis(ttl));
            let blocked = hits > limit;
            self.set_headers(ctx, &t.name, limit, hits, time_to_expire, blocked);
            if blocked {
                return Err(HttpException::new("ThrottlerException: Too Many Requests", 429));
            }
        }
        Ok(true)
    }

    fn controls_access(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttler_guard_does_not_control_access() {
        let guard: Arc<dyn CanActivate> = Arc::new(ThrottlerGuard {
            storage: Arc::new(ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 3)])),
            reflector: Arc::new(Reflector),
        });
        assert!(!guard.controls_access());
        assert!(guard.guard_name().ends_with("ThrottlerGuard"));
    }

    /// A gateway message is counted but gets no headers.
    #[tokio::test]
    async fn websocket_context_records_no_headers() {
        use crate::common::context::{EnhancerHeaders, Target};
        use crate::http::Request;
        let guard = ThrottlerGuard {
            storage: Arc::new(ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 1)])),
            reflector: Arc::new(Reflector),
        };
        let request = Request::new(http::Method::GET, "/".parse().unwrap(), http::HeaderMap::new(), bytes::Bytes::new()).unwrap();
        let ctx = ExecutionContext::for_ws(
            request.clone(),
            Arc::new(Target::new("Gw")),
            Arc::new(Target::new("on_message")),
            None,
            Arc::new(()),
            serde_json::json!({}),
        );
        assert!(guard.can_activate(&ctx).await.unwrap());
        assert!(EnhancerHeaders::of(&request).is_none());
        assert!(!ctx.switch_to_http().set_response_header("X-Trace", "abc"));
        assert_eq!(guard.can_activate(&ctx).await.unwrap_err().status(), 429, "still counted and blocked");
        assert!(EnhancerHeaders::of(&request).is_none());
    }

    /// Keys are stored hashed (fixed size), expired counters are purged without waiting for
    /// 10 000 entries, and the table never grows past `max_entries`.
    #[test]
    fn storage_memory_is_bounded() {
        let storage = ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 3)]).with_max_entries(100);
        let big = "A".repeat(400_000);
        for i in 0..50 {
            storage.increment(&format!("default:{i}-{big}:C:h"), Duration::from_millis(20));
        }
        assert!(storage.memory_usage() < 50 * 1024, "{} bytes", storage.memory_usage());
        std::thread::sleep(Duration::from_millis(40));
        storage.increment("default:fresh:C:h", Duration::from_secs(60));
        assert_eq!(storage.len(), 1, "expired counters must be purged");
        for i in 0..1_000 {
            storage.increment(&format!("k{i}"), Duration::from_secs(60));
        }
        assert!(storage.len() <= 100, "{}", storage.len());
    }

    #[test]
    fn counts_per_key_and_resets_after_ttl() {
        let storage = ThrottlerStorage::new(vec![]);
        assert_eq!(storage.increment("a", Duration::from_millis(30)).0, 1);
        assert_eq!(storage.increment("a", Duration::from_millis(30)).0, 2);
        assert_eq!(storage.increment("b", Duration::from_millis(30)).0, 1);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(storage.increment("a", Duration::from_millis(30)).0, 1);
    }
}
