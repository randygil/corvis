//! `ApiKeyStrategy`: reads the key from the configured sources and hands it to the application's validator.

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::BoxFuture;
use http::HeaderMap;
use serde::Serialize;
use serde_json::{Value, json};

use super::key::{ApiKey, constant_time_eq};
use super::options::{ApiKeyStrategyOptions, Place, Resolved};
use crate::auth::auth_header;
use crate::auth::outcome::{user_outcome, with_deadline};
use crate::auth::warn_limit::WarnLimiter;
use crate::auth::{AuthenticateOptions, Strategy, StrategyOutcome};
use crate::common::context::ExecutionContext;
use crate::di::{DiError, Hooks, Injectable, ResolveCtx};
use crate::http::body_parser::latin1;
use crate::http::{HttpException, Request};
use crate::logger::Logger;

/// The failure reasons, the only texts the strategy ever reports. None repeats request data.
const MISSING: &str = "Missing API Key";
const CONFLICT: &str = "Conflicting API keys";
const TOO_LONG: &str = "API key too long";
const BAD_CHARACTERS: &str = "Invalid API key characters";
const AMBIGUOUS: &str = "Ambiguous API key";

/// The class `PassportStrategy(Strategy)` of `passport-headerapikey` is written as: the options and `validate`.
/// `validate` runs only for a key that was found in exactly one place (or the same key in several), within the
/// limits of the options.
///
/// Keep keys out of your database: store [`ApiKey::sha256_hex`] of each key next to a public id, look the record up
/// by that id and compare with [`ExpectedApiKey::matches_or_dummy`](super::ExpectedApiKey::matches_or_dummy). Answer
/// `Ok(None)` for every key that is not valid (unknown, revoked, expired): the response is the same 401 for all of
/// them. Answer an infrastructure failure (database down) with an error: the strategy turns any error of status 500
/// or above into the same body for every key. Keep keys out of the errors and logs of the validator.
///
/// ```ignore
/// #[injectable]
/// pub struct ServiceKeys { keys: Arc<KeysRepository> }
///
/// #[async_trait]
/// impl ApiKeyValidator for ServiceKeys {
///     type User = Service;
///     async fn validate(&self, _: &Request, key: &ApiKey) -> Result<Option<Service>, HttpException> {
///         let Some(id) = key.id('.') else { return Ok(None) };
///         let record = self.keys.find(id).await.map_err(|_| ServiceUnavailableException::empty())?;
///         let stored = record.as_ref().map(|r| &r.expected);
///         Ok(ExpectedApiKey::matches_or_dummy(stored, key).then(|| record.map(|r| r.service)).flatten())
///     }
/// }
/// ```
#[async_trait]
pub trait ApiKeyValidator: Injectable {
    /// The name routes use: `AuthGuard::new("api-key")`. The original names its strategy `headerapikey`; set that
    /// here to port a configuration, or another name to have several API key strategies.
    const NAME: &'static str = "api-key";

    /// The user handed to the handler (`User<Self::User>`). Do not put the key, or its digest, in it.
    type User: Serialize + Clone + Send + Sync + 'static;

    /// The options, read once when the application starts. The default reads the `X-Api-Key` header only.
    fn options(&self) -> ApiKeyStrategyOptions {
        ApiKeyStrategyOptions::default()
    }

    /// `validate(request, key)`: the user, `None` to refuse the key (401), or an exception. A user that is `false`,
    /// `null`, `0` or `""` once serialized is not a user: it is a failure like `None` (and logs a warning), so write
    /// `Ok(matches.then_some(user))`, never `Ok(Some(matches))`. A 4xx exception is
    /// passed on as it is (use it only for a deliberate answer: it tells the client about the key); a status of 500
    /// or above, or below 400, is replaced by the plain exception of that status, so no key-specific text reaches the
    /// client. A validator that runs longer than the `validator_timeout` of the options is a 503.
    async fn validate(&self, req: &Request, key: &ApiKey) -> Result<Option<Self::User>, HttpException>;

    /// The auth info of a successful attempt (`Request::auth_info`), none by default. Never put the key in it.
    fn info(&self, _user: &Self::User) -> Option<Value> {
        None
    }
}

/// The strategy of an [`ApiKeyValidator`]: register it with `Provider::strategy::<ApiKeyStrategy<V>>()`.
pub struct ApiKeyStrategy<V: ApiKeyValidator> {
    validator: Arc<V>,
    resolved: Resolved,
    warnings: WarnLimiter,
    _marker: PhantomData<fn() -> V>,
}

