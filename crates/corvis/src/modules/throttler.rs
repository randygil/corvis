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

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::common::context::{ContextType, ExecutionContext};
use crate::common::guard::CanActivate;
use crate::common::reflector::Reflector;
use crate::di::{DynamicModule, Module, ModuleDefinition, Provider, Token};
use crate::http::HttpException;
use crate::logger::Logger;
use crate::prelude::Arc;

/// `@SkipThrottle()` – `#[set_metadata(SKIP_THROTTLE, true)]`.
pub const SKIP_THROTTLE: &str = "THROTTLER:SKIP";
/// `@Throttle({ default: { limit } })` – `#[set_metadata(THROTTLE_LIMIT, 3)]`.
pub const THROTTLE_LIMIT: &str = "THROTTLER:LIMIT";
/// TTL override in milliseconds – `#[set_metadata(THROTTLE_TTL, 60_000)]`.
pub const THROTTLE_TTL: &str = "THROTTLER:TTL";
/// Block duration override in milliseconds, for every throttler – `#[set_metadata(THROTTLE_BLOCK_DURATION, 60_000)]`.
/// `0` turns the block off.
pub const THROTTLE_BLOCK_DURATION: &str = "THROTTLER:BLOCK_DURATION";

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

/// Default maximum number of keys kept by [`ThrottlerStorage`].
pub const DEFAULT_THROTTLER_MAX_ENTRIES: usize = 100_000;

/// Most buckets a key keeps. Hits of one key are grouped in buckets of `ttl / BUCKETS`, so a key's memory does not
/// depend on its limit or on how many requests it sends.
const BUCKETS: usize = 64;

/// Longest duration added to a clock reading; keeps `Instant` arithmetic from overflowing on absurd values.
const MAX_SPAN: Duration = Duration::from_secs(10 * 365 * 24 * 3600);

type KeyHash = [u8; 32];

/// What [`ThrottlerStorage::record_hit`] reports for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ThrottlerRecord {
    /// Hits of the key that have not expired, including this one when it was counted.
    pub total_hits: u32,
    /// Time until the key's reporting window ends (feeds `X-RateLimit-Reset`).
    pub time_to_expire: Duration,
    /// Whether the key is blocked, so the request must be rejected.
    pub is_blocked: bool,
    /// Time until the key is let in again; zero when it is not blocked.
    pub time_to_block_expire: Duration,
}

/// Hits of one key that expire within the same `ttl / BUCKETS` slice.
#[derive(Clone, Copy)]
struct Bucket {
    /// When the newest hit of the bucket expires: all its hits are counted until then.
    expiry: Instant,
    count: u32,
}

/// The state of one key.
struct Record {
    /// Counted hits, grouped by expiry, in ascending order; at most `BUCKETS`.
    buckets: VecDeque<Bucket>,
    /// Sum of the bucket counts.
    hits: u64,
    /// Reference point for the bucket slices.
    origin: Instant,
    /// End of the reporting window.
    window_end: Instant,
}

/// A blocked key: no buckets, only what is needed to answer during the block and to resume after it.
#[derive(Clone, Copy)]
struct Blocked {
    /// End of the block.
    until: Instant,
    /// End of the reporting window, kept so `X-RateLimit-Reset` is the same as without the separate table.
    window_end: Instant,
}

impl Blocked {
    fn idle_at(&self) -> Instant {
        self.until.max(self.window_end)
    }
}

impl Record {
    fn new(now: Instant, window_end: Instant) -> Self {
        Record { buckets: VecDeque::new(), hits: 0, origin: now, window_end }
    }

    /// The instant after which the record holds nothing that matters.
    fn idle_at(&self) -> Instant {
        match self.buckets.back() {
            Some(last) => self.window_end.max(last.expiry),
            None => self.window_end,
        }
    }

    fn prune(&mut self, now: Instant) {
        while let Some(front) = self.buckets.front() {
            if front.expiry > now {
                break;
            }
            self.hits -= u64::from(front.count);
            self.buckets.pop_front();
        }
    }

