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

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::common::context::ExecutionContext;
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

/// In-memory storage of hit counters.
pub struct ThrottlerStorage {
    pub(crate) throttlers: Vec<ThrottlerOptions>,
    hits: Mutex<HashMap<String, (u32, Instant)>>,
}

impl ThrottlerStorage {
    pub fn new(throttlers: Vec<ThrottlerOptions>) -> Self {
        ThrottlerStorage { throttlers, hits: Mutex::new(HashMap::new()) }
    }

    /// Increments the counter for `key`; returns `(hits, time_to_expire)`.
    pub fn increment(&self, key: &str, ttl: Duration) -> (u32, Duration) {
        let mut map = self.hits.lock().unwrap();
        let now = Instant::now();
        let entry = map.entry(key.to_string()).or_insert((0, now + ttl));
        if entry.1 <= now {
            *entry = (0, now + ttl);
        }
        entry.0 += 1;
        if map.len() > 10_000 {
            map.retain(|_, (_, exp)| *exp > now);
        }
        let e = map.get(key).copied().unwrap_or((1, now + ttl));
        (e.0, e.1.saturating_duration_since(now))
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
        DynamicModule::new::<ThrottlerModule>()
            .provider(Provider::use_value(Token::of::<ThrottlerStorage>(), ThrottlerStorage::new(throttlers)))
            .export(Token::of::<ThrottlerStorage>())
            .global(true)
    }
}

/// Rate limiting guard (keyed by client IP + handler).
#[crate::injectable]
pub struct ThrottlerGuard {
    storage: Arc<ThrottlerStorage>,
    reflector: Arc<Reflector>,
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
            let (hits, _) = self.storage.increment(&key, Duration::from_millis(ttl));
            if hits > limit {
                return Err(HttpException::new("ThrottlerException: Too Many Requests", 429));
            }
        }
        Ok(true)
    }
}

