//! Session options and their startup checks.

use std::sync::Arc;
use std::time::Duration;

use super::memory::MemorySessionStore;
use super::store::SessionStore;
use crate::http::CookieKeys;
use crate::http::res::{valid_cookie_domain, valid_cookie_name, valid_cookie_path};

/// The default idle timeout: 24 hours since the last request of the session.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(24 * 3600);
/// The default absolute timeout: 7 days since the session identifier was generated.
pub const DEFAULT_ABSOLUTE_TIMEOUT: Duration = Duration::from_secs(7 * 24 * 3600);
/// The shortest idle timeout.
pub const MIN_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// The longest absolute timeout (and cookie `max_age`).
pub const MAX_TIMEOUT: Duration = Duration::from_secs(400 * 24 * 3600);
/// The default limit on one serialized session record, 16 KiB.
pub const DEFAULT_MAX_SESSION_BYTES: usize = 16 * 1024;
/// The allowed range of `max_session_bytes`.
pub const MAX_SESSION_BYTES_RANGE: std::ops::RangeInclusive<usize> = 1024..=(1024 * 1024);
/// The default bound on one store call.
pub const DEFAULT_STORE_TIMEOUT: Duration = Duration::from_secs(5);
/// The allowed range of `store_timeout`.
pub const STORE_TIMEOUT_RANGE: std::ops::RangeInclusive<Duration> = Duration::from_millis(100)..=Duration::from_secs(60);

fn err(message: impl Into<String>) -> SessionOptionsError {
    SessionOptionsError::new(message)
}

const MAX_NAME_BYTES: usize = 128;
const MAX_PATH_BYTES: usize = 1024;
const MAX_DOMAIN_BYTES: usize = 253;

/// The `SameSite` attribute of the session cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    /// Sent with cross-site requests; it requires `Secure`.
    None,
}

impl SameSite {
    /// The value of the `SameSite` attribute.
    pub(crate) fn attribute(self) -> &'static str {
        match self {
            SameSite::Strict => "Strict",
            SameSite::Lax => "Lax",
            SameSite::None => "None",
        }
    }

    /// The lowercase form `express-session` keeps in the `cookie` object of the record.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            SameSite::Strict => "strict",
            SameSite::Lax => "lax",
            SameSite::None => "none",
        }
    }
}

/// The attributes of the session cookie. The defaults are `Path=/`, `HttpOnly`, `SameSite=Lax` and `Secure`, without
/// `Domain` and without an expiry (a browser-session cookie). `express-session` defaults to no `SameSite` and no
/// `Secure`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCookie {
    pub(crate) path: String,
    pub(crate) domain: Option<String>,
    pub(crate) http_only: bool,
    pub(crate) secure: bool,
    pub(crate) same_site: Option<SameSite>,
    pub(crate) max_age: Option<Duration>,
}

impl Default for SessionCookie {
    fn default() -> Self {
        SessionCookie { path: "/".into(), domain: None, http_only: true, secure: true, same_site: Some(SameSite::Lax), max_age: None }
    }
}

impl SessionCookie {
    /// `Path` (default `/`). The session middleware does nothing for a request whose path does not start with it.
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    /// `Domain` (default: none, so only the host that set the cookie gets it back). A `Domain` also gives the cookie
    /// to every subdomain: prefer none.
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// `HttpOnly` (default on). Turning it off lets scripts of the page read the cookie, and logs a startup warning.
    pub fn http_only(mut self, on: bool) -> Self {
        self.http_only = on;
        self
    }

    /// `Secure` (default on): browsers send the cookie over HTTPS only. Turn it off for plain-HTTP development; it
    /// logs a startup warning. The cookie is sent with its attributes on every connection, the browser decides.
    pub fn secure(mut self, on: bool) -> Self {
        self.secure = on;
        self
    }

    /// `SameSite` (default `Lax`).
    pub fn same_site(mut self, same_site: SameSite) -> Self {
        self.same_site = Some(same_site);
        self
    }

    /// No `SameSite` attribute, as `express-session` sends by default. Browsers then apply their own default.
    pub fn without_same_site(mut self) -> Self {
        self.same_site = None;
        self
    }

    /// Makes the cookie persistent: `Expires` is set that long after each response that sets the cookie. At most the
    /// absolute timeout.
    pub fn max_age(mut self, max_age: Duration) -> Self {
        self.max_age = Some(max_age);
        self
    }
}

/// A configuration the application refuses to start with.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SessionOptionsError(pub(crate) String);

impl SessionOptionsError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        SessionOptionsError(message.into())
    }
}

