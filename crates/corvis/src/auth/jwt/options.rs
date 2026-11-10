//! The options of a JWT strategy, their startup checks and the warnings about risky settings.

use std::sync::Arc;
use std::time::Duration;

use jsonwebtoken::{Algorithm, DecodingKey};

use super::extract::JwtExtractor;
use super::key::{JwtKey, JwtKeyProvider, KeyFamily, allowed_for, default_algorithms, family_of};
use crate::jwt::TimeRules;

/// Largest `clock_tolerance` the strategy starts with.
pub const MAX_CLOCK_TOLERANCE: Duration = Duration::from_secs(300);

/// The default of [`max_token_length`](JwtStrategyOptions::max_token_length).
pub const DEFAULT_MAX_TOKEN_LENGTH: usize = 8192;

/// The default of [`key_provider_timeout`](JwtStrategyOptions::key_provider_timeout) and of
/// [`validator_timeout`](JwtStrategyOptions::validator_timeout).
pub const DEFAULT_CALLBACK_TIMEOUT: Duration = Duration::from_secs(5);

const MIN_CALLBACK_TIMEOUT: Duration = Duration::from_millis(1);
const MAX_CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

/// Shortest HMAC secret the strategy starts with (RFC 7518 section 3.2 asks for at least the size of the hash).
pub const MIN_SECRET_LENGTH: usize = 32;

/// One value or a list of values, for `issuer` and `audience`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringList(Vec<String>);

impl From<&str> for StringList {
    fn from(v: &str) -> Self {
        StringList(vec![v.to_string()])
    }
}

impl From<String> for StringList {
    fn from(v: String) -> Self {
        StringList(vec![v])
    }
}

impl<const N: usize> From<[&str; N]> for StringList {
    fn from(v: [&str; N]) -> Self {
        StringList(v.iter().map(|s| s.to_string()).collect())
    }
}

impl From<Vec<String>> for StringList {
    fn from(v: Vec<String>) -> Self {
        StringList(v)
    }
}

impl From<Vec<&str>> for StringList {
    fn from(v: Vec<&str>) -> Self {
        StringList(v.into_iter().map(str::to_string).collect())
    }
}

/// The options of a JWT strategy, with the names of `passport-jwt`'s. Returned by
/// [`JwtValidator::options`](super::JwtValidator::options) and checked once, when the application starts.
///
/// ```ignore
/// JwtStrategyOptions::new()
///     .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
///     .secret_or_key(JwtKey::secret(config.get("JWT_SECRET")?))
///     .issuer("https://auth.example")
///     .audience(["web", "mobile"])
///     .algorithms([JwtAlgorithm::HS256])
/// ```
#[derive(Clone)]
pub struct JwtStrategyOptions {
    pub(crate) extractor: Option<JwtExtractor>,
    pub(crate) key: Option<JwtKey>,
    pub(crate) provider: Option<Arc<dyn JwtKeyProvider>>,
    pub(crate) issuer: Option<StringList>,
    pub(crate) audience: Option<StringList>,
    pub(crate) algorithms: Option<Vec<Algorithm>>,
    pub(crate) ignore_expiration: bool,
    pub(crate) allow_missing_exp: bool,
    pub(crate) clock_tolerance: Duration,
    pub(crate) max_token_length: usize,
    pub(crate) key_provider_timeout: Duration,
    pub(crate) validator_timeout: Duration,
}

impl Default for JwtStrategyOptions {
    fn default() -> Self {
        JwtStrategyOptions {
            extractor: None,
            key: None,
            provider: None,
            issuer: None,
            audience: None,
            algorithms: None,
            ignore_expiration: false,
            allow_missing_exp: false,
            clock_tolerance: Duration::ZERO,
            max_token_length: DEFAULT_MAX_TOKEN_LENGTH,
            key_provider_timeout: DEFAULT_CALLBACK_TIMEOUT,
            validator_timeout: DEFAULT_CALLBACK_TIMEOUT,
        }
    }
}

/// Prints which options are set, never the key.
impl std::fmt::Debug for JwtStrategyOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwtStrategyOptions")
            .field("extractor", &self.extractor)
            .field("key", &self.key)
            .field("key_provider", &self.provider.is_some())
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("algorithms", &self.algorithms)
            .field("ignore_expiration", &self.ignore_expiration)
            .field("allow_missing_exp", &self.allow_missing_exp)
            .field("clock_tolerance", &self.clock_tolerance)
            .field("max_token_length", &self.max_token_length)
            .field("key_provider_timeout", &self.key_provider_timeout)
            .field("validator_timeout", &self.validator_timeout)
            .finish()
    }
}

