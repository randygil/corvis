//! `OAuth2Strategy<V>`: the authorization code flow with `state` and PKCE.

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::client::{Grant, OAuth2Error, OAuth2TokenResponse, Provider};
use super::options::{OAuth2Options, StateStorage, check_scope, state_cookie_name};
use super::profile::OAuth2Profile;
use super::redeem::{ClientSlots, Redeemed};
use super::secret::OAuth2Token;
use super::state::{self, Flow, StateCookie};
use crate::auth::outcome::service_unavailable;
use crate::auth::session::{SameSite, SessionManager};
use crate::auth::warn_limit::WarnLimiter;
use crate::auth::{AuthenticateOptions, AuthenticatedUser, Strategy, StrategyOutcome};
use crate::common::context::{ExecutionContext, GuardPhase};
use crate::di::{DiError, Injectable, ResolveCtx};
use crate::http::res::has_cookie_prefix;
use crate::http::{HttpException, Request};
use crate::logger::Logger;

/// What a successful callback hands to [`OAuth2Validator::validate`]: the original's `validate(accessToken,
/// refreshToken, params, profile)` as one value, so a field can be added later without breaking validators.
///
/// The tokens are never logged and never stored by the framework. If the application keeps one, it should store it
/// encrypted at rest with the user record, not in a cookie or in the session. `Debug` prints field names only.
#[non_exhaustive]
pub struct OAuth2Grant {
    pub access_token: OAuth2Token,
    /// Present when the provider sent one.
    pub refresh_token: Option<OAuth2Token>,
    /// The token response without the tokens; its `id_token()` is **unverified**.
    pub params: OAuth2TokenResponse,
    /// The normalized profile; `None` when no profile source is configured.
    pub profile: Option<OAuth2Profile>,
}

impl std::fmt::Debug for OAuth2Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2Grant")
            .field("access_token", &"..")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| ".."))
            .field("params", &self.params)
            .field("profile", &self.profile.as_ref().map(|_| ".."))
            .finish()
    }
}

/// The class `PassportStrategy(Strategy)` of `passport-oauth2` is written as: the options and `validate`.
///
/// ```ignore
/// #[injectable]
/// pub struct GoogleAuth { users: Arc<UsersService>, config: Arc<ConfigService> }
///
/// #[async_trait]
/// impl OAuth2Validator for GoogleAuth {
///     const NAME: &'static str = "google";
///     type User = SessionUser;
///
///     fn options(&self) -> OAuth2Options {
///         OAuth2Options::google(self.config.get("GOOGLE_ID"), self.config.get("GOOGLE_SECRET"), "https://app.example/auth/google/callback")
///             .scope(["openid", "email", "profile"])
///             .state_cookie(CookieKeys::new([self.config.get("OAUTH_STATE_SECRET")]).unwrap())
///     }
///
///     async fn validate(&self, _: &Request, grant: OAuth2Grant) -> Result<Option<SessionUser>, HttpException> {
///         let Some(profile) = grant.profile else { return Ok(None) };
///         // Identify by (provider, id), never by e-mail alone; link by e-mail only through verified_email().
///         Ok(Some(self.users.find_or_create(&profile.provider, &profile.id, profile.verified_email()).await?))
///     }
/// }
///
/// // providers = [GoogleAuth, Provider::strategy::<OAuth2Strategy<GoogleAuth>>()]
/// // #[get("auth/google")]          #[use_guards(AuthGuard::new("google"))]   // redirects to the provider
/// // #[get("auth/google/callback")] #[use_guards(AuthGuard::new("google"))]   // the handler runs with User<T>
/// ```
#[async_trait]
pub trait OAuth2Validator: Injectable {
    /// The name routes use: `AuthGuard::new("google")`.
    const NAME: &'static str = "oauth2";

    /// The user handed to the handler (`User<Self::User>`). Do not put tokens in it.
    type User: Serialize + Clone + Send + Sync + 'static;