/// The options of [`SessionModule`](super::SessionModule).
///
/// ```ignore
/// SessionOptions::new(CookieKeys::new([secret])?)
///     .cookie(SessionCookie::default().secure(false))      // plain-HTTP development
///     .idle_timeout(Duration::from_secs(3600))
///     .store(MemorySessionStore::new().max_sessions(10_000))
/// ```
#[derive(Clone)]
pub struct SessionOptions {
    pub(crate) keys: CookieKeys,
    pub(crate) name: String,
    pub(crate) cookie: SessionCookie,
    pub(crate) idle_timeout: Duration,
    pub(crate) absolute_timeout: Option<Duration>,
    pub(crate) rolling: bool,
    pub(crate) resave: bool,
    pub(crate) save_uninitialized: bool,
    pub(crate) keep_session_info_on_login: bool,
    pub(crate) max_session_bytes: usize,
    pub(crate) store_timeout: Duration,
    pub(crate) store: Arc<dyn SessionStore>,
}

/// Prints no secret.
impl std::fmt::Debug for SessionOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionOptions").field("name", &self.name).field("cookie", &self.cookie).finish_non_exhaustive()
    }
}

impl SessionOptions {
    /// Options with the defaults of the specification and the in-memory store, signing with `keys`.
    pub fn new(keys: CookieKeys) -> Self {
        SessionOptions {
            keys,
            name: "connect.sid".into(),
            cookie: SessionCookie::default(),
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            absolute_timeout: Some(DEFAULT_ABSOLUTE_TIMEOUT),
            rolling: false,
            resave: false,
            save_uninitialized: false,
            keep_session_info_on_login: false,
            max_session_bytes: DEFAULT_MAX_SESSION_BYTES,
            store_timeout: DEFAULT_STORE_TIMEOUT,
            store: Arc::new(MemorySessionStore::new()),
        }
    }

    /// Like [`new`](Self::new) with keys built from `secrets` (1 to 8, at least 32 bytes each, the first one signs).
    /// The error names the session secret and its position and length, never the secret.
    pub fn with_secrets<I, S>(secrets: I) -> Result<Self, SessionOptionsError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<[u8]>,
    {
        use crate::http::CookieKeysError;
        match CookieKeys::new(secrets) {
            Ok(keys) => Ok(Self::new(keys)),
            Err(CookieKeysError::TooShort { index, len }) => Err(SessionOptionsError::new(format!(
                "the session secret at position {index} has {len} bytes: the minimum is {} bytes",
                crate::http::signed_cookie::MIN_SECRET_BYTES
            ))),
            Err(e) => Err(SessionOptionsError::new(format!("the session secrets are not valid: {e}"))),
        }
    }