impl JwtStrategyOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// `jwtFromRequest`, required: where the token is read from.
    pub fn jwt_from_request(mut self, extractor: JwtExtractor) -> Self {
        self.extractor = Some(extractor);
        self
    }

    /// `secretOrKey`: the key every token is verified with. Give this or a provider, not both.
    pub fn secret_or_key(mut self, key: JwtKey) -> Self {
        self.key = Some(key);
        self
    }

    /// `secretOrKeyProvider`: chooses the key per request.
    pub fn secret_or_key_provider(mut self, provider: impl JwtKeyProvider) -> Self {
        self.provider = Some(Arc::new(provider));
        self
    }

    /// `issuer`: the token's `iss` must equal one of the values.
    pub fn issuer(mut self, issuer: impl Into<StringList>) -> Self {
        self.issuer = Some(issuer.into());
        self
    }

    /// `audience`: the token's `aud` (a string or a list) must contain one of the values.
    pub fn audience(mut self, audience: impl Into<StringList>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    /// `algorithms`: the algorithms a token may use. By default, the ones of the key's family: HS256, HS384 and HS512
    /// for a secret; RS* and PS* for an RSA key; ES256 and ES384 for an EC key. An Ed25519 key needs
    /// `algorithms([EdDSA])`. With a static key the list must belong to the key's family; with a key provider it is
    /// narrowed to the family of the key each request gets. `none` cannot be expressed.
    pub fn algorithms(mut self, algorithms: impl IntoIterator<Item = Algorithm>) -> Self {
        self.algorithms = Some(algorithms.into_iter().collect());
        self
    }

    /// `ignoreExpiration`: skip the `exp` check and the `exp` requirement. Logs a startup warning.
    pub fn ignore_expiration(mut self, ignore: bool) -> Self {
        self.ignore_expiration = ignore;
        self
    }

    /// Accepts tokens without an `exp` claim, which never expire (they fail by default; `passport-jwt` accepts them).
    /// Logs a startup warning.
    pub fn allow_missing_exp(mut self) -> Self {
        self.allow_missing_exp = true;
        self
    }

    /// `clockTolerance`: seconds of skew allowed on `exp` and `nbf`, 0 by default and at most 300.
    pub fn clock_tolerance(mut self, tolerance: Duration) -> Self {
        self.clock_tolerance = tolerance;
        self
    }

    /// The longest token accepted, in bytes (8192 by default). It is checked before the token is decoded.
    pub fn max_token_length(mut self, bytes: usize) -> Self {
        self.max_token_length = bytes;
        self
    }

    /// How long the key provider may run for one request, 5 seconds by default, 1 ms to 300 s. The provider runs for
    /// every well-formed token, forged ones included. One that takes longer is dropped (its future is cancelled) and
    /// the request is answered with a `503 Service Unavailable` that is the same for every token.
    pub fn key_provider_timeout(mut self, timeout: Duration) -> Self {
        self.key_provider_timeout = timeout;
        self
    }

    /// How long the validator may run, 5 seconds by default, 1 ms to 300 s. A validator that takes longer is dropped
    /// and the request is answered with the same `503 Service Unavailable`.
    pub fn validator_timeout(mut self, timeout: Duration) -> Self {
        self.validator_timeout = timeout;
        self
    }
}

/// How the strategy gets its key.
pub(crate) enum KeySource {
    Static { key: DecodingKey, allowed: Vec<Algorithm> },
    Provider(Arc<dyn JwtKeyProvider>),
}

/// The options after the startup checks.
pub(crate) struct Resolved {
    pub(crate) extractor: JwtExtractor,
    pub(crate) key: KeySource,
    /// The configured list; per request it is narrowed to the family of the key.
    pub(crate) algorithms: Option<Vec<Algorithm>>,
    pub(crate) issuer: Option<Vec<String>>,
    pub(crate) audience: Option<Vec<String>>,
    pub(crate) rules: TimeRules,
    pub(crate) max_token_length: usize,
    pub(crate) key_provider_timeout: Duration,
    pub(crate) validator_timeout: Duration,
    pub(crate) warnings: Vec<String>,
}

fn check_list(what: &str, list: &Option<StringList>) -> Result<Option<Vec<String>>, String> {
    match list {
        None => Ok(None),
        Some(StringList(values)) if values.is_empty() => Err(format!("{what} is an empty list")),
        Some(StringList(values)) if values.iter().any(String::is_empty) => Err(format!("{what} has an empty value")),
        Some(StringList(values)) => Ok(Some(values.clone())),
    }
}

/// Checks a secret's size without printing it.
pub(crate) fn check_secret(key: &JwtKey) -> Result<(), String> {
    match key.secret_len() {
        Some(0) => Err("the HMAC secret is empty".into()),
        Some(n) if n < MIN_SECRET_LENGTH => {
            Err(format!("the HMAC secret is {n} bytes long; use at least {MIN_SECRET_LENGTH} bytes (RFC 7518 section 3.2)"))
        }
        _ => Ok(()),
    }
}