    /// Counts a hit that expires at `expiry`. Hits whose expiries fall in the same slice share a bucket, which then
    /// lives until the newest of them expires (never shorter than the exact count: it fails closed).
    fn add_hit(&mut self, expiry: Instant, slice: u128) {
        self.hits += 1;
        let index = |e: Instant| e.saturating_duration_since(self.origin).as_nanos() / slice;
        let at = self.buckets.partition_point(|b| b.expiry < expiry);
        if at > 0 && index(self.buckets[at - 1].expiry) == index(expiry) {
            let b = &mut self.buckets[at - 1];
            b.expiry = expiry;
            b.count = b.count.saturating_add(1);
        } else if at < self.buckets.len() && (index(self.buckets[at].expiry) == index(expiry) || self.buckets.len() >= BUCKETS) {
            let b = &mut self.buckets[at];
            b.count = b.count.saturating_add(1);
        } else if self.buckets.len() >= BUCKETS {
            let b = self.buckets.back_mut().expect("buckets are full");
            b.expiry = b.expiry.max(expiry);
            b.count = b.count.saturating_add(1);
        } else {
            self.buckets.insert(at, Bucket { expiry, count: 1 });
        }
    }
}

#[derive(Default)]
struct Counters {
    /// `sha256(key)` -> record.
    records: HashMap<KeyHash, Record>,
    /// The same keys ordered by idle time, for amortized purging and eviction.
    idle: BTreeSet<(Instant, KeyHash)>,
    /// Blocked keys. A separate, compact table with its own cap, so a flood of blocked keys cannot displace counters.
    blocked: HashMap<KeyHash, Blocked>,
    /// Blocked keys ordered by when they become idle.
    blocked_idle: BTreeSet<(Instant, KeyHash)>,
}

impl Counters {
    fn remove(&mut self, hash: &KeyHash) -> Option<Record> {
        let record = self.records.remove(hash)?;
        self.idle.remove(&(record.idle_at(), *hash));
        Some(record)
    }

    /// Drops the counter that becomes idle soonest. Returns false when the table is empty.
    fn evict_one(&mut self) -> bool {
        let Some((_, hash)) = self.idle.pop_first() else { return false };
        self.records.remove(&hash);
        true
    }

    /// Drops every counter and every block that is idle at `now`.
    fn purge(&mut self, now: Instant) {
        while let Some(&(idle, hash)) = self.idle.first() {
            if idle > now {
                break;
            }
            self.idle.pop_first();
            self.records.remove(&hash);
        }
        while let Some(&(idle, hash)) = self.blocked_idle.first() {
            if idle > now {
                break;
            }
            self.blocked_idle.pop_first();
            self.blocked.remove(&hash);
        }
    }

    fn insert(&mut self, hash: KeyHash, record: Record) {
        self.idle.insert((record.idle_at(), hash));
        self.records.insert(hash, record);
    }

    fn remove_blocked(&mut self, hash: &KeyHash) -> Option<Blocked> {
        let b = self.blocked.remove(hash)?;
        self.blocked_idle.remove(&(b.idle_at(), *hash));
        Some(b)
    }

    /// Stores a block, first dropping the blocks that end soonest while the table is full (those are keys the
    /// attacker pinned, not counters of other clients).
    fn insert_blocked(&mut self, hash: KeyHash, b: Blocked, max: usize) {
        while self.blocked.len() >= max {
            let Some((_, h)) = self.blocked_idle.pop_first() else { break };
            self.blocked.remove(&h);
        }
        self.blocked_idle.insert((b.idle_at(), hash));
        self.blocked.insert(hash, b);
    }
}

/// `now + span`, saturating to a far instant instead of panicking or going back to `now`: a block that cannot be
/// represented must last, not vanish.
fn after(now: Instant, span: Duration) -> Instant {
    now.checked_add(span.min(MAX_SPAN))
        .or_else(|| [365 * 24 * 3600, 30 * 24 * 3600, 24 * 3600, 3600].into_iter().find_map(|s| now.checked_add(Duration::from_secs(s))))
        .unwrap_or(now)
}