    /// The options, read once when the application starts. Invalid options stop `init()`.
    fn options(&self) -> OAuth2Options;

    /// The grant of a callback that passed every check: the user, `None` to refuse (401), or an exception (passed on
    /// as it is).
    async fn validate(&self, req: &Request, grant: OAuth2Grant) -> Result<Option<Self::User>, HttpException>;

    /// The auth info of a successful attempt (`Request::auth_info`), none by default.
    fn info(&self, _user: &Self::User) -> Option<Value> {
        None
    }
}

/// How the strategy keeps its flows.
enum Storage {
    Cookie(StateCookie),
    Session,
}

/// The strategy of an [`OAuth2Validator`]: register it with `Provider::strategy::<OAuth2Strategy<V>>()`.
pub struct OAuth2Strategy<V: OAuth2Validator> {
    validator: Arc<V>,
    options: OAuth2Options,
    storage: Storage,
    provider: Provider,
    warnings: WarnLimiter,
    redeemed: Redeemed,
    slots: Arc<ClientSlots>,
    _marker: PhantomData<fn() -> V>,
}

/// Prints nothing but the strategy name.
impl<V: OAuth2Validator> std::fmt::Debug for OAuth2Strategy<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OAuth2Strategy({})", V::NAME)
    }
}

fn factory_error<V: OAuth2Validator>(problem: String) -> DiError {
    DiError::Factory {
        token: format!("OAuth2Strategy \"{}\" ({})", V::NAME, std::any::type_name::<V>()),
        cause: Arc::new(anyhow::anyhow!("invalid OAuth2 options: {problem}")),
    }
}

impl<V: OAuth2Validator> Injectable for OAuth2Strategy<V> {
    fn construct(ctx: ResolveCtx) -> BoxFuture<'static, Result<Self, DiError>> {
        async move {
            let validator = ctx.get_or_construct::<V>().await?;
            let options = validator.options();
            let problems = options.problems();
            if !problems.is_empty() {
                return Err(factory_error::<V>(problems.join("; ")));
            }
            let log = Logger::new("OAuth2Strategy");
            if super::options::PRESET_NAMES.contains(&V::NAME) && options.preset_name() != Some(V::NAME) {
                log.warn(format!(
                    "strategy \"{0}\": it is named like the {0} preset but does not use that preset's own endpoints, so its profiles get the provider \"{0}@{1}\" instead of \"{0}\" (their ids belong to another identity provider); give the strategy a name of its own to choose the provider value",
                    V::NAME,
                    options.issuer()
                ));
            }
            let storage = match &options.storage {
                StateStorage::Cookie(keys) => {
                    let name = state_cookie_name(&options, V::NAME).map_err(factory_error::<V>)?;
                    if !options.cookie_secure {
                        log.warn(format!(
                            "strategy \"{}\": the state cookie is not Secure and has no __Host- prefix (state_cookie_secure(false)): use it for plain-HTTP development only",
                            V::NAME
                        ));
                    }
                    if !has_cookie_prefix(&name, "__Host-") {
                        log.warn(format!(
                            "strategy \"{}\": the state cookie \"{name}\" has no __Host- prefix, so a page on a sibling subdomain can plant its own state cookie and sign a victim in to the attacker's account (login CSRF): name it __Host-... on a Secure HTTPS host, or keep the application off a domain shared with untrusted subdomains",
                            V::NAME
                        ));
                    }
                    Storage::Cookie(StateCookie::new(name, keys.clone(), &options))
                }
                StateStorage::Session => {
                    let manager = ctx.get::<SessionManager>().await.map_err(|_| {
                        factory_error::<V>("state_in_session: SessionModule is not imported, so there is no session to keep the state in".into())
                    })?;
                    if manager.options.cookie.same_site == Some(SameSite::Strict) {
                        return Err(factory_error::<V>(
                            "state_in_session: the session cookie is SameSite=Strict, so the callback would arrive without the session".into(),
                        ));
                    }
                    if !has_cookie_prefix(&manager.options.name, "__Host-") {
                        log.warn(format!(
                            "strategy \"{}\": the session cookie \"{}\" that keeps the login state has no __Host- prefix, so a page on a sibling subdomain can plant its own session cookie and sign a victim in to the attacker's account (login CSRF): name the session cookie __Host-... on a Secure HTTPS host with Path=/ and no Domain",
                            V::NAME,
                            manager.options.name
                        ));
                    }
                    Storage::Session
                }
                StateStorage::Unset | StateStorage::Invalid(_) => return Err(factory_error::<V>("state storage is not chosen".into())),
            };
            if !options.pkce {
                log.warn(format!("strategy \"{}\": PKCE is turned off (pkce(false)); state is still sent and checked", V::NAME));
            }
            let provider = Provider::new(&options).map_err(factory_error::<V>)?;
            // A flow stays valid in the browser for `state_ttl` plus the accepted clock skew.
            let redeemed = Redeemed::new(options.redeemed_states_max, options.state_ttl.as_secs() as i64 + state::MAX_SKEW_SECONDS);
            let slots = ClientSlots::new(options.http.per_client_slots());
            Ok(OAuth2Strategy {
                validator,
                options,
                storage,
                provider,
                warnings: WarnLimiter::new(Duration::from_secs(60)),
                redeemed,
                slots,
                _marker: PhantomData,
            })
        }
        .boxed()
    }
}