/// Prints nothing but the strategy name.
impl<V: ApiKeyValidator> std::fmt::Debug for ApiKeyStrategy<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ApiKeyStrategy({})", V::NAME)
    }
}

impl<V: ApiKeyValidator> Injectable for ApiKeyStrategy<V> {
    fn construct(ctx: ResolveCtx) -> BoxFuture<'static, Result<Self, DiError>> {
        async move {
            let validator = ctx.get_or_construct::<V>().await?;
            let resolved = Resolved::new(&validator.options()).map_err(|problem| DiError::Factory {
                token: format!("ApiKeyStrategy \"{}\" ({})", V::NAME, std::any::type_name::<V>()),
                cause: Arc::new(anyhow::anyhow!("invalid API key strategy options: {problem}")),
            })?;
            Ok(ApiKeyStrategy { validator, resolved, warnings: WarnLimiter::new(Duration::from_secs(60)), _marker: PhantomData })
        }
        .boxed()
    }

    fn hooks() -> Hooks<Self> {
        Hooks {
            on_module_init: Some(|strategy| {
                async move {
                    let log = Logger::new("ApiKeyStrategy");
                    for name in &strategy.resolved.query_names {
                        log.warn(format!(
                            "ApiKeyStrategy \"{}\" accepts keys in the query parameter \"{name}\": URLs are written to proxy and access logs, browser history and Referer headers",
                            V::NAME
                        ));
                    }
                    Ok(())
                }
                .boxed()
            }),
            ..Hooks::default()
        }
    }
}

