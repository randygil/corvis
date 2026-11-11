//! JWT: `JwtModule.register()` / `registerAsync()` and `JwtService`.
//!
//! ```ignore
//! #[module(imports = [JwtModule::register(JwtModuleOptions::new("secret").expires_in("60s").global(true))])]
//! pub struct AuthModule;
//!
//! let token = self.jwt.sign(&json!({ "sub": user.id, "username": user.name }))?;
//! let claims: Claims = self.jwt.verify(&token)?;
//! ```

use std::borrow::Cow;
use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Validation};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::di::{DynamicModule, Module, ModuleDefinition, Provider, ResolveCtx, Token};
use crate::http::{HttpException, UnauthorizedException};
use crate::untrusted_json::PlainJson;

pub use jsonwebtoken::Algorithm as JwtAlgorithm;

/// Options for `JwtModule::register`. `Debug` shows whether a secret or key is set, never its value.
#[derive(Clone)]
pub struct JwtModuleOptions {
    /// HMAC secret (`HS256`/`HS384`/`HS512`). A non-empty `secret` takes
    /// precedence over `private_key`/`public_key`: leave it empty (`JwtModuleOptions::new("")`)
    /// to use PEM keys with `RS*`/`PS*`/`ES*`/`EdDSA`. Signing and verifying fail without a key.
    pub secret: String,
    /// `signOptions.expiresIn`, with the grammar of the `ms` package: `"60s"`, `"15m"`,
    /// `"1.5h"`, `"2 days"`, `"-1h"`, `"1y"`... A bare number is a number of **seconds**
    /// (`"3600"`). A value that cannot be parsed makes `sign` fail.
    pub expires_in: Option<String>,
    pub algorithm: Algorithm,
    pub issuer: Option<String>,
    pub audience: Option<String>,
    pub global: bool,
    /// `privateKey`: PEM key used to sign with `RS*`/`PS*`/`ES*`/`EdDSA`.
    pub private_key: Option<String>,
    /// `publicKey`: PEM key used to verify with `RS*`/`PS*`/`ES*`/`EdDSA`.
    pub public_key: Option<String>,
}

impl JwtModuleOptions {
    pub fn new(secret: impl Into<String>) -> Self {
        JwtModuleOptions {
            secret: secret.into(),
            expires_in: None,
            algorithm: Algorithm::HS256,
            issuer: None,
            audience: None,
            global: false,
            private_key: None,
            public_key: None,
        }
    }
    pub fn expires_in(mut self, v: impl Into<String>) -> Self {
        self.expires_in = Some(v.into());
        self
    }
    pub fn algorithm(mut self, a: Algorithm) -> Self {
        self.algorithm = a;
        self
    }
    pub fn issuer(mut self, i: impl Into<String>) -> Self {
        self.issuer = Some(i.into());
        self
    }
    pub fn audience(mut self, a: impl Into<String>) -> Self {
        self.audience = Some(a.into());
        self
    }
    pub fn global(mut self, g: bool) -> Self {
        self.global = g;
        self
    }
    /// `privateKey` (PEM) for asymmetric algorithms.
    pub fn private_key(mut self, pem: impl Into<String>) -> Self {
        self.private_key = Some(pem.into());
        self
    }
    /// `publicKey` (PEM) for asymmetric algorithms.
    pub fn public_key(mut self, pem: impl Into<String>) -> Self {
        self.public_key = Some(pem.into());
        self
    }
}

impl std::fmt::Debug for JwtModuleOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let set = |present: bool| if present { "<set>" } else { "<unset>" };
        f.debug_struct("JwtModuleOptions")
            .field("secret", &set(!self.secret.is_empty()))
            .field("expires_in", &self.expires_in)
            .field("algorithm", &self.algorithm)
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("global", &self.global)
            .field("private_key", &set(self.private_key.is_some()))
            .field("public_key", &set(self.public_key.is_some()))
            .finish()
    }
}

/// Per-call sign options (`jwtService.sign(payload, { expiresIn, subject, jwtid, notBefore, secret, privateKey })`).
/// Build it with `..Default::default()`. `Debug` shows whether a secret or key is set, never its value.
#[derive(Clone, Default)]
pub struct JwtSignOptions {
    /// `expiresIn`: overrides the module's `expires_in`. `exp` is counted from the payload's `iat` when it has one.
    pub expires_in: Option<String>,
    /// `secret`: an HMAC secret for this call, taken before the module's keys.
    pub secret: Option<String>,
    /// `subject`: the `sub` claim. A payload that already has `sub` is an error.
    pub subject: Option<String>,
    /// `privateKey`: a PEM key for this call (`RS*`/`PS*`/`ES*`/`EdDSA`).
    pub private_key: Option<String>,
    /// `notBefore`: a timespan with the grammar of `expires_in`; `nbf` is counted from the payload's `iat` when it has
    /// one, from the clock otherwise. A payload that already has `nbf` is an error.
    pub not_before: Option<String>,
    /// `jwtid`: the `jti` claim. A payload that already has `jti` is an error.
    pub jwt_id: Option<String>,
}

impl std::fmt::Debug for JwtSignOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let set = |present: bool| if present { "<set>" } else { "<unset>" };
        f.debug_struct("JwtSignOptions")
            .field("expires_in", &self.expires_in)
            .field("secret", &set(self.secret.is_some()))
            .field("subject", &self.subject)
            .field("private_key", &set(self.private_key.is_some()))
            .field("not_before", &self.not_before)
            .field("jwt_id", &self.jwt_id)
            .finish()
    }
}

/// Parses a timespan with the grammar of the `ms` package and returns milliseconds:
/// an optional `-`, a decimal number (`1`, `1.5`, `.5`), optional spaces and an optional
/// case-insensitive unit (`ms`, `s`, `m`, `h`, `d`, `w`, `y` and their long forms such as
/// `msecs`, `seconds`, `mins`, `hrs`, `days`, `weeks`, `years`). Without a unit the number is
/// taken as seconds (Corvis keeps its documented behaviour; `ms` would read milliseconds).
fn parse_timespan_ms(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() || s.len() > 100 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut i = usize::from(bytes[0] == b'-');
    let int_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - int_start;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == frac_start {
            return None;
        }
        digits += i - frac_start;
    }
    if digits == 0 {
        return None;
    }
    let value: f64 = s[..i].parse().ok()?;
    let unit = s[i..].trim_start_matches(' ').to_ascii_lowercase();
    let mult = match unit.as_str() {
        "" => 1_000.0,
        "milliseconds" | "millisecond" | "msecs" | "msec" | "ms" => 1.0,
        "seconds" | "second" | "secs" | "sec" | "s" => 1_000.0,
        "minutes" | "minute" | "mins" | "min" | "m" => 60_000.0,
        "hours" | "hour" | "hrs" | "hr" | "h" => 3_600_000.0,
        "days" | "day" | "d" => 86_400_000.0,
        "weeks" | "week" | "w" => 604_800_000.0,
        "years" | "year" | "yrs" | "yr" | "y" => 31_557_600_000.0,
        _ => return None,
    };
    let ms = value * mult;
    ms.is_finite().then_some(ms)
}

