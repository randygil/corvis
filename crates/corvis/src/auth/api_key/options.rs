//! The options of the API key strategy and their startup validation.

use std::time::Duration;

use http::HeaderName;

/// The default of `max_key_bytes`.
pub const DEFAULT_MAX_KEY_BYTES: usize = 512;

/// The largest `max_key_bytes` accepted.
pub const MAX_KEY_BYTES_LIMIT: usize = 4096;

/// The most sources one strategy can read.
pub const MAX_SOURCES: usize = 8;

/// The default of `validator_timeout`.
pub const DEFAULT_VALIDATOR_TIMEOUT: Duration = Duration::from_secs(5);

const MIN_VALIDATOR_TIMEOUT: Duration = Duration::from_millis(1);
const MAX_VALIDATOR_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_PREFIX_BYTES: usize = 64;
const MAX_NAME_BYTES: usize = 128;

/// A place the key is read from. The strategy reads only the sources it was configured with: never the body or a
/// cookie.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKeySource {
    pub(super) kind: SourceKind,
    pub(super) prefix: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum SourceKind {
    Header(String),
    Bearer,
    Query(String),
}

/// Prints where the source reads, never a request value.
impl std::fmt::Debug for ApiKeySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            SourceKind::Header(name) => write!(f, "ApiKeySource::header({name:?})")?,
            SourceKind::Bearer => f.write_str("ApiKeySource::bearer()")?,
            SourceKind::Query(name) => write!(f, "ApiKeySource::query({name:?})")?,
        }
        match &self.prefix {
            Some(prefix) => write!(f, ".prefix({prefix:?})"),
            None => Ok(()),
        }
    }
}

impl ApiKeySource {
    /// The header `name` (compared without regard to case, `X-Api-Key`). Repeated headers are joined with `", "` as
    /// Node does, so two values never reach the validator as the first one only. `Cookie` is refused at startup.
    pub fn header(name: impl Into<String>) -> Self {
        ApiKeySource { kind: SourceKind::Header(name.into()), prefix: None }
    }

    /// `Authorization: Bearer <key>`, parsed as `passport-jwt` parses its bearer header: the scheme ignoring case,
    /// the key is the next run of non-space characters. Only the first `Authorization` header is read. When the same
    /// route also accepts JWTs, a chain with the JWT strategy first keeps the two apart.
    pub fn bearer() -> Self {
        ApiKeySource { kind: SourceKind::Bearer, prefix: None }
    }

    /// The query parameter `name`, only when the value is a single string. Keys in URLs end up in proxy and access
    /// logs, browser history and `Referer` headers: the strategy logs a warning at startup, and the parameter is
    /// hidden in the URL `LoggingInterceptor` logs, but nothing in front of the application is.
    pub fn query(name: impl Into<String>) -> Self {
        ApiKeySource { kind: SourceKind::Query(name.into()), prefix: None }
    }

    /// For a header source: the value must start with `prefix`, compared byte by byte and case-sensitively, and the
    /// prefix is removed (`passport-headerapikey` builds an unescaped regular expression from it instead). A value
    /// without it is no key. The prefix is printable ASCII, 1 to 64 bytes.
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }
}

/// Where the API key strategy reads the key and its limits. [`Default`] reads the `X-Api-Key` header only.
///
/// ```ignore
/// ApiKeyStrategyOptions::new()
///     .source(ApiKeySource::header("X-Api-Key"))
///     .source(ApiKeySource::header("Authorization").prefix("Api-Key "))
/// ```
///
/// When more than one source carries a key in a request, the keys must be equal; two different keys fail the
/// strategy without calling the validator.
#[derive(Debug, Clone)]
pub struct ApiKeyStrategyOptions {
    pub(super) sources: Vec<ApiKeySource>,
    pub(super) max_key_bytes: usize,
    pub(super) validator_timeout: Duration,
}

impl Default for ApiKeyStrategyOptions {
    /// One source: the `X-Api-Key` header with no prefix.
    fn default() -> Self {
        ApiKeyStrategyOptions {
            sources: vec![ApiKeySource::header("X-Api-Key")],
            max_key_bytes: DEFAULT_MAX_KEY_BYTES,
            validator_timeout: DEFAULT_VALIDATOR_TIMEOUT,
        }
    }
}