/// The level of a line of [`OAuth2Strategy::note`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoteLevel {
    /// Something the operator should see: a fault, a limit reached, a provider that refused.
    Warn,
    /// What a client can cause on purpose: a forged or stale callback.
    Debug,
}

/// The part of a login a failure happened in: the start, or the callback (which may go to the failure page).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    Callback,
}

/// The key a client's slots are counted under: the address the request resolved to (so `trust proxy` is honoured),
/// normalized, IPv6 reduced to its prefix. A value that is not an address falls back to the socket's peer, so a header
/// cannot pick a fresh key per request.
fn client_key(req: &Request) -> String {
    use crate::modules::throttler::{DEFAULT_IPV6_SUBNET_PREFIX, normalize_ip};
    let is_address = |key: &str| key.split('/').next().is_some_and(|a| a.parse::<std::net::IpAddr>().is_ok());
    let key = normalize_ip(&req.ip(), DEFAULT_IPV6_SUBNET_PREFIX);
    if is_address(&key) {
        return key;
    }
    req.remote_addr().map(|a| normalize_ip(&a.ip().to_string(), DEFAULT_IPV6_SUBNET_PREFIX)).unwrap_or_else(|| "127.0.0.1".to_string())
}

/// The return-to path of the login a successful callback belongs to, set on the request by the strategy only.
#[derive(Clone)]
struct ReturnTo(String);

/// The path the user was on when the login started, after a successful callback: the value of the configured
/// `return_to_param` at login start, when it passed the relative-path rule. `None` for a callback that failed, one whose
/// login had none, and any request the strategy did not authenticate. The callback's own query is never consulted.
pub fn return_to(req: &Request) -> Option<String> {
    req.get::<ReturnTo>().map(|r| r.0)
}

/// An internal error with a fixed text, already logged by the strategy.
#[derive(Debug)]
struct Fault(&'static str);

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Fault {}

fn percent(value: &str) -> String {
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
    // `encodeURIComponent`: everything but letters, digits and `-_.!~*'()`.
    const KEEP: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'!')
        .remove(b'~')
        .remove(b'*')
        .remove(b'\'')
        .remove(b'(')
        .remove(b')');
    utf8_percent_encode(value, KEEP).to_string()
}

/// Sets `key` to `value` in place when present (what `utils.merge` of the original does), appends it otherwise.
fn set_param(pairs: &mut Vec<(String, String)>, key: &str, value: String) {
    match pairs.iter_mut().find(|(k, _)| k == key) {
        Some(pair) => pair.1 = value,
        None => pairs.push((key.to_string(), value)),
    }
}

