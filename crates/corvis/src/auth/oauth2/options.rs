//! The options of an OAuth2 strategy, their defaults, bounds and validation.

use std::time::Duration;

use super::secret::ClientSecret;
use crate::http::signed_cookie::CookieKeys;

/// The default of [`OAuth2Options::state_ttl`].
pub const DEFAULT_STATE_TTL: Duration = Duration::from_secs(600);
/// The bounds of [`OAuth2Options::state_ttl`].
pub const MIN_STATE_TTL: Duration = Duration::from_secs(60);
pub const MAX_STATE_TTL: Duration = Duration::from_secs(3600);
/// The default of [`OAuth2Options::max_code_bytes`] and its bounds.
pub const DEFAULT_MAX_CODE_BYTES: usize = 2048;
pub const MIN_MAX_CODE_BYTES: usize = 16;
pub const MAX_MAX_CODE_BYTES: usize = 8192;
/// The default connect timeout (TCP and TLS) and its bounds.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const MIN_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
pub const MAX_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// The default total timeout of one request to the provider and its bounds.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
pub const MIN_TIMEOUT: Duration = Duration::from_secs(1);
pub const MAX_TIMEOUT: Duration = Duration::from_secs(60);
/// The default size limit of a token response and its bounds.
pub const DEFAULT_MAX_TOKEN_BYTES: usize = 64 * 1024;
pub const MIN_MAX_TOKEN_BYTES: usize = 1024;
pub const MAX_MAX_TOKEN_BYTES: usize = 1024 * 1024;
/// The default size limit of each profile response and its bounds.
pub const DEFAULT_MAX_PROFILE_BYTES: usize = 256 * 1024;
pub const MIN_MAX_PROFILE_BYTES: usize = 1024;
pub const MAX_MAX_PROFILE_BYTES: usize = 4 * 1024 * 1024;
/// The default number of requests to the provider in flight per strategy and its bounds.
pub const DEFAULT_MAX_IN_FLIGHT: usize = 32;
pub const MIN_MAX_IN_FLIGHT: usize = 1;
pub const MAX_MAX_IN_FLIGHT: usize = 1024;
/// The default of [`OAuth2Options::redeemed_states_max`] and its bounds.
pub const DEFAULT_REDEEMED_STATES_MAX: usize = 100_000;
pub const MIN_REDEEMED_STATES_MAX: usize = 16;
pub const MAX_REDEEMED_STATES_MAX: usize = 10_000_000;

/// The most scope values and extra authorization parameters accepted.
pub const MAX_SCOPES: usize = 64;
pub const MAX_AUTHORIZATION_PARAMS: usize = 16;
/// The longest scope value, parameter name or parameter value (bytes).
pub const MAX_SCOPE_BYTES: usize = 128;
pub const MAX_PARAM_BYTES: usize = 256;
/// The longest URL accepted in the options.
const MAX_URL_BYTES: usize = 2048;
/// The longest relative path `return_to` and `failure_redirect` accept.
pub const MAX_RELATIVE_PATH_BYTES: usize = 1024;
/// The longest extra trust root (DER bytes) and the most roots accepted.
const MAX_ROOT_BYTES: usize = 16 * 1024;
const MAX_EXTRA_ROOTS: usize = 16;

/// The authorization request parameters the strategy owns: an extra parameter cannot use them.
const RESERVED_PARAMS: [&str; 7] =
    ["response_type", "client_id", "redirect_uri", "scope", "state", "code_challenge", "code_challenge_method"];

/// A configuration the application refuses to start with. The text names the strategy and the option and never holds
/// the client secret or a cookie key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct OAuth2OptionsError(pub(crate) String);

/// How the client authenticates at the token endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClientAuth {
    /// `client_id` and `client_secret` in the form body, what `passport-oauth2` sends (the default).
    #[default]
    RequestBody,
    /// HTTP Basic with the form-encoded id and secret (RFC 6749 section 2.3.1), and nothing in the body.
    Basic,
}

/// Which profile mapping a strategy uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileKind {
    Generic,
    Google,
    GitHub,
}

/// The URLs of the provider. They come from the application's configuration only: nothing reads them from the
/// request or from the provider's answers.
#[derive(Debug, Clone)]
pub struct OAuth2Endpoints {
    pub(crate) authorization_url: String,
    pub(crate) token_url: String,
    pub(crate) userinfo_url: Option<String>,
    pub(crate) emails_url: Option<String>,
}

impl OAuth2Endpoints {
    fn same_as(&self, other: &OAuth2Endpoints) -> bool {
        self.authorization_url == other.authorization_url
            && self.token_url == other.token_url
            && self.userinfo_url == other.userinfo_url
            && self.emails_url == other.emails_url
    }

    /// The authorization and token endpoints.
    pub fn new(authorization_url: impl Into<String>, token_url: impl Into<String>) -> Self {
        OAuth2Endpoints { authorization_url: authorization_url.into(), token_url: token_url.into(), userinfo_url: None, emails_url: None }
    }

    /// The OpenID userinfo endpoint (for GitHub, the `/user` endpoint). Without one a generic strategy loads no
    /// profile.
    pub fn userinfo_url(mut self, url: impl Into<String>) -> Self {
        self.userinfo_url = Some(url.into());
        self
    }

    /// The e-mail list endpoint of the GitHub preset (`/user/emails`), read when the scope asks for it.
    pub fn emails_url(mut self, url: impl Into<String>) -> Self {
        self.emails_url = Some(url.into());
        self
    }
}

/// The limits of the private HTTP client that talks to the provider.
#[derive(Clone)]
pub struct OAuth2HttpOptions {
    pub(crate) connect_timeout: Duration,
    pub(crate) timeout: Duration,
    pub(crate) max_token_bytes: usize,
    pub(crate) max_profile_bytes: usize,
    pub(crate) max_in_flight: usize,
    pub(crate) max_in_flight_per_client: Option<usize>,
    pub(crate) extra_roots: Vec<Vec<u8>>,
}