    /// The cookie name (default `connect.sid`, the one `express-session` uses, so a session written by the original
    /// can be read). A `__Host-` or `__Secure-` prefix is honored: it is checked against the cookie attributes.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn cookie(mut self, cookie: SessionCookie) -> Self {
        self.cookie = cookie;
        self
    }

    /// How long a session may go unused (default 24 hours; 60 seconds up to the absolute timeout). Enforced by the
    /// middleware from the record's timestamps.
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// The longest a session lives from the moment its identifier was generated, however active it is (default
    /// 7 days, at most 400 days). A login and a logout generate a new identifier and so start a new period.
    pub fn absolute_timeout(mut self, timeout: Duration) -> Self {
        self.absolute_timeout = Some(timeout);
        self
    }

    /// Sessions have no maximum age. Explicit, and logs a startup warning: a stolen cookie then works for as long as
    /// somebody keeps using it.
    pub fn no_absolute_timeout(mut self) -> Self {
        self.absolute_timeout = None;
        self
    }

    /// Sets the cookie on every response of an existing session (default off), which renews its expiry in the
    /// browser. The idle timeout counts in either case.
    pub fn rolling(mut self, on: bool) -> Self {
        self.rolling = on;
        self
    }

    /// Writes the session back on every request even when it did not change (default off).
    pub fn resave(mut self, on: bool) -> Self {
        self.resave = on;
        self
    }

    /// Saves a new session that was never modified (default off): an anonymous visit creates no session and no
    /// cookie.
    pub fn save_uninitialized(mut self, on: bool) -> Self {
        self.save_uninitialized = on;
        self
    }

    /// Keeps the session data of before a login or a logout (default off: it is dropped, as `passport` does).
    pub fn keep_session_info_on_login(mut self, on: bool) -> Self {
        self.keep_session_info_on_login = on;
        self
    }

    /// The most bytes one serialized session record may have (default 16 KiB; 1 KiB to 1 MiB). A larger session is
    /// not written and the response is a 500.
    pub fn max_session_bytes(mut self, max: usize) -> Self {
        self.max_session_bytes = max;
        self
    }

    /// How long one store call may take (default 5 seconds; 100 ms to 60 s). A call that takes longer fails like a
    /// store error.
    pub fn store_timeout(mut self, timeout: Duration) -> Self {
        self.store_timeout = timeout;
        self
    }

    /// The store (default: [`MemorySessionStore`] with its default bounds).
    pub fn store(mut self, store: impl SessionStore) -> Self {
        self.store = Arc::new(store);
        self
    }

    /// Like [`store`](Self::store) for a store that is already shared.
    pub fn store_arc(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.store = store;
        self
    }

    /// Checks everything that can be checked before the first request. `Ok` carries the startup notes to log.
    pub(crate) fn validate(&self) -> Result<Vec<String>, SessionOptionsError> {
        let name = &self.name;
        if !valid_cookie_name(name) || name.len() > MAX_NAME_BYTES {
            return Err(err(format!(
                "the session cookie name is not valid: it must be 1 to {MAX_NAME_BYTES} bytes without spaces, ';', '=' or control characters"
            )));
        }
        let c = &self.cookie;
        if !valid_cookie_path(&c.path) || c.path.len() > MAX_PATH_BYTES || !c.path.starts_with('/') {
            return Err(err(
                "the session cookie path is not valid: it must start with '/', have no ';' or control characters and be at most 1024 bytes",
            ));
        }
        if let Some(domain) = &c.domain
            && (!valid_cookie_domain(domain) || domain.len() > MAX_DOMAIN_BYTES)
        {
            return Err(err("the session cookie domain is not valid"));
        }
        if c.same_site == Some(SameSite::None) && !c.secure {
            return Err(err(
                "the session cookie has same_site(SameSite::None) and secure(false): browsers refuse SameSite=None without Secure; set secure(true) or another SameSite",
            ));
        }
        if name.starts_with("__Host-") && (!c.secure || c.domain.is_some() || c.path != "/") {
            return Err(err(
                "the session cookie name starts with __Host-, which requires Secure, Path=/ and no Domain: the cookie has secure(false), a domain or another path",
            ));
        }
        if name.starts_with("__Secure-") && !c.secure {
            return Err(err("the session cookie name starts with __Secure-, which requires Secure: the cookie has secure(false)"));
        }
        let absolute_limit = self.absolute_timeout.unwrap_or(MAX_TIMEOUT);
        if let Some(absolute) = self.absolute_timeout
            && (absolute < MIN_IDLE_TIMEOUT || absolute > MAX_TIMEOUT)
        {
            return Err(err(format!(
                "the session absolute_timeout ({}s) must be between {}s and {}s",
                absolute.as_secs(),
                MIN_IDLE_TIMEOUT.as_secs(),
                MAX_TIMEOUT.as_secs()
            )));
        }
        if self.idle_timeout < MIN_IDLE_TIMEOUT || self.idle_timeout > absolute_limit {
            return Err(err(format!(
                "the session idle_timeout ({}s) must be between {}s and the absolute timeout ({}s)",
                self.idle_timeout.as_secs(),
                MIN_IDLE_TIMEOUT.as_secs(),
                absolute_limit.as_secs()
            )));
        }
        if let Some(max_age) = c.max_age
            && (max_age.is_zero() || max_age > absolute_limit)
        {
            return Err(err(format!(
                "the session cookie max_age ({}s) must be positive and at most the absolute timeout ({}s)",
                max_age.as_secs(),
                absolute_limit.as_secs()
            )));
        }
        if !MAX_SESSION_BYTES_RANGE.contains(&self.max_session_bytes) {
            return Err(err(format!(
                "the session max_session_bytes ({}) must be between {} and {}",
                self.max_session_bytes,
                MAX_SESSION_BYTES_RANGE.start(),
                MAX_SESSION_BYTES_RANGE.end()
            )));
        }
        if !STORE_TIMEOUT_RANGE.contains(&self.store_timeout) {
            return Err(err(format!(
                "the session store_timeout ({}ms) must be between {}ms and {}ms",
                self.store_timeout.as_millis(),
                STORE_TIMEOUT_RANGE.start().as_millis(),
                STORE_TIMEOUT_RANGE.end().as_millis()
            )));
        }
        self.store.validate().map_err(SessionOptionsError)?;

        let mut notes = Vec::new();
        if !c.secure {
            notes.push(
                "the session cookie is not Secure: it is sent over plain HTTP. Use secure(false) for local development only".to_string(),
            );
        }
        if !c.http_only {
            notes.push("the session cookie is not HttpOnly: scripts of the page can read it".to_string());
        }
        if self.absolute_timeout.is_none() {
            notes.push(
                "sessions have no maximum age (no_absolute_timeout): a stolen session cookie works for as long as somebody keeps using it"
                    .to_string(),
            );
        }
        if self.store.in_process() {
            notes.push("sessions are kept in memory: they are lost on restart and not shared between instances (development and single-process use only)".to_string());
        }
        Ok(notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn options() -> SessionOptions {
        SessionOptions::with_secrets([SECRET]).unwrap()
    }

    fn problem(o: SessionOptions) -> String {
        o.validate().unwrap_err().to_string()
    }

    #[test]
    fn the_defaults_are_valid_and_note_the_memory_store() {
        let notes = options().validate().unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("memory"), "{notes:?}");
    }

    #[test]
    fn a_short_secret_names_the_session_secret_without_printing_it() {
        let e = SessionOptions::with_secrets(["twenty-bytes-secret!!"]).unwrap_err().to_string();
        assert!(e.contains("session secret") && e.contains("32") && e.contains("21 bytes"), "{e}");
        assert!(!e.contains("twenty-bytes"), "{e}");
    }

    #[test]
    fn same_site_none_needs_secure() {
        let e = problem(options().cookie(SessionCookie::default().same_site(SameSite::None).secure(false)));
        assert!(e.contains("same_site") && e.contains("secure"), "{e}");
        assert!(options().cookie(SessionCookie::default().same_site(SameSite::None)).validate().is_ok());
    }

    #[test]
    fn cookie_prefixes_are_checked() {
        let host = options().name("__Host-sid");
        assert!(host.clone().validate().is_ok());
        assert!(problem(host.clone().cookie(SessionCookie::default().domain("example.com"))).contains("__Host-"));
        assert!(problem(host.clone().cookie(SessionCookie::default().path("/app"))).contains("__Host-"));
        assert!(problem(host.cookie(SessionCookie::default().secure(false))).contains("__Host-"));
        let secure = options().name("__Secure-sid");
        assert!(secure.clone().validate().is_ok());
        assert!(problem(secure.cookie(SessionCookie::default().secure(false))).contains("__Secure-"));
    }

    #[test]
    fn names_paths_and_domains_that_could_panic_later_are_refused() {
        for name in ["a;b", "a b", "", "a=b", "caf\u{e9}", "line\nbreak"] {
            assert!(options().name(name).validate().is_err(), "{name:?}");
        }
        for path in ["app", "/a;b", "/a\nb", "/caf\u{e9}"] {
            assert!(options().cookie(SessionCookie::default().path(path)).validate().is_err(), "{path:?}");
        }
        for domain in ["", "a b", "-x.com", "a..b"] {
            assert!(options().cookie(SessionCookie::default().domain(domain)).validate().is_err(), "{domain:?}");
        }
    }

    #[test]
    fn timeouts_are_bounded() {
        assert!(problem(options().idle_timeout(Duration::from_secs(10))).contains("idle_timeout"));
        assert!(problem(options().idle_timeout(DEFAULT_ABSOLUTE_TIMEOUT + Duration::from_secs(1))).contains("idle_timeout"));
        assert!(problem(options().absolute_timeout(MAX_TIMEOUT + Duration::from_secs(1))).contains("absolute_timeout"));
        assert!(problem(options().cookie(SessionCookie::default().max_age(DEFAULT_ABSOLUTE_TIMEOUT * 2))).contains("max_age"));
        assert!(problem(options().max_session_bytes(10)).contains("max_session_bytes"));
        assert!(problem(options().max_session_bytes(2 << 20)).contains("max_session_bytes"));
        assert!(problem(options().store_timeout(Duration::from_millis(1))).contains("store_timeout"));
        assert!(problem(options().store(MemorySessionStore::new().max_sessions(0))).contains("max_sessions"));
        assert!(problem(options().store(MemorySessionStore::new().max_total_bytes(0))).contains("max_total_bytes"));
    }

    #[test]
    fn unsafe_choices_log_a_note_each() {
        let notes = options().cookie(SessionCookie::default().secure(false).http_only(false)).no_absolute_timeout().validate().unwrap();
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("not Secure")));
        assert!(notes.iter().any(|n| n.contains("not HttpOnly")));
        assert!(notes.iter().any(|n| n.contains("no maximum age")));
    }

    #[test]
    fn debug_output_holds_no_secret() {
        assert!(!format!("{:?}", options()).contains("0123456789"));
    }
}