/// What one source found.
#[derive(Debug, PartialEq)]
enum Read {
    /// Nothing usable here: no value, an empty one, another scheme, a value without the prefix.
    Absent,
    Key(String),
    /// A value is there but unusable (too long, control characters, ambiguous): the whole attempt fails, so a
    /// source cannot be made to look empty to let another one decide.
    Invalid(&'static str),
}

fn has_control(bytes: &[u8]) -> bool {
    bytes.iter().any(|&b| b < 0x20 || b == 0x7f)
}

/// `true` when the value of the header `name` as [`read_header`] joins it starts with `prefix`. Reads the first
/// `prefix.len()` bytes at most and copies nothing.
fn joined_starts_with(headers: &HeaderMap, name: &http::HeaderName, prefix: &[u8]) -> bool {
    let mut matched = 0;
    let mut first = true;
    for value in headers.get_all(name) {
        let separator: &[u8] = if first { b"" } else { b", " };
        first = false;
        for &byte in separator.iter().chain(value.as_bytes()) {
            if matched == prefix.len() {
                return true;
            }
            if prefix[matched] != byte {
                return false;
            }
            matched += 1;
        }
    }
    matched == prefix.len()
}

/// The value of the header `name` as Node shows it (repeated values joined with `", "`), read without ever holding
/// more than `prefix + max` bytes, so a request with thousands of values cannot make this scan or copy them.
fn read_header(headers: &HeaderMap, name: &http::HeaderName, prefix: Option<&str>, max: usize) -> Read {
    // A value that does not start with the prefix is not ours, whatever its length or content: it is checked before
    // the limits, so an unrelated header cannot make a valid key in another source fail.
    if prefix.is_some_and(|prefix| !joined_starts_with(headers, name, prefix.as_bytes())) {
        return Read::Absent;
    }
    let limit = prefix.map_or(0, str::len) + max;
    let mut joined: Vec<u8> = Vec::new();
    let mut first = true;
    for value in headers.get_all(name) {
        let bytes = value.as_bytes();
        if joined.len() + bytes.len() + if first { 0 } else { 2 } > limit {
            return Read::Invalid(TOO_LONG);
        }
        if !first {
            joined.extend_from_slice(b", ");
        }
        joined.extend_from_slice(bytes);
        first = false;
    }
    if joined.is_empty() {
        return Read::Absent;
    }
    if has_control(&joined) {
        return Read::Invalid(BAD_CHARACTERS);
    }
    let key = match prefix {
        None => &joined[..],
        Some(prefix) if joined.starts_with(prefix.as_bytes()) => &joined[prefix.len()..],
        Some(_) => return Read::Absent,
    };
    if key.is_empty() { Read::Absent } else { Read::Key(latin1(key)) }
}

/// `Authorization: Bearer <key>` as `passport-jwt` parses it (`/(\S+)\s+(\S+)/`, the scheme ignoring case), over the
/// first value of the header only and without scanning more than `max + 128` bytes of leading space, scheme and
/// separator, nor more than `max` bytes of key.
fn read_bearer(headers: &HeaderMap, max: usize) -> Read {
    let Some(value) = auth_header::first_value(headers) else { return Read::Absent };
    let Some(credentials) = auth_header::parse(value, Some(max + 128)) else { return Read::Absent };
    if !credentials.scheme.eq_ignore_ascii_case(b"bearer") {
        return Read::Absent;
    }
    let Ok(key) = credentials.token(Some(max)) else { return Read::Invalid(TOO_LONG) };
    if key.is_empty() {
        return Read::Absent;
    }
    if has_control(key) {
        return Read::Invalid(BAD_CHARACTERS);
    }
    Read::Key(latin1(key))
}

fn read_query(req: &Request, name: &str, max: usize) -> Read {
    match req.query().get(name) {
        Some(Value::String(s)) if s.is_empty() => Read::Absent,
        Some(Value::String(s)) if s.len() > max => Read::Invalid(TOO_LONG),
        Some(Value::String(s)) if has_control(s.as_bytes()) => Read::Invalid(BAD_CHARACTERS),
        Some(Value::String(s)) => Read::Key(s.clone()),
        // A repeated or bracketed parameter: two values (or a structure) are no single key.
        Some(Value::Array(_) | Value::Object(_)) => Read::Invalid(AMBIGUOUS),
        _ => Read::Absent,
    }
}

impl Resolved {
    /// The one key of a request, or the reason it has none. Every source is read; a value that is present but
    /// unusable fails the attempt, and two sources with different keys fail it too.
    pub(super) fn extract(&self, req: &Request) -> Result<String, &'static str> {
        let max = self.max_key_bytes;
        let mut found: Vec<String> = Vec::new();
        for place in &self.places {
            let read = match place {
                Place::Header { name, prefix } => read_header(req.headers(), name, prefix.as_deref(), max),
                Place::Bearer => read_bearer(req.headers(), max),
                Place::Query { name } => read_query(req, name, max),
            };
            match read {
                Read::Absent => {}
                Read::Key(key) => found.push(key),
                Read::Invalid(reason) => return Err(reason),
            }
        }
        let Some(first) = found.first() else { return Err(MISSING) };
        // No early exit: every candidate is compared whatever the earlier ones gave.
        let mut same = true;
        for other in &found[1..] {
            same &= constant_time_eq(first.as_bytes(), other.as_bytes());
        }
        if !same {
            return Err(CONFLICT);
        }
        Ok(found.swap_remove(0))
    }
}

fn fail(reason: &str) -> StrategyOutcome {
    StrategyOutcome::Fail { challenge: Some(json!({ "message": reason })), status: None }
}

/// The text of the 500 for a validator that failed: fixed, so it never depends on the key.
#[derive(Debug)]
struct ValidatorFailed;

impl std::fmt::Display for ValidatorFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the API key validator failed")
    }
}

impl std::error::Error for ValidatorFailed {}

impl<V: ApiKeyValidator> ApiKeyStrategy<V> {
    /// A warning a client can trigger: at most one per reason a minute, with no request data.
    fn warn(&self, reason: &'static str, message: &str) {
        if let Some(line) = self.warnings.line(reason, Instant::now(), || format!("strategy \"{}\": {message}", V::NAME)) {
            Logger::new("ApiKeyStrategy").warn(line);
        }
    }

    /// An error of the validator as the client sees it: a 4xx as written, anything else the plain exception of its
    /// status (500 for a status below 400), whose body no key can influence.
    fn uniform(&self, e: HttpException) -> HttpException {
        let status = e.get_status();
        if (400..500).contains(&status) {
            return e;
        }
        self.warn("validator-failed", &format!("the validator failed with status {status}"));
        if status == 500 || status < 400 { HttpException::internal(ValidatorFailed) } else { HttpException::new((), e.status()) }
    }
}

#[async_trait]
impl<V: ApiKeyValidator> Strategy for ApiKeyStrategy<V> {
    fn name(&self) -> &str {
        V::NAME
    }

    fn credential_query_params(&self) -> Vec<String> {
        self.resolved.query_names.clone()
    }

