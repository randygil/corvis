//! `MemorySessionStore`: sessions in this process, bounded.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
/// The default bound on the sessions of one user.
pub const DEFAULT_MAX_SESSIONS_PER_USER: usize = 16;
/// Records dropped for being expired at most per write, so that no request pays for a large purge.
const PURGE_BATCH: usize = 64;
/// What an entry costs besides its identifier, its serialized record and its user key: the map slots, the order
/// indexes and the per-user list (measured at about 520 bytes for the cheapest logged-in record, one user each).
/// It is counted against `max_total_bytes` for every entry, so the live heap of the store stays below the bound
/// whatever the shape of the data (a test with a counting allocator checks it).
pub(crate) const ENTRY_OVERHEAD: usize = 640;

/// The place of a user in [`Inner::ranking`]: sessions held, the (reversed) tick of the oldest one, the user.
type Rank = (usize, Reverse<u64>, Arc<str>);

struct Entry {
    /// The record as the store serialized it: a `Value` tree costs many times its serialized size, a string costs
    /// what it says.
    record: Box<str>,
    /// Identifier, serialized record, user key and overhead.
    size: usize,
    expires_ms: i64,
    /// Position in the recency order, unique.
    tick: u64,
    /// The serialized `passport.user` of a logged-in record: such a record is evicted after every anonymous one, and
    /// counts against the cap of its user.
    user: Option<Arc<str>>,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<Arc<str>, Entry>,
    /// Recency order of the anonymous records (`tick` to identifier), oldest first.
    anonymous: BTreeMap<u64, Arc<str>>,
    /// The identifiers of the records of each user, oldest first.
    by_user: HashMap<Arc<str>, Vec<Arc<str>>>,
    /// `(number of sessions, oldest session first, user)`, so that the last element is the user with the most
    /// sessions, the one whose oldest session is the oldest among those.
    ranking: BTreeSet<Rank>,
    /// Expiry order: `(expires_ms, tick)` to identifier.
    expiry: BTreeMap<(i64, u64), Arc<str>>,
    total_bytes: usize,
    next_tick: u64,
}

impl Inner {
    /// Where `user` stands in the ranking, from the state as it is now.
    fn rank_of(&self, user: &Arc<str>) -> Option<Rank> {
        let ids = self.by_user.get(user)?;
        let oldest = self.entries.get(ids.first()?)?;
        Some((ids.len(), Reverse(oldest.tick), user.clone()))
    }

    /// Takes `user` out of the ranking. Call it before the sessions of the user change, and [`rank`](Self::rank)
    /// after.
    fn unrank(&mut self, user: &Arc<str>) {
        if let Some(rank) = self.rank_of(user) {
            self.ranking.remove(&rank);
        }
    }

    fn rank(&mut self, user: &Arc<str>) {
        if let Some(rank) = self.rank_of(user) {
            self.ranking.insert(rank);
        }
    }

    fn remove(&mut self, id: &str) -> Option<Entry> {
        let user = self.entries.get(id)?.user.clone();
        if let Some(user) = &user {
            self.unrank(user);
        }
        let entry = self.entries.remove(id)?;
        self.total_bytes -= entry.size;
        match &user {
            None => {
                self.anonymous.remove(&entry.tick);
            }
            Some(user) => {
                if let Some(ids) = self.by_user.get_mut(user) {
                    ids.retain(|other| &**other != id);
                    if ids.is_empty() {
                        self.by_user.remove(user);
                    }
                }
                self.rank(user);
            }
        }
        self.expiry.remove(&(entry.expires_ms, entry.tick));
        Some(entry)
    }

    fn insert(&mut self, id: Arc<str>, record: Box<str>, user: Option<Arc<str>>, size: usize, expires_ms: i64) {
        let tick = self.next_tick;
        self.next_tick += 1;
        if let Some(user) = &user {
            self.unrank(user);
        }
        self.expiry.insert((expires_ms, tick), id.clone());
        self.total_bytes += size;
        match &user {
            None => {
                self.anonymous.insert(tick, id.clone());
                self.entries.insert(id, Entry { record, size, expires_ms, tick, user });
            }
            Some(user) => {
                let user = user.clone();
                self.by_user.entry(user.clone()).or_default().push(id.clone());
                self.entries.insert(id, Entry { record, size, expires_ms, tick, user: Some(user.clone()) });
                self.rank(&user);
            }
        }
    }