impl OAuth2HttpOptions {
    /// The slots one client address may hold at once: the configured share, or a quarter of `max_in_flight` (at
    /// least 1).
    pub(crate) fn per_client_slots(&self) -> usize {
        self.max_in_flight_per_client.unwrap_or(self.max_in_flight / 4).max(1)
    }
}

impl Default for OAuth2HttpOptions {
    fn default() -> Self {
        OAuth2HttpOptions {
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            timeout: DEFAULT_TIMEOUT,
            max_token_bytes: DEFAULT_MAX_TOKEN_BYTES,
            max_profile_bytes: DEFAULT_MAX_PROFILE_BYTES,
            max_in_flight: DEFAULT_MAX_IN_FLIGHT,
            max_in_flight_per_client: None,
            extra_roots: Vec::new(),
        }
    }
}

impl std::fmt::Debug for OAuth2HttpOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2HttpOptions")
            .field("connect_timeout", &self.connect_timeout)
            .field("timeout", &self.timeout)
            .field("max_token_bytes", &self.max_token_bytes)
            .field("max_profile_bytes", &self.max_profile_bytes)
            .field("max_in_flight", &self.max_in_flight)
            .field("max_in_flight_per_client", &self.per_client_slots())
            .field("extra_roots", &self.extra_roots.len())
            .finish()
    }
}

impl OAuth2HttpOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// The deadline for TCP and TLS together, 5 s by default (1 s to 30 s).
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// The deadline of a whole request: resolution, connection, request and the entire response body. 10 s by
    /// default (1 s to 60 s).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The largest token response body, 64 KiB by default (1 KiB to 1 MiB).
    pub fn max_token_bytes(mut self, max: usize) -> Self {
        self.max_token_bytes = max;
        self
    }

    /// The largest body of a profile response (each profile request), 256 KiB by default (1 KiB to 4 MiB).
    pub fn max_profile_bytes(mut self, max: usize) -> Self {
        self.max_profile_bytes = max;
        self
    }

    /// The most requests to the provider in flight at once for the strategy, 32 by default (1 to 1024). A callback
    /// that finds the limit reached is answered 503 without contacting the provider.
    pub fn max_in_flight(mut self, max: usize) -> Self {
        self.max_in_flight = max;
        self
    }

    /// The most callbacks of one client address (the address the application sees, honouring `trust proxy`, IPv6
    /// reduced to its /64) that may be waiting on the provider at once: a quarter of `max_in_flight` by default, at
    /// least 1 (1 to 1024). A client over its share is answered 503 while other clients proceed.
    pub fn max_in_flight_per_client(mut self, max: usize) -> Self {
        self.max_in_flight_per_client = Some(max);
        self
    }

    /// Trusts one more root certificate (DER bytes), for a provider behind a private certificate authority. The
    /// bundled web PKI roots stay trusted; there is no way to turn certificate or host name verification off.
    pub fn extra_root_certificate(mut self, der: impl Into<Vec<u8>>) -> Self {
        self.extra_roots.push(der.into());
        self
    }
}

/// Where the state record of a login lives between the redirect and the callback.
#[derive(Clone)]
pub(crate) enum StateStorage {
    /// Not chosen: a startup error.
    Unset,
    /// The keys given to `state_cookie_secrets` were refused: a startup error with this text.
    Invalid(String),
    /// A cookie signed with these keys.
    Cookie(CookieKeys),
    /// The session of the request.
    Session,
}

/// The options of an OAuth2 strategy: the provider's URLs, the client credentials, the redirect URI and the limits.
/// Build one with [`OAuth2Options::google`], [`OAuth2Options::github`] or [`OAuth2Options::new`]; the strategy
/// validates it when the application starts.
///
/// ```ignore
/// OAuth2Options::google(client_id, client_secret, "https://app.example/auth/google/callback")
///     .scope(["openid", "email", "profile"])
///     .state_cookie(CookieKeys::new([secret])?)
///     .return_to_param("return_to")
///     .failure_redirect("/login?error=oauth")
/// ```
#[derive(Clone)]
pub struct OAuth2Options {
    pub(crate) kind: ProfileKind,
    pub(crate) endpoints: OAuth2Endpoints,
    pub(crate) client_id: String,
    pub(crate) client_secret: ClientSecret,
    pub(crate) redirect_uri: String,
    pub(crate) scope: Vec<String>,
    pub(crate) scope_separator: char,
    pub(crate) refresh_scope: Option<Vec<String>>,
    pub(crate) authorization_params: Vec<(String, String)>,
    pub(crate) client_auth: ClientAuth,
    pub(crate) pkce: bool,
    pub(crate) state_ttl: Duration,
    pub(crate) storage: StateStorage,
    pub(crate) cookie_secure: bool,
    pub(crate) cookie_name: Option<String>,
    pub(crate) return_to_param: Option<String>,
    pub(crate) failure_redirect: Option<String>,
    pub(crate) max_code_bytes: usize,
    pub(crate) redeemed_states_max: usize,
    pub(crate) http: OAuth2HttpOptions,
}

/// Prints the client id, the endpoint hosts, the redirect URI and the scope: never the secret or the keys.
impl std::fmt::Debug for OAuth2Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let host = |url: &str| url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        f.debug_struct("OAuth2Options")
            .field("client_id", &self.client_id)
            .field("authorization_host", &host(&self.endpoints.authorization_url))
            .field("token_host", &host(&self.endpoints.token_url))
            .field("userinfo_host", &self.endpoints.userinfo_url.as_deref().map(host))
            .field("redirect_uri", &self.redirect_uri)
            .field("scope", &self.scope)
            .field(
                "state_storage",
                &match self.storage {
                    StateStorage::Unset | StateStorage::Invalid(_) => "unset",
                    StateStorage::Cookie(_) => "cookie",
                    StateStorage::Session => "session",
                },
            )
            .field("pkce", &self.pkce)
            .field("client_auth", &self.client_auth)
            .field("http", &self.http)
            .finish_non_exhaustive()
    }
}

