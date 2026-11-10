//! `MemorySessionStore`: sessions in this process, bounded.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;

use super::store::{SessionStore, SessionStoreError, now_ms};
use crate::auth::warn_limit::WarnLimiter;

/// The default bound on the number of sessions (the throttler's default).
pub const DEFAULT_MAX_SESSIONS: usize = 100_000;
/// The default bound on the bytes of serialized records, 256 MiB.
pub const DEFAULT_MAX_TOTAL_BYTES: usize = 256 << 20;
/// Records dropped for being expired at most per write, so that no request pays for a large purge.
const PURGE_BATCH: usize = 64;
/// What an entry costs besides its identifier and record (map slots, the order indexes).
const ENTRY_OVERHEAD: usize = 160;

struct Entry {
    record: Value,
    /// Identifier plus serialized record plus overhead.
    size: usize,
    expires_ms: i64,
    /// Position in the recency order, unique.
    tick: u64,
    /// `true` when the record holds a logged-in user: such a record is evicted after every anonymous one.
    authenticated: bool,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<Arc<str>, Entry>,
    /// Recency order of the anonymous records (`tick` to identifier), oldest first.
    anonymous: BTreeMap<u64, Arc<str>>,
    /// Recency order of the records with a user, oldest first.
    authenticated: BTreeMap<u64, Arc<str>>,
    /// Expiry order: `(expires_ms, tick)` to identifier.
    expiry: BTreeMap<(i64, u64), Arc<str>>,
    total_bytes: usize,
    next_tick: u64,
}

impl Inner {
    fn order(&mut self, authenticated: bool) -> &mut BTreeMap<u64, Arc<str>> {
        if authenticated { &mut self.authenticated } else { &mut self.anonymous }
    }

    fn remove(&mut self, id: &str) -> Option<Entry> {
        let entry = self.entries.remove(id)?;
        self.total_bytes -= entry.size;
        self.order(entry.authenticated).remove(&entry.tick);
        self.expiry.remove(&(entry.expires_ms, entry.tick));
        Some(entry)
    }

    fn insert(&mut self, id: Arc<str>, record: Value, size: usize, expires_ms: i64) {
        let authenticated = holds_user(&record);
        let tick = self.next_tick;
        self.next_tick += 1;
        self.order(authenticated).insert(tick, id.clone());
        self.expiry.insert((expires_ms, tick), id.clone());
        self.total_bytes += size;
        self.entries.insert(id, Entry { record, size, expires_ms, tick, authenticated });
    }

    /// Marks the entry as the most recently used and gives it the expiry `expires`.
    fn renew(&mut self, id: &str, expires: i64) {
        let tick = self.next_tick;
        let Some(entry) = self.entries.get_mut(id) else { return };
        self.next_tick += 1;
        let (old_tick, old_expires, authenticated) = (entry.tick, entry.expires_ms, entry.authenticated);
        entry.tick = tick;
        entry.expires_ms = expires;
        let order = if authenticated { &mut self.authenticated } else { &mut self.anonymous };
        if let Some(key) = order.remove(&old_tick) {
            order.insert(tick, key);
        }
        if let Some(key) = self.expiry.remove(&(old_expires, old_tick)) {
            self.expiry.insert((expires, tick), key);
        }
    }

    /// Drops up to `limit` expired records.
    fn purge(&mut self, now: i64, limit: usize) -> usize {
        let mut dropped = 0;
        while dropped < limit {
            let Some((&(expires, _), id)) = self.expiry.iter().next() else { break };
            if expires > now {
                break;
            }
            let id = id.clone();
            self.remove(&id);
            dropped += 1;
        }
        dropped
    }

    /// Evicts the least recently used record, anonymous ones first. `false` when the store is empty.
    fn evict_one(&mut self) -> bool {
        let victim = self.anonymous.values().next().or_else(|| self.authenticated.values().next()).cloned();
        match victim {
            Some(id) => self.remove(&id).is_some(),
            None => false,
        }
    }
}

/// `true` when the record has a non-null `passport.user`.
fn holds_user(record: &Value) -> bool {
    record.get("passport").and_then(|p| p.get("user")).is_some_and(|u| !u.is_null())
}

/// Sessions kept in this process: lost on restart and not shared between instances, so it is for development and
/// single-process deployments.
///
/// It holds at most `max_sessions` records (default 100 000) and `max_total_bytes` of serialized data (default
/// 256 MiB). Expired records are dropped without a background task: on `load`, and a few at a time on every write.
/// When a new record does not fit, the least recently used one is evicted, anonymous sessions (no logged-in user)
/// before any session with a user, so a flood of anonymous sessions cannot log users out while it can fill the store
/// with its own; a warning, at most one a minute and without identifiers, reports the evictions.
///
/// ```ignore
/// SessionOptions::new(keys).store(MemorySessionStore::new().max_sessions(10_000).max_total_bytes(64 << 20))
/// ```
pub struct MemorySessionStore {
    inner: Mutex<Inner>,
    max_sessions: usize,
    max_total_bytes: usize,
    evicted: AtomicU64,
    warnings: WarnLimiter,
}

impl Default for MemorySessionStore {
    fn default() -> Self {
        MemorySessionStore::new()
    }
}