/// In-memory storage of the hits of each key, counted like the original throttler library: every hit expires `ttl`
/// after it was counted (a sliding window), and a key that goes over the limit is blocked for the block duration.
///
/// Keys are stored as `sha256(key)`, so their size does not depend on the request. Each key holds at most 64 buckets
/// of hits, whatever its limit: hits whose expiries fall in the same `ttl / 64` slice share a bucket and expire
/// together, so `X-RateLimit-Remaining` and `Reset` can differ from the original library by up to `ttl / 64` when a
/// client sends hits closer together than that.
///
/// Blocked keys live in a separate compact table (no buckets: their hits are dropped when the block starts) with its
/// own cap, so a flood of blocked keys cannot displace the counters of other clients. Memory is bounded by
/// `max_entries` counters plus `max_blocked` blocks: idle entries are removed on every call, and when a table is full
/// the entries that become idle soonest are dropped.
pub struct ThrottlerStorage {
    pub(crate) throttlers: Vec<ThrottlerOptions>,
    pub(crate) block_durations: HashMap<String, u64>,
    counters: Mutex<Counters>,
    max_entries: usize,
    max_blocked: Option<usize>,
    headers: bool,
}

impl ThrottlerStorage {
    /// Builds the storage. The throttlers are checked in ascending `ttl` order (ties keep the given order), as the
    /// original throttler library does. Two throttlers with the same name share their counters and block durations,
    /// so a warning is logged.
    pub fn new(mut throttlers: Vec<ThrottlerOptions>) -> Self {
        throttlers.sort_by_key(|t| t.ttl);
        let mut seen = std::collections::HashSet::new();
        for t in &throttlers {
            if !seen.insert(t.name.as_str()) {
                Logger::new("ThrottlerModule").warn(format!("more than one throttler is named `{}`; give each a distinct name", t.name));
            }
        }
        ThrottlerStorage {
            throttlers,
            block_durations: HashMap::new(),
            counters: Mutex::new(Counters::default()),
            max_entries: DEFAULT_THROTTLER_MAX_ENTRIES,
            max_blocked: None,
            headers: true,
        }
    }

    /// Maximum number of blocked keys kept in memory (default: the value of [`with_max_entries`](Self::with_max_entries)).
    /// A blocked key takes about 120 bytes. When the table is full the block that ends soonest is dropped, which lets
    /// that key start counting again; counters are not affected.
    pub fn with_max_blocked(mut self, max_blocked: usize) -> Self {
        self.max_blocked = Some(max_blocked.max(1));
        self
    }

    /// Whether [`ThrottlerGuard`] sends the `X-RateLimit-*` and `Retry-After` headers (default `true`, as in
    /// the original throttler library). They reveal each limit, its window and the throttler names to every client; pass `false`
    /// to hide them. Counting and blocking are not affected.
    pub fn with_headers(mut self, headers: bool) -> Self {
        self.headers = headers;
        self
    }

    /// Maximum number of keys kept in memory (default [`DEFAULT_THROTTLER_MAX_ENTRIES`]). Each key takes at most
    /// 64 buckets of 24 bytes plus a fixed overhead, whatever the limit.
    pub fn with_max_entries(mut self, max_entries: usize) -> Self {
        self.max_entries = max_entries.max(1);
        self
    }

    /// How long the throttler called `name` blocks a key that goes over its limit, in milliseconds (default: the
    /// throttler's `ttl`, as in the original). `0` turns the block off: requests are rejected while the key holds
    /// `limit` unexpired hits. A route's `THROTTLE_BLOCK_DURATION` metadata takes precedence.
    ///
    /// # Panics
    ///
    /// When no configured throttler is called `name`.
    pub fn with_block_duration(mut self, name: &str, block_duration: u64) -> Self {
        assert!(self.throttlers.iter().any(|t| t.name == name), "ThrottlerStorage::with_block_duration: no throttler is named `{name}`");
        self.block_durations.insert(name.to_string(), block_duration);
        self
    }

