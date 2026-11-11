//! `SessionManager`: the validated options, the store and the load and commit steps of the middleware.

use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;

use super::options::SessionOptions;
use super::session::{Loaded, Session, State, read_record, valid_id};
use super::store::{SessionStoreError, now_ms};
use crate::auth::AuthenticatedUser;
use crate::auth::warn_limit::WarnLimiter;
use crate::http::res::{CookieOptions, serialize_cookie};
use crate::http::signed_cookie::MAX_SIGNED_BYTES;
use crate::http::{HttpException, Request};
use crate::logger::Logger;

/// The raw (percent-encoded) cookie value is decoded only up to this length: a signed value of 4096 bytes encodes to at
/// most three times that.
const MAX_RAW_COOKIE_BYTES: usize = 3 * MAX_SIGNED_BYTES;

/// The deepest nesting a saved record may have, counting the object that holds the session as level 1 (so a value of
/// the session may nest 99 levels). `serde_json` refuses to read more than 128, and the body parser accepts values
/// nested almost that deep: the limit leaves room for the record's own levels and is checked when the session is
/// written, where the application can still see the failure.
pub(crate) const MAX_RECORD_DEPTH: usize = 100;

/// Counts the bytes a serialization writes and stops it once they pass `limit`, so a record is measured without a
/// buffer and an oversized one costs at most the limit.
struct Measure {
    written: usize,
    limit: usize,
}