impl JwtStrategyOptions {
    /// The startup checks. The error names the problem; the caller adds the strategy.
    pub(crate) fn resolve(&self) -> Result<Resolved, String> {
        let extractor = self
            .extractor
            .clone()
            .ok_or("no token extractor: set jwt_from_request (tokens are read only from the places you configure)")?;
        if self.max_token_length == 0 {
            return Err("max_token_length is 0".into());
        }
        for (name, timeout) in [("key_provider_timeout", self.key_provider_timeout), ("validator_timeout", self.validator_timeout)] {
            if !(MIN_CALLBACK_TIMEOUT..=MAX_CALLBACK_TIMEOUT).contains(&timeout) {
                return Err(format!("{name} must be between 1 ms and 300 s"));
            }
        }
        if self.clock_tolerance > MAX_CLOCK_TOLERANCE {
            return Err(format!(
                "clock_tolerance is {}s; the most allowed is {}s",
                self.clock_tolerance.as_secs(),
                MAX_CLOCK_TOLERANCE.as_secs()
            ));
        }
        let issuer = check_list("issuer", &self.issuer)?;
        let audience = check_list("audience", &self.audience)?;
        if let Some(list) = &self.algorithms
            && list.is_empty()
        {
            return Err("algorithms is an empty list".into());
        }
        let key = match (&self.key, &self.provider) {
            (Some(_), Some(_)) => return Err("both secret_or_key and secret_or_key_provider are set; give one".into()),
            (None, None) => return Err("no key: set secret_or_key or secret_or_key_provider".into()),
            (None, Some(provider)) => KeySource::Provider(provider.clone()),
            (Some(key), None) => {
                check_secret(key)?;
                let family = key.family();
                let decoding = key.decoding_key()?;
                let allowed = match &self.algorithms {
                    Some(list) => {
                        let families: Vec<KeyFamily> = list.iter().map(|a| family_of(*a)).collect();
                        if families.iter().any(|f| *f != families[0]) {
                            return Err("algorithms mixes key families; with a fixed key list algorithms of one family (several are allowed with a key provider)".into());
                        }
                        if families[0] != family {
                            return Err(format!("algorithms are {} algorithms but the key is {}", families[0].name(), family.name()));
                        }
                        list.clone()
                    }
                    None if default_algorithms(family).is_empty() => {
                        return Err(format!("a {} key needs an explicit algorithms list (for example [EdDSA])", family.name()));
                    }
                    None => allowed_for(family, None),
                };
                KeySource::Static { key: decoding, allowed }
            }
        };
        let mut warnings = Vec::new();
        if self.ignore_expiration {
            warnings.push("ignore_expiration is on: expired tokens are accepted".to_string());
        }
        if self.allow_missing_exp {
            warnings.push("allow_missing_exp is on: tokens without an exp claim are accepted and never expire".to_string());
        }
        if extractor.reads_query() {
            warnings
                .push("the token is read from the URL query: URLs end up in access logs, browser history and Referer headers".to_string());
        }
        for name in extractor.cookie_names() {
            if !name.starts_with("__Host-") {
                warnings.push(format!(
                    "the token is read from the cookie \"{name}\", which has no __Host- prefix: a page on a sibling subdomain can plant a cookie of that name (a request that carries it twice is refused, but a planted one can still be the only one); name it __Host-... on a Secure HTTPS host with Path=/ and no Domain"
                ));
            }
        }
        Ok(Resolved {
            extractor,
            key,
            algorithms: self.algorithms.clone(),
            issuer,
            audience,
            rules: TimeRules {
                tolerance: self.clock_tolerance.as_secs(),
                ignore_expiration: self.ignore_expiration,
                require_exp: !self.allow_missing_exp,
            },
            max_token_length: self.max_token_length,
            key_provider_timeout: self.key_provider_timeout,
            validator_timeout: self.validator_timeout,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::extract::ExtractJwt;
    use super::*;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";
    const RSA_PUBLIC: &str = include_str!("../../../tests/fixtures/jwt_rsa_public.pem");

    fn base() -> JwtStrategyOptions {
        JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key(JwtKey::secret(SECRET))
    }

    fn error(options: JwtStrategyOptions) -> String {
        match options.resolve() {
            Ok(_) => panic!("the options were accepted"),
            Err(e) => e,
        }
    }

    struct NoKeys;

    #[async_trait::async_trait]
    impl JwtKeyProvider for NoKeys {
        async fn key(&self, _: &crate::http::Request, _: &str) -> anyhow::Result<JwtKey> {
            anyhow::bail!("none")
        }
    }

    #[test]
    fn valid_options_resolve_with_the_documented_defaults() {
        let r = base().resolve().unwrap();
        assert_eq!(r.max_token_length, 8192);
        assert_eq!(r.rules.tolerance, 0);
        assert!(r.rules.require_exp && !r.rules.ignore_expiration);
        assert!(r.warnings.is_empty());
        match r.key {
            KeySource::Static { allowed, .. } => {
                assert_eq!(allowed, vec![Algorithm::HS256, Algorithm::HS384, Algorithm::HS512]);
            }
            KeySource::Provider(_) => panic!("static key expected"),
        }
    }

    #[test]
    fn extractor_and_key_are_required() {
        assert!(error(JwtStrategyOptions::new().secret_or_key(JwtKey::secret(SECRET))).contains("no token extractor"));
        let no_key = JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_cookie("jwt"));
        assert!(error(no_key).contains("no key"));
        assert!(error(base().secret_or_key_provider(NoKeys)).contains("both"));
        let with_provider = JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_cookie("jwt")).secret_or_key_provider(NoKeys);
        assert!(with_provider.resolve().is_ok());
    }

    #[test]
    fn secrets_and_keys_are_checked() {
        assert!(error(base().secret_or_key(JwtKey::secret(""))).contains("empty"));
        let short = error(base().secret_or_key(JwtKey::secret("changeme")));
        assert!(short.contains("8 bytes") && !short.contains("changeme"), "{short}");
        assert!(base().secret_or_key(JwtKey::secret(vec![b'x'; 32])).resolve().is_ok());
        assert!(error(base().secret_or_key(JwtKey::secret(vec![b'x'; 31]))).contains("31 bytes"));
        assert!(error(base().secret_or_key(JwtKey::rsa_pem("junk"))).contains("not valid PEM"));
    }

    #[test]
    fn algorithm_lists_are_checked_against_the_key() {
        assert!(error(base().algorithms([])).contains("empty list"));
        let rsa = base().secret_or_key(JwtKey::rsa_pem(RSA_PUBLIC));
        assert!(error(rsa.clone().algorithms([Algorithm::HS256])).contains("HMAC algorithms but the key is RSA"));
        assert!(error(rsa.clone().algorithms([Algorithm::RS256, Algorithm::ES256])).contains("mixes"));
        assert!(error(base().algorithms([Algorithm::HS256, Algorithm::RS256])).contains("mixes"));
        assert!(rsa.clone().algorithms([Algorithm::RS256]).resolve().is_ok());
        assert!(rsa.resolve().is_ok());
        let ed = base().secret_or_key(JwtKey::ed_pem(
            "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAGb9ECWmEzf6FQbrBZ9w7lshQhqowtrbLDFw4rXAxZuE=\n-----END PUBLIC KEY-----\n",
        ));
        assert!(error(ed.clone()).contains("explicit algorithms"));
        assert!(ed.algorithms([Algorithm::EdDSA]).resolve().is_ok());
        // Several families are fine with a provider.
        let provider = JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_cookie("jwt"))
            .secret_or_key_provider(NoKeys)
            .algorithms([Algorithm::HS256, Algorithm::RS256]);
        assert!(provider.resolve().is_ok());
    }