impl std::fmt::Debug for MemorySessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemorySessionStore")
            .field("max_sessions", &self.max_sessions)
            .field("max_total_bytes", &self.max_total_bytes)
            .finish_non_exhaustive()
    }
}

impl MemorySessionStore {
    pub fn new() -> Self {
        MemorySessionStore {
            inner: Mutex::new(Inner::default()),
            max_sessions: DEFAULT_MAX_SESSIONS,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            evicted: AtomicU64::new(0),
            warnings: WarnLimiter::new(Duration::from_secs(60)),
        }
    }

    /// The most sessions held (at least 1; checked when the application starts).
    pub fn max_sessions(mut self, max: usize) -> Self {
        self.max_sessions = max;
        self
    }

    /// The most bytes of serialized records held (at least 1; checked when the application starts).
    pub fn max_total_bytes(mut self, max: usize) -> Self {
        self.max_total_bytes = max;
        self
    }

    /// How many sessions are held, expired ones included until they are dropped.
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bytes accounted for the sessions held.
    pub fn total_bytes(&self) -> usize {
        self.lock().total_bytes
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn warn_evictions(&self, count: u64) {
        self.evicted.fetch_add(count, Ordering::Relaxed);
        if self.warnings.allow("evict", Instant::now()).is_some() {
            let total = self.evicted.swap(0, Ordering::Relaxed);
            crate::logger::Logger::new("MemorySessionStore").warn(format!(
                "the memory session store is full: {total} least recently used session(s) were evicted since the last warning (max_sessions {}, max_total_bytes {}). Use a larger bound or a shared store",
                self.max_sessions, self.max_total_bytes
            ));
        }
    }

    fn write(&self, id: &str, record: &Value, ttl: Duration, mode: Mode) -> Result<bool, SessionStoreError> {
        let size = id.len() + record.to_string().len() + ENTRY_OVERHEAD;
        if size > self.max_total_bytes {
            return Err(SessionStoreError::backend("the record is larger than the max_total_bytes of the memory store"));
        }
        let now = now_ms();
        let expires = now.saturating_add(ttl.as_millis().min(i64::MAX as u128 / 2) as i64);
        let mut evicted = 0u64;
        {
            let mut inner = self.lock();
            inner.purge(now, PURGE_BATCH);
            match inner.entries.get(id).map(|e| e.expires_ms <= now) {
                // An expired record is gone for every purpose.
                Some(true) => {
                    inner.remove(id);
                    if mode != Mode::Create {
                        return Ok(false);
                    }
                }
                Some(false) if mode == Mode::Create => return Err(SessionStoreError::Exists),
                None if mode != Mode::Create => return Ok(false),
                _ => {}
            }
            if mode == Mode::Create || mode == Mode::Update {
                inner.remove(id);
                // Room for the new record: expired ones first, then the least recently used.
                while inner.entries.len() >= self.max_sessions || inner.total_bytes + size > self.max_total_bytes {
                    if inner.purge(now, usize::MAX) > 0 {
                        continue;
                    }
                    if !inner.evict_one() {
                        break;
                    }
                    evicted += 1;
                }
                inner.insert(Arc::from(id), record.clone(), size, expires);
            } else {
                // `touch`: the expiry and the timestamps only, so a request that changed nothing cannot overwrite
                // what another request saved.
                let Some(entry) = inner.entries.get_mut(id) else { return Ok(false) };
                for key in ["cookie", "__corvis"] {
                    if let (Some(object), Some(new)) = (entry.record.as_object_mut(), record.get(key)) {
                        object.insert(key.to_string(), new.clone());
                    }
                }
                inner.renew(id, expires);
            }
        }
        if evicted > 0 {
            self.warn_evictions(evicted);
        }
        Ok(true)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Create,
    Update,
    Touch,
}

#[async_trait]
impl SessionStore for MemorySessionStore {
    async fn load(&self, id: &str) -> Result<Option<Value>, SessionStoreError> {
        let now = now_ms();
        let mut inner = self.lock();
        match inner.entries.get(id).map(|e| e.expires_ms <= now) {
            None => Ok(None),
            Some(true) => {
                inner.remove(id);
                Ok(None)
            }
            Some(false) => {
                let expires = inner.entries.get(id).map_or(0, |e| e.expires_ms);
                inner.renew(id, expires);
                Ok(inner.entries.get(id).map(|e| e.record.clone()))
            }
        }
    }

    async fn create(&self, id: &str, record: &Value, ttl: Duration) -> Result<(), SessionStoreError> {
        self.write(id, record, ttl, Mode::Create).map(|_| ())
    }

    async fn update(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError> {
        self.write(id, record, ttl, Mode::Update)
    }

    async fn touch(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError> {
        self.write(id, record, ttl, Mode::Touch)
    }

    async fn destroy(&self, id: &str) -> Result<(), SessionStoreError> {
        self.lock().remove(id);
        Ok(())
    }

    fn in_process(&self) -> bool {
        true
    }

    fn validate(&self) -> Result<(), String> {
        if self.max_sessions == 0 {
            return Err("the memory session store's max_sessions must be at least 1".to_string());
        }
        if self.max_total_bytes == 0 {
            return Err("the memory session store's max_total_bytes must be at least 1".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