    async fn authenticate(&self, ctx: &ExecutionContext, _options: &AuthenticateOptions) -> StrategyOutcome {
        let req = ctx.request();
        // The names are also registered for the whole application at startup; the mark stays so a request that
        // reaches here is covered when the strategy is used outside an application (tests of a guard).
        for name in &self.resolved.query_names {
            req.redact_query_param(name);
        }
        let key = match self.resolved.extract(req) {
            Ok(key) => ApiKey::new(key),
            Err(reason) => return fail(reason),
        };
        let warn = || self.warn("validator-timeout", "the validator did not answer within validator_timeout; answered 503");
        match with_deadline(self.resolved.validator_timeout, self.validator.validate(req, &key), warn).await {
            Ok(result) => user_outcome(result.map_err(|e| self.uniform(e)), |user| self.validator.info(user)),
            Err(unavailable) => unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use http::{HeaderName, HeaderValue};

    use super::super::options::{ApiKeySource, ApiKeyStrategyOptions};
    use super::*;

    const KEY: &str = "test-key-000000000000000000000000000001";
    const OTHER: &str = "test-key-000000000000000000000000000002";

    fn resolved(options: ApiKeyStrategyOptions) -> Resolved {
        Resolved::new(&options).unwrap()
    }

    fn request(url: &str, headers: &[(&str, &[u8])]) -> Request {
        let mut req = Request::new(http::Method::GET, url.parse().unwrap(), HeaderMap::new(), bytes::Bytes::new()).unwrap();
        for (name, value) in headers {
            req.headers_mut().append(HeaderName::from_bytes(name.as_bytes()).unwrap(), HeaderValue::from_bytes(value).unwrap());
        }
        req
    }

    fn default_extract(headers: &[(&str, &[u8])]) -> Result<String, &'static str> {
        resolved(ApiKeyStrategyOptions::default()).extract(&request("/", headers))
    }

    #[test]
    fn the_default_source_is_the_x_api_key_header_in_any_case() {
        assert_eq!(default_extract(&[("x-api-key", KEY.as_bytes())]).as_deref(), Ok(KEY));
        assert_eq!(default_extract(&[("X-API-KEY", KEY.as_bytes())]).as_deref(), Ok(KEY));
        assert_eq!(default_extract(&[]), Err(MISSING));
        assert_eq!(default_extract(&[("x-api-key", b"")]), Err(MISSING));
        assert_eq!(default_extract(&[("x-other", KEY.as_bytes())]), Err(MISSING));
    }

    #[test]
    fn repeated_values_are_joined_like_node_does() {
        assert_eq!(default_extract(&[("x-api-key", b"a"), ("x-api-key", b"b")]).as_deref(), Ok("a, b"));
        assert_eq!(default_extract(&[("x-api-key", b""), ("x-api-key", b"")]).as_deref(), Ok(", "));
    }

    #[test]
    fn header_bytes_are_read_as_latin1() {
        assert_eq!(default_extract(&[("x-api-key", b"caf\xe9-key")]).as_deref(), Ok("caf\u{e9}-key"));
    }

    #[test]
    fn a_header_prefix_is_literal_and_case_sensitive() {
        let options = || ApiKeyStrategyOptions::new().source(ApiKeySource::header("Authorization").prefix("Api-Key "));
        let read = |v: &[u8]| resolved(options()).extract(&request("/", &[("authorization", v)]));
        assert_eq!(read(b"Api-Key abc").as_deref(), Ok("abc"));
        assert_eq!(read(b"api-key abc"), Err(MISSING));
        assert_eq!(read(b"abc"), Err(MISSING));
        assert_eq!(read(b"Api-Key "), Err(MISSING));
        let plus = || ApiKeyStrategyOptions::new().source(ApiKeySource::header("X-Key").prefix("Key+"));
        let read = |v: &[u8]| resolved(plus()).extract(&request("/", &[("x-key", v)]));
        assert_eq!(read(b"Key+abc").as_deref(), Ok("abc"));
        assert_eq!(read(b"Keyyabc"), Err(MISSING), "no regular expression is built from the prefix");
        assert_eq!(read(b"Keyabc"), Err(MISSING));
    }

    fn bearer(value: &[u8]) -> Result<String, &'static str> {
        resolved(ApiKeyStrategyOptions::new().source(ApiKeySource::bearer())).extract(&request("/", &[("authorization", value)]))
    }