/// Largest `exp` accepted: `Number.MAX_SAFE_INTEGER`, so the claim survives a JSON round trip.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Parses `"60s"`, `"15m"`, `"1.5h"`, `"2 days"`, `"-1h"`, `"1y"` (the `ms` grammar) or a
/// number of seconds, and returns whole seconds (rounded down, like node-jsonwebtoken).
/// `None` when the value cannot be parsed or is out of range.
pub fn parse_duration_secs(s: &str) -> Option<i64> {
    let secs = (parse_timespan_ms(s)? / 1000.0).floor();
    (secs.abs() <= MAX_SAFE_INTEGER).then_some(secs as i64)
}

const EXPIRES_IN_ERROR: &str = "\"expiresIn\" should be a number of seconds or string representing a timespan eg: \"1d\", \"20h\", 60";
const NOT_BEFORE_ERROR: &str = "\"notBefore\" should be a number of seconds or string representing a timespan eg: \"1d\", \"20h\", 60";

/// node-jsonwebtoken's `timespan(value, iat)`: `floor(iat + milliseconds / 1000)`, as an integer claim. `None` when
/// `value` is not a timespan or the result is beyond `Number.MAX_SAFE_INTEGER`.
fn timespan(value: &str, timestamp: f64) -> Option<Value> {
    let at = (timestamp + parse_timespan_ms(value)? / 1000.0).floor();
    (at.abs() <= MAX_SAFE_INTEGER).then(|| Value::from(at as i64))
}

#[cfg(test)]
thread_local! {
    /// How many keys `JwtService` parsed on this thread, so a test can show that a call parsed none.
    static KEY_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How many token headers were checked on this thread.
    pub(crate) static HEADER_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn encoding_key(alg: Algorithm, key: &str) -> Result<EncodingKey, jsonwebtoken::errors::Error> {
    #[cfg(test)]
    KEY_PARSES.with(|c| c.set(c.get() + 1));
    let pem = key.as_bytes();
    Ok(match alg {
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => EncodingKey::from_secret(pem),
        Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 | Algorithm::PS256 | Algorithm::PS384 | Algorithm::PS512 => {
            EncodingKey::from_rsa_pem(pem)?
        }
        Algorithm::ES256 | Algorithm::ES384 => EncodingKey::from_ec_pem(pem)?,
        Algorithm::EdDSA => EncodingKey::from_ed_pem(pem)?,
    })
}

fn decoding_key(alg: Algorithm, key: &str) -> Result<DecodingKey, jsonwebtoken::errors::Error> {
    #[cfg(test)]
    KEY_PARSES.with(|c| c.set(c.get() + 1));
    let pem = key.as_bytes();
    Ok(match alg {
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => DecodingKey::from_secret(pem),
        Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 | Algorithm::PS256 | Algorithm::PS384 | Algorithm::PS512 => {
            DecodingKey::from_rsa_pem(pem)?
        }
        Algorithm::ES256 | Algorithm::ES384 => DecodingKey::from_ec_pem(pem)?,
        Algorithm::EdDSA => DecodingKey::from_ed_pem(pem)?,
    })
}

/// The first non-empty key of `candidates`.
fn first_key<'a>(candidates: impl IntoIterator<Item = Option<&'a str>>) -> Option<&'a str> {
    candidates.into_iter().flatten().find(|k| !k.is_empty())
}

/// `jsonwebtoken`'s `Validation` for `algorithms`: the crate checks only the signature and the algorithm (no required
/// claims, no leeway); the claims are checked by the caller with node-jsonwebtoken's rules (the crate accepts a token
/// whose `iss` or `aud` is missing, has another type or, for `iss`, is a list that merely intersects).
pub(crate) fn signature_validation(algorithms: Vec<Algorithm>) -> Validation {
    let mut validation = Validation::new(algorithms[0]);
    validation.algorithms = algorithms;
    validation.required_spec_claims = HashSet::new();
    validation.leeway = 0;
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.validate_aud = false;
    validation
}

/// How the time claims of a verified token are checked (see [`check_time_claims`]).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TimeRules {
    /// Seconds of clock skew allowed on `exp` and `nbf`.
    pub tolerance: u64,
    /// Skip `exp` altogether (`ignoreExpiration`).
    pub ignore_expiration: bool,
    /// A token without `exp` fails.
    pub require_exp: bool,
}

