//! The `Session` handle of a request and the record it is saved as.

use std::sync::{Arc, Mutex, MutexGuard};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use rand::rngs::OsRng;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use super::SessionManager;
use super::options::SessionOptions;

/// The key of the `cookie` object of an `express-session` record.
pub(crate) const COOKIE_KEY: &str = "cookie";
/// The key of the timestamps the middleware enforces expiry from.
pub(crate) const META_KEY: &str = "__corvis";
/// Where `passport` keeps the serialized user, `passport.user`.
pub(crate) const PASSPORT_KEY: &str = "passport";
pub(crate) const USER_KEY: &str = "user";

/// The longest session identifier read from a cookie. Ours have 32 characters.
pub(crate) const MAX_ID_BYTES: usize = 128;
/// Bytes of randomness in a new identifier (192 bits).
const ID_BYTES: usize = 24;

/// Why [`Session::set`] refused a value.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// `cookie` and `__corvis` belong to the middleware.
    #[error("the session key \"{0}\" is reserved")]
    ReservedKey(String),
    /// The value does not serialize to JSON.
    #[error("the session value could not be serialized: {0}")]
    Serialize(String),
}

/// A new identifier from the operating system's random generator, 32 characters of base64url. `None` when the
/// generator fails: there is no fallback to a weaker one.
pub(crate) fn new_id() -> Option<String> {
    let mut bytes = [0u8; ID_BYTES];
    OsRng.try_fill_bytes(&mut bytes).ok()?;
    Some(URL_SAFE_NO_PAD.encode(bytes))
}

/// `true` for an identifier that may come out of a verified cookie: 1 to 128 bytes of visible ASCII.
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_BYTES && id.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// What the middleware tracks about the session of one request.
#[derive(Clone)]
pub(crate) struct State {
    /// The current identifier.
    pub(crate) id: String,
    /// A record exists in the store under `id` (as far as this request knows).
    pub(crate) persisted: bool,
    /// The identifier was generated after the request arrived (login, logout, `regenerate`).
    pub(crate) id_changed: bool,
    pub(crate) data: Map<String, Value>,
    /// The data as loaded (or as created): the session is modified when `data` differs from it.
    pub(crate) baseline: Map<String, Value>,
    pub(crate) created_at: i64,
    pub(crate) last_access: i64,
    /// Identifiers to destroy in the store before anything is written.
    pub(crate) destroy: Vec<String>,
    /// The session the cookie named is gone (destroyed or expired): clear the cookie unless a new one is set.
    pub(crate) clear_cookie: bool,
    /// The request carried a session cookie, valid or not.
    pub(crate) had_cookie: bool,
    /// The cookie verified only with a retired secret: send it again signed with the current one.
    pub(crate) resign: bool,
    /// The random generator failed: nothing may be written for this request.
    pub(crate) failed: bool,
    /// The identifier of the record this request loaded, kept when the identifier is replaced: at commit the record
    /// must still exist, or the session was ended by another request and nothing of it is written back.
    pub(crate) origin: Option<String>,
    /// The state before a login made by an `AuthGuard`: restored at commit when the response is a rejection (status 400
    /// or more), so that a login followed by a failing guard or handler leaves the session as it was.
    pub(crate) before_guard_login: Option<Box<State>>,
}

impl State {
    /// A session with a new identifier.
    pub(crate) fn fresh(now: i64) -> State {
        let id = new_id();
        State {
            failed: id.is_none(),
            id: id.unwrap_or_default(),
            persisted: false,
            id_changed: false,
            data: Map::new(),
            baseline: Map::new(),
            created_at: now,
            last_access: now,
            destroy: Vec::new(),
            clear_cookie: false,
            had_cookie: false,
            resign: false,
            origin: None,
            before_guard_login: None,
        }
    }

    pub(crate) fn modified(&self) -> bool {
        self.id_changed || self.data != self.baseline
    }

    /// An existing record, loaded from the store under the identifier of the cookie.
    pub(crate) fn loaded(id: String, data: Map<String, Value>, created_at: i64, last_access: i64) -> State {
        State {
            origin: Some(id.clone()),
            id,
            persisted: true,
            id_changed: false,
            baseline: data.clone(),
            data,
            created_at,
            last_access,
            destroy: Vec::new(),
            clear_cookie: false,
            had_cookie: true,
            resign: false,
            failed: false,
            before_guard_login: None,
        }
    }

    /// Replaces the identifier. The old one is destroyed in the store at commit when it has a record there.
    pub(crate) fn regenerate(&mut self, now: i64) {
        if self.persisted && !self.destroy.contains(&self.id) {
            self.destroy.push(self.id.clone());
        }
        match new_id() {
            Some(id) => self.id = id,
            None => self.failed = true,
        }
        self.persisted = false;
        self.id_changed = true;
        self.created_at = now;
        self.last_access = now;
    }