    /// Number of live keys, counters and blocked keys.
    pub fn len(&self) -> usize {
        let c = self.counters.lock().unwrap_or_else(|e| e.into_inner());
        c.records.len() + c.blocked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Approximate memory used by the keys, their buckets and the blocked keys, in bytes.
    pub fn memory_usage(&self) -> usize {
        let per_entry = std::mem::size_of::<(KeyHash, Record)>() + std::mem::size_of::<(Instant, KeyHash)>() + 16;
        let per_blocked = std::mem::size_of::<(KeyHash, Blocked)>() + std::mem::size_of::<(Instant, KeyHash)>() + 16;
        let c = self.counters.lock().unwrap_or_else(|e| e.into_inner());
        c.records.len() * per_entry
            + c.records.values().map(|r| r.buckets.capacity() * std::mem::size_of::<Bucket>()).sum::<usize>()
            + c.blocked.len() * per_blocked
    }

    /// Counts a hit for `key` in a sliding window and returns `(hits, time_to_expire)`. Equivalent to
    /// [`record_hit`](Self::record_hit) with no limit.
    pub fn increment(&self, key: &str, ttl: Duration) -> (u32, Duration) {
        let r = self.record_hit(key, ttl, u32::MAX, ttl);
        (r.total_hits, r.time_to_expire)
    }

    /// Counts a hit for `key` as the original throttler library does. Each hit expires `ttl` after it was counted;
    /// the hit that takes the count over `limit` blocks the key for `block_duration` (requests during the block are
    /// not counted and do not extend it; the first one after it starts from one hit). With a zero `block_duration`
    /// nothing is blocked: the request is rejected while the key holds `limit` unexpired hits and is not counted.
    /// A `limit` of 0 rejects every request. Hits closer together than `ttl / 64` share a bucket (see
    /// [`ThrottlerStorage`]).
    pub fn record_hit(&self, key: &str, ttl: Duration, limit: u32, block_duration: Duration) -> ThrottlerRecord {
        self.record_hit_at(key, Instant::now(), ttl, limit, block_duration)
    }

    fn record_hit_at(&self, key: &str, now: Instant, ttl: Duration, limit: u32, block_duration: Duration) -> ThrottlerRecord {
        use sha2::Digest as _;
        let hash: KeyHash = sha2::Sha256::digest(key.as_bytes()).into();
        let slice = (ttl.as_nanos() / BUCKETS as u128).max(1);
        let mut guard = self.counters.lock().unwrap_or_else(|e| e.into_inner());
        let c = &mut *guard;

        // Idle keys behave like fresh ones, so removing them is not observable.
        c.purge(now);

        let max_blocked = self.max_blocked.unwrap_or(self.max_entries);

        // A blocked key is answered from the compact table; when its block has ended it resumes as a fresh counter that
        // keeps the reporting window.
        let mut resumed = None;
        let blocked = if block_duration.is_zero() { None } else { c.remove_blocked(&hash) };
        if let Some(mut b) = blocked {
            if b.window_end <= now {
                b.window_end = after(now, ttl);
            }
            if b.until > now {
                c.insert_blocked(hash, b, max_blocked);
                return ThrottlerRecord {
                    total_hits: limit.saturating_add(1),
                    time_to_expire: b.window_end.saturating_duration_since(now),
                    is_blocked: true,
                    time_to_block_expire: b.until.saturating_duration_since(now),
                };
            }
            resumed = Some(Record::new(now, b.window_end));
        }

        let mut rec = resumed.or_else(|| c.remove(&hash)).unwrap_or_else(|| Record::new(now, after(now, ttl)));
        rec.prune(now);
        if rec.window_end <= now {
            rec.window_end = after(now, ttl);
        }
        let time_to_expire = rec.window_end.saturating_duration_since(now);

        let (is_blocked, time_to_block_expire);
        if block_duration.is_zero() {
            let full = rec.hits >= u64::from(limit);
            if !full {
                rec.add_hit(after(now, ttl), slice);
            }
            is_blocked = full;
            time_to_block_expire = match (full, rec.buckets.front()) {
                (false, _) => Duration::ZERO,
                (true, Some(oldest)) => oldest.expiry.saturating_duration_since(now),
                (true, None) => time_to_expire,
            };
        } else {
            rec.add_hit(after(now, ttl), slice);
            if rec.hits > u64::from(limit) {
                // The hits are not used during the block and are discarded when it ends: only the block is kept.
                let until = after(now, block_duration);
                let total_hits = u32::try_from(rec.hits).unwrap_or(u32::MAX);
                c.insert_blocked(hash, Blocked { until, window_end: rec.window_end }, max_blocked);
                return ThrottlerRecord {
                    total_hits,
                    time_to_expire,
                    is_blocked: true,
                    time_to_block_expire: until.saturating_duration_since(now),
                };
            }
            is_blocked = false;
            time_to_block_expire = Duration::ZERO;
        }
        let total_hits = u32::try_from(rec.hits + u64::from(is_blocked)).unwrap_or(u32::MAX);

        while c.records.len() >= self.max_entries {
            if !c.evict_one() {
                break;
            }
        }
        c.insert(hash, rec);
        ThrottlerRecord { total_hits, time_to_expire, is_blocked, time_to_block_expire }
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
    /// request (`time_to_block_expire`, or the time until its oldest hit expires with no block), otherwise `X-RateLimit-Limit`, `-Remaining` and `-Reset`; names carry a
    /// `-<name>` suffix unless the throttler is `default`. Values are seconds, rounded up. Nothing is set outside
    /// HTTP or when the storage has headers turned off. A throttler name that is not a valid header token makes
    /// the header be dropped.
    fn set_headers(&self, ctx: &ExecutionContext, name: &str, limit: u32, record: &ThrottlerRecord) {
        if !self.storage.headers || ctx.get_type() != ContextType::Http {
            return;
        }
        let suffix = if name == "default" { String::new() } else { format!("-{name}") };
        let seconds = |d: Duration| d.as_millis().div_ceil(1000);
        let http = ctx.switch_to_http();
        if record.is_blocked {
            http.set_response_header(&format!("Retry-After{suffix}"), &seconds(record.time_to_block_expire).max(1).to_string());
        } else {
            http.set_response_header(&format!("X-RateLimit-Limit{suffix}"), &limit.to_string());
            http.set_response_header(&format!("X-RateLimit-Remaining{suffix}"), &limit.saturating_sub(record.total_hits).to_string());
            http.set_response_header(&format!("X-RateLimit-Reset{suffix}"), &seconds(record.time_to_expire).to_string());
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
            let block_duration = self
                .reflector
                .get_all_and_override::<u64>(THROTTLE_BLOCK_DURATION, &targets)
                .or_else(|| self.storage.block_durations.get(&t.name).copied())
                .unwrap_or(ttl);
            let record = self.storage.record_hit(&key, Duration::from_millis(ttl), limit, Duration::from_millis(block_duration));
            self.set_headers(ctx, &t.name, limit, &record);
            if record.is_blocked {
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

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Calls `record_hit_at` at `base + t` milliseconds.
    fn hit(storage: &ThrottlerStorage, base: Instant, t: u64, key: &str, ttl: u64, limit: u32, block: u64) -> ThrottlerRecord {
        storage.record_hit_at(key, base + ms(t), ms(ttl), limit, ms(block))
    }

    fn secs(d: Duration) -> i64 {
        d.as_millis().div_ceil(1000) as i64
    }

    /// Replays the calls recorded from the original throttler library's `ThrottlerStorageService.increment`
    /// (`tests/fixtures/throttler/storage.mjs`) with the same clock and compares every field.
    #[test]
    fn replays_the_original_storage_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!("../../tests/fixtures/throttler/storage.json")).unwrap();
        let base = Instant::now();
        for scenario in fixture.as_array().unwrap() {
            let name = scenario["name"].as_str().unwrap();
            let storage = ThrottlerStorage::new(vec![]);
            let calls = scenario["calls"].as_array().unwrap();
            // Hits spaced more than ttl / 64 apart are counted exactly; closer ones share a bucket and only the blocked
            // state and the times (within ttl / 64) are compared.
            let mut last: HashMap<&str, u64> = HashMap::new();
            let mut exact = true;
            for call in calls {
                let (key, t, ttl) = (call["key"].as_str().unwrap(), call["t"].as_u64().unwrap(), call["ttl"].as_u64().unwrap());
                if let Some(prev) = last.insert(key, t) {
                    exact &= t - prev > ttl / 64;
                }
            }
            assert_eq!(exact, name != "dense_hits", "{name}: the fixture mixes exact and bucketed cases");
            for (i, call) in calls.iter().enumerate() {
                let n = |k: &str| call[k].as_u64().unwrap();
                let r = hit(&storage, base, n("t"), call["key"].as_str().unwrap(), n("ttl"), n("limit") as u32, n("blockDuration"));
                let want = &call["result"];
                let at = format!("{name} #{i} t={}", n("t"));
                let tolerance = if exact { 0 } else { (n("ttl") / 64).div_ceil(1000) as i64 + 1 };
                let close = |got: i64, want: i64| (got - want).abs() <= tolerance;
                // A blocked key drops its hits, so the count of a blocked response is not compared (the guard ignores it).
                if exact && !r.is_blocked {
                    assert_eq!(r.total_hits as u64, want["totalHits"].as_u64().unwrap(), "{at}: total_hits");
                }
                assert_eq!(r.is_blocked, want["isBlocked"].as_bool().unwrap(), "{at}: is_blocked");
                assert!(close(secs(r.time_to_expire), want["timeToExpire"].as_i64().unwrap()), "{at}: time_to_expire");
                let want_block = want["timeToBlockExpire"].as_i64().unwrap().max(0);
                assert!(close(secs(r.time_to_block_expire), want_block), "{at}: time_to_block_expire");
            }
        }
    }

    #[test]
    fn no_double_burst_around_a_window_boundary() {
        let storage = ThrottlerStorage::new(vec![]);
        let base = Instant::now();
        let out: Vec<bool> =
            [0, 9_500, 9_600, 10_100, 10_200].iter().map(|&t| hit(&storage, base, t, "k", 10_000, 3, 10_000).is_blocked).collect();
        assert_eq!(out, [false, false, false, false, true]);
    }

    #[test]
    fn rejected_requests_are_not_counted_and_the_block_ends_on_time() {
        let storage = ThrottlerStorage::new(vec![]);
        let base = Instant::now();
        hit(&storage, base, 0, "k", 60_000, 2, 60_000);
        hit(&storage, base, 1_000, "k", 60_000, 2, 60_000);
        let r = hit(&storage, base, 2_000, "k", 60_000, 2, 60_000);
        assert!(r.is_blocked);
        assert_eq!(secs(r.time_to_block_expire), 60);
        let r = hit(&storage, base, 32_000, "k", 60_000, 2, 60_000);
        assert!(r.is_blocked, "retries do not extend the block");
        assert_eq!(secs(r.time_to_block_expire), 30);
        let r = hit(&storage, base, 63_000, "k", 60_000, 2, 60_000);
        assert!(!r.is_blocked);
        assert_eq!(r.total_hits, 1);
        assert_eq!(secs(r.time_to_expire), 60);
    }

    #[test]
    fn block_longer_than_ttl() {
        let storage = ThrottlerStorage::new(vec![]);
        let base = Instant::now();
        for t in [0, 1_000, 2_000] {
            hit(&storage, base, t, "k", 60_000, 2, 300_000);
        }
        let r = hit(&storage, base, 63_000, "k", 60_000, 2, 300_000);
        assert!(r.is_blocked);
        assert_eq!(secs(r.time_to_block_expire), 239);
    }

    #[test]
    fn block_duration_zero_is_a_sliding_limit() {
        let storage = ThrottlerStorage::new(vec![]);
        let base = Instant::now();
        assert!(!hit(&storage, base, 0, "k", 10_000, 2, 0).is_blocked);
        assert!(!hit(&storage, base, 1_000, "k", 10_000, 2, 0).is_blocked);
        let r = hit(&storage, base, 2_000, "k", 10_000, 2, 0);
        assert!(r.is_blocked);
        assert_eq!(secs(r.time_to_block_expire), 8);
        let r = hit(&storage, base, 10_500, "k", 10_000, 2, 0);
        assert!(!r.is_blocked);
        assert_eq!(r.total_hits, 2);
    }

    #[test]
    fn a_limit_of_zero_rejects_every_request_even_after_a_block() {
        let storage = ThrottlerStorage::new(vec![]);
        let base = Instant::now();
        for t in [0, 500, 1_500, 3_000, 10_000] {
            assert!(hit(&storage, base, t, "k", 1_000, 0, 1_000).is_blocked, "t={t}");
        }
        for t in [20_000, 20_001, 40_000] {
            assert!(hit(&storage, base, t, "z", 1_000, 0, 0).is_blocked, "no block, t={t}");
        }
    }

    /// Security review, mode `churn`: many keys sent round-robin must not get more than `limit` per window each.
    #[test]
    fn round_robin_keys_get_at_most_the_limit_each() {
        let storage = ThrottlerStorage::new(vec![]);
        let base = Instant::now();
        let (keys, limit) = (2_000usize, 100u32);
        let (mut allowed, mut blocked) = (0u64, 0u64);
        for round in 0..(limit as u64 + 50) {
            for k in 0..keys {
                if hit(&storage, base, round, &format!("ip{k}"), 3_600_000, limit, 3_600_000).is_blocked {
                    blocked += 1;
                } else {
                    allowed += 1;
                }
            }
        }
        assert_eq!(allowed, keys as u64 * u64::from(limit));
        assert_eq!(blocked, keys as u64 * 50);
    }

    /// Security review, mode `unblock`: flooding other keys does not unblock a blocked key, even when the table is full.
    #[test]
    fn a_blocked_key_stays_blocked_while_others_flood() {
        let storage = ThrottlerStorage::new(vec![]).with_max_entries(100);
        let base = Instant::now();
        for _ in 0..=10 {
            hit(&storage, base, 0, "me", 3_600_000, 10, 3_600_000);
        }
        assert!(hit(&storage, base, 1, "me", 3_600_000, 10, 3_600_000).is_blocked);
        for k in 0..5_000 {
            for _ in 0..10 {
                hit(&storage, base, 2, &format!("other{k}"), 3_600_000, 10, 3_600_000);
            }
        }
        assert!(storage.len() <= 101, "100 counters and the blocked key");
        assert!(hit(&storage, base, 3, "me", 3_600_000, 10, 3_600_000).is_blocked);
    }

    /// Security review, mode `pin`: filling the table with blocked keys must not turn counting off for other clients.
    #[test]
    fn pinning_the_blocked_table_does_not_disable_other_counters() {
        let storage = ThrottlerStorage::new(vec![]).with_max_entries(100);
        let base = Instant::now();
        for k in 0..1_000 {
            hit(&storage, base, 0, &format!("login{k}"), 3_600_000, 1, 3_600_000);
            assert!(hit(&storage, base, 0, &format!("login{k}"), 3_600_000, 1, 3_600_000).is_blocked);
        }
        assert!(storage.len() <= 100, "blocked table is capped: {}", storage.len());
        for key in ["a", "b"] {
            let allowed = (0..20).filter(|_| !hit(&storage, base, 1, key, 3_600_000, 10, 3_600_000).is_blocked).count();
            assert_eq!(allowed, 10, "{key}");
        }
        let allowed = (0..50).filter(|_| !hit(&storage, base, 2, "victim", 3_600_000, 3, 3_600_000).is_blocked).count();
        assert_eq!(allowed, 3);
    }

    #[test]
    fn a_short_block_keeps_the_reporting_window() {
        let storage = ThrottlerStorage::new(vec![]);
        let base = Instant::now();
        hit(&storage, base, 0, "k", 60_000, 1, 5_000);
        assert!(hit(&storage, base, 100, "k", 60_000, 1, 5_000).is_blocked);
        let r = hit(&storage, base, 6_000, "k", 60_000, 1, 5_000);
        assert!(!r.is_blocked);
        assert_eq!(r.total_hits, 1);
        assert_eq!(secs(r.time_to_expire), 54);
    }

    #[test]
    fn the_blocked_table_has_its_own_cap_and_drops_the_soonest_block() {
        let storage = ThrottlerStorage::new(vec![]).with_max_blocked(2);
        let base = Instant::now();
        for (i, key) in ["a", "b", "c"].iter().enumerate() {
            let block = 10_000 * (i as u64 + 1);
            hit(&storage, base, 0, key, 1_000, 0, block);
        }
        assert_eq!(storage.len(), 2);
        assert!(!hit(&storage, base, 1, "a", 1_000, 5, 10_000).is_blocked, "a was the soonest block and was dropped");
        assert!(hit(&storage, base, 1, "c", 1_000, 5, 30_000).is_blocked);
    }

    /// A limit as large as the old global budget still blocks: the count does not depend on stored timestamps.
    #[test]
    fn a_large_limit_still_blocks() {
        let storage = ThrottlerStorage::new(vec![]);
        let base = Instant::now();
        let limit = 1_500_000u32;
        let mut blocked = false;
        for i in 0..=limit as u64 {
            blocked = storage.record_hit_at("k", base + ms(i / 30_000), ms(60_000), limit, ms(60_000)).is_blocked;
        }
        assert!(blocked);
    }

    #[test]
    fn a_key_uses_bounded_memory_whatever_its_hits() {
        let storage = ThrottlerStorage::new(vec![]);
        let base = Instant::now();
        for i in 0..20_000u64 {
            storage.record_hit_at("k", base + ms(i), ms(60_000), u32::MAX, ms(60_000));
        }
        assert_eq!(storage.len(), 1);
        assert!(storage.memory_usage() < 8 * 1024, "{}", storage.memory_usage());
        let r = hit(&storage, base, 20_000, "k", 60_000, u32::MAX, 60_000);
        assert_eq!(r.total_hits, 20_001);
    }

    #[test]
    fn unrepresentable_durations_saturate_instead_of_vanishing() {
        let now = Instant::now();
        assert!(after(now, Duration::MAX) > now + Duration::from_secs(3600));
        let storage = ThrottlerStorage::new(vec![]);
        let r = storage.record_hit("k", Duration::from_secs(60), 1, Duration::MAX);
        assert!(!r.is_blocked);
        let r = storage.record_hit("k", Duration::from_secs(60), 1, Duration::MAX);
        assert!(r.is_blocked);
        assert!(r.time_to_block_expire > Duration::from_secs(3600));
    }

    #[test]
    fn idle_keys_are_removed() {
        let storage = ThrottlerStorage::new(vec![]);
        let base = Instant::now();
        hit(&storage, base, 0, "a", 1_000, 5, 1_000);
        hit(&storage, base, 1_500, "b", 1_000, 5, 1_000);
        assert_eq!(storage.len(), 1);
    }

    #[test]
    fn a_blocked_key_is_not_removed_while_the_block_lasts() {
        let storage = ThrottlerStorage::new(vec![]);
        let base = Instant::now();
        for t in [0, 1, 2] {
            hit(&storage, base, t, "a", 1_000, 2, 30_000);
        }
        hit(&storage, base, 5_000, "b", 1_000, 2, 30_000);
        assert_eq!(storage.len(), 2);
        assert!(hit(&storage, base, 6_000, "a", 1_000, 2, 30_000).is_blocked);
    }

    #[test]
    fn absurd_durations_do_not_panic() {
        let storage = ThrottlerStorage::new(vec![]);
        let huge = Duration::from_millis(u64::MAX);
        let r = storage.record_hit("k", huge, 1, huge);
        assert!(!r.is_blocked);
        assert!(storage.record_hit("k", huge, 1, huge).is_blocked);
        assert_eq!(storage.record_hit("z", huge, u32::MAX, Duration::ZERO).total_hits, 1);
        storage.increment("y", huge);
    }

    #[test]
    #[should_panic(expected = "login")]
    fn block_duration_for_an_unknown_throttler_panics() {
        let _ = ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 2)]).with_block_duration("login", 1_000);
    }

    #[test]
    fn block_duration_is_kept_per_throttler() {
        let storage = ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 2)]).with_block_duration("default", 300_000);
        assert_eq!(storage.block_durations.get("default"), Some(&300_000));
    }

    #[test]
    fn throttlers_are_sorted_by_ttl_keeping_the_order_of_ties() {
        let t = |name: &str, ttl| ThrottlerOptions { name: name.into(), ttl, limit: 1 };
        let storage = ThrottlerStorage::new(vec![t("long", 60_000), t("b", 1_000), t("a", 1_000)]);
        let names: Vec<&str> = storage.throttlers.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["b", "a", "long"]);
        let from: ThrottlerStorage = vec![t("long", 60_000), t("short", 1_000)].into();
        assert_eq!(from.throttlers[0].name, "short");
    }
}