impl ApiKeyStrategyOptions {
    /// No source: add at least one with [`source`](Self::source), or the application does not start.
    pub fn new() -> Self {
        ApiKeyStrategyOptions { sources: Vec::new(), ..Self::default() }
    }

    /// Adds a source. At most 8; the order does not matter for the result (see the conflict rule above).
    pub fn source(mut self, source: ApiKeySource) -> Self {
        self.sources.push(source);
        self
    }

    /// The longest key accepted, 512 bytes by default, 1 to 4096. A longer key fails the strategy before anything
    /// is hashed, compared or handed to the validator.
    pub fn max_key_bytes(mut self, max: usize) -> Self {
        self.max_key_bytes = max;
        self
    }

    /// How long the validator may run, 5 seconds by default, 1 ms to 300 s. A validator that takes longer is
    /// answered with a `503 Service Unavailable` that is the same for every key.
    pub fn validator_timeout(mut self, timeout: Duration) -> Self {
        self.validator_timeout = timeout;
        self
    }
}

/// A source after validation.
#[derive(Debug, Clone)]
pub(super) enum Place {
    Header { name: HeaderName, prefix: Option<String> },
    Bearer,
    Query { name: String },
}

/// The options after validation.
pub(super) struct Resolved {
    pub(super) places: Vec<Place>,
    pub(super) max_key_bytes: usize,
    pub(super) validator_timeout: Duration,
    /// The query parameters that are read, for the startup warning.
    pub(super) query_names: Vec<String>,
}

impl Resolved {
    pub(super) fn new(options: &ApiKeyStrategyOptions) -> Result<Self, String> {
        if options.sources.is_empty() {
            return Err("no source is configured: add at least one ApiKeySource".to_string());
        }
        if options.sources.len() > MAX_SOURCES {
            return Err(format!("{} sources are configured, at most {MAX_SOURCES} are allowed", options.sources.len()));
        }
        if !(1..=MAX_KEY_BYTES_LIMIT).contains(&options.max_key_bytes) {
            return Err(format!("max_key_bytes must be between 1 and {MAX_KEY_BYTES_LIMIT} (got {})", options.max_key_bytes));
        }
        if !(MIN_VALIDATOR_TIMEOUT..=MAX_VALIDATOR_TIMEOUT).contains(&options.validator_timeout) {
            return Err("validator_timeout must be between 1 ms and 300 s".to_string());
        }
        let mut places = Vec::new();
        let mut query_names = Vec::new();
        for source in &options.sources {
            let place = match &source.kind {
                SourceKind::Header(name) => {
                    if name.is_empty() {
                        return Err("the header source has an empty header name".to_string());
                    }
                    let parsed = HeaderName::from_bytes(name.as_bytes())
                        .map_err(|_| format!("the header source {name:?} is not a valid HTTP header name"))?;
                    if matches!(parsed.as_str(), "cookie" | "cookie2" | "set-cookie") {
                        return Err(format!("the header source {name:?} is a cookie header: cookies are never a source"));
                    }
                    let prefix = match &source.prefix {
                        None => None,
                        Some(prefix) => Some(check_prefix(name, prefix)?),
                    };
                    Place::Header { name: parsed, prefix }
                }
                SourceKind::Bearer => {
                    if source.prefix.is_some() {
                        return Err("a prefix applies to header sources only, not to the bearer source".to_string());
                    }
                    Place::Bearer
                }
                SourceKind::Query(name) => {
                    if name.is_empty() {
                        return Err("the query source has an empty parameter name".to_string());
                    }
                    if name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
                        return Err("the query source has a parameter name that is too long or holds a control character".to_string());
                    }
                    if source.prefix.is_some() {
                        return Err("a prefix applies to header sources only, not to the query source".to_string());
                    }
                    query_names.push(name.clone());
                    Place::Query { name: name.clone() }
                }
            };
            places.push(place);
        }
        Ok(Resolved { places, max_key_bytes: options.max_key_bytes, validator_timeout: options.validator_timeout, query_names })
    }
}