impl OAuth2Options {
    fn base(
        kind: ProfileKind,
        endpoints: OAuth2Endpoints,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        OAuth2Options {
            kind,
            endpoints,
            client_id: client_id.into(),
            client_secret: ClientSecret::new(client_secret),
            redirect_uri: redirect_uri.into(),
            scope: Vec::new(),
            scope_separator: ' ',
            refresh_scope: None,
            authorization_params: Vec::new(),
            client_auth: ClientAuth::RequestBody,
            pkce: true,
            state_ttl: DEFAULT_STATE_TTL,
            storage: StateStorage::Unset,
            cookie_secure: true,
            cookie_name: None,
            return_to_param: None,
            failure_redirect: None,
            max_code_bytes: DEFAULT_MAX_CODE_BYTES,
            redeemed_states_max: DEFAULT_REDEEMED_STATES_MAX,
            http: OAuth2HttpOptions::default(),
        }
    }

    /// A generic provider. `redirect_uri` is the absolute URL of the callback route: it is sent as is in the
    /// authorization and the token request and is never built from the request.
    pub fn new(
        endpoints: OAuth2Endpoints,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        Self::base(ProfileKind::Generic, endpoints, client_id, client_secret, redirect_uri)
    }

    /// Google: the endpoints of `passport-google-oauth20` 2.0.0 and its OpenID userinfo profile. A scope is required
    /// (`["openid", "email", "profile"]` is the usual one).
    pub fn google(client_id: impl Into<String>, client_secret: impl Into<String>, redirect_uri: impl Into<String>) -> Self {
        Self::base(ProfileKind::Google, Self::google_endpoints(), client_id, client_secret, redirect_uri)
    }

    /// GitHub: the endpoints of `passport-github2` 0.1.12. Scope values are joined with a comma, as that package does.
    /// Ask for `user:email` to get the verified e-mail addresses.
    pub fn github(client_id: impl Into<String>, client_secret: impl Into<String>, redirect_uri: impl Into<String>) -> Self {
        let mut options = Self::base(ProfileKind::GitHub, Self::github_endpoints(), client_id, client_secret, redirect_uri);
        options.scope_separator = ',';
        options
    }

    /// Replaces the URLs, keeping the profile mapping of the preset: for a provider behind a proxy, a test double, or
    /// a self-hosted GitHub Enterprise.
    pub fn endpoints(mut self, endpoints: OAuth2Endpoints) -> Self {
        self.endpoints = endpoints;
        self
    }

    /// The scopes of the authorization request. A guard can replace them for one route with
    /// `AuthGuard::with_options(json!({ "scope": [..] }))`.
    pub fn scope<I, S>(mut self, scope: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.scope = scope.into_iter().map(Into::into).collect();
        self
    }

    /// The scope sent with a refresh request ([`OAuth2Client::refresh`](super::OAuth2Client::refresh)); none by
    /// default, which keeps the scope of the original grant.
    pub fn refresh_scope<I, S>(mut self, scope: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.refresh_scope = Some(scope.into_iter().map(Into::into).collect());
        self
    }

    /// An extra parameter of the authorization request (`access_type=offline`, `prompt=consent`, `hd=example.com`).
    /// At most 16; `response_type`, `client_id`, `redirect_uri`, `scope`, `state`, `code_challenge` and
    /// `code_challenge_method` are the strategy's and a startup error here.
    pub fn authorization_param(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.authorization_params.push((name.into(), value.into()));
        self
    }

    /// How the client authenticates at the token endpoint: [`ClientAuth::RequestBody`] (default) or
    /// [`ClientAuth::Basic`].
    pub fn client_auth(mut self, auth: ClientAuth) -> Self {
        self.client_auth = auth;
        self
    }

    /// PKCE with `S256` is on. Turning it off (for a provider that refuses unknown parameters) logs a warning at
    /// startup; `state` stays on and there is no `plain` method.
    pub fn pkce(mut self, on: bool) -> Self {
        self.pkce = on;
        self
    }

    /// How long a login may take between the redirect and the callback, 600 s by default (60 s to 3600 s).
    pub fn state_ttl(mut self, ttl: Duration) -> Self {
        self.state_ttl = ttl;
        self
    }

    /// Keeps the state of a login in a cookie signed with `keys` (each secret at least 32 bytes). The cookie is
    /// `__Host-oauth2.<name>`, `HttpOnly`, `SameSite=Lax`, `Secure`, `Path=/` and lives as long as the state. Use a
    /// secret of your own, not the session's.
    pub fn state_cookie(mut self, keys: CookieKeys) -> Self {
        self.storage = StateStorage::Cookie(keys);
        self
    }