    #[test]
    fn issuer_audience_and_limits_are_checked() {
        assert!(error(base().audience("")).contains("audience has an empty value"));
        assert!(error(base().audience(["web", ""])).contains("empty value"));
        assert!(error(base().audience(Vec::<String>::new())).contains("audience is an empty list"));
        assert!(error(base().issuer("")).contains("issuer has an empty value"));
        assert!(error(base().issuer(Vec::<&str>::new())).contains("issuer is an empty list"));
        assert!(base().issuer("https://auth.example").audience(["web", "mobile"]).resolve().is_ok());
        assert!(error(base().clock_tolerance(Duration::from_secs(301))).contains("301s"));
        assert!(base().clock_tolerance(Duration::from_secs(300)).resolve().is_ok());
        assert!(error(base().clock_tolerance(Duration::from_millis(300_500))).contains("clock_tolerance"));
        assert!(error(base().max_token_length(0)).contains("max_token_length"));
        assert!(base().max_token_length(16384).resolve().is_ok());
    }

    #[test]
    fn risky_options_produce_warnings() {
        let r = base().ignore_expiration(true).resolve().unwrap();
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains("ignore_expiration"));
        assert!(!r.rules.require_exp || r.rules.ignore_expiration);
        let r = base().allow_missing_exp().resolve().unwrap();
        assert!(!r.rules.require_exp && r.warnings[0].contains("allow_missing_exp"));
        let query = JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_extractors([ExtractJwt::from_url_query_parameter("t")]))
            .secret_or_key(JwtKey::secret(SECRET));
        assert!(query.resolve().unwrap().warnings[0].contains("URL query"));
    }

    #[test]
    fn debug_output_does_not_print_the_key() {
        let printed = format!("{:?}", base());
        assert!(!printed.contains(SECRET), "{printed}");
    }
}