fn check_prefix(header: &str, prefix: &str) -> Result<String, String> {
    if prefix.is_empty() {
        return Err(format!("the header source {header:?} has an empty prefix"));
    }
    if prefix.len() > MAX_PREFIX_BYTES || !prefix.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err(format!("the prefix of the header source {header:?} must be 1 to {MAX_PREFIX_BYTES} printable ASCII characters"));
    }
    Ok(prefix.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(options: ApiKeyStrategyOptions) -> String {
        match Resolved::new(&options) {
            Ok(_) => panic!("the options were accepted"),
            Err(e) => e,
        }
    }

    #[test]
    fn the_defaults_read_one_header() {
        let r = Resolved::new(&ApiKeyStrategyOptions::default()).unwrap();
        assert_eq!(r.places.len(), 1);
        assert!(matches!(&r.places[0], Place::Header { name, prefix: None } if name == "x-api-key"));
        assert_eq!(r.max_key_bytes, 512);
        assert!(r.query_names.is_empty());
    }

    #[test]
    fn invalid_options_are_refused() {
        assert!(err(ApiKeyStrategyOptions::new()).contains("no source"));
        assert!(err(ApiKeyStrategyOptions::new().source(ApiKeySource::header(""))).contains("empty header name"));
        assert!(err(ApiKeyStrategyOptions::new().source(ApiKeySource::header("bad name"))).contains("not a valid HTTP header name"));
        assert!(err(ApiKeyStrategyOptions::new().source(ApiKeySource::header("Cookie"))).contains("cookie"));
        assert!(err(ApiKeyStrategyOptions::new().source(ApiKeySource::header("X-Key").prefix(""))).contains("empty prefix"));
        assert!(err(ApiKeyStrategyOptions::new().source(ApiKeySource::header("X-Key").prefix("é"))).contains("printable ASCII"));
        assert!(err(ApiKeyStrategyOptions::new().source(ApiKeySource::header("X-Key").prefix("a\r\n"))).contains("printable ASCII"));
        assert!(err(ApiKeyStrategyOptions::new().source(ApiKeySource::header("X-Key").prefix("p".repeat(65)))).contains("printable ASCII"));
        assert!(err(ApiKeyStrategyOptions::new().source(ApiKeySource::query(""))).contains("empty parameter name"));
        assert!(err(ApiKeyStrategyOptions::new().source(ApiKeySource::query("a\nb"))).contains("control character"));
        assert!(err(ApiKeyStrategyOptions::new().source(ApiKeySource::bearer().prefix("x"))).contains("header sources only"));
        assert!(err(ApiKeyStrategyOptions::new().source(ApiKeySource::query("k").prefix("x"))).contains("header sources only"));
        let nine = (0..9).fold(ApiKeyStrategyOptions::new(), |o, i| o.source(ApiKeySource::header(format!("x-key-{i}"))));
        assert!(err(nine).contains("at most 8"));
        for max in [0, 4097, 5000, usize::MAX] {
            assert!(err(ApiKeyStrategyOptions::default().max_key_bytes(max)).contains("max_key_bytes"), "{max}");
        }
        for timeout in [Duration::ZERO, Duration::from_secs(301)] {
            assert!(err(ApiKeyStrategyOptions::default().validator_timeout(timeout)).contains("validator_timeout"));
        }
    }

    #[test]
    fn valid_options_are_accepted_and_query_names_are_listed() {
        let eight = (0..8).fold(ApiKeyStrategyOptions::new(), |o, i| o.source(ApiKeySource::header(format!("x-key-{i}"))));
        assert!(Resolved::new(&eight).is_ok());
        for max in [1, 512, 4096] {
            assert!(Resolved::new(&ApiKeyStrategyOptions::default().max_key_bytes(max)).is_ok());
        }
        let options = ApiKeyStrategyOptions::new()
            .source(ApiKeySource::header("Authorization").prefix("Api-Key "))
            .source(ApiKeySource::bearer())
            .source(ApiKeySource::query("api_key"));
        let r = Resolved::new(&options).unwrap();
        assert_eq!(r.query_names, ["api_key"]);
        assert!(matches!(&r.places[0], Place::Header { prefix: Some(p), .. } if p == "Api-Key "));
    }

    #[test]
    fn debug_output_shows_the_places_only() {
        let text = format!("{:?}", ApiKeyStrategyOptions::default().source(ApiKeySource::query("api_key")));
        assert!(text.contains("x-api-key") || text.contains("X-Api-Key"));
        assert!(text.contains("api_key"));
    }
}