    /// [`state_cookie`](Self::state_cookie) with the keys built from `secrets` (each at least 32 bytes, at most 8). Unlike
    /// building the keys first, a secret that is missing or too short does not fail here: it stops the application when
    /// it starts, with an error that names the option, which is what a validator that reads its secret from the
    /// environment needs (`options()` cannot return an error).
    pub fn state_cookie_secrets<I, S>(mut self, secrets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<[u8]>,
    {
        self.storage = match CookieKeys::new(secrets) {
            Ok(keys) => StateStorage::Cookie(keys),
            Err(e) => StateStorage::Invalid(e.to_string()),
        };
        self
    }

    /// Keeps the state of a login in the session (`SessionModule` must be installed and its cookie must not be
    /// `SameSite=Strict`). Every login start creates a session record: rate limit the login route.
    pub fn state_in_session(mut self) -> Self {
        self.storage = StateStorage::Session;
        self
    }

    /// `Secure` for the state cookie (default on). Turning it off, for plain-HTTP development, removes the `__Host-`
    /// prefix and logs a warning at startup.
    pub fn state_cookie_secure(mut self, secure: bool) -> Self {
        self.cookie_secure = secure;
        self
    }

    /// The name of the state cookie. A `__Host-` or `__Secure-` name needs `Secure`.
    pub fn state_cookie_name(mut self, name: impl Into<String>) -> Self {
        self.cookie_name = Some(name.into());
        self
    }

    /// Reads a path to come back to from this query parameter of the login request (off by default). Only a relative
    /// path that passes the documented rule is kept, inside the state record; read it after the callback with
    /// [`return_to`](super::return_to).
    pub fn return_to_param(mut self, name: impl Into<String>) -> Self {
        self.return_to_param = Some(name.into());
        self
    }

    /// Answers a failed callback with a 302 to this URL (a relative path under the return-to rule, or an absolute
    /// `https` URL) instead of the 401. Nothing from the request is added to it.
    pub fn failure_redirect(mut self, url: impl Into<String>) -> Self {
        self.failure_redirect = Some(url.into());
        self
    }

    /// The longest `code` a callback may carry, 2048 bytes by default (16 to 8192).
    pub fn max_code_bytes(mut self, max: usize) -> Self {
        self.max_code_bytes = max;
        self
    }

    /// The limits of the HTTP client.
    pub fn http(mut self, http: OAuth2HttpOptions) -> Self {
        self.http = http;
        self
    }

    /// What the effective scope of a request is: the guard's, when it set one.
    /// How many redeemed `state` values each strategy instance remembers (for `state_ttl`) to refuse a replayed
    /// callback before it reaches the token endpoint, 100000 by default (16 to 10000000). When the set is full the
    /// oldest entries are forgotten. It is a cost control of one instance, not a guarantee across instances.
    pub fn redeemed_states_max(mut self, max: usize) -> Self {
        self.redeemed_states_max = max;
        self
    }

    /// The `provider` value of this strategy's profiles: the preset's name only when the preset's own endpoints are
    /// in use; the strategy's name when any endpoint was changed and for a generic strategy. A strategy name is unique
    /// in the application, so `(provider, id)` does not collide across instances of one provider. A strategy that is
    /// not a preset and is named like one (`github`, `google`) gets `name@issuer` instead, so it cannot take the
    /// namespace of the real provider's ids (see `provider_namespace`).
    ///
    /// This is the preset's name when the preset's own endpoints are in use unchanged, `None` otherwise.
    pub(crate) fn preset_name(&self) -> Option<&'static str> {
        let (name, own) = match self.kind {
            ProfileKind::Google => ("google", Self::google_endpoints()),
            ProfileKind::GitHub => ("github", Self::github_endpoints()),
            ProfileKind::Generic => return None,
        };
        self.endpoints.same_as(&own).then_some(name)
    }

    /// The authorization endpoint's host (with the port when there is one), the `issuer` of the profiles.
    pub(crate) fn issuer(&self) -> String {
        url::Url::parse(&self.endpoints.authorization_url)
            .ok()
            .and_then(|u| u.host_str().map(|h| u.port().map_or_else(|| h.to_string(), |p| format!("{h}:{p}"))))
            .unwrap_or_default()
    }

    fn google_endpoints() -> OAuth2Endpoints {
        OAuth2Endpoints::new("https://accounts.google.com/o/oauth2/v2/auth", "https://www.googleapis.com/oauth2/v4/token")
            .userinfo_url("https://www.googleapis.com/oauth2/v3/userinfo")
    }

    fn github_endpoints() -> OAuth2Endpoints {
        OAuth2Endpoints::new("https://github.com/login/oauth/authorize", "https://github.com/login/oauth/access_token")
            .userinfo_url("https://api.github.com/user")
            .emails_url("https://api.github.com/user/emails")
    }

    pub(crate) fn can_read_emails(scope: &[String]) -> bool {
        scope.iter().any(|s| s == "user" || s == "user:email")
    }