    #[test]
    fn the_bearer_source_parses_like_passport_jwt() {
        assert_eq!(bearer(b"Bearer abc").as_deref(), Ok("abc"));
        assert_eq!(bearer(b"bearer   abc").as_deref(), Ok("abc"));
        assert_eq!(bearer(b"BEARER\tabc").as_deref(), Ok("abc"));
        assert_eq!(bearer(b"Bearer\xa0abc").as_deref(), Ok("abc"));
        assert_eq!(bearer(b"  Bearer abc extra").as_deref(), Ok("abc"));
        assert_eq!(bearer(b"Basic abc"), Err(MISSING));
        assert_eq!(bearer(b"Bearer"), Err(MISSING));
        assert_eq!(bearer(b"Bearer "), Err(MISSING));
        assert_eq!(bearer(b"abc"), Err(MISSING));
        assert_eq!(bearer(b"Bearerabc"), Err(MISSING));
        assert_eq!(bearer(b"Bearer\x85abc"), Err(MISSING));
    }

    #[test]
    fn only_the_first_authorization_header_is_read() {
        let r = resolved(ApiKeyStrategyOptions::new().source(ApiKeySource::bearer()));
        let req = request("/", &[("authorization", b"Basic x"), ("authorization", b"Bearer abc")]);
        assert_eq!(r.extract(&req), Err(MISSING));
        let req = request("/", &[("authorization", b"Bearer abc"), ("authorization", b"Bearer other")]);
        assert_eq!(r.extract(&req).as_deref(), Ok("abc"));
    }

    #[test]
    fn the_query_source_reads_single_strings_only() {
        let r = resolved(ApiKeyStrategyOptions::new().source(ApiKeySource::query("api_key")));
        let read = |url: &str| r.extract(&request(url, &[]));
        assert_eq!(read("/d?api_key=abc").as_deref(), Ok("abc"));
        assert_eq!(read("/d?api_key=a%20b").as_deref(), Ok("a b"));
        assert_eq!(read("/d"), Err(MISSING));
        assert_eq!(read("/d?api_key="), Err(MISSING));
        assert_eq!(read("/d?other=abc"), Err(MISSING));
        assert_eq!(read("/d?api_key=a&api_key=b"), Err(AMBIGUOUS));
    }

    #[test]
    fn a_header_key_is_not_read_when_only_the_query_is_configured() {
        let r = resolved(ApiKeyStrategyOptions::new().source(ApiKeySource::query("api_key")));
        assert_eq!(r.extract(&request("/d", &[("x-api-key", KEY.as_bytes())])), Err(MISSING));
        let r = resolved(ApiKeyStrategyOptions::default());
        assert_eq!(r.extract(&request("/d?api_key=abc", &[])), Err(MISSING));
        assert_eq!(r.extract(&request("/d", &[("cookie", b"x-api-key=abc"), ("authorization", b"Bearer abc")])), Err(MISSING));
    }

    #[test]
    fn oversized_keys_fail_in_every_source() {
        let long = "k".repeat(513);
        assert_eq!(default_extract(&[("x-api-key", long.as_bytes())]), Err(TOO_LONG));
        assert_eq!(default_extract(&[("x-api-key", "k".repeat(512).as_bytes())]).map(|k| k.len()), Ok(512));
        assert_eq!(bearer(format!("Bearer {long}").as_bytes()), Err(TOO_LONG));
        assert_eq!(bearer(format!("Bearer {}", "k".repeat(512)).as_bytes()).map(|k| k.len()), Ok(512));
        let r = resolved(ApiKeyStrategyOptions::new().source(ApiKeySource::query("api_key")));
        assert_eq!(r.extract(&request(&format!("/d?api_key={long}"), &[])), Err(TOO_LONG));
        // The prefix does not count against the key.
        let r = resolved(ApiKeyStrategyOptions::new().source(ApiKeySource::header("X-Key").prefix("Key ")));
        let ok = format!("Key {}", "k".repeat(512));
        assert_eq!(r.extract(&request("/", &[("x-key", ok.as_bytes())])).map(|k| k.len()), Ok(512));
        let too_long = format!("Key {}", "k".repeat(513));
        assert_eq!(r.extract(&request("/", &[("x-key", too_long.as_bytes())])), Err(TOO_LONG));
        // The limit is configurable.
        let r = resolved(ApiKeyStrategyOptions::default().max_key_bytes(8));
        assert_eq!(r.extract(&request("/", &[("x-api-key", b"12345678")])).as_deref(), Ok("12345678"));
        assert_eq!(r.extract(&request("/", &[("x-api-key", b"123456789")])), Err(TOO_LONG));
    }