impl std::io::Write for Measure {
    fn write(&mut self, chunk: &[u8]) -> std::io::Result<usize> {
        self.written = self.written.saturating_add(chunk.len());
        if self.written > self.limit {
            return Err(std::io::Error::other("the record is larger than the limit"));
        }
        Ok(chunk.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `true` when `record` serializes to at most `limit` bytes.
pub(crate) fn fits(record: &Value, limit: usize) -> bool {
    serde_json::to_writer(&mut Measure { written: 0, limit }, record).is_ok()
}

/// The error of every session fault that reaches the client as a 500. The text is fixed and holds nothing about the
/// request, the session or the store.
#[derive(Debug)]
struct SessionFault(&'static str);

impl std::fmt::Display for SessionFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for SessionFault {}

/// Turns the user of a request into what the session stores and back. Implemented by `SessionStrategy<S>` over the
/// application's `SessionSerializer`.
#[async_trait]
pub(crate) trait UserCodec: Send + Sync {
    async fn serialize(&self, user: &AuthenticatedUser) -> Result<Value, HttpException>;
    async fn deserialize(&self, req: &Request, stored: Value) -> Result<Option<AuthenticatedUser>, HttpException>;
}

/// The options, the keys and the store of the application's sessions, built (and validated) when the application
/// starts. Provided by [`SessionModule`](super::SessionModule).
pub struct SessionManager {
    pub(crate) options: SessionOptions,
    pub(crate) notes: Vec<String>,
    codec: OnceLock<Arc<dyn UserCodec>>,
    warnings: WarnLimiter,
}

/// Prints no secret and no session.
impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager").field("cookie_name", &self.options.name).finish_non_exhaustive()
    }
}

impl SessionManager {
    pub(crate) fn new(options: SessionOptions) -> Result<Self, super::SessionOptionsError> {
        let notes = options.validate()?;
        Ok(SessionManager { options, notes, codec: OnceLock::new(), warnings: WarnLimiter::new(Duration::from_secs(60)) })
    }

    /// Registers the session user codec. `false` when one is registered already.
    pub(crate) fn set_codec(&self, codec: Arc<dyn UserCodec>) -> bool {
        self.codec.set(codec).is_ok()
    }

    pub(crate) fn codec(&self) -> Option<&Arc<dyn UserCodec>> {
        self.codec.get()
    }

    /// The attributes of the session cookie as a startup log line.
    pub(crate) fn describe_cookie(&self) -> String {
        let c = &self.options.cookie;
        format!(
            "session cookie \"{}\": Path={}{}{}{}{}",
            self.options.name,
            c.path,
            c.domain.as_ref().map(|d| format!("; Domain={d}")).unwrap_or_default(),
            if c.http_only { "; HttpOnly" } else { "" },
            if c.secure { "; Secure" } else { "" },
            c.same_site.map(|s| format!("; SameSite={}", s.attribute())).unwrap_or_default(),
        )
    }

    /// One store call, bounded in time.
    pub(crate) async fn store_call<T>(&self, call: impl Future<Output = Result<T, SessionStoreError>>) -> Result<T, SessionStoreError> {
        match tokio::time::timeout(self.options.store_timeout, call).await {
            Ok(result) => result,
            Err(_) => Err(SessionStoreError::Timeout),
        }
    }

    /// A failure of the store or of the session itself: one rate-limited warning per reason (no identifier, no data,
    /// no message of the store) and the same 500 for every session.
    pub(crate) fn fault(&self, reason: &'static str, error: Option<&SessionStoreError>) -> HttpException {
        let kind = match error {
            Some(SessionStoreError::Timeout) => " (the store did not answer in time)",
            Some(SessionStoreError::Exists) => " (the identifier was taken)",
            Some(SessionStoreError::Backend(_)) => " (the store reported a failure)",
            None => "",
        };
        // The message of a backend error is never logged: it is the store's text and could hold an identifier. A store
        // that wants its failures in the log writes them itself.
        let line = || format!("session failure: {reason}{kind}; the request was answered with a 500");
        if let Some(line) = self.warnings.line(reason, Instant::now(), line) {
            Logger::new("SessionMiddleware").warn(line);
        }
        HttpException::internal(SessionFault("session failure")).already_logged()
    }

    /// The warning for a stored record over `max_session_bytes`, at most one a minute, with no identifier.
    fn warn_oversized(&self) {
        let line = || {
            format!(
                "a stored session record is larger than max_session_bytes ({}) and was ignored: the request goes on without a session",
                self.options.max_session_bytes
            )
        };
        if let Some(line) = self.warnings.line("oversized record", Instant::now(), line) {
            Logger::new("SessionMiddleware").warn(line);
        }
    }

    /// The session of a request: the one the cookie names when it verifies and the store has it, a new one in every
    /// other case.
    ///
    /// A cookie that is absent, repeated, unsigned, badly signed, too long or malformed is ignored without a store
    /// access. An identifier that verifies but that the store does not know is never adopted: the new session has a
    /// new identifier. An expired record is destroyed.
    pub(crate) async fn load(self: &Arc<Self>, req: &Request) -> Result<Session, HttpException> {
        let now = now_ms();
        let name = &self.options.name;
        // `Request::cookie` keeps the first of several cookies with one name. Here two means a cookie planted from
        // somewhere else (a sibling subdomain can set cookies for the whole domain), so none is believed.
        let values = req.cookie_values(name, 2, MAX_RAW_COOKIE_BYTES);
        let mut state = State::fresh(now);
        state.had_cookie = !values.is_empty();
        let session = |state: State| Session::new(self.clone(), state);
        if values.len() != 1 {
            return Ok(session(state));
        }
        let Some((id, current)) = self.options.keys.verify(&values[0]) else { return Ok(session(state)) };
        if !valid_id(id) {
            return Ok(session(state));
        }
        let record = self
            .store_call(self.options.store.load(id))
            .await
            .map_err(|e| self.fault("the store failed while loading a session", Some(&e)))?;
        let Some(record) = record else { return Ok(session(state)) };
        // The limit applies to what is read as well as to what is written: a record this large (put there by another
        // writer of a shared store) could never be saved again, so it is no session. It is not deleted and its
        // identifier is not adopted.
        if !fits(&record, self.options.max_session_bytes) {
            self.warn_oversized();
            return Ok(session(state));
        }
        // Likewise a record nested deeper than a write is allowed to nest: it could never be saved again.
        if super::session::too_deep(&record, MAX_RECORD_DEPTH) {
            let line = || {
                format!(
                    "a stored session record is nested deeper than {MAX_RECORD_DEPTH} levels and was ignored: the request goes on without a session"
                )
            };
            if let Some(line) = self.warnings.line("too deep record", Instant::now(), line) {
                Logger::new("SessionMiddleware").warn(line);
            }
            return Ok(session(state));
        }
        match read_record(record, &self.options, now) {
            Loaded::Invalid => Ok(session(state)),
            Loaded::Expired => {
                self.store_call(self.options.store.destroy(id))
                    .await
                    .map_err(|e| self.fault("the store failed while destroying an expired session", Some(&e)))?;
                state.clear_cookie = true;
                Ok(session(state))
            }
            Loaded::Session { data, created_at, last_access } => {
                let mut loaded = State::loaded(id.to_string(), data, created_at, last_access);
                loaded.resign = !current;
                Ok(session(loaded))
            }
        }
    }

    /// The limits of a record the store is asked to keep: its serialized size, and its nesting depth, which must stay
    /// readable (a record deeper than the parser's own limit would be saved and then never loaded).
    fn check_writable(&self, record: &Value) -> Result<(), HttpException> {
        if !fits(record, self.options.max_session_bytes) {
            return Err(self.fault("a session is larger than max_session_bytes and was not saved", None));
        }
        if super::session::too_deep(record, MAX_RECORD_DEPTH) {
            return Err(self.fault("a session is nested deeper than the limit and was not saved", None));
        }
        Ok(())
    }

    /// The time the store is asked to keep a record: the smallest of the idle timeout, what is left of the absolute
    /// timeout and the cookie's remaining life, at least one second.
    fn ttl(&self, state: &State, now: i64) -> Duration {
        let mut ms = self.options.idle_timeout.as_millis() as i64;
        if let Some(absolute) = self.options.absolute_timeout {
            ms = ms.min(state.created_at.saturating_add(absolute.as_millis() as i64).saturating_sub(now));
        }
        if let Some(expires) = state.cookie_expires(&self.options, now) {
            ms = ms.min(expires.saturating_sub(now));
        }
        Duration::from_millis(ms.max(1000) as u64)
    }

    /// The `Set-Cookie` value for session `id`.
    fn cookie_header(&self, id: &str, expires: Option<i64>) -> String {
        let c = &self.options.cookie;
        let mut options = CookieOptions::new().path(c.path.clone()).http_only(c.http_only).secure(c.secure);
        if let Some(domain) = &c.domain {
            options = options.domain(domain.clone());
        }
        if let Some(same_site) = c.same_site {
            options = options.same_site(same_site.as_str());
        }
        if let Some(ms) = expires {
            options = options.expires(chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default());
        }
        // Name, path and domain were validated at startup and the value is `s:` + base64url + `.` + base64, so the
        // panics of `serialize_cookie` cannot be reached.
        serialize_cookie(&self.options.name, &self.options.keys.sign(id), &options)
    }

    /// A header that makes the browser drop the cookie.
    fn clearing_header(&self) -> String {
        let c = &self.options.cookie;
        let mut options = CookieOptions::new().path(c.path.clone()).http_only(c.http_only).secure(c.secure);
        if let Some(domain) = &c.domain {
            options = options.domain(domain.clone());
        }
        if let Some(same_site) = c.same_site {
            options = options.same_site(same_site.as_str());
        }
        let epoch = chrono::DateTime::from_timestamp_millis(0).unwrap_or_default();
        serialize_cookie(&self.options.name, "", &options.expires(epoch))
    }

    /// Writes what the request left in the session and decides on the cookie. `status` is the status of the response,
    /// `None` when the request ended in an error (the response does not exist yet): a new session is then not stored,
    /// because its cookie could not reach the client. `rejected` is set by the router when the pipeline produced an
    /// exception or a guard ended the request, before any filter or interceptor could change the response. A login made
    /// by an `AuthGuard` is discarded when `rejected` is set or there is no response, and kept otherwise, whatever the
    /// status of the response is.
    ///
    /// The order is: destroy the identifiers queued (a login or logout destroys the old one before the new one is
    /// created), then `create` for a new identifier, or `update` / `touch` for an existing one. `update` and `touch`
    /// never create a record, so a session destroyed by another request stays destroyed. A session that this request
    /// loaded is also treated as destroyed when its record is gone at commit time (the queued `destroy` found nothing,
    /// or `update` / `touch` found no record): nothing is created or updated and the cookie is cleared, so a logout
    /// racing a slow request cannot be undone by it.
    ///
    /// Returns the `Set-Cookie` header value, if one has to be sent.
    pub(crate) async fn commit(&self, session: &Session, status: Option<u16>, rejected: bool) -> Result<Option<String>, HttpException> {
        let o = &self.options;
        let now = now_ms();
        let can_set_cookie = status.is_some();
        {
            let mut s = session.state();
            // Whether a guard login stands depends on what the pipeline did, never on the status of the response.
            if let Some(before) = s.before_guard_login.take()
                && (rejected || status.is_none())
            {
                *s = *before;
            }
        }
        let (queued, origin, fresh_login) = {
            let s = session.state();
            (s.destroy.clone(), s.origin.clone(), s.fresh_login)
        };
        let mut origin_gone = false;
        for old in &queued {
            let existed = self
                .store_call(o.store.destroy(old))
                .await
                .map_err(|e| self.fault("the store failed while destroying a session", Some(&e)))?;
            origin_gone |= !existed && origin.as_deref() == Some(old.as_str());
        }
        // A login that kept nothing of the previous session has nothing to bring back from it, so the previous record
        // being gone (a logout in another request, or an eviction by a flood of other sessions) does not stop it: the
        // user just proved who they are. The old identifier was destroyed above all the same.
        if origin_gone && !fresh_login {
            // Another request ended the session while this one ran: it stays ended.
            return Ok((can_set_cookie && session.state().had_cookie).then(|| self.clearing_header()));
        }
        // Everything decided from one view of the state; nothing is awaited while the lock is held.
        let (persisted, modified, clear, resign, had_cookie) = {
            let s = session.state();
            (s.persisted, s.modified(), s.clear_cookie, s.resign, s.had_cookie)
        };
        if !persisted {
            let write = (modified || o.save_uninitialized) && can_set_cookie;
            if !write {
                return Ok((clear && had_cookie && can_set_cookie).then(|| self.clearing_header()));
            }
            // One retry with a new identifier when the first one is taken (a collision of 192-bit identifiers is
            // not expected: it means the store or the generator misbehaves).
            for attempt in 0..2 {
                let (id, record, ttl, expires) = {
                    let mut s = session.state();
                    // The identifier of a new session is drawn here, when it is written, unless the request asked for
                    // it. A draw that failed, now or earlier: nothing is written and no weaker identifier is used.
                    let id = if s.failed { None } else { s.ensure_id().map(str::to_string) };
                    let Some(id) = id else { return Err(self.fault("the random generator failed", None)) };
                    (id, s.record(o, now), self.ttl(&s, now), s.cookie_expires(o, now))
                };
                self.check_writable(&record)?;
                match self.store_call(o.store.create(&id, &record, ttl)).await {
                    Ok(()) => return Ok(Some(self.cookie_header(&id, expires))),
                    // The next attempt draws another identifier.
                    Err(SessionStoreError::Exists) if attempt == 0 => session.state().forget_id(),
                    Err(e) => return Err(self.fault("the store failed while creating a session", Some(&e))),
                }
            }
            return Err(self.fault("two new session identifiers were taken", Some(&SessionStoreError::Exists)));
        }
        let write_full = modified || o.resave;
        let (id, record, ttl, expires) = {
            let s = session.state();
            // An unchanged session is touched with its timestamps only: `touch` may not write the data, so it gets none.
            let record = if write_full { s.record(o, now) } else { s.touch_record(o, now) };
            // A persisted session always has the identifier it was loaded (or created) under.
            (s.current_id().unwrap_or_default().to_string(), record, self.ttl(&s, now), s.cookie_expires(o, now))
        };
        let present = if write_full {
            self.check_writable(&record)?;
            self.store_call(o.store.update(&id, &record, ttl)).await
        } else {
            self.store_call(o.store.touch(&id, &record, ttl)).await
        }
        .map_err(|e| self.fault("the store failed while saving a session", Some(&e)))?;
        if !present {
            // The session was destroyed or expired in the store while this request ran: nothing is written and the
            // cookie is cleared.
            return Ok(can_set_cookie.then(|| self.clearing_header()));
        }
        let set_cookie = o.rolling || (o.cookie.max_age.is_some() && modified) || resign;
        Ok((set_cookie && can_set_cookie).then(|| self.cookie_header(&id, expires)))
    }

    /// Destroys the identifiers a request queued, for a request that was cancelled before it could commit. Failures
    /// are logged like any other store fault and otherwise ignored.
    pub(crate) async fn destroy_queued(self: Arc<Self>, ids: Vec<String>) {
        for id in ids {
            if let Err(e) = self.store_call(self.options.store.destroy(&id)).await {
                let _ = self.fault("the store failed while destroying a session of a cancelled request", Some(&e));
            }
        }
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