    /// Checks every option. One entry per problem, each naming the option; no entry holds the client secret.
    pub(crate) fn problems(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut add = |option: &str, problem: String| out.push(format!("{option}: {problem}"));
        for (option, url) in
            [("authorization_url", Some(&self.endpoints.authorization_url)), ("token_url", Some(&self.endpoints.token_url))]
        {
            if let Some(url) = url
                && let Err(p) = check_url(url, UrlKind::Endpoint)
            {
                add(option, p);
            }
        }
        if let Some(url) = &self.endpoints.userinfo_url
            && let Err(p) = check_url(url, UrlKind::Endpoint)
        {
            add("userinfo_url", p);
        }
        if let Some(url) = &self.endpoints.emails_url
            && let Err(p) = check_url(url, UrlKind::Endpoint)
        {
            add("emails_url", p);
        }
        if let Err(p) = check_url(&self.redirect_uri, UrlKind::Redirect) {
            add("redirect_uri", p);
        }
        if self.client_id.is_empty() {
            add("client_id", "is empty".into());
        } else if self.client_id.len() > 1024 || self.client_id.chars().any(char::is_control) {
            add("client_id", "is longer than 1024 bytes or has a control character".into());
        }
        if self.client_secret.is_empty() {
            add("client_secret", "is empty".into());
        } else if self.client_secret.expose().len() > 4096 {
            add("client_secret", "is longer than 4096 bytes".into());
        }
        if let Err(p) = check_scope(&self.scope, self.scope_separator) {
            add("scope", p);
        }
        if self.kind == ProfileKind::Google && self.scope.is_empty() {
            add("scope", "the Google preset needs at least one scope (for example openid, email and profile)".into());
        }
        if let Some(scope) = &self.refresh_scope
            && let Err(p) = check_scope(scope, self.scope_separator)
        {
            add("refresh_scope", p);
        }
        if self.authorization_params.len() > MAX_AUTHORIZATION_PARAMS {
            add("authorization_param", format!("at most {MAX_AUTHORIZATION_PARAMS} extra parameters are accepted"));
        }
        for (name, value) in &self.authorization_params {
            if name.is_empty() || name.len() > MAX_PARAM_BYTES || value.len() > MAX_PARAM_BYTES {
                add(
                    "authorization_param",
                    format!("a parameter name must be 1 to {MAX_PARAM_BYTES} bytes and a value at most {MAX_PARAM_BYTES} bytes"),
                );
            } else if RESERVED_PARAMS.contains(&name.as_str()) {
                add("authorization_param", format!("the parameter \"{name}\" is set by the strategy and cannot be given"));
            }
        }
        check_range(&mut out, "state_ttl", self.state_ttl, MIN_STATE_TTL, MAX_STATE_TTL);
        check_range(&mut out, "max_code_bytes", self.max_code_bytes, MIN_MAX_CODE_BYTES, MAX_MAX_CODE_BYTES);
        check_range(&mut out, "connect_timeout", self.http.connect_timeout, MIN_CONNECT_TIMEOUT, MAX_CONNECT_TIMEOUT);
        check_range(&mut out, "timeout", self.http.timeout, MIN_TIMEOUT, MAX_TIMEOUT);
        check_range(&mut out, "max_token_bytes", self.http.max_token_bytes, MIN_MAX_TOKEN_BYTES, MAX_MAX_TOKEN_BYTES);
        check_range(&mut out, "max_profile_bytes", self.http.max_profile_bytes, MIN_MAX_PROFILE_BYTES, MAX_MAX_PROFILE_BYTES);
        check_range(&mut out, "max_in_flight", self.http.max_in_flight, MIN_MAX_IN_FLIGHT, MAX_MAX_IN_FLIGHT);
        if let Some(per_client) = self.http.max_in_flight_per_client {
            check_range(&mut out, "max_in_flight_per_client", per_client, MIN_MAX_IN_FLIGHT, MAX_MAX_IN_FLIGHT);
        }
        check_range(&mut out, "redeemed_states_max", self.redeemed_states_max, MIN_REDEEMED_STATES_MAX, MAX_REDEEMED_STATES_MAX);
        if self.http.extra_roots.len() > MAX_EXTRA_ROOTS || self.http.extra_roots.iter().any(|r| r.is_empty() || r.len() > MAX_ROOT_BYTES) {
            out.push(format!(
                "extra_root_certificate: at most {MAX_EXTRA_ROOTS} certificates of 1 to {MAX_ROOT_BYTES} DER bytes each are accepted"
            ));
        }
        match &self.storage {
            StateStorage::Unset => {
                out.push("state storage: choose where the state of a login lives with state_cookie(keys) or state_in_session()".into())
            }
            StateStorage::Invalid(problem) => out.push(format!("state_cookie_secrets: {problem}")),
            _ => {}
        }
        if let Some(name) = &self.return_to_param
            && (name.is_empty()
                || name.len() > MAX_PARAM_BYTES
                || name.chars().any(|c| c.is_control() || c == '%')
                || ["code", "state", "error"].contains(&name.as_str()))
        {
            out.push(format!(
                "return_to_param: must be 1 to {MAX_PARAM_BYTES} bytes without control characters or `%`, and not `code`, `state` or `error` (the callback's parameters)"
            ));
        }
        if let Some(url) = &self.failure_redirect
            && !is_safe_relative_path(url)
            && check_failure_url(url).is_err()
        {
            out.push("failure_redirect: must be a relative path (one `/`, not followed by `/` or `\\`, no backslash or control character, ASCII, at most 1024 bytes) or an absolute https URL".into());
        }
        out
    }
}

fn check_range<T: PartialOrd + std::fmt::Debug>(out: &mut Vec<String>, option: &str, value: T, min: T, max: T) {
    if value < min || value > max {
        out.push(format!("{option}: {value:?} is outside the bounds {min:?} to {max:?}"));
    }
}