impl<V: OAuth2Validator> OAuth2Strategy<V> {
    fn now() -> i64 {
        crate::auth::session::now_ms().div_euclid(1000)
    }

    fn ttl(&self) -> i64 {
        self.options.state_ttl.as_secs() as i64
    }

    /// A line for the log, at most once a minute per reason: a client can trigger every failure of the callback.
    fn note(&self, reason: &str, level: NoteLevel, detail: String) {
        if let Some(line) = self.warnings.line(reason, Instant::now(), || format!("strategy \"{}\": {reason}{detail}", V::NAME)) {
            let log = Logger::new("OAuth2Strategy");
            match level {
                NoteLevel::Warn => log.warn(line),
                NoteLevel::Debug => log.debug(line),
            }
        }
    }

    fn internal(&self, reason: &'static str, detail: String) -> StrategyOutcome {
        self.note(reason, NoteLevel::Warn, detail);
        StrategyOutcome::Error(HttpException::internal(Fault(reason)).already_logged())
    }

    /// The scope of this request: the guard's `scope` option, validated, or the configured one. `Err` for a guard
    /// option that is not a list of valid scope values.
    fn scope(&self, options: &AuthenticateOptions) -> Result<Vec<String>, ()> {
        let Some(value) = options.get("scope") else { return Ok(self.options.scope.clone()) };
        let Value::Array(items) = value else { return Err(()) };
        let mut scope = Vec::with_capacity(items.len());
        for item in items {
            scope.push(item.as_str().ok_or(())?.to_string());
        }
        check_scope(&scope, self.options.scope_separator).map_err(|_| ())?;
        Ok(scope)
    }

    fn fixed_headers(ctx: &ExecutionContext) {
        let http = ctx.switch_to_http();
        http.set_response_header("cache-control", "no-store");
        http.set_response_header("referrer-policy", "no-referrer");
    }

    /// A failed callback: the 401 with a fixed challenge, or the redirect to the configured failure page.
    fn fail(&self, reason: &'static str, challenge_error: Option<String>, phase: Phase) -> StrategyOutcome {
        self.note(reason, NoteLevel::Debug, challenge_error.as_deref().map(|c| format!(" ({c})")).unwrap_or_default());
        if phase == Phase::Callback
            && let Some(url) = &self.options.failure_redirect
        {
            return StrategyOutcome::Redirect { url: url.clone(), status: Some(302) };
        }
        let mut challenge = Map::new();
        challenge.insert("message".into(), json!(reason));
        if let Some(code) = challenge_error {
            challenge.insert("error".into(), json!(code));
        }
        StrategyOutcome::Fail { challenge: Some(Value::Object(challenge)), status: Some(401) }
    }

    /// The validator refused the login (no user, or one that never authenticates): the failure page when one is
    /// configured, otherwise the plain 401 of the guard. The reason stays with the validator.
    fn refused(&self) -> StrategyOutcome {
        match &self.options.failure_redirect {
            Some(url) => {
                self.note("the validator refused the login", NoteLevel::Debug, String::new());
                StrategyOutcome::Redirect { url: url.clone(), status: Some(302) }
            }
            None => StrategyOutcome::Fail { challenge: None, status: None },
        }
    }