    /// The record the store receives: `cookie` first, as `express-session` writes it, then the data, then the
    /// timestamps.
    pub(crate) fn record(&self, options: &SessionOptions, now: i64) -> Value {
        let mut object = Map::new();
        object.insert(COOKIE_KEY.to_string(), self.cookie_json(options, now));
        for (key, value) in &self.data {
            object.insert(key.clone(), value.clone());
        }
        object.insert(META_KEY.to_string(), json!({ "createdAt": self.created_at, "lastAccess": now }));
        Value::Object(object)
    }

    /// When the cookie expires: `max_age` after `now`, never later than the absolute limit of the session.
    pub(crate) fn cookie_expires(&self, options: &SessionOptions, now: i64) -> Option<i64> {
        let max_age = options.cookie.max_age?;
        let mut expires = now.saturating_add(max_age.as_millis().min(i64::MAX as u128 / 4) as i64);
        if let Some(absolute) = options.absolute_timeout {
            expires = expires.min(self.created_at.saturating_add(absolute.as_millis() as i64));
        }
        Some(expires)
    }

    /// The `cookie` object of the record, with the keys and value formats of `express-session` 1.19.0.
    fn cookie_json(&self, options: &SessionOptions, now: i64) -> Value {
        let c = &options.cookie;
        let mut cookie = Map::new();
        cookie.insert("originalMaxAge".into(), c.max_age.map_or(Value::Null, |d| json!(d.as_millis() as u64)));
        cookie.insert("expires".into(), self.cookie_expires(options, now).map_or(Value::Null, |ms| Value::String(iso(ms))));
        cookie.insert("secure".into(), Value::Bool(c.secure));
        cookie.insert("httpOnly".into(), Value::Bool(c.http_only));
        if let Some(domain) = &c.domain {
            cookie.insert("domain".into(), Value::String(domain.clone()));
        }
        cookie.insert("path".into(), Value::String(c.path.clone()));
        if let Some(same_site) = c.same_site {
            cookie.insert("sameSite".into(), Value::String(same_site.as_str().to_string()));
        }
        Value::Object(cookie)
    }
}

/// `Date.prototype.toJSON`: ISO 8601 with milliseconds and a `Z`.
pub(crate) fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The data, `created_at` and `last_access` of a stored record, or why it must not be used.
pub(crate) enum Loaded {
    /// A usable session.
    Session { data: Map<String, Value>, created_at: i64, last_access: i64 },
    /// The record is past its idle or absolute timeout or its cookie expiry, or its timestamps are not numbers.
    Expired,
    /// Not a session record (not a JSON object): ignored.
    Invalid,
}

/// Reads a record and enforces expiry from its timestamps, whatever the store did with the time to live. A record
/// without `__corvis` (written by the original) is accepted and starts its periods at `now`.
pub(crate) fn read_record(record: Value, options: &SessionOptions, now: i64) -> Loaded {
    let Value::Object(mut object) = record else { return Loaded::Invalid };
    if let Some(expires) = object.get(COOKIE_KEY).and_then(|c| c.get("expires")) {
        match expires {
            Value::Null => {}
            Value::String(text) => match chrono::DateTime::parse_from_rfc3339(text) {
                Ok(at) if at.timestamp_millis() > now => {}
                _ => return Loaded::Expired,
            },
            _ => return Loaded::Expired,
        }
    }
    let (mut created_at, mut last_access) = (now, now);
    match object.get(META_KEY) {
        None => {}
        Some(Value::Object(meta)) => {
            match (meta.get("createdAt").and_then(Value::as_i64), meta.get("lastAccess").and_then(Value::as_i64)) {
                (Some(created), Some(last)) => {
                    // A timestamp from the future (another instance's clock) counts as now: it cannot extend a session.
                    created_at = created.min(now);
                    last_access = last.min(now);
                }
                _ => return Loaded::Expired,
            }
        }
        Some(_) => return Loaded::Expired,
    }
    if now.saturating_sub(last_access) > options.idle_timeout.as_millis() as i64 {
        return Loaded::Expired;
    }
    if let Some(absolute) = options.absolute_timeout
        && now.saturating_sub(created_at) > absolute.as_millis() as i64
    {
        return Loaded::Expired;
    }
    object.remove(COOKIE_KEY);
    object.remove(META_KEY);
    Loaded::Session { data: object, created_at, last_access }
}

pub(crate) struct Shared {
    pub(crate) manager: Arc<SessionManager>,
    pub(crate) state: Mutex<State>,
}