/// A scope value is 1 to 128 visible ASCII characters without `"`, `\` or the separator, and there are at most 64.
pub(crate) fn check_scope(scope: &[String], separator: char) -> Result<(), String> {
    if scope.len() > MAX_SCOPES {
        return Err(format!("at most {MAX_SCOPES} values are accepted"));
    }
    for value in scope {
        let ok = !value.is_empty()
            && value.len() <= MAX_SCOPE_BYTES
            && value.chars().all(|c| c.is_ascii_graphic() && c != '"' && c != '\\' && c != separator);
        if !ok {
            return Err(format!(
                "each value must be 1 to {MAX_SCOPE_BYTES} visible ASCII characters without space, `\"`, `\\` or `{separator}`"
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UrlKind {
    Endpoint,
    Redirect,
}

/// Whether the host of a URL is a loopback name or address literal.
pub(crate) fn host_is_loopback(host: &url::Host<&str>) -> bool {
    match host {
        url::Host::Domain(d) => d.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(a) => a.is_loopback(),
        url::Host::Ipv6(a) => a.is_loopback(),
    }
}

/// The rule of an endpoint or redirect URL: absolute, `https` (or `http` to a loopback host), no user information, no
/// fragment, a host, at most 2048 bytes.
fn check_url(text: &str, kind: UrlKind) -> Result<url::Url, String> {
    if text.len() > MAX_URL_BYTES {
        return Err(format!("is longer than {MAX_URL_BYTES} bytes"));
    }
    let url = url::Url::parse(text).map_err(|_| {
        if kind == UrlKind::Redirect {
            "must be an absolute URL (a relative path is not accepted: the redirect URI is never built from the request)".to_string()
        } else {
            "is not a valid absolute URL".to_string()
        }
    })?;
    let Some(host) = url.host() else { return Err("has no host".into()) };
    match url.scheme() {
        "https" => {}
        "http" if host_is_loopback(&host) => {}
        "http" => return Err("uses http with a host that is not loopback: use https".into()),
        _ => return Err("must use https (http is accepted for a loopback host only)".into()),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("has user information".into());
    }
    if url.fragment().is_some() {
        return Err("has a fragment".into());
    }
    Ok(url)
}

/// The URL form of `failure_redirect`: absolute `https`, no user information, ASCII with no control character, space or
/// backslash (it is sent as it is).
fn check_failure_url(text: &str) -> Result<(), ()> {
    if text.len() > MAX_RELATIVE_PATH_BYTES || !text.bytes().all(|b| b.is_ascii_graphic()) || text.contains('\\') {
        return Err(());
    }
    let url = url::Url::parse(text).map_err(|_| ())?;
    if url.scheme() != "https" || url.host().is_none() || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some()
    {
        return Err(());
    }
    Ok(())
}

/// The relative-path rule of `return_to` and `failure_redirect`: starts with one `/` that is not followed by `/` or
/// `\`, has no backslash, no control character and no non-ASCII byte, and is at most 1024 bytes.
pub fn is_safe_relative_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_RELATIVE_PATH_BYTES || bytes[0] != b'/' {
        return false;
    }
    if matches!(bytes.get(1), Some(b'/') | Some(b'\\')) {
        return false;
    }
    bytes.iter().all(|&b| (0x20..0x80).contains(&b) && b != 0x7f && b != b'\\')
}

/// Whether a cookie name is a token (RFC 6265) of at most 128 bytes. Stricter on purpose than
/// [`crate::http::res::valid_cookie_name`], which is what `res.cookie` accepts (the parity of the `cookie` package): this
/// one checks a name the application configures for the state cookie, where the RFC's token rule and a length bound
/// cost nothing.
pub(crate) fn is_token_cookie_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// The names the presets give their profiles: the namespaces of the ids of google.com and github.com themselves.
pub(crate) const PRESET_NAMES: [&str; 2] = ["google", "github"];

/// The `provider` of a strategy's profiles. `preset` is the preset's name while the preset's own endpoints are in use
/// (`OAuth2Options::preset_name`). Any other strategy is named after itself, unless its name is a preset's: then the
/// name alone would put the ids of another identity provider into the space of the real one (a generic strategy
/// named `github`, next to the real GitHub preset under another name), so the issuer is appended.
pub(crate) fn provider_namespace(preset: Option<&'static str>, strategy: &str, issuer: &str) -> String {
    match preset {
        Some(name) => name.to_string(),
        None if PRESET_NAMES.contains(&strategy) => format!("{strategy}@{issuer}"),
        None => strategy.to_string(),
    }
}

/// The name of the state cookie for the strategy `strategy`, or the problem with the configured one.
pub(crate) fn state_cookie_name(options: &OAuth2Options, strategy: &str) -> Result<String, String> {
    let name = match &options.cookie_name {
        Some(name) => name.clone(),
        None if options.cookie_secure => format!("__Host-oauth2.{strategy}"),
        None => format!("oauth2.{strategy}"),
    };
    if !is_token_cookie_name(&name) {
        return Err(if options.cookie_name.is_some() {
            "state_cookie_name: is not a valid cookie name (RFC 6265 token, at most 128 bytes)".into()
        } else {
            "state_cookie_name: the strategy name does not make a valid cookie name: set state_cookie_name".into()
        });
    }
    // Browsers match these prefixes without regard to case.
    let prefixed = |prefix: &str| crate::http::res::has_cookie_prefix(&name, prefix);
    if !options.cookie_secure && (prefixed("__Host-") || prefixed("__Secure-")) {
        return Err("state_cookie_name: a name with the __Host- or __Secure- prefix needs a Secure cookie".into());
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "very-secret-client-secret-value";

    fn keys() -> CookieKeys {
        CookieKeys::new(["0123456789abcdef0123456789abcdef"]).unwrap()
    }

    fn good() -> OAuth2Options {
        OAuth2Options::google("id", SECRET, "https://app.example/cb").scope(["email"]).state_cookie(keys())
    }

    fn problems(options: OAuth2Options) -> String {
        options.problems().join("; ")
    }

    #[test]
    fn the_preset_names_the_provider_only_while_its_endpoints_are_unchanged() {
        let own = OAuth2Options::github("i", "s", "https://a.example/cb");
        assert_eq!(own.preset_name(), Some("github"));
        let ghe = OAuth2Options::github("i", "s", "https://a.example/cb").endpoints(
            OAuth2Endpoints::new("https://ghe.corp.example/login/oauth/authorize", "https://ghe.corp.example/login/oauth/access_token")
                .userinfo_url("https://ghe.corp.example/api/v3/user")
                .emails_url("https://ghe.corp.example/api/v3/user/emails"),
        );
        assert_eq!(ghe.preset_name(), None, "the strategy's name is the namespace");
        assert_eq!(ghe.issuer(), "ghe.corp.example");
        let google = OAuth2Options::google("i", "s", "https://a.example/cb");
        assert_eq!(google.preset_name(), Some("google"));
        assert_eq!(google.issuer(), "accounts.google.com");
        // one endpoint changed is enough
        let mut one = OAuth2Options::github("i", "s", "https://a.example/cb");
        one.endpoints.userinfo_url = Some("https://proxy.example/user".into());
        assert_eq!(one.preset_name(), None);
        let generic =
            OAuth2Options::new(OAuth2Endpoints::new("https://a.example/a", "https://a.example/t"), "i", "s", "https://a.example/cb");
        assert_eq!(generic.preset_name(), None);
    }

    #[test]
    fn a_client_address_gets_a_quarter_of_the_slots_and_at_least_one() {
        let slots = |max: usize, share: Option<usize>| {
            let mut http = OAuth2HttpOptions::new().max_in_flight(max);
            if let Some(share) = share {
                http = http.max_in_flight_per_client(share);
            }
            http.per_client_slots()
        };
        assert_eq!(slots(32, None), 8);
        assert_eq!(slots(1, None), 1);
        assert_eq!(slots(3, None), 1);
        assert_eq!(slots(1024, None), 256);
        assert_eq!(slots(32, Some(5)), 5);
    }

    #[test]
    fn presets_hold_the_documented_urls() {
        let g = OAuth2Options::google("i", "s", "https://a.example/cb");
        assert_eq!(g.endpoints.authorization_url, "https://accounts.google.com/o/oauth2/v2/auth");
        assert_eq!(g.endpoints.token_url, "https://www.googleapis.com/oauth2/v4/token");
        assert_eq!(g.endpoints.userinfo_url.as_deref(), Some("https://www.googleapis.com/oauth2/v3/userinfo"));
        let h = OAuth2Options::github("i", "s", "https://a.example/cb");
        assert_eq!(h.endpoints.authorization_url, "https://github.com/login/oauth/authorize");
        assert_eq!(h.endpoints.token_url, "https://github.com/login/oauth/access_token");
        assert_eq!(h.endpoints.userinfo_url.as_deref(), Some("https://api.github.com/user"));
        assert_eq!(h.endpoints.emails_url.as_deref(), Some("https://api.github.com/user/emails"));
        assert_eq!(h.scope_separator, ',');
    }

    #[test]
    fn a_good_configuration_has_no_problem() {
        assert_eq!(problems(good()), "");
        assert_eq!(problems(OAuth2Options::github("i", "s", "http://127.0.0.1:3000/cb").state_in_session()), "");
        assert_eq!(problems(OAuth2Options::github("i", "s", "http://[::1]:3000/cb").state_in_session()), "");
        assert_eq!(problems(OAuth2Options::github("i", "s", "http://localhost/cb").state_in_session()), "");
    }

    #[test]
    fn every_rule_names_its_option_and_never_the_secret() {
        let cases: Vec<(OAuth2Options, &str)> = vec![
            (good().endpoints(OAuth2Endpoints::new("http://provider.example/a", "https://p.example/t")), "authorization_url"),
            (good().endpoints(OAuth2Endpoints::new("https://p.example/a", "http://provider.example/token")), "token_url"),
            (good().endpoints(OAuth2Endpoints::new("https://p.example/a#frag", "https://p.example/t")), "authorization_url"),
            (good().endpoints(OAuth2Endpoints::new("https://u:pw@p.example/a", "https://p.example/t")), "authorization_url"),
            (good().endpoints(OAuth2Endpoints::new("ftp://p.example/a", "https://p.example/t")), "authorization_url"),
            (good().endpoints(OAuth2Endpoints::new("/a", "https://p.example/t")), "authorization_url"),
            (
                good().endpoints(OAuth2Endpoints::new("https://p.example/a", "https://p.example/t").userinfo_url("http://x.example/u")),
                "userinfo_url",
            ),
            (OAuth2Options { redirect_uri: "/auth/google/callback".into(), ..good() }, "redirect_uri"),
            (OAuth2Options { redirect_uri: "https://a.example/cb#x".into(), ..good() }, "redirect_uri"),
            (OAuth2Options { redirect_uri: "https://user@a.example/cb".into(), ..good() }, "redirect_uri"),
            (OAuth2Options { redirect_uri: "http://a.example/cb".into(), ..good() }, "redirect_uri"),
            (OAuth2Options { client_id: String::new(), ..good() }, "client_id"),
            (OAuth2Options { client_secret: ClientSecret::new(""), ..good() }, "client_secret"),
            (good().scope(["a b"]), "scope"),
            (good().scope(["a\"b"]), "scope"),
            (good().scope(["a\\b"]), "scope"),
            (good().scope([""]), "scope"),
            (good().scope(vec!["x"; 65]), "scope"),
            (good().scope(Vec::<String>::new()), "scope"),
            (good().authorization_param("state", "fixed"), "\"state\""),
            (good().authorization_param("response_type", "token"), "response_type"),
            (good().authorization_param("redirect_uri", "x"), "redirect_uri"),
            (good().authorization_param("a".repeat(257), "x"), "authorization_param"),
            (good().state_ttl(Duration::from_secs(59)), "state_ttl"),
            (good().state_ttl(Duration::from_secs(3601)), "state_ttl"),
            (good().max_code_bytes(15), "max_code_bytes"),
            (good().max_code_bytes(8193), "max_code_bytes"),
            (good().http(OAuth2HttpOptions::new().timeout(Duration::from_secs(600))), "timeout"),
            (good().http(OAuth2HttpOptions::new().timeout(Duration::from_millis(999))), "timeout"),
            (good().http(OAuth2HttpOptions::new().connect_timeout(Duration::from_secs(31))), "connect_timeout"),
            (good().http(OAuth2HttpOptions::new().max_token_bytes(1023)), "max_token_bytes"),
            (good().http(OAuth2HttpOptions::new().max_profile_bytes(5 * 1024 * 1024)), "max_profile_bytes"),
            (good().http(OAuth2HttpOptions::new().max_in_flight(0)), "max_in_flight"),
            (good().http(OAuth2HttpOptions::new().max_in_flight(1025)), "max_in_flight"),
            (OAuth2Options { storage: StateStorage::Unset, ..good() }, "state_cookie(keys)"),
            (good().return_to_param(""), "return_to_param"),
            (good().return_to_param("state"), "return_to_param"),
            (good().return_to_param("code"), "return_to_param"),
            (good().failure_redirect("//evil.example"), "failure_redirect"),
            (good().failure_redirect("http://evil.example/x"), "failure_redirect"),
            (good().failure_redirect("/a\\b"), "failure_redirect"),
            (good().failure_redirect("login"), "failure_redirect"),
        ];
        for (options, wanted) in cases {
            let text = problems(options);
            assert!(text.contains(wanted), "expected {wanted:?} in {text:?}");
            assert!(!text.contains(SECRET), "{text}");
        }
    }

    #[test]
    fn the_limits_accept_their_bounds() {
        let ok = good().state_ttl(MIN_STATE_TTL).max_code_bytes(MAX_MAX_CODE_BYTES).http(
            OAuth2HttpOptions::new()
                .connect_timeout(MAX_CONNECT_TIMEOUT)
                .timeout(MAX_TIMEOUT)
                .max_token_bytes(MAX_MAX_TOKEN_BYTES)
                .max_profile_bytes(MIN_MAX_PROFILE_BYTES)
                .max_in_flight(1024),
        );
        assert_eq!(problems(ok), "");
        assert_eq!(problems(good().state_ttl(MAX_STATE_TTL).max_code_bytes(MIN_MAX_CODE_BYTES)), "");
    }

    #[test]
    fn google_without_scope_is_refused_and_github_is_not() {
        assert!(problems(OAuth2Options::google("i", "s", "https://a.example/cb").state_in_session()).contains("scope"));
        assert_eq!(problems(OAuth2Options::github("i", "s", "https://a.example/cb").state_in_session()), "");
    }

    #[test]
    fn several_problems_are_all_reported() {
        let text = problems(OAuth2Options::google("", "", "/cb"));
        for wanted in ["client_id", "client_secret", "redirect_uri", "scope", "state storage"] {
            assert!(text.contains(wanted), "{wanted} in {text}");
        }
    }

    #[test]
    fn the_github_scope_cannot_hold_the_separator() {
        assert!(problems(OAuth2Options::github("i", "s", "https://a.example/cb").state_in_session().scope(["a,b"])).contains("scope"));
        assert_eq!(problems(OAuth2Options::google("i", "s", "https://a.example/cb").state_in_session().scope(["a,b"])), "");
    }

    #[test]
    fn debug_shows_the_client_id_and_hosts_but_no_secret() {
        let printed = format!("{:?}", good());
        assert!(printed.contains("\"id\"") && printed.contains("accounts.google.com") && printed.contains("app.example"), "{printed}");
        assert!(!printed.contains(SECRET) && !printed.contains("0123456789abcdef"), "{printed}");
        let http = format!("{:?}", OAuth2HttpOptions::new().extra_root_certificate(vec![1, 2, 3]));
        assert!(!http.contains("[1, 2, 3]"), "{http}");
    }

    #[test]
    fn state_cookie_secrets_report_a_bad_secret_at_validation_without_printing_it() {
        let ok = good().state_cookie_secrets([STATE_SECRET_FOR_TEST]);
        assert_eq!(problems(ok), "");
        let short = good().state_cookie_secrets(["too-short-SECRETVALUE"]);
        let text = problems(short);
        assert!(text.contains("state_cookie_secrets") && text.contains("32") && !text.contains("SECRETVALUE"), "{text}");
        let none = good().state_cookie_secrets(Vec::<&str>::new());
        assert!(problems(none).contains("state_cookie_secrets"));
    }

    const STATE_SECRET_FOR_TEST: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn loopback_hosts() {
        for (url, loopback) in [
            ("http://127.0.0.1/", true),
            ("http://[::1]/", true),
            ("http://localhost/", true),
            ("http://LOCALHOST/", true),
            ("http://127.0.0.2/", true),
            ("http://10.0.0.1/", false),
            ("http://localhost.example/", false),
            ("http://example.com/", false),
            ("http://[2001:db8::1]/", false),
        ] {
            let parsed = url::Url::parse(url).unwrap();
            assert_eq!(host_is_loopback(&parsed.host().unwrap()), loopback, "{url}");
        }
    }

    #[test]
    fn cookie_names() {
        let o = good();
        assert_eq!(state_cookie_name(&o, "google").unwrap(), "__Host-oauth2.google");
        let o = good().state_cookie_secure(false);
        assert_eq!(state_cookie_name(&o, "google").unwrap(), "oauth2.google");
        assert!(state_cookie_name(&good().state_cookie_secure(false).state_cookie_name("__Host-x"), "g").is_err());
        assert!(state_cookie_name(&good().state_cookie_secure(false).state_cookie_name("__Secure-x"), "g").is_err());
        assert!(state_cookie_name(&good().state_cookie_name("__Host-x"), "g").is_ok());
        assert!(state_cookie_name(&good(), "has space").is_err());
        assert!(state_cookie_name(&good().state_cookie_name("a;b"), "g").is_err());
        assert!(state_cookie_name(&good().state_cookie_name("has space"), "g").is_err());
        assert!(state_cookie_name(&good().state_cookie_name("good-name"), "has space").is_ok());
    }

    #[test]
    fn the_relative_path_rule() {
        for ok in ["/", "/cats", "/cats/7?tab=toys", "/a%5Cb", "/a#frag"] {
            assert!(is_safe_relative_path(ok), "{ok}");
        }
        let long = format!("/{}", "a".repeat(1023));
        assert!(is_safe_relative_path(&long));
        let too_long = format!("/{}", "a".repeat(1024));
        for bad in [
            "",
            "cats",
            "https://evil.example",
            "//evil.example",
            "/\\evil.example",
            "/a\\b",
            "/a\nb",
            "/a\tb",
            "/a\u{7f}b",
            "/caf\u{e9}",
            "\\evil",
            "/\u{0}",
            too_long.as_str(),
        ] {
            assert!(!is_safe_relative_path(bad), "{bad:?}");
        }
        // 2000 bytes
        assert!(!is_safe_relative_path(&format!("/{}", "a".repeat(2000))));
    }

    proptest::proptest! {
        #[test]
        fn an_accepted_path_is_always_relative_and_clean(s in ".{0,64}") {
            if is_safe_relative_path(&s) {
                proptest::prop_assert!(s.starts_with('/'));
                proptest::prop_assert!(!s.starts_with("//"));
                proptest::prop_assert!(!s.contains('\\'));
                proptest::prop_assert!(!s.chars().any(|c| c.is_control() || !c.is_ascii()));
            }
        }

        #[test]
        fn a_slash_prefix_with_only_safe_characters_is_accepted(tail in "[a-zA-Z0-9._~%?=&-]{0,40}") {
            let value = format!("/{tail}");
            proptest::prop_assert!(is_safe_relative_path(&value));
        }
    }
}