    /// The login: a new flow stored, then the redirect to the provider.
    fn start(&self, ctx: &ExecutionContext, scope: &[String]) -> StrategyOutcome {
        let req = ctx.request();
        // The Authenticator refuses a redirect outside the guard phase; nothing is generated or stored first.
        if !GuardPhase::is_active(req) {
            return self.internal("a login was started outside the guard phase", String::new());
        }
        let (Some(state), Some(random_verifier)) = (state::random_token(), state::random_token()) else {
            return self.internal("the random generator failed", String::new());
        };
        let issued = Self::now();
        let return_to = self
            .options
            .return_to_param
            .as_deref()
            .and_then(|name| req.query().get(name))
            .and_then(|v| v.as_str())
            .filter(|v| super::options::is_safe_relative_path(v))
            .map(str::to_string);
        let (verifier, cookie_header) = match &self.storage {
            Storage::Cookie(cookie) => {
                let verifier = self.options.pkce.then(|| state::derive_verifier(&cookie.keys, V::NAME, &state, issued));
                let mut set = cookie.read(req, V::NAME);
                set.prune(issued, self.ttl());
                // Tied to the session the browser has now (or to having none): the callback must arrive under the
                // same one, so a logout and another user's login in between cancel the flow.
                let binding = state::session_binding(&cookie.keys, V::NAME, &state, req.session().as_ref());
                // And to the user this request carries, whatever authenticated it (see `state::user_binding`).
                let user = Some(state::user_binding(Some(&cookie.keys), V::NAME, &state, req.user().as_ref()));
                set.push(Flow { state: state.clone(), verifier: None, issued, return_to, binding, user });
                (verifier, Some(cookie.write(&set, V::NAME)))
            }
            Storage::Session => {
                let Some(session) = req.session() else {
                    return self.internal("the request has no session: SessionModule did not run for it", String::new());
                };
                let mut set = state::read_session(&session, V::NAME);
                set.prune(issued, self.ttl());
                let user = Some(state::user_binding(None, V::NAME, &state, req.user().as_ref()));
                set.push(Flow { state: state.clone(), verifier: Some(random_verifier.clone()), issued, return_to, binding: None, user });
                if !state::write_session(&session, V::NAME, &set) {
                    return self.internal("the session refused the state record", String::new());
                }
                (self.options.pkce.then_some(random_verifier), None)
            }
        };
        let Ok(base) = url::Url::parse(&self.options.endpoints.authorization_url) else {
            return self.internal("the authorization URL is not valid", String::new());
        };
        // The order of the original: the URL's own parameters, the extra ones, `response_type`, `redirect_uri`,
        // `scope`, the challenge, `state`, then `client_id`.
        let mut pairs: Vec<(String, String)> = base.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        for (k, v) in &self.options.authorization_params {
            set_param(&mut pairs, k, v.clone());
        }
        set_param(&mut pairs, "response_type", "code".into());
        set_param(&mut pairs, "redirect_uri", self.options.redirect_uri.clone());
        if !scope.is_empty() {
            set_param(&mut pairs, "scope", scope.join(&self.options.scope_separator.to_string()));
        }
        if let Some(verifier) = &verifier {
            set_param(&mut pairs, "code_challenge", state::challenge(verifier));
            set_param(&mut pairs, "code_challenge_method", "S256".into());
        }
        set_param(&mut pairs, "state", state);
        set_param(&mut pairs, "client_id", self.options.client_id.clone());
        let query = pairs.iter().map(|(k, v)| format!("{}={}", percent(k), percent(v))).collect::<Vec<_>>().join("&");
        let mut location = base;
        location.set_query(Some(&query));
        let http = ctx.switch_to_http();
        if let Some(header) = cookie_header
            && !http.append_response_header("set-cookie", &header)
        {
            return self.internal("the state cookie could not be added to the response", String::new());
        }
        Self::fixed_headers(ctx);
        StrategyOutcome::Redirect { url: location.to_string(), status: Some(302) }
    }