    /// Marks the entry as the most recently used and gives it the expiry `expires`.
    fn renew(&mut self, id: &str, expires: i64) {
        let tick = self.next_tick;
        let Some(user) = self.entries.get(id).map(|e| e.user.clone()) else { return };
        if let Some(user) = &user {
            self.unrank(user);
        }
        self.next_tick += 1;
        let Some(entry) = self.entries.get_mut(id) else { return };
        let (old_tick, old_expires) = (entry.tick, entry.expires_ms);
        entry.tick = tick;
        entry.expires_ms = expires;
        match &user {
            None => {
                if let Some(key) = self.anonymous.remove(&old_tick) {
                    self.anonymous.insert(tick, key);
                }
            }
            Some(user) => {
                if let Some(ids) = self.by_user.get_mut(user)
                    && let Some(position) = ids.iter().position(|other| &**other == id)
                {
                    let moved = ids.remove(position);
                    ids.push(moved);
                }
                self.rank(user);
            }
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

    /// Evicts one record to make room: the least recently used anonymous one, and when there is none the oldest
    /// session of the user with the most sessions. `false` when the store is empty.
    fn evict_one(&mut self) -> bool {
        let victim = match self.anonymous.values().next() {
            Some(id) => Some(id.clone()),
            None => self.ranking.iter().next_back().and_then(|(_, _, user)| self.by_user.get(user)).and_then(|ids| ids.first()).cloned(),
        };
        match victim {
            Some(id) => self.remove(&id).is_some(),
            None => false,
        }
    }

    /// Evicts the oldest session of `user`. `false` when it has none.
    fn evict_oldest_of(&mut self, user: &Arc<str>) -> bool {
        let victim = self.by_user.get(user).and_then(|ids| ids.first()).cloned();
        match victim {
            Some(id) => self.remove(&id).is_some(),
            None => false,
        }
    }

    fn sessions_of(&self, user: &Arc<str>) -> usize {
        self.by_user.get(user).map_or(0, Vec::len)
    }
}

/// The serialized `passport.user` of a record, `None` when it has no non-null one.
fn stored_user(record: &Value) -> Option<Arc<str>> {
    let user = record.get("passport")?.get("user")?;
    if user.is_null() { None } else { Some(Arc::from(user.to_string())) }
}

/// Sessions kept in this process: lost on restart and not shared between instances, so it is for development and
/// single-process deployments.
///
/// Records are kept serialized (a parsed JSON tree can cost thirty times its serialized size) and the bound counts
/// the allocation of each one plus a fixed overhead per entry, so `max_total_bytes` bounds the memory the store uses,
/// not just the text of the sessions.
///
/// It holds at most `max_sessions` records (default 100 000), `max_total_bytes` (default 256 MiB) and
/// `max_sessions_per_user` sessions of one user (default 16, keyed by the serialized `passport.user`). Expired
/// records are dropped without a background task: on `load`, and a few at a time on every write.
///
/// A new session of a user who is at the cap evicts that same user's oldest session, never another user's, so one
/// account that logs in again and again cannot log the others out. When the store is full, the least recently used
/// anonymous session (no logged-in user) is evicted first, then the oldest session of the user with the most
/// sessions; a warning, at most one a minute and without identifiers, reports the evictions. Other stores need their
/// own policy: a shared store has to bound the sessions of a user itself.
///
/// ```ignore
/// SessionOptions::new(keys).store(MemorySessionStore::new().max_sessions(10_000).max_total_bytes(64 << 20))
/// ```
pub struct MemorySessionStore {
    inner: Mutex<Inner>,
    max_sessions: usize,
    max_total_bytes: usize,
    max_sessions_per_user: usize,
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
            .field("max_sessions_per_user", &self.max_sessions_per_user)
            .finish_non_exhaustive()
    }
}

impl MemorySessionStore {
    pub fn new() -> Self {
        MemorySessionStore {
            inner: Mutex::new(Inner::default()),
            max_sessions: DEFAULT_MAX_SESSIONS,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            max_sessions_per_user: DEFAULT_MAX_SESSIONS_PER_USER,
            evicted: AtomicU64::new(0),
            warnings: WarnLimiter::new(Duration::from_secs(60)),
        }
    }