    #[test]
    fn many_repeated_values_stop_at_the_limit() {
        let mut headers: Vec<(&str, &[u8])> = Vec::new();
        for _ in 0..5000 {
            headers.push(("x-api-key", b"aa"));
        }
        assert_eq!(default_extract(&headers), Err(TOO_LONG));
    }

    #[test]
    fn a_long_run_of_white_space_before_the_scheme_is_not_scanned_to_the_end() {
        let mut value = vec![b' '; 100_000];
        value.extend_from_slice(b"Bearer abc");
        assert_eq!(bearer(&value), Err(MISSING));
    }

    #[test]
    fn control_characters_are_refused() {
        assert_eq!(default_extract(&[("x-api-key", b"ab\tcd")]), Err(BAD_CHARACTERS));
        let r = resolved(ApiKeyStrategyOptions::new().source(ApiKeySource::query("api_key")));
        assert_eq!(r.extract(&request("/d?api_key=abc%00def", &[])), Err(BAD_CHARACTERS));
        assert_eq!(r.extract(&request("/d?api_key=abc%7Fdef", &[])), Err(BAD_CHARACTERS));
        assert_eq!(r.extract(&request("/d?api_key=abc%0Adef", &[])), Err(BAD_CHARACTERS));
    }

    fn both() -> Resolved {
        resolved(ApiKeyStrategyOptions::new().source(ApiKeySource::header("X-Api-Key")).source(ApiKeySource::query("api_key")))
    }

    #[test]
    fn different_keys_in_two_sources_conflict() {
        let r = both();
        assert_eq!(r.extract(&request(&format!("/d?api_key={OTHER}"), &[("x-api-key", KEY.as_bytes())])), Err(CONFLICT));
        assert_eq!(r.extract(&request(&format!("/d?api_key={KEY}"), &[("x-api-key", KEY.as_bytes())])).as_deref(), Ok(KEY));
        assert_eq!(r.extract(&request(&format!("/d?api_key={KEY}"), &[])).as_deref(), Ok(KEY));
        assert_eq!(r.extract(&request("/d", &[("x-api-key", KEY.as_bytes())])).as_deref(), Ok(KEY));
        assert_eq!(r.extract(&request("/d", &[])), Err(MISSING));
    }

    #[test]
    fn a_present_but_unusable_value_does_not_let_another_source_decide() {
        let r = both();
        let long = "k".repeat(600);
        assert_eq!(r.extract(&request(&format!("/d?api_key={KEY}"), &[("x-api-key", long.as_bytes())])), Err(TOO_LONG));
        assert_eq!(r.extract(&request(&format!("/d?api_key={long}"), &[("x-api-key", KEY.as_bytes())])), Err(TOO_LONG));
        assert_eq!(r.extract(&request("/d?api_key=a&api_key=b", &[("x-api-key", KEY.as_bytes())])), Err(AMBIGUOUS));
        assert_eq!(r.extract(&request(&format!("/d?api_key={KEY}"), &[("x-api-key", b"a\tb")])), Err(BAD_CHARACTERS));
    }

    #[test]
    fn two_sources_on_the_authorization_header_agree_or_conflict() {
        let r = resolved(
            ApiKeyStrategyOptions::new().source(ApiKeySource::header("Authorization").prefix("Api-Key ")).source(ApiKeySource::bearer()),
        );
        assert_eq!(r.extract(&request("/", &[("authorization", b"Api-Key abc")])).as_deref(), Ok("abc"));
        assert_eq!(r.extract(&request("/", &[("authorization", b"Bearer abc")])).as_deref(), Ok("abc"));
        assert_eq!(r.extract(&request("/", &[("authorization", b"Basic abc")])), Err(MISSING));
    }

    fn prefixed_and_plain() -> Resolved {
        resolved(
            ApiKeyStrategyOptions::new()
                .source(ApiKeySource::header("X-Api-Key"))
                .source(ApiKeySource::header("Authorization").prefix("Api-Key ")),
        )
    }

