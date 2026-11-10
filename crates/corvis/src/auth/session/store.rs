//! The session store contract.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::Value;

/// Why a store operation failed. The framework logs the kind of failure (at most one line a minute per reason) and
/// never the message of a [`Backend`](SessionStoreError::Backend) error, which is the store's text and could hold a
/// session identifier; a store that wants its failures logged does it itself, without identifiers or session data.
#[derive(Debug, thiserror::Error)]
pub enum SessionStoreError {
    /// [`SessionStore::create`] found the identifier taken.
    #[error("a session with this identifier already exists")]
    Exists,
    /// The store did not answer within the `store_timeout` of the options.
    #[error("the session store did not answer in time")]
    Timeout,
    /// The backend failed (connection lost, query error, a record too large for the store, ...).
    #[error("the session store failed: {0}")]
    Backend(String),
}

impl SessionStoreError {
    /// A backend failure. Keep identifiers and session data out of `message`.
    pub fn backend(message: impl Into<String>) -> Self {
        SessionStoreError::Backend(message.into())
    }
}

/// Where sessions are kept: the contract of an `express-session` store (`get`, `set`, `touch`, `destroy`), with `set`
/// split so that two guarantees are the store's to give:
///
/// - [`create`](Self::create) never overwrites a record (two requests cannot end up with the same identifier);
/// - [`update`](Self::update) and [`touch`](Self::touch) never create a record (a session destroyed by a logout in
///   another request cannot be brought back by a slower request that started before it).
///
/// A record is the JSON object `express-session` stores: the session data, the `cookie` object and the reserved key
/// `__corvis` with the timestamps the middleware enforces expiry from. The `ttl` of a write is how long the record
/// is worth keeping; a store should drop it after that, but expiry does not depend on it: the middleware refuses a
/// record whose timestamps are past the idle or the absolute timeout whatever the store kept.
///
/// Every call is bounded by the `store_timeout` of the options. Calls must be safe to repeat (`destroy` of a missing
/// session is not an error).
///
/// The guarantees above are what keep a slow request from undoing a logout, so a store must give them atomically
/// (a compare-and-set, `SET ... XX`, a transaction): checking that the record exists and then writing it in two steps
/// reopens the window. `destroy` reports whether a record was removed, which is how the middleware learns that the
/// session was ended by another request while this one ran. `touch` changes the expiry and the timestamps only, never
/// the data: a request that changed nothing must not overwrite what a concurrent request saved, so there is no default
/// for it.
///
/// ```ignore
/// #[async_trait]
/// impl SessionStore for MyStore {
///     async fn load(&self, id: &str) -> Result<Option<Value>, SessionStoreError> { /* GET sess:<id> */ }
///     async fn create(&self, id: &str, record: &Value, ttl: Duration) -> Result<(), SessionStoreError> { /* SET NX */ }
///     async fn update(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError> { /* SET XX */ }
///     async fn touch(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError> { /* EXPIRE only */ }
///     async fn destroy(&self, id: &str) -> Result<bool, SessionStoreError> { /* DEL > 0 */ }
/// }
/// ```
#[async_trait]
pub trait SessionStore: Send + Sync + 'static {
    /// The record of `id`, `None` when there is none.
    async fn load(&self, id: &str) -> Result<Option<Value>, SessionStoreError>;

    /// Writes a new record. Fails with [`SessionStoreError::Exists`] when the identifier is taken; the existing
    /// record is left as it is.
    async fn create(&self, id: &str, record: &Value, ttl: Duration) -> Result<(), SessionStoreError>;

    /// Replaces the record of `id`. `Ok(false)` when there is no such session: nothing is written.
    async fn update(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError>;

    /// Renews the record of `id` after a request that did not change the session: the `ttl`, and the timestamps of
    /// `record` (`cookie`, `__corvis`), and nothing else. It must not write the data of `record`: that copy was read
    /// when the request began and may be stale. `Ok(false)` when there is no such session: nothing is written.
    async fn touch(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError>;

    /// Removes the record of `id`: `Ok(true)` when there was one, `Ok(false)` when it was already gone or expired
    /// (not an error).
    async fn destroy(&self, id: &str) -> Result<bool, SessionStoreError>;

    /// `true` for a store that keeps sessions in this process (lost on restart, not shared between instances): the
    /// application logs a startup warning for it.
    fn in_process(&self) -> bool {
        false
    }

    /// Checked when the application starts: `Err` names a configuration that cannot work.
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Milliseconds since the Unix epoch: the wall clock, so that a record is readable by another process, moved by the
/// offset of tokio's clock from the real one, which is zero unless a test paused the clock and advanced it.
pub(crate) fn now_ms() -> i64 {
    let wall = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let tokio_now = tokio::time::Instant::now().into_std();
    let real_now = std::time::Instant::now();
    let offset = if tokio_now >= real_now {
        tokio_now.duration_since(real_now).as_millis() as i64
    } else {
        -(real_now.duration_since(tokio_now).as_millis() as i64)
    };
    wall + offset
}