    /// The most sessions held (at least 1; checked when the application starts).
    pub fn max_sessions(mut self, max: usize) -> Self {
        self.max_sessions = max;
        self
    }

    /// The most bytes held, serialized records plus a fixed overhead per entry (at least 1; checked when the
    /// application starts).
    pub fn max_total_bytes(mut self, max: usize) -> Self {
        self.max_total_bytes = max;
        self
    }

    /// The most sessions of one user held (default 16, at least 1; checked when the application starts). A session
    /// beyond it evicts the oldest session of the same user.
    pub fn max_sessions_per_user(mut self, max: usize) -> Self {
        self.max_sessions_per_user = max;
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
                "the memory session store is full: {total} session(s) were evicted since the last warning (max_sessions {}, max_total_bytes {}, max_sessions_per_user {}). Use a larger bound or a shared store",
                self.max_sessions, self.max_total_bytes, self.max_sessions_per_user
            ));
        }
    }

    /// The whole of a write is one step under the lock: the check that the record exists (or does not) and the write
    /// cannot be separated by another request.
    fn write(&self, id: &str, record: &Value, ttl: Duration, mode: Mode) -> Result<bool, SessionStoreError> {
        // Serialized before the lock is taken; a touch serializes the merged record under it.
        let prepared = (mode != Mode::Touch).then(|| (record.to_string().into_boxed_str(), stored_user(record)));
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
            let (text, user) = match prepared {
                Some(prepared) => prepared,
                None => {
                    // `touch`: the expiry and the timestamps only, so a request that changed nothing cannot overwrite
                    // what another request saved.
                    let Some(entry) = inner.entries.get(id) else { return Ok(false) };
                    let mut stored: Value = serde_json::from_str(&entry.record).unwrap_or(Value::Null);
                    if let Some(object) = stored.as_object_mut() {
                        for key in ["cookie", "__corvis"] {
                            if let Some(new) = record.get(key) {
                                object.insert(key.to_string(), new.clone());
                            }
                        }
                    }
                    (stored.to_string().into_boxed_str(), entry.user.clone())
                }
            };
            let size = id.len() + text.len() + user.as_ref().map_or(0, |u| u.len()) + ENTRY_OVERHEAD;
            if size > self.max_total_bytes {
                return Err(SessionStoreError::backend("the record is larger than the max_total_bytes of the memory store"));
            }
            inner.remove(id);
            if let Some(user) = &user {
                // The cap of the user: its oldest session goes, nobody else's.
                while inner.sessions_of(user) >= self.max_sessions_per_user {
                    if !inner.evict_oldest_of(user) {
                        break;
                    }
                    evicted += 1;
                }
            }
            // Room for the new record: expired ones first, then the anonymous ones, then the user with the most sessions.
            while inner.entries.len() >= self.max_sessions || inner.total_bytes + size > self.max_total_bytes {
                if inner.purge(now, usize::MAX) > 0 {
                    continue;
                }
                if !inner.evict_one() {
                    break;
                }
                evicted += 1;
            }
            inner.insert(Arc::from(id), text, user, size, expires);
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
                // The text is what this store serialized, so it parses.
                Ok(inner.entries.get(id).and_then(|e| serde_json::from_str(&e.record).ok()))
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

    async fn destroy(&self, id: &str) -> Result<bool, SessionStoreError> {
        let now = now_ms();
        Ok(self.lock().remove(id).is_some_and(|entry| entry.expires_ms > now))
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
        if self.max_sessions_per_user == 0 {
            return Err("the memory session store's max_sessions_per_user must be at least 1".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