    #[test]
    fn a_value_without_the_prefix_is_absent_whatever_its_length_or_content() {
        let r = prefixed_and_plain();
        let long = format!("Bearer {}", "k".repeat(600));
        for other in [long.as_bytes(), b"Bearer a\tb", b"Basic \xff\xfe", b"", b"Api-Key", b"Api-", b"api-key abc"] {
            let req = request("/", &[("x-api-key", KEY.as_bytes()), ("authorization", other)]);
            assert_eq!(r.extract(&req).as_deref(), Ok(KEY), "{}", String::from_utf8_lossy(other));
        }
        // Many values and a huge one: still absent, and nothing is copied or scanned past the prefix.
        let mut headers: Vec<(&str, &[u8])> = vec![("x-api-key", KEY.as_bytes())];
        for _ in 0..5000 {
            headers.push(("authorization", b"Bearer xx"));
        }
        assert_eq!(r.extract(&request("/", &headers)).as_deref(), Ok(KEY));
        let huge = vec![b'x'; 60_000];
        assert_eq!(r.extract(&request("/", &[("x-api-key", KEY.as_bytes()), ("authorization", &huge)])).as_deref(), Ok(KEY));
        // Only the prefixed source: a value that is not ours is a missing key.
        let only = resolved(ApiKeyStrategyOptions::new().source(ApiKeySource::header("Authorization").prefix("Api-Key ")));
        assert_eq!(only.extract(&request("/", &[("authorization", long.as_bytes())])), Err(MISSING));
        assert_eq!(only.extract(&request("/", &[("authorization", b"Bearer a\tb")])), Err(MISSING));
    }

    #[test]
    fn a_value_with_the_prefix_is_still_checked_for_length_and_characters() {
        let r = prefixed_and_plain();
        let long = format!("Api-Key {}", "k".repeat(600));
        assert_eq!(r.extract(&request("/", &[("x-api-key", KEY.as_bytes()), ("authorization", long.as_bytes())])), Err(TOO_LONG));
        assert_eq!(r.extract(&request("/", &[("x-api-key", KEY.as_bytes()), ("authorization", b"Api-Key a\tb")])), Err(BAD_CHARACTERS));
        assert_eq!(
            r.extract(&request("/", &[("x-api-key", KEY.as_bytes()), ("authorization", format!("Api-Key {KEY}").as_bytes())])).as_deref(),
            Ok(KEY)
        );
        assert_eq!(
            r.extract(&request("/", &[("x-api-key", KEY.as_bytes()), ("authorization", format!("Api-Key {OTHER}").as_bytes())])),
            Err(CONFLICT)
        );
    }

    #[test]
    fn the_prefix_is_matched_on_the_joined_value_like_the_key_is_read() {
        let r = resolved(ApiKeyStrategyOptions::new().source(ApiKeySource::header("X-Key").prefix("A, b")));
        assert_eq!(r.extract(&request("/", &[("x-key", b"A"), ("x-key", b"bc")])).as_deref(), Ok("c"));
        assert_eq!(r.extract(&request("/", &[("x-key", b"A"), ("x-key", b"xc")])), Err(MISSING));
        assert_eq!(r.extract(&request("/", &[("x-key", b"A")])), Err(MISSING));
    }

    #[test]
    fn credential_query_params_lists_the_query_sources() {
        assert_eq!(both().query_names, ["api_key"]);
    }

    #[test]
    fn fuzz_random_bytes_never_panic_and_stay_within_the_limits() {
        use proptest::prelude::*;
        let r = both();
        proptest!(ProptestConfig::with_cases(512), |(value in proptest::collection::vec(any::<u8>(), 0..1200), query in "\\PC{0,200}")| {
            let mut req = Request::empty();
            if let Ok(v) = HeaderValue::from_bytes(&value) {
                req.headers_mut().append(HeaderName::from_static("x-api-key"), v.clone());
                req.headers_mut().append(HeaderName::from_static("authorization"), v);
            }
            for r in [&r, &resolved(ApiKeyStrategyOptions::new().source(ApiKeySource::bearer()))] {
                if let Ok(key) = r.extract(&req) {
                    prop_assert!(key.len() <= 2 * 512);
                    prop_assert!(!key.bytes().any(|b| b < 0x20 || b == 0x7f));
                }
            }
            let encoded = form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
            let req = request(&format!("/d?api_key={encoded}"), &[]);
            if let Ok(key) = r.extract(&req) {
                prop_assert!(key.len() <= 512);
                prop_assert!(!key.bytes().any(|b| b < 0x20 || b == 0x7f));
            }
        });
    }
}