    /// Takes the flow of `state` out of the store, and rewrites the store with the other flows (or clears it).
    /// `Err` is the 500 outcome.
    fn take_flow(&self, ctx: &ExecutionContext, state: Option<&str>) -> Result<Option<Flow>, StrategyOutcome> {
        let req = ctx.request();
        let now = Self::now();
        let shaped = state.filter(|s| state::is_state_shape(s));
        match &self.storage {
            Storage::Cookie(cookie) => {
                let mut set = cookie.read(req, V::NAME);
                set.prune(now, self.ttl());
                let taken = shaped.and_then(|s| set.take(s));
                let header = if set.is_empty() { cookie.clear() } else { cookie.write(&set, V::NAME) };
                if !ctx.switch_to_http().append_response_header("set-cookie", &header) {
                    return Err(self.internal("the state cookie could not be added to the response", String::new()));
                }
                Ok(taken)
            }
            Storage::Session => {
                let Some(session) = req.session() else {
                    return Err(self.internal("the request has no session: SessionModule did not run for it", String::new()));
                };
                let mut set = state::read_session(&session, V::NAME);
                set.prune(now, self.ttl());
                let taken = shaped.and_then(|s| set.take(s));
                if !state::write_session(&session, V::NAME, &set) {
                    return Err(self.internal("the session refused the state record", String::new()));
                }
                Ok(taken)
            }
        }
    }