/// Why [`decode_claims`] refused a token.
#[derive(Debug)]
pub(crate) enum VerifyError {
    /// The header, the algorithm, the key or the signature did not pass.
    Decode(jsonwebtoken::errors::Error),
    /// A claim did not pass; the text is node-jsonwebtoken's message where one exists.
    Claim(&'static str),
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::Decode(e) => e.fmt(f),
            VerifyError::Claim(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for VerifyError {}

/// The time rules of node-jsonwebtoken's `verify`, with `now` in whole seconds: `iat`, `nbf` and `exp` must be numbers
/// when present (`exp` is not looked at with `ignore_expiration`), `nbf > now + tolerance` is not active yet and
/// `now >= exp + tolerance` is expired. A missing `exp` fails when `require_exp` is set.
pub(crate) fn check_time_claims(claims: &Value, rules: &TimeRules, now: i64) -> Result<(), VerifyError> {
    let now = now as f64;
    let tolerance = rules.tolerance as f64;
    let number = |name: &str, message: &'static str| -> Result<Option<f64>, VerifyError> {
        match claims.get(name) {
            None => Ok(None),
            Some(v) => v.as_f64().map(Some).ok_or(VerifyError::Claim(message)),
        }
    };
    number("iat", "invalid iat value")?;
    if let Some(nbf) = number("nbf", "invalid nbf value")?
        && nbf > now + tolerance
    {
        return Err(VerifyError::Claim("jwt not active"));
    }
    if !rules.ignore_expiration {
        match number("exp", "invalid exp value")? {
            Some(exp) if now >= exp + tolerance => return Err(VerifyError::Claim("jwt expired")),
            None if rules.require_exp => return Err(VerifyError::Claim("exp claim required")),
            _ => {}
        }
    }
    Ok(())
}

/// Decodes `token` with `validation` (algorithms, and `iss`/`aud` when set on it) and checks its time claims against
/// the clock. Shared by [`JwtService::verify`] and the JWT strategy. The caller checks the header first
/// ([`check_header`]): the decoder skips unknown header members without validating them.
pub(crate) fn decode_claims(token: &str, key: &DecodingKey, validation: &Validation, rules: &TimeRules) -> Result<Value, VerifyError> {
    let claims = jsonwebtoken::decode::<PlainJson>(token, key, validation).map_err(VerifyError::Decode)?.claims.0;
    check_time_claims(&claims, rules, chrono::Utc::now().timestamp())?;
    Ok(claims)
}

/// node-jsonwebtoken's audience rule: one of the token's `aud` values (a string or a list of strings) equals one of
/// the allowed. A missing claim or any other type does not match.
pub(crate) fn audience_matches(claim: Option<&Value>, allowed: &[String]) -> bool {
    match claim {
        Some(Value::String(aud)) => allowed.contains(aud),
        Some(Value::Array(list)) => list.iter().any(|v| v.as_str().is_some_and(|aud| allowed.iter().any(|a| a == aud))),
        _ => false,
    }
}

/// node-jsonwebtoken's issuer rule: the token's `iss` must be a string equal to one of the allowed.
pub(crate) fn issuer_matches(claim: Option<&Value>, allowed: &[String]) -> bool {
    claim.and_then(Value::as_str).is_some_and(|iss| allowed.iter().any(|a| a == iss))
}

fn base64url_segment(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A cheap syntactic check: exactly three non-empty base64url segments (no padding).
pub(crate) fn has_three_segments(token: &str) -> bool {
    let mut parts = token.split('.');
    matches!((parts.next(), parts.next(), parts.next(), parts.next()), (Some(a), Some(b), Some(c), None)
        if base64url_segment(a) && base64url_segment(b) && base64url_segment(c))
}

/// What is wrong with the protected header of a token, found before any key is looked up.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HeaderProblem {
    /// The first segment is not base64url of a JSON object that the strict text parser accepts.
    Malformed,
    /// The header has a `crit` member (RFC 7515 section 4.1.11); Corvis supports no critical extensions.
    Crit,
}

/// What the header check needs to know about a document, found without keeping any of it.
struct HeaderShape {
    object: bool,
    crit: bool,
}

/// Deserializes and discards a value. Unlike `serde::de::IgnoredAny` it goes through `deserialize_any`, so the text
/// parser validates everything (numbers, escapes, surrogates, UTF-8, the depth limit) as for a value that is kept.
struct Discard;

impl<'de> serde::Deserialize<'de> for Discard {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{MapAccess, SeqAccess, Visitor};
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Discard;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_bool<E>(self, _: bool) -> Result<Discard, E> {
                Ok(Discard)
            }
            fn visit_i64<E>(self, _: i64) -> Result<Discard, E> {
                Ok(Discard)
            }
            fn visit_u64<E>(self, _: u64) -> Result<Discard, E> {
                Ok(Discard)
            }
            fn visit_f64<E>(self, _: f64) -> Result<Discard, E> {
                Ok(Discard)
            }
            fn visit_str<E>(self, _: &str) -> Result<Discard, E> {
                Ok(Discard)
            }
            fn visit_unit<E>(self) -> Result<Discard, E> {
                Ok(Discard)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Discard, A::Error> {
                while seq.next_element::<Discard>()?.is_some() {}
                Ok(Discard)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Discard, A::Error> {
                while map.next_entry::<Discard, Discard>()?.is_some() {}
                Ok(Discard)
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// A member name reduced to whether it is exactly `crit` (after unescaping).
struct IsCrit(bool);

impl<'de> serde::Deserialize<'de> for IsCrit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = IsCrit;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a member name")
            }
            fn visit_str<E>(self, v: &str) -> Result<IsCrit, E> {
                Ok(IsCrit(v == "crit"))
            }
        }
        deserializer.deserialize_str(V)
    }
}

impl<'de> serde::Deserialize<'de> for HeaderShape {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{MapAccess, SeqAccess, Visitor};
        struct V;
        const OTHER: HeaderShape = HeaderShape { object: false, crit: false };
        impl<'de> Visitor<'de> for V {
            type Value = HeaderShape;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_bool<E>(self, _: bool) -> Result<HeaderShape, E> {
                Ok(OTHER)
            }
            fn visit_i64<E>(self, _: i64) -> Result<HeaderShape, E> {
                Ok(OTHER)
            }
            fn visit_u64<E>(self, _: u64) -> Result<HeaderShape, E> {
                Ok(OTHER)
            }
            fn visit_f64<E>(self, _: f64) -> Result<HeaderShape, E> {
                Ok(OTHER)
            }
            fn visit_str<E>(self, _: &str) -> Result<HeaderShape, E> {
                Ok(OTHER)
            }
            fn visit_unit<E>(self) -> Result<HeaderShape, E> {
                Ok(OTHER)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<HeaderShape, A::Error> {
                while seq.next_element::<Discard>()?.is_some() {}
                Ok(OTHER)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<HeaderShape, A::Error> {
                let mut crit = false;
                while let Some(IsCrit(is_crit)) = map.next_key::<IsCrit>()? {
                    crit |= is_crit;
                    map.next_value::<Discard>()?;
                }
                Ok(HeaderShape { object: true, crit })
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// Checks the protected header and fails closed: a header that is not a JSON object accepted by the strict text
/// parser is refused, because the decoder skips unknown members without validating them and would let such a header
/// carry a `crit` member unseen. Only the exact member name `crit` counts. The header is validated while it is read
/// and nothing of it is kept, so the memory used does not grow with the document.
pub(crate) fn check_header(token: &str) -> Option<HeaderProblem> {
    #[cfg(test)]
    HEADER_CHECKS.with(|c| c.set(c.get() + 1));
    use base64::Engine as _;
    let header = token.split('.').next().unwrap_or("");
    let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(header) else { return Some(HeaderProblem::Malformed) };
    match serde_json::from_slice::<HeaderShape>(&bytes) {
        Ok(HeaderShape { object: true, crit }) => crit.then_some(HeaderProblem::Crit),
        _ => Some(HeaderProblem::Malformed),
    }
}

/// A key of the module, parsed when the service is created: `None` when the options set none, the parse error
/// otherwise (returned by each call that needs the key, as the original fails per call).
type PreparedKey<K> = Option<Result<K, jsonwebtoken::errors::Error>>;

/// What `JwtService::new` prepares once.
struct Prepared {
    /// The module secret when it is not empty, else the module private key.
    encoding: PreparedKey<EncodingKey>,
    /// The module secret when it is not empty, else the module public key.
    decoding: PreparedKey<DecodingKey>,
    /// The `Validation` of `verify`: the configured algorithm, no required claims, no leeway, no `exp`/`nbf`/`aud`.
    validation: Validation,
}

/// Signs and verifies JSON Web Tokens. The module keys are parsed when it is created (a key that does not parse makes
/// each `sign` or `verify` fail, the application still starts); cloning it shares them.
#[derive(Clone)]
pub struct JwtService {
    options: JwtModuleOptions,
    prepared: Arc<Prepared>,
}

impl JwtService {
    pub fn new(options: JwtModuleOptions) -> Self {
        let alg = options.algorithm;
        let encoding = first_key([Some(options.secret.as_str()), options.private_key.as_deref()]).map(|key| encoding_key(alg, key));
        let decoding = first_key([Some(options.secret.as_str()), options.public_key.as_deref()]).map(|key| decoding_key(alg, key));
        let prepared = Prepared { encoding, decoding, validation: signature_validation(vec![alg]) };
        JwtService { options, prepared: Arc::new(prepared) }
    }

    /// The key `sign` uses. The first non-empty key wins: per-call secret, module secret, per-call private key, module
    /// private key. A per-call key is parsed for this call; a module key was parsed by `new`.
    fn signing_key(&self, opts: &JwtSignOptions) -> crate::Result<Cow<'_, EncodingKey>> {
        let alg = self.options.algorithm;
        let module_key = || match &self.prepared.encoding {
            Some(Ok(key)) => Ok(Cow::Borrowed(key)),
            Some(Err(e)) => Err(e.clone().into()),
            None => Err(anyhow::anyhow!("secretOrPrivateKey must have a value")),
        };
        if let Some(secret) = first_key([opts.secret.as_deref()]) {
            return Ok(Cow::Owned(encoding_key(alg, secret)?));
        }
        if !self.options.secret.is_empty() {
            return module_key();
        }
        if let Some(private_key) = first_key([opts.private_key.as_deref()]) {
            return Ok(Cow::Owned(encoding_key(alg, private_key)?));
        }
        module_key()
    }

    /// `jwtService.sign(payload)`: [`sign_with`](Self::sign_with) without per-call options.
    pub fn sign(&self, payload: &impl Serialize) -> crate::Result<String> {
        self.sign_with(payload, JwtSignOptions::default())
    }

    /// `jwtService.sign(payload, options)`, with the rules of the original (`jsonwebtoken` 9 under the JWT module),
    /// verified against a recording of it:
    ///
    /// - an object payload's `iat`, `exp` and `nbf` must be numbers (`"iat" should be a number of seconds`);
    /// - `iat` is the payload's when it is a number other than 0, the clock otherwise; `nbf` (`not_before`) and `exp`
    ///   (`expires_in`, per call or of the module) are counted from it;
    /// - a payload claim together with its option is an error, never overwritten: `exp` with `expires_in`, `nbf` with
    ///   `not_before`, `aud` with the module `audience`, `iss` with the module `issuer`, `sub` with `subject`, `jti`
    ///   with `jwt_id` (`Bad "options.subject" option. The payload already has an "sub" property.`);
    /// - the claims are added after the payload's own keys in the order `iat`, `nbf`, `exp`, `aud`, `iss`, `sub`, `jti`
    ///   (a payload `iat` keeps its place);
    /// - a string payload is signed as it is, with a header without `typ`, and is refused when any sign option is set;
    ///   a number or a boolean is signed as its JSON text, without `typ`; an array or `null` is refused.
    ///
    /// It also fails when no key is set (`secretOrPrivateKey must have a value`), the key does not parse, or
    /// `expires_in` / `not_before` is not a valid timespan. A bare number is a number of seconds.
    pub fn sign_with(&self, payload: &impl Serialize, opts: JwtSignOptions) -> crate::Result<String> {
        self.sign_at(payload, opts, chrono::Utc::now().timestamp())
    }

    /// [`sign_with`](Self::sign_with) at the clock `now` (seconds), so a test can replay recorded tokens.
    fn sign_at(&self, payload: &impl Serialize, opts: JwtSignOptions, now: i64) -> crate::Result<String> {
        let alg = self.options.algorithm;
        let claims = serde_json::to_value(payload)?;
        // The JWT module refuses a string payload with sign options before anything else.
        if claims.is_string() {
            let names = self.sign_option_names(&opts);
            if !names.is_empty() {
                anyhow::bail!("Payload as string is not allowed with the following sign options: {}", names.join(", "));
            }
        }
        // `jsonwebtoken` checks the key before the payload.
        let encoding = self.signing_key(&opts)?;
        let (object, body) = match claims {
            Value::Object(map) => (true, serde_json::to_vec(&Value::Object(self.claims_of(map, &opts, now)?))?),
            Value::String(text) => (false, text.into_bytes()),
            Value::Array(_) | Value::Null => anyhow::bail!("Expected \"payload\" to be a plain object."),
            other => {
                let set = self.object_option_names(&opts);
                if !set.is_empty() {
                    let kind = if other.is_boolean() { "boolean" } else { "number" };
                    anyhow::bail!("invalid {} option for {kind} payload", set.join(","));
                }
                (false, serde_json::to_vec(&other)?)
            }
        };
        // node-jsonwebtoken (jws) writes the header as `{"alg":..,"typ":"JWT"}`, in that order, and without `typ` for a
        // payload that is not an object; the `jsonwebtoken` crate serializes `typ` first, so the header is built by hand.
        use base64::Engine as _;
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let header = if object { format!("{{\"alg\":\"{alg:?}\",\"typ\":\"JWT\"}}") } else { format!("{{\"alg\":\"{alg:?}\"}}") };
        let message = format!("{}.{}", b64(header.as_bytes()), b64(&body));
        let signature = jsonwebtoken::crypto::sign(message.as_bytes(), &encoding, alg)?;
        Ok(format!("{message}.{signature}"))
    }

    /// The claims of an object payload, by the rules of [`sign_with`](Self::sign_with), in the original's order of
    /// checks.
    fn claims_of(&self, mut map: Map<String, Value>, opts: &JwtSignOptions, now: i64) -> crate::Result<Map<String, Value>> {
        for (key, value) in &map {
            if matches!(key.as_str(), "iat" | "exp" | "nbf") && !value.is_number() {
                anyhow::bail!("\"{key}\" should be a number of seconds");
            }
        }
        let expires_in = opts.expires_in.as_deref().or(self.options.expires_in.as_deref());
        if map.contains_key("exp") && expires_in.is_some() {
            anyhow::bail!("Bad \"options.expiresIn\" option the payload already has an \"exp\" property.");
        }
        if map.contains_key("nbf") && opts.not_before.is_some() {
            anyhow::bail!("Bad \"options.notBefore\" option the payload already has an \"nbf\" property.");
        }
        // `payload.iat || now`: an `iat` of 0 is replaced by the clock, in place.
        let timestamp = match map.get("iat").and_then(Value::as_f64) {
            Some(iat) if iat != 0.0 => iat,
            _ => {
                map.insert("iat".into(), Value::from(now));
                now as f64
            }
        };
        if let Some(not_before) = &opts.not_before {
            map.insert("nbf".into(), timespan(not_before, timestamp).ok_or_else(|| anyhow::anyhow!(NOT_BEFORE_ERROR))?);
        }
        if let Some(expires_in) = expires_in {
            map.insert("exp".into(), timespan(expires_in, timestamp).ok_or_else(|| anyhow::anyhow!(EXPIRES_IN_ERROR))?);
        }
        let added = [
            ("audience", "aud", self.options.audience.as_ref()),
            ("issuer", "iss", self.options.issuer.as_ref()),
            ("subject", "sub", opts.subject.as_ref()),
            ("jwtid", "jti", opts.jwt_id.as_ref()),
        ];
        for (option, claim, value) in added {
            let Some(value) = value else { continue };
            if map.contains_key(claim) {
                anyhow::bail!("Bad \"options.{option}\" option. The payload already has an \"{claim}\" property.");
            }
            map.insert(claim.into(), Value::String(value.clone()));
        }
        Ok(map)
    }

    /// The names of the original's sign options this call has, in the order the JWT module lists them: the module's
    /// (`algorithm` when it is not the default HS256), then the call's.
    fn sign_option_names(&self, opts: &JwtSignOptions) -> Vec<&'static str> {
        let o = &self.options;
        let mut names = Vec::new();
        if o.algorithm != Algorithm::HS256 {
            names.push("algorithm");
        }
        if o.expires_in.is_some() || opts.expires_in.is_some() {
            names.push("expiresIn");
        }
        for (name, set) in [("issuer", o.issuer.is_some()), ("audience", o.audience.is_some())] {
            if set {
                names.push(name);
            }
        }
        for (name, set) in [("subject", opts.subject.is_some()), ("jwtid", opts.jwt_id.is_some()), ("notBefore", opts.not_before.is_some())]
        {
            if set {
                names.push(name);
            }
        }
        names
    }

    /// The options `jsonwebtoken` accepts only with an object payload, in its order.
    fn object_option_names(&self, opts: &JwtSignOptions) -> Vec<&'static str> {
        let o = &self.options;
        [
            ("expiresIn", o.expires_in.is_some() || opts.expires_in.is_some()),
            ("notBefore", opts.not_before.is_some()),
            ("audience", o.audience.is_some()),
            ("issuer", o.issuer.is_some()),
            ("subject", opts.subject.is_some()),
            ("jwtid", opts.jwt_id.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, set)| set.then_some(name))
        .collect()
    }

    /// `await jwtService.signAsync(payload)`.
    pub async fn sign_async(&self, payload: &impl Serialize) -> crate::Result<String> {
        self.sign(payload)
    }

    /// `jwtService.verify(token)`: validates the signature (only with the configured
    /// algorithm), then, like node-jsonwebtoken, `nbf` (`jwt not active`), `exp` (`jwt
    /// expired` once `now >= exp`), `iss`/`aud` when configured, and that `exp`, `nbf` and
    /// `iat` are numbers when present.
    pub fn verify<T: DeserializeOwned>(&self, token: &str) -> crate::Result<T> {
        // The module secret, else the module public key, parsed by `new`.
        let key = match &self.prepared.decoding {
            Some(Ok(key)) => key,
            Some(Err(e)) => return Err(e.clone().into()),
            None => anyhow::bail!("secret or public key must be provided"),
        };
        match check_header(token) {
            Some(HeaderProblem::Crit) => return Err(VerifyError::Claim("critical header parameters are not supported").into()),
            Some(HeaderProblem::Malformed) => return Err(VerifyError::Claim("jwt malformed").into()),
            None => {}
        }
        // `exp` and `nbf` are checked with node-jsonwebtoken's rules, `iss` and `aud` below with the strategy's.
        let claims = match decode_claims(token, key, &self.prepared.validation, &TimeRules::default()) {
            Ok(claims) => claims,
            // Callers can still downcast to `jsonwebtoken::errors::Error` for what that crate refused.
            Err(VerifyError::Decode(e)) => return Err(e.into()),
            Err(e @ VerifyError::Claim(_)) => return Err(e.into()),
        };
        if let Some(iss) = &self.options.issuer
            && !issuer_matches(claims.get("iss"), std::slice::from_ref(iss))
        {
            return Err(jsonwebtoken::errors::Error::from(jsonwebtoken::errors::ErrorKind::InvalidIssuer).into());
        }
        if let Some(aud) = &self.options.audience
            && !audience_matches(claims.get("aud"), std::slice::from_ref(aud))
        {
            return Err(jsonwebtoken::errors::Error::from(jsonwebtoken::errors::ErrorKind::InvalidAudience).into());
        }
        Ok(serde_json::from_value(claims)?)
    }

    /// `await jwtService.verifyAsync(token)`.
    pub async fn verify_async<T: DeserializeOwned>(&self, token: &str) -> crate::Result<T> {
        self.verify(token)
    }

    /// Verifies a token mapping failures to `401 Unauthorized`.
    pub fn verify_or_unauthorized<T: DeserializeOwned>(&self, token: &str) -> Result<T, HttpException> {
        self.verify(token).map_err(|_| UnauthorizedException.into())
    }

    /// `jwtService.decode(token)` – no signature verification.
    pub fn decode(&self, token: &str) -> Option<Value> {
        let payload = token.split('.').nth(1)?;
        use base64::Engine;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
        crate::untrusted_json::parse(&bytes)
    }
}

/// Extracts the bearer token from the `Authorization` header: the value is split at its first space, the scheme
/// compared with `bearer` ignoring ASCII case, and the rest trimmed (so `Bearer a b` gives `a b`); a value that is not
/// visible ASCII gives none.
///
/// This is not the JWT strategy's parser: [`ExtractJwt::from_auth_header_as_bearer_token`](crate::auth::jwt::ExtractJwt::from_auth_header_as_bearer_token)
/// follows `passport-jwt` (any JavaScript whitespace between scheme and token, words after the token ignored). Use
/// that one for the strategy's rules; this helper keeps its own, which applications already rely on.
pub fn extract_bearer_token(req: &crate::http::Request) -> Option<String> {
    let h = req.header("authorization")?;
    let (kind, token) = h.split_once(' ')?;
    kind.eq_ignore_ascii_case("bearer").then(|| token.trim().to_string())
}

/// `JwtModule`.
#[derive(Default)]
pub struct JwtModule;
crate::default_injectable!(JwtModule);

impl Module for JwtModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::new::<JwtModule>("JwtModule").module_provider(Provider::class::<JwtModule>())
    }
}

impl JwtModule {
    /// `JwtModule.register({ secret, signOptions: { expiresIn } })`.
    pub fn register(options: JwtModuleOptions) -> DynamicModule {
        let global = options.global;
        DynamicModule::new::<JwtModule>()
            .provider(Provider::use_value(Token::of::<JwtService>(), JwtService::new(options)))
            .export(Token::of::<JwtService>())
            .global(global)
    }

    /// `JwtModule.registerAsync({ useFactory: async (config) => ({ ... }), inject: [ConfigService] })`.
    pub fn register_async<F, Fut>(global: bool, factory: F) -> DynamicModule
    where
        F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = crate::Result<JwtModuleOptions>> + Send + 'static,
    {
        let factory = std::sync::Arc::new(factory);
        DynamicModule::new::<JwtModule>()
            .provider(Provider::use_factory(Token::of::<JwtService>(), move |ctx| {
                let f = factory.clone();
                async move { Ok(JwtService::new(f(ctx).await?)) }
            }))
            .export(Token::of::<JwtService>())
            .global(global)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let jwt = JwtService::new(JwtModuleOptions::new("s3cr3t").expires_in("1h"));
        let token = jwt.sign(&serde_json::json!({ "sub": 1, "username": "john" })).unwrap();
        // Same header bytes as node-jsonwebtoken: {"alg":"HS256","typ":"JWT"}.
        assert!(token.starts_with("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9."));
        let claims: Value = jwt.verify(&token).unwrap();
        assert_eq!(claims["username"], "john");
        assert!(claims["exp"].as_i64().unwrap() > claims["iat"].as_i64().unwrap());
        assert_eq!(jwt.decode(&token).unwrap()["sub"], 1);
        let other = JwtService::new(JwtModuleOptions::new("other"));
        assert!(other.verify::<Value>(&token).is_err());
        assert_eq!(parse_duration_secs("15m"), Some(900));
        assert_eq!(parse_duration_secs("7d"), Some(604_800));
    }

    fn forge(header: &str, claims: &Value, secret: &[u8]) -> String {
        use base64::Engine as _;
        let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
        let message = format!("{}.{}", b64(header.as_bytes()), b64(claims.to_string().as_bytes()));
        let key = EncodingKey::from_secret(secret);
        let sig = jsonwebtoken::crypto::sign(message.as_bytes(), &key, Algorithm::HS256).unwrap();
        format!("{message}.{sig}")
    }
    const HS256: &str = r#"{"alg":"HS256","typ":"JWT"}"#;

    /// A header the decoder skips over but `serde_json::Value` refuses must not hide a `crit` member.
    #[test]
    fn a_header_that_is_not_plain_json_is_refused_before_the_decoder() {
        use base64::Engine as _;
        let jwt = JwtService::new(JwtModuleOptions::new("s"));
        let claims = serde_json::json!({ "sub": 1 });
        let nested = format!(r#"{{"alg":"HS256","crit":["urn:x"],"urn:x":true,"z":{}{}}}"#, "[".repeat(200), "]".repeat(200));
        let headers = [
            r#"{"alg":"HS256","crit":["urn:x"],"urn:x":true,"z":1e400}"#,
            r#"{"alg":"HS256","crit":["urn:x"],"urn:x":true,"z":"\ud800"}"#,
            nested.as_str(),
            r#"{"alg":"HS256","b64":false,"crit":["b64"],"z":1e999}"#,
            r#"{"alg":"HS256","crit":["urn:x"],"urn:x":true}"#,
            "[1]",
        ];
        for header in headers {
            assert!(jwt.verify::<Value>(&forge(header, &claims, b"s")).is_err(), "{header}");
        }
        // Raw invalid UTF-8 cannot go through a `&str`: build that token by hand.
        let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
        let mut bytes = br#"{"alg":"HS256","crit":["urn:x"],"z":""#.to_vec();
        bytes.extend([0xFF, 0xFE]);
        bytes.extend(br#""}"#);
        let message = format!("{}.{}", b64(&bytes), b64(claims.to_string().as_bytes()));
        let sig = jsonwebtoken::crypto::sign(message.as_bytes(), &EncodingKey::from_secret(b"s"), Algorithm::HS256).unwrap();
        assert!(jwt.verify::<Value>(&format!("{message}.{sig}")).is_err());
        assert!(jwt.verify::<Value>("a.a.a").is_err());
        // Another case of the name is not `crit`.
        assert!(jwt.verify::<Value>(&forge(r#"{"alg":"HS256","CRIT":["x"]}"#, &claims, b"s")).is_ok());
    }

    /// node-jsonwebtoken: `secretOrPrivateKey must have a value` / `secret or public key must be provided`.
    #[test]
    fn empty_secret_is_rejected() {
        let jwt = JwtService::new(JwtModuleOptions::new(""));
        assert!(jwt.sign(&serde_json::json!({ "sub": 1 })).is_err());
        let opts = JwtSignOptions { secret: Some(String::new()), ..Default::default() };
        assert!(jwt.sign_with(&serde_json::json!({ "sub": 1 }), opts).is_err());
        // An empty per-call secret falls back to the module secret.
        let ok = JwtService::new(JwtModuleOptions::new("s"));
        let opts = JwtSignOptions { secret: Some(String::new()), ..Default::default() };
        let token = ok.sign_with(&serde_json::json!({ "sub": 1 }), opts).unwrap();
        assert!(ok.verify::<Value>(&token).is_ok());
        let forged = forge(HS256, &serde_json::json!({ "username": "admin" }), b"");
        assert!(jwt.verify::<Value>(&forged).is_err());
    }

    /// `expires_in` follows the `ms` grammar; anything else is an error, never an eternal token.
    #[test]
    fn expires_in_uses_the_ms_grammar() {
        assert_eq!(parse_duration_secs("1.5h"), Some(5400));
        assert_eq!(parse_duration_secs("2.5 days"), Some(216_000));
        assert_eq!(parse_duration_secs("-1h"), Some(-3600));
        assert_eq!(parse_duration_secs(".5m"), Some(30));
        assert_eq!(parse_duration_secs("15mins"), Some(900));
        assert_eq!(parse_duration_secs("1 Year"), Some(31_557_600));
        assert_eq!(parse_duration_secs("100ms"), Some(0));
        assert_eq!(parse_duration_secs("3600"), Some(3600));
        assert_eq!(parse_duration_secs("1d"), Some(86_400));
        assert_eq!(parse_duration_secs("8h"), Some(28_800));
        for bad in ["1h30m", "", "abc", "1.h", "+1h", "1e3s", "99999999999999999999999y"] {
            assert_eq!(parse_duration_secs(bad), None, "{bad}");
        }
        let jwt = JwtService::new(JwtModuleOptions::new("s3cr3t").expires_in("1h30m"));
        assert!(jwt.sign(&serde_json::json!({ "sub": 1 })).is_err());
        let jwt = JwtService::new(JwtModuleOptions::new("s3cr3t").expires_in("1.5h"));
        let claims: Value = jwt.verify(&jwt.sign(&serde_json::json!({ "sub": 1 })).unwrap()).unwrap();
        assert_eq!(claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(), 5400);
        let opts = JwtSignOptions { expires_in: Some("-1h".into()), ..Default::default() };
        let expired = jwt.sign_with(&serde_json::json!({ "sub": 1 }), opts).unwrap();
        assert!(jwt.verify::<Value>(&expired).is_err());
        let opts = JwtSignOptions { expires_in: Some("forever".into()), ..Default::default() };
        assert!(jwt.sign_with(&serde_json::json!({ "sub": 1 }), opts).is_err());
    }

    /// `nbf` in the future and non-numeric `exp`/`nbf`/`iat` are rejected, like node-jsonwebtoken.
    #[test]
    fn registered_claims_are_validated() {
        let jwt = JwtService::new(JwtModuleOptions::new("k"));
        let now = chrono::Utc::now().timestamp();
        let check = |claims: Value| jwt.verify::<Value>(&forge(HS256, &claims, b"k"));
        assert!(check(serde_json::json!({ "sub": 1, "nbf": now + 3600 })).is_err());
        assert!(check(serde_json::json!({ "sub": 1, "nbf": now - 10 })).is_ok());
        assert!(check(serde_json::json!({ "sub": 1, "exp": "1" })).is_err());
        assert!(check(serde_json::json!({ "sub": 1, "exp": -1 })).is_err());
        assert!(check(serde_json::json!({ "sub": 1, "exp": null })).is_err());
        assert!(check(serde_json::json!({ "sub": 1, "exp": now })).is_err());
        assert!(check(serde_json::json!({ "sub": 1, "exp": now + 60 })).is_ok());
        assert!(check(serde_json::json!({ "sub": 1, "nbf": "0" })).is_err());
        assert!(check(serde_json::json!({ "sub": 1, "iat": "now" })).is_err());
    }

    /// `public_key` / `private_key` for RS256, with an empty secret.
    #[test]
    fn asymmetric_keys() {
        let jwt = JwtService::new(
            JwtModuleOptions::new("")
                .algorithm(Algorithm::RS256)
                .private_key(TEST_RSA_PRIVATE)
                .public_key(TEST_RSA_PUBLIC)
                .expires_in("1h"),
        );
        let token = jwt.sign(&serde_json::json!({ "sub": 7 })).unwrap();
        assert!(token.starts_with("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9."));
        let claims: Value = jwt.verify(&token).unwrap();
        assert_eq!(claims["sub"], 7);
        // An HS256 token "signed" with the public key is not accepted (no algorithm confusion).
        let confused = forge(HS256, &serde_json::json!({ "sub": 7 }), TEST_RSA_PUBLIC.as_bytes());
        assert!(jwt.verify::<Value>(&confused).is_err());
    }

    #[test]
    fn time_claims_follow_the_boundaries_of_node_jsonwebtoken() {
        let rules = TimeRules { tolerance: 30, ..Default::default() };
        let check = |claims: Value, rules: &TimeRules| check_time_claims(&claims, rules, 1000).map_err(|e| e.to_string());
        // exp: `now >= exp + tolerance` fails, one second before passes.
        assert_eq!(check(serde_json::json!({ "exp": 970 }), &rules), Err("jwt expired".into()));
        assert_eq!(check(serde_json::json!({ "exp": 971 }), &rules), Ok(()));
        assert_eq!(check(serde_json::json!({ "exp": 1000 }), &TimeRules::default()), Err("jwt expired".into()));
        assert_eq!(check(serde_json::json!({ "exp": 1001 }), &TimeRules::default()), Ok(()));
        // nbf: `nbf > now + tolerance` fails, equal passes.
        assert_eq!(check(serde_json::json!({ "nbf": 1030 }), &rules), Ok(()));
        assert_eq!(check(serde_json::json!({ "nbf": 1031 }), &rules), Err("jwt not active".into()));
        assert_eq!(check(serde_json::json!({ "nbf": 1000 }), &TimeRules::default()), Ok(()));
        assert_eq!(check(serde_json::json!({ "nbf": 1001 }), &TimeRules::default()), Err("jwt not active".into()));
        // Types.
        assert_eq!(check(serde_json::json!({ "exp": "2099" }), &rules), Err("invalid exp value".into()));
        assert_eq!(check(serde_json::json!({ "nbf": null }), &rules), Err("invalid nbf value".into()));
        assert_eq!(check(serde_json::json!({ "iat": "x" }), &rules), Err("invalid iat value".into()));
        // require_exp and ignore_expiration.
        let strict = TimeRules { require_exp: true, ..Default::default() };
        assert_eq!(check(serde_json::json!({}), &strict), Err("exp claim required".into()));
        assert_eq!(check(serde_json::json!({ "exp": 2000 }), &strict), Ok(()));
        let ignore = TimeRules { require_exp: true, ignore_expiration: true, ..Default::default() };
        assert_eq!(check(serde_json::json!({}), &ignore), Ok(()));
        assert_eq!(check(serde_json::json!({ "exp": 1 }), &ignore), Ok(()));
        assert_eq!(check(serde_json::json!({ "exp": "junk" }), &ignore), Ok(()));
    }

    const TEST_RSA_PRIVATE: &str = include_str!("../tests/fixtures/jwt_rsa_private.pem");
    const TEST_RSA_PUBLIC: &str = include_str!("../tests/fixtures/jwt_rsa_public.pem");

    fn rs256_service() -> JwtService {
        JwtService::new(
            JwtModuleOptions::new("")
                .algorithm(Algorithm::RS256)
                .private_key(TEST_RSA_PRIVATE)
                .public_key(TEST_RSA_PUBLIC)
                .expires_in("1h"),
        )
    }

    fn key_parses() -> usize {
        KEY_PARSES.with(|c| c.get())
    }

    fn header_checks() -> usize {
        HEADER_CHECKS.with(|c| c.get())
    }

    /// Spec "Many calls, no parse": the module keys are parsed by `new`, never by `sign` or `verify`.
    #[test]
    fn module_keys_are_parsed_once() {
        let before = key_parses();
        let jwt = rs256_service();
        assert_eq!(key_parses() - before, 2, "the encoding and the decoding key, once each");
        let created = key_parses();
        for i in 0..100 {
            let token = jwt.sign(&serde_json::json!({ "sub": i })).unwrap();
            let claims: Value = jwt.clone().verify(&token).unwrap();
            assert_eq!(claims["sub"], i);
        }
        assert_eq!(key_parses(), created, "no module key is parsed after the service was created");
        // A key given for one call is parsed for that call.
        let opts = JwtSignOptions { secret: Some("per-call".into()), ..Default::default() };
        let _ = JwtService::new(JwtModuleOptions::new("module")).sign_with(&serde_json::json!({ "sub": 1 }), opts).unwrap();
        assert!(key_parses() > created);
    }

    /// Spec "A key that does not parse": the service is created, and each call fails with an error of
    /// `jsonwebtoken`'s type, as before.
    #[test]
    fn a_key_that_does_not_parse_fails_each_call_with_its_type() {
        let bad = JwtService::new(JwtModuleOptions::new("").algorithm(Algorithm::RS256).private_key("not a pem").public_key("nor this"));
        for _ in 0..2 {
            let e = bad.sign(&serde_json::json!({ "sub": 1 })).unwrap_err();
            assert!(e.downcast_ref::<jsonwebtoken::errors::Error>().is_some(), "{e}");
            let e = bad.verify::<Value>("a.b.c").unwrap_err();
            assert!(e.downcast_ref::<jsonwebtoken::errors::Error>().is_some(), "{e}");
        }
        let none = JwtService::new(JwtModuleOptions::new(""));
        assert_eq!(none.sign(&serde_json::json!({})).unwrap_err().to_string(), "secretOrPrivateKey must have a value");
        assert_eq!(none.verify::<Value>("a.b.c").unwrap_err().to_string(), "secret or public key must be provided");
    }

    #[test]
    fn verify_errors_keep_their_type_with_cached_keys() {
        use jsonwebtoken::errors::ErrorKind;
        let kind = |e: anyhow::Error| e.downcast_ref::<jsonwebtoken::errors::Error>().map(|e| e.kind().clone());
        let jwt = JwtService::new(JwtModuleOptions::new("k").issuer("https://auth.example").audience("web"));
        let ok = serde_json::json!({ "sub": 1, "iss": "https://auth.example", "aud": "web" });
        for _ in 0..2 {
            assert!(jwt.verify::<Value>(&forge(HS256, &ok, b"k")).is_ok());
            assert!(matches!(kind(jwt.verify::<Value>(&forge(HS256, &ok, b"other")).unwrap_err()), Some(ErrorKind::InvalidSignature)));
            let no_iss = serde_json::json!({ "sub": 1, "aud": "web" });
            assert!(matches!(kind(jwt.verify::<Value>(&forge(HS256, &no_iss, b"k")).unwrap_err()), Some(ErrorKind::InvalidIssuer)));
            let other_aud = serde_json::json!({ "sub": 1, "iss": "https://auth.example", "aud": "app" });
            assert!(matches!(kind(jwt.verify::<Value>(&forge(HS256, &other_aud, b"k")).unwrap_err()), Some(ErrorKind::InvalidAudience)));
        }
        let rs = rs256_service();
        assert!(matches!(kind(rs.verify::<Value>(&forge(HS256, &ok, b"k")).unwrap_err()), Some(ErrorKind::InvalidAlgorithm)));
    }

    /// `verify` checks the header itself, once, now that `decode_claims` does not.
    #[test]
    fn verify_still_refuses_crit_after_the_single_header_check() {
        let jwt = JwtService::new(JwtModuleOptions::new("s"));
        let claims = serde_json::json!({ "sub": 1 });
        let before = header_checks();
        let e = jwt.verify::<Value>(&forge(r#"{"alg":"HS256","crit":["urn:x"],"urn:x":true}"#, &claims, b"s")).unwrap_err();
        assert_eq!(e.to_string(), "critical header parameters are not supported");
        assert_eq!(jwt.verify::<Value>(&forge("[1]", &claims, b"s")).unwrap_err().to_string(), "jwt malformed");
        assert!(jwt.verify::<Value>(&forge(HS256, &claims, b"s")).is_ok());
        assert_eq!(header_checks() - before, 3, "one header check per call");
    }

    /// What the original `JwtService.sign` returned or threw, recorded by `tests/fixtures/auth/jwt_sign.mjs`.
    const SIGN_FIXTURE: &str = include_str!("../tests/fixtures/auth/jwt_sign.json");

    /// An option of the recording as Corvis takes it: a timespan string as it is, a number `n` as `"n"` (a bare number
    /// is seconds in Corvis, the documented reading).
    fn option_text(value: &Value) -> Option<String> {
        match value {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }

    fn fixture_module(module: &Value) -> JwtModuleOptions {
        let key = |name: &Value| match name.as_str() {
            Some("s1") => "corvis-jwt-secret-one-0123456789abcdef",
            Some("rsa") => TEST_RSA_PRIVATE,
            other => panic!("unknown key {other:?}"),
        };
        let mut options = match (module.get("secret"), module.get("privateKey")) {
            (Some(secret), None) => JwtModuleOptions::new(key(secret)),
            (None, Some(private)) => JwtModuleOptions::new("").private_key(key(private)),
            _ => panic!("a module has a secret or a private key: {module}"),
        };
        if let Some(alg) = module.get("algorithm").and_then(Value::as_str) {
            options = options.algorithm(serde_json::from_value(Value::String(alg.into())).unwrap());
        }
        options.expires_in = module.get("expiresIn").and_then(option_text);
        options.issuer = module.get("issuer").and_then(option_text);
        options.audience = module.get("audience").and_then(option_text);
        options
    }

    fn fixture_call(call: &Value) -> JwtSignOptions {
        let field = |name: &str| call.get(name).and_then(option_text);
        JwtSignOptions {
            expires_in: field("expiresIn"),
            subject: field("subject"),
            jwt_id: field("jwtid"),
            not_before: field("notBefore"),
            // `mutatePayload` has no counterpart: Corvis borrows the payload (the case carries a divergence).
            ..Default::default()
        }
    }

    /// `JwtService::sign` returns what the original returns, byte for byte, or fails with its error text, for every
    /// recorded case at the recorded clock; a case with a `divergence` returns what the divergence says.
    #[test]
    fn sign_matches_the_recorded_original() {
        let fixture: Value = serde_json::from_str(SIGN_FIXTURE).unwrap();
        let clock = fixture["clock"].as_i64().unwrap();
        let mut failures = Vec::new();
        let cases = fixture["cases"].as_array().unwrap();
        assert!(cases.len() >= 34, "the recording has the cases of the design");
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let service = JwtService::new(fixture_module(&case["module"]));
            let options = if case["call"].is_null() { JwtSignOptions::default() } else { fixture_call(&case["call"]) };
            let expected = match &case["divergence"] {
                Value::Null => {
                    assert_eq!(case["payloadAfter"], case["payload"], "{name}: the original copies the payload");
                    &case["result"]
                }
                divergence => {
                    assert!(divergence["reason"].as_str().is_some_and(|r| !r.is_empty()), "{name}: a divergence has its reason");
                    &divergence["result"]
                }
            };
            let got = match service.sign_at(&case["payload"], options, clock) {
                Ok(token) => serde_json::json!({ "token": token }),
                Err(e) => serde_json::json!({ "error": e.to_string() }),
            };
            if &got != expected {
                failures.push(format!("{name}: expected {expected}, got {got}"));
            }
        }
        assert!(failures.is_empty(), "{} case(s) differ:\n{}", failures.len(), failures.join("\n"));
    }
}
