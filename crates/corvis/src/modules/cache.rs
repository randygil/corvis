//! Caching: `CacheModule.register()`, the `CacheManager`
//! (`@Inject(CACHE_MANAGER)`) and the `CacheInterceptor`.
//!
//! ```ignore
//! #[module(imports = [CacheModule::register(CacheOptions::default())])]
//! pub struct AppModule;
//!
//! #[get]
//! #[use_interceptors(CacheInterceptor)]
//! #[set_metadata(CACHE_TTL_METADATA, 10_000)]
//! async fn find_all(&self) -> Vec<Cat> { ... }
//! ```

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::common::context::ExecutionContext;
use crate::common::interceptor::{CallHandler, CorvisInterceptor};
use crate::common::reflector::Reflector;
use crate::di::{DynamicModule, Module, ModuleDefinition, Provider, Token};
use crate::http::{Body, HandlerResult, Response};
use crate::prelude::Arc;

/// `CACHE_MANAGER` token (the manager is also injectable as `Arc<CacheManager>`).
pub const CACHE_MANAGER: &str = "CACHE_MANAGER";
/// Metadata key overriding the cache key of a route.
pub const CACHE_KEY_METADATA: &str = "cache_module:cache_key";
/// Metadata key overriding the TTL (milliseconds) of a route.
pub const CACHE_TTL_METADATA: &str = "cache_module:cache_ttl";

#[derive(Debug, Clone)]
pub struct CacheOptions {
    /// Default time-to-live in milliseconds (`0` = never expires).
    pub ttl: u64,
    /// Maximum number of items.
    pub max: usize,
    pub is_global: bool,
}

impl Default for CacheOptions {
    fn default() -> Self {
        CacheOptions { ttl: 5_000, max: 100, is_global: false }
    }
}

struct Entry {
    value: Value,
    expires: Option<Instant>,
}

/// In-memory cache store.
pub struct CacheManager {
    options: CacheOptions,
    entries: Mutex<(HashMap<String, Entry>, VecDeque<String>)>,
}

impl CacheManager {
    pub fn new(options: CacheOptions) -> Self {
        CacheManager { options, entries: Mutex::new((HashMap::new(), VecDeque::new())) }
    }

    /// `await cacheManager.get(key)`.
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.get_value(key).and_then(|v| serde_json::from_value(v).ok())
    }

    pub fn get_value(&self, key: &str) -> Option<Value> {
        let mut guard = self.entries.lock().unwrap();
        let (map, _) = &mut *guard;
        match map.get(key) {
            Some(e) if e.expires.map(|x| x > Instant::now()).unwrap_or(true) => Some(e.value.clone()),
            Some(_) => {
                map.remove(key);
                None
            }
            None => None,
        }
    }

    /// `await cacheManager.set(key, value, ttl)` – `ttl` in milliseconds (`None` = default).
    pub async fn set(&self, key: &str, value: impl Serialize, ttl: Option<u64>) {
        let v = serde_json::to_value(value).unwrap_or(Value::Null);
        self.set_value(key, v, ttl);
    }

    pub fn set_value(&self, key: &str, value: Value, ttl: Option<u64>) {
        let ttl = ttl.unwrap_or(self.options.ttl);
        let expires = (ttl > 0).then(|| Instant::now() + Duration::from_millis(ttl));
        let mut guard = self.entries.lock().unwrap();
        let (map, order) = &mut *guard;
        if !map.contains_key(key) {
            order.push_back(key.to_string());
        }
        map.insert(key.to_string(), Entry { value, expires });
        while map.len() > self.options.max.max(1) {
            match order.pop_front() {
                Some(old) => {
                    map.remove(&old);
                }
                None => break,
            }
        }
    }

    /// `await cacheManager.del(key)`.
    pub async fn del(&self, key: &str) {
        let mut guard = self.entries.lock().unwrap();
        guard.0.remove(key);
        guard.1.retain(|k| k != key);
    }

    /// `await cacheManager.clear()`.
    pub async fn clear(&self) {
        let mut guard = self.entries.lock().unwrap();
        guard.0.clear();
        guard.1.clear();
    }

    /// Returns the cached value or computes and stores it.
    pub async fn wrap<T, F, Fut>(&self, key: &str, ttl: Option<u64>, f: F) -> T
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        if let Some(v) = self.get::<T>(key).await {
            return v;
        }
        let v = f().await;
        self.set(key, &v, ttl).await;
        v
    }
}

/// `CacheModule`.
#[derive(Default)]
pub struct CacheModule;
crate::default_injectable!(CacheModule);

impl Module for CacheModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::new::<CacheModule>("CacheModule").module_provider(Provider::class::<CacheModule>())
    }
}

impl CacheModule {
    /// `CacheModule.register({ ttl, max, isGlobal })`.
    pub fn register(options: CacheOptions) -> DynamicModule {
        let global = options.is_global;
        let manager = Arc::new(CacheManager::new(options));
        DynamicModule::new::<CacheModule>()
            .provider(Provider::use_value_arc(Token::of::<CacheManager>(), manager.clone()))
            .provider(Provider::use_value_arc(CACHE_MANAGER, manager))
            .export(Token::of::<CacheManager>())
            .export(CACHE_MANAGER)
            .global(global)
    }
}

/// Caches `GET` responses (keyed by URL, or by `CACHE_KEY_METADATA`).
#[crate::injectable]
pub struct CacheInterceptor {
    cache: Arc<CacheManager>,
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CorvisInterceptor for CacheInterceptor {
    async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        let req = ctx.request();
        if req.method() != http::Method::GET {
            return next.handle().await;
        }
        let targets = [ctx.get_handler(), ctx.get_class()];
        let key: String = self.reflector.get_all_and_override(CACHE_KEY_METADATA, &targets).unwrap_or_else(|| req.url());
        let ttl: Option<u64> = self.reflector.get_all_and_override(CACHE_TTL_METADATA, &targets);
        if let Some(cached) = self.cache.get_value(&key) {
            let mut r = match cached {
                Value::Object(mut o) if o.contains_key("__corvis_text") => {
                    Response::text(o.remove("__corvis_text").and_then(|v| v.as_str().map(String::from)).unwrap_or_default())
                }
                v => Response { body: Body::Json(v), ..Default::default() },
            };
            r.set_status(200);
            return Ok(r);
        }
        let res = next.handle().await?;
        let ok = res.status().map(|s| s.is_success()).unwrap_or(true);
        if ok {
            match res.body() {
                Body::Json(v) => self.cache.set_value(&key, v.clone(), ttl),
                Body::Text(t) | Body::Html(t) => self.cache.set_value(&key, serde_json::json!({ "__corvis_text": t }), ttl),
                _ => {}
            }
        }
        Ok(res)
    }
}