    async fn callback(&self, ctx: &ExecutionContext, scope: &[String]) -> StrategyOutcome {
        let req = ctx.request();
        Self::fixed_headers(ctx);
        let query = req.query();
        // Each parameter is absent or one string; anything else is a malformed callback.
        let single = |name: &str| -> Result<Option<&str>, ()> {
            match query.get(name) {
                None => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.as_str())),
                Some(_) => Err(()),
            }
        };
        let (code, state, error) = (single("code"), single("state"), single("error"));
        // The flow is taken out of the store first, whatever else is wrong with the request.
        let taken = match self.take_flow(ctx, state.as_ref().ok().copied().flatten()) {
            Ok(taken) => taken,
            Err(outcome) => return outcome,
        };
        let (Ok(code), Ok(state), Ok(error)) = (code, state, error) else {
            return self.fail("invalid callback", None, Phase::Callback);
        };
        if code.is_some() && error.is_some() {
            return self.fail("invalid callback", None, Phase::Callback);
        }
        if let Some(code) = code
            && (code.len() > self.options.max_code_bytes || code.is_empty())
        {
            return self.fail("invalid callback", None, Phase::Callback);
        }
        if state.is_none() {
            return self.fail("state mismatch", None, Phase::Callback);
        }
        let Some(flow) = taken else {
            return self.fail("state mismatch", None, Phase::Callback);
        };
        // A flow that started under a session (or under none) completes under the same one only: with the state in a
        // cookie, a logout and another user's login on a shared browser would otherwise let the pending callback finish
        // as the new user. The flow is already taken out of the cookie, so a refusal burns it.
        if let Storage::Cookie(cookie) = &self.storage {
            let current = state::session_binding(&cookie.keys, V::NAME, &flow.state, req.session().as_ref());
            if !state::binding_matches(&flow, current.as_deref()) {
                return self.fail("state mismatch", None, Phase::Callback);
            }
        }
        // And for the user it was started by, when the callback carries one: a token or an API key does not live in
        // the session, so the session binding alone lets another user's callback route finish a pending flow and link
        // the first user's provider account to the second.
        let keys = match &self.storage {
            Storage::Cookie(cookie) => Some(&cookie.keys),
            Storage::Session => None,
        };
        let current_user = state::user_binding(keys, V::NAME, &flow.state, req.user().as_ref());
        if !state::user_matches(&flow, &current_user) {
            return self.fail("state mismatch", None, Phase::Callback);
        }
        if let Some(error) = error {
            let code = super::client::sanitize_error_code(error);
            return if error == "access_denied" {
                self.fail("access denied", None, Phase::Callback)
            } else {
                self.fail("provider error", code, Phase::Callback)
            };
        }
        let Some(code) = code else {
            return self.fail("invalid callback", None, Phase::Callback);
        };
        let verifier = match &self.storage {
            Storage::Cookie(cookie) => self.options.pkce.then(|| state::derive_verifier(&cookie.keys, V::NAME, &flow.state, flow.issued)),
            Storage::Session => self.options.pkce.then(|| flow.verifier.clone()).flatten(),
        };
        // A client address holds a limited share of the in-flight slots for the whole exchange. The slot is taken before
        // the state is claimed, so a refusal here burns nothing of the flow on the server (the answer still clears the
        // state cookie in the browser, like every refusal of the callback).
        let Some(slot) = self.slots.acquire(client_key(req)) else {
            self.note("one client address holds its share of the requests to the provider", NoteLevel::Warn, String::new());
            return StrategyOutcome::Error(service_unavailable());
        };
        // Only a state that verified reaches this point; it is redeemed once, so a replay (or a concurrent copy of the
        // same callback) costs no token request.
        if !self.redeemed.claim(&flow.state, Self::now()) {
            return self.fail("state mismatch", None, Phase::Callback);
        }
        let host = self.provider.token_host();
        let token = match self.provider.token(Grant::AuthorizationCode { code, verifier: verifier.as_deref() }).await {
            Ok(token) => token,
            Err(OAuth2Error::Rejected { code }) => {
                self.note(
                    "the provider rejected the code",
                    NoteLevel::Warn,
                    format!(" by {host}{}", code.as_deref().map(|c| format!(" ({c})")).unwrap_or_default()),
                );
                return self.fail("code rejected", code, Phase::Callback);
            }
            Err(OAuth2Error::Busy) => {
                self.note("too many requests to the provider are in flight", NoteLevel::Warn, String::new());
                return StrategyOutcome::Error(service_unavailable());
            }
            Err(error) => return self.internal("the token request failed", format!(" ({host}): {error}")),
        };
        let profile = match self.provider.profile(V::NAME, &token.access_token, scope).await {
            Ok(profile) => profile,
            Err(OAuth2Error::Busy) => {
                self.note("too many requests to the provider are in flight", NoteLevel::Warn, String::new());
                return StrategyOutcome::Error(service_unavailable());
            }
            Err(error) => return self.internal("the profile request failed", format!(": {error}")),
        };
        drop(slot);
        let grant = OAuth2Grant { access_token: token.access_token, refresh_token: token.refresh_token, params: token.response, profile };
        match self.validator.validate(req, grant).await {
            Err(e) => StrategyOutcome::Error(e),
            Ok(None) => self.refused(),
            Ok(Some(user)) => {
                let info = self.validator.info(&user);
                match AuthenticatedUser::new(user) {
                    // The authenticator turns this into a 401; with a failure page configured the callback goes there.
                    Ok(user) if user.is_empty() && self.options.failure_redirect.is_some() => self.refused(),
                    Ok(user) if user.is_empty() => StrategyOutcome::Success { user, info },
                    Ok(user) => {
                        if let Some(path) = flow.return_to {
                            req.insert(ReturnTo(path));
                        }
                        StrategyOutcome::Success { user, info }
                    }
                    Err(e) => StrategyOutcome::Error(e),
                }
            }
        }
    }
}

#[async_trait]
impl<V: OAuth2Validator> Strategy for OAuth2Strategy<V> {
    fn name(&self) -> &str {
        V::NAME
    }

    /// The callback carries the authorization `code` and the `state` in its query string.
    fn credential_query_params(&self) -> Vec<String> {
        // `error_description` and `error_uri` are no credentials, but they are text the other side of the redirect
        // chose: hidden too, so no log line or error body that prints the redacted URL reflects them.
        ["code", "state", "error_description", "error_uri"].map(String::from).to_vec()
    }

    async fn authenticate(&self, ctx: &ExecutionContext, options: &AuthenticateOptions) -> StrategyOutcome {
        let req = ctx.request();
        if req.method() != http::Method::GET {
            return self.fail("invalid callback", None, Phase::Start);
        }
        let Ok(scope) = self.scope(options) else {
            return self.internal("the guard's scope option is not a list of valid scope values", String::new());
        };
        let query = req.query();
        if query.contains_key("code") || query.contains_key("error") { self.callback(ctx, &scope).await } else { self.start(ctx, &scope) }
    }
}