/// The session of a request: a cheap clone of a shared handle, with synchronous methods. All store access happens in
/// the middleware before and after the handler, so guards, strategies, interceptors and handlers need no `await`.
/// Read it with `Request::session()`.
///
/// ```ignore
/// #[get("views")]
/// fn views(&self, req: Request) -> Value {
///     let session = req.session().expect("the session module is imported");
///     let views = session.get::<u64>("views").unwrap_or(0) + 1;
///     session.set("views", views).unwrap();
///     json!({ "views": views })
/// }
/// ```
///
/// `Debug` prints neither the identifier nor the data.
#[derive(Clone)]
pub struct Session {
    pub(crate) shared: Arc<Shared>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session { .. }")
    }
}

impl Session {
    pub(crate) fn new(manager: Arc<SessionManager>, state: State) -> Session {
        Session { shared: Arc::new(Shared { manager, state: Mutex::new(state) }) }
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn manager(&self) -> &Arc<SessionManager> {
        &self.shared.manager
    }

    /// The identifier of the session. It is the credential of the browser: do not log it or send it anywhere.
    pub fn id(&self) -> String {
        self.state().id.clone()
    }

    /// `true` when no record of this session exists in the store yet (a new session, or one whose identifier was
    /// regenerated in this request).
    pub fn is_new(&self) -> bool {
        !self.state().persisted
    }

    /// The value of `key` as a `T`, `None` when it is absent or not a `T`.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.get_value(key).and_then(|v| serde_json::from_value(v).ok())
    }

    /// The JSON value of `key`. The reserved keys (`cookie`, `__corvis`) are never shown.
    pub fn get_value(&self, key: &str) -> Option<Value> {
        if is_reserved(key) {
            return None;
        }
        self.state().data.get(key).cloned()
    }

    /// Every value of the session as a JSON object, without the reserved keys.
    pub fn data(&self) -> serde_json::Map<String, Value> {
        self.state().data.clone()
    }

    /// Stores `value` under `key`. Fails for a reserved key or a value that does not serialize; the session is left
    /// as it was.
    pub fn set<T: Serialize>(&self, key: &str, value: T) -> Result<(), SessionError> {
        if is_reserved(key) {
            return Err(SessionError::ReservedKey(key.to_string()));
        }
        let value = serde_json::to_value(value).map_err(|e| SessionError::Serialize(e.to_string()))?;
        self.state().data.insert(key.to_string(), value);
        Ok(())
    }

    pub fn remove(&self, key: &str) {
        if !is_reserved(key) {
            self.state().data.shift_remove(key);
        }
    }

    /// Removes every value (the session itself stays).
    pub fn clear(&self) {
        self.state().data.clear();
    }

    /// `req.session.regenerate()`: a new, empty session with a new identifier. The old record is destroyed in the
    /// store when the request ends and its data is dropped. A login does this by itself.
    pub fn regenerate(&self) {
        let mut s = self.state();
        s.regenerate(super::store::now_ms());
        s.data.clear();
    }

    /// Replaces the identifier now and keeps the data. The old record is destroyed in the store when the request ends.
    /// Call it when privileges change and the session has to carry on (a role was granted, a step-up check passed): a
    /// new identifier cannot be one somebody else planted or learned.
    pub fn regenerate_keeping_data(&self) {
        self.state().regenerate(super::store::now_ms());
    }

    /// Ends the session: its record is destroyed in the store when the request ends and the cookie is cleared. The
    /// data is dropped; values set afterwards start a new session with a new identifier.
    pub fn destroy(&self) {
        let mut s = self.state();
        if s.persisted && !s.destroy.contains(&s.id) {
            let id = s.id.clone();
            s.destroy.push(id);
        }
        let now = super::store::now_ms();
        let fresh = State::fresh(now);
        s.id = fresh.id;
        s.failed |= fresh.failed;
        s.persisted = false;
        s.id_changed = false;
        s.data.clear();
        s.baseline.clear();
        s.created_at = now;
        s.last_access = now;
        s.clear_cookie = s.had_cookie;
        s.resign = false;
    }
}

pub(crate) fn is_reserved(key: &str) -> bool {
    key == COOKIE_KEY || key == META_KEY
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn identifiers_have_the_shape_and_are_unique() {
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let id = new_id().unwrap();
            assert_eq!(id.len(), 32, "{id}");
            assert!(id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "{id}");
            assert!(valid_id(&id));
            assert!(seen.insert(id));
        }
    }

    #[test]
    fn incoming_identifiers_are_bounded_and_visible_ascii() {
        assert!(valid_id("abc-123_XYZ"));
        assert!(valid_id(&"a".repeat(128)));
        assert!(!valid_id(""));
        assert!(!valid_id(&"a".repeat(129)));
        for bad in ["a b", "a\tb", "a\u{e9}", "a\0b", "a\r\nb", "a\u{7f}"] {
            assert!(!valid_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_iso_format_is_the_one_of_date_to_json() {
        assert_eq!(iso(1_794_182_400_123), "2026-11-09T00:00:00.123Z");
    }
}
