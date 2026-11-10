//! `LocalStrategy`: the port of `passport-local` 1.0.0, with bounded credentials.

use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::{Value, json};

use crate::auth::{AuthenticateOptions, AuthenticatedUser, Strategy, StrategyOutcome};
use crate::common::context::ExecutionContext;
use crate::di::{DiError, Injectable, ResolveCtx};
use crate::http::{HttpException, Request};
use crate::logger::Logger;

/// The default of `max_username_bytes` and `max_password_bytes`.
pub const DEFAULT_MAX_CREDENTIAL_BYTES: usize = 1024;

/// The options of the local strategy, the `usernameField` and `passwordField` of `passport-local` plus the length
/// limits.
#[derive(Debug, Clone)]
pub struct LocalStrategyOptions {
    username_field: String,
    password_field: String,
    max_username_bytes: usize,
    max_password_bytes: usize,
    allow_query_credentials: bool,
}

impl Default for LocalStrategyOptions {
    fn default() -> Self {
        LocalStrategyOptions {
            username_field: "username".to_string(),
            password_field: "password".to_string(),
            max_username_bytes: DEFAULT_MAX_CREDENTIAL_BYTES,
            max_password_bytes: DEFAULT_MAX_CREDENTIAL_BYTES,
            allow_query_credentials: false,
        }
    }
}

impl LocalStrategyOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// The field that holds the username (`usernameField`), `username` by default. `a[b]` reads `body.a.b`.
    pub fn username_field(mut self, field: impl Into<String>) -> Self {
        self.username_field = field.into();
        self
    }

    /// The field that holds the password (`passwordField`), `password` by default.
    pub fn password_field(mut self, field: impl Into<String>) -> Self {
        self.password_field = field.into();
        self
    }

    /// The longest username accepted, 1024 bytes by default.
    pub fn max_username_bytes(mut self, max: usize) -> Self {
        self.max_username_bytes = max;
        self
    }

    /// The longest password accepted, 1024 bytes by default.
    pub fn max_password_bytes(mut self, max: usize) -> Self {
        self.max_password_bytes = max;
        self
    }

    /// Also reads the username and the password from the query string when the body has none, as `passport-local`
    /// does. Off by default (a deliberate divergence): URLs end up in access logs, browser history and `Referer`
    /// headers. A warning is logged at startup when it is on.
    pub fn allow_query_credentials(mut self) -> Self {
        self.allow_query_credentials = true;
        self
    }
}

/// The class `PassportStrategy(Strategy)` of `passport-local` is written as: the options and `validate`.
/// `validate` runs only when both credentials were found and are within their limits.
///
/// ```ignore
/// #[injectable]
/// pub struct LocalAuth { users: Arc<UsersService>, hasher: Arc<PasswordHasher> }
///
/// #[async_trait]
/// impl LocalValidator for LocalAuth {
///     type User = PublicUser;
///     async fn validate(&self, _: &Request, username: &str, password: &str) -> Result<Option<PublicUser>, HttpException> {
///         let user = self.users.find_by_username(username).await;
///         let ok = self.hasher.verify_or_dummy(password, user.as_ref().map(|u| u.password_hash.as_str())).await
///             .map_err(PasswordError::into_http_exception)?; // Busy is a 503, the same for every user
///         Ok(user.filter(|_| ok).map(PublicUser::from))
///     }
/// }
/// ```
#[async_trait]
pub trait LocalValidator: Injectable {
    /// The name routes use: `AuthGuard::new("local")`.
    const NAME: &'static str = "local";

    /// The user handed to the handler (`User<Self::User>`). Do not put the password hash in it.
    type User: Serialize + Clone + Send + Sync + 'static;

    /// The options, read once when the application starts.
    fn options(&self) -> LocalStrategyOptions {
        LocalStrategyOptions::default()
    }

    /// `validate(username, password)`: the user, `None` to refuse the credentials (401), or an exception (passed on
    /// as it is). Verify the password with [`PasswordHasher::verify_or_dummy`](super::PasswordHasher::verify_or_dummy)
    /// so an unknown username costs as much as a wrong password.
    async fn validate(&self, req: &Request, username: &str, password: &str) -> Result<Option<Self::User>, HttpException>;

    /// The auth info of a successful attempt (`Request::auth_info`), none by default.
    fn info(&self, _user: &Self::User) -> Option<Value> {
        None
    }
}

struct Resolved {
    username: Vec<String>,
    password: Vec<String>,
    max_username_bytes: usize,
    max_password_bytes: usize,
    allow_query: bool,
}

/// `field.split(']').join('').split('[')` of `utils.lookup`.
fn split_path(field: &str) -> Vec<String> {
    field.replace(']', "").split('[').map(str::to_string).collect()
}

impl Resolved {
    fn new(options: &LocalStrategyOptions) -> Result<Self, String> {
        if options.username_field.is_empty() {
            return Err("the username field name is empty".to_string());
        }
        if options.password_field.is_empty() {
            return Err("the password field name is empty".to_string());
        }
        if options.max_username_bytes == 0 || options.max_password_bytes == 0 {
            return Err("max_username_bytes and max_password_bytes must be at least 1".to_string());
        }
        Ok(Resolved {
            username: split_path(&options.username_field),
            password: split_path(&options.password_field),
            max_username_bytes: options.max_username_bytes,
            max_password_bytes: options.max_password_bytes,
            allow_query: options.allow_query_credentials,
        })
    }

    /// The query the lookup reads: the request's when `allow_query_credentials` is on, `Null` (nothing) otherwise.
    fn query_of(&self, req: &Request) -> Value {
        if self.allow_query { Value::Object(req.query().clone()) } else { Value::Null }
    }

    /// The username the strategy would read from `req`: `None` when it is missing or longer than the limit (the cases
    /// where the strategy answers "Missing credentials" or "Credentials too long" before the validator). The one
    /// place both [`LocalStrategy`] and [`login_tracker`] get a username from, so they cannot disagree.
    fn username_of(&self, req: &Request) -> Option<String> {
        let query = self.query_of(req);
        credential(req.body(), &query, &self.username).filter(|u| u.len() <= self.max_username_bytes).map(str::to_string)
    }
}

/// A throttler tracker (for `ThrottlerStorage::with_named_tracker`) that counts login attempts per account: the
/// username the local strategy configured with `options` will read from the same request (same field, same body then
/// query order, same length limit), as `"acct:"` and the hex SHA-256 of its bytes. The digest keeps usernames out of
/// the storage and bounds the key. A request with no readable username is counted by the normalized client address,
/// as the built-in tracker does, so it is still limited. The username is used as submitted (no case folding): wrap the
/// tracker when the application treats `Ana@x.io` and `ana@x.io` as one account.
///
/// ```ignore
/// ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 100), ThrottlerOptions::named("login", 60_000, 5)])
///     .with_named_tracker("login-account", login_tracker(LocalStrategyOptions::default()))
/// // on the route: #[apply_decorators(throttle("login").tracker("login-account"))]
/// ```
///
/// # Panics
///
/// When `options` are not valid for the strategy (an empty field name, a zero length limit): the strategy would stop
/// the application at startup for the same reason.
pub fn login_tracker(
    options: LocalStrategyOptions,
) -> impl Fn(ExecutionContext) -> std::future::Ready<Result<String, HttpException>> + Send + Sync + 'static {
    let resolved = Resolved::new(&options).unwrap_or_else(|problem| panic!("login_tracker: invalid local strategy options: {problem}"));
    move |ctx| {
        let req = ctx.request();
        std::future::ready(Ok(match resolved.username_of(req) {
            Some(username) => account_key(&username),
            None => client_key(&ctx),
        }))
    }
}

/// `"acct:"` and the lowercase hex SHA-256 of the username's bytes.
fn account_key(username: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let digest = Sha256::digest(username.as_bytes());
    let mut key = String::with_capacity(5 + 64);
    key.push_str("acct:");
    for byte in digest {
        let _ = write!(key, "{byte:02x}");
    }
    key
}

/// The built-in tracker's value for the request: the client address, normalized as the throttler normalizes it.
fn client_key(ctx: &ExecutionContext) -> String {
    let prefix = ctx
        .module_ref()
        .and_then(|m| m.get::<crate::modules::throttler::ThrottlerStorage>().ok())
        .map(|s| s.ipv6_subnet_prefix())
        .unwrap_or(crate::modules::throttler::DEFAULT_IPV6_SUBNET_PREFIX);
    crate::modules::throttler::normalize_ip(&ctx.request().ip(), prefix)
}

/// What a lookup of a field in one source found.
#[derive(Debug, PartialEq)]
enum Found<'a> {
    /// A non-empty string.
    Text(&'a str),
    /// Nothing usable: absent, empty, an object or array at the end of the path, or not a string.
    Nothing,
}

/// `utils.lookup` of `passport-local` over a JSON value. A segment of an object is a key and a segment of an array is
/// a canonical index. A scalar met on the way ends the lookup, even before the end of the path; only a non-empty
/// string counts (the original also passes numbers and booleans on, which Corvis treats as missing).
fn lookup<'a>(root: &'a Value, path: &[String]) -> Found<'a> {
    let mut current = root;
    for segment in path {
        let next = match current {
            Value::Object(map) => map.get(segment),
            Value::Array(items) => segment.parse::<usize>().ok().filter(|i| i.to_string() == *segment).and_then(|i| items.get(i)),
            _ => None,
        };
        match next {
            None => return Found::Nothing,
            Some(Value::String(s)) if !s.is_empty() => return Found::Text(s),
            Some(inner @ (Value::Object(_) | Value::Array(_))) => current = inner,
            Some(_) => return Found::Nothing,
        }
    }
    Found::Nothing
}

/// `lookup(req.body, field) || lookup(req.query, field)`, the second only when the query is read (`Null` otherwise).
fn credential<'a>(body: &'a Value, query: &'a Value, path: &[String]) -> Option<&'a str> {
    match lookup(body, path) {
        Found::Text(s) => Some(s),
        Found::Nothing => match lookup(query, path) {
            Found::Text(s) => Some(s),
            Found::Nothing => None,
        },
    }
}

/// The strategy of a [`LocalValidator`]: register it with `Provider::strategy::<LocalStrategy<V>>()`.
pub struct LocalStrategy<V: LocalValidator> {
    validator: Arc<V>,
    resolved: Resolved,
    _marker: PhantomData<fn() -> V>,
}

/// Prints nothing but the strategy name.
impl<V: LocalValidator> std::fmt::Debug for LocalStrategy<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LocalStrategy({})", V::NAME)
    }
}

impl<V: LocalValidator> Injectable for LocalStrategy<V> {
    fn construct(ctx: ResolveCtx) -> BoxFuture<'static, Result<Self, DiError>> {
        async move {
            let validator = ctx.get_or_construct::<V>().await?;
            let options = validator.options();
            if options.allow_query_credentials {
                Logger::new("LocalStrategy").warn(format!(
                    "strategy \"{}\" reads credentials from the query string (allow_query_credentials): URLs end up in access logs, browser history and Referer headers",
                    V::NAME
                ));
            }
            let resolved = Resolved::new(&options).map_err(|problem| DiError::Factory {
                token: format!("LocalStrategy \"{}\" ({})", V::NAME, std::any::type_name::<V>()),
                cause: Arc::new(anyhow::anyhow!("invalid local strategy options: {problem}")),
            })?;
            Ok(LocalStrategy { validator, resolved, _marker: PhantomData })
        }
        .boxed()
    }
}

fn fail(message: &str) -> StrategyOutcome {
    StrategyOutcome::Fail { challenge: Some(json!({ "message": message })), status: Some(400) }
}

#[async_trait]
impl<V: LocalValidator> Strategy for LocalStrategy<V> {
    fn name(&self) -> &str {
        V::NAME
    }

    fn credential_query_params(&self) -> Vec<String> {
        if !self.resolved.allow_query {
            return Vec::new();
        }
        // The first segment of `user[name]` is the parameter the query parser files the pair under.
        [&self.resolved.username, &self.resolved.password].into_iter().filter_map(|path| path.first().cloned()).collect()
    }

    async fn authenticate(&self, ctx: &ExecutionContext, _options: &AuthenticateOptions) -> StrategyOutcome {
        let req = ctx.request();
        // Without `allow_query_credentials` only the body is read: an empty query never yields a value.
        let query = self.resolved.query_of(req);
        let (Some(username), Some(password)) =
            (credential(req.body(), &query, &self.resolved.username), credential(req.body(), &query, &self.resolved.password))
        else {
            return fail("Missing credentials");
        };
        if username.len() > self.resolved.max_username_bytes || password.len() > self.resolved.max_password_bytes {
            return fail("Credentials too long");
        }
        match self.validator.validate(req, username, password).await {
            Err(e) => StrategyOutcome::Error(e),
            Ok(None) => StrategyOutcome::Fail { challenge: None, status: None },
            Ok(Some(user)) => {
                let info = self.validator.info(&user);
                match AuthenticatedUser::new(user) {
                    Ok(user) => StrategyOutcome::Success { user, info },
                    Err(e) => StrategyOutcome::Error(e),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(field: &str) -> Vec<String> {
        split_path(field)
    }

    #[test]
    fn paths_split_like_passport_local() {
        assert_eq!(path("username"), ["username"]);
        assert_eq!(path("user[email]"), ["user", "email"]);
        assert_eq!(path("a[b][c]"), ["a", "b", "c"]);
        assert_eq!(path("a]"), ["a"]);
        assert_eq!(path("a[]"), ["a", ""]);
    }

    #[test]
    fn lookup_follows_the_original_rules() {
        let body = json!({ "u": "ana", "n": 5, "b": false, "e": "", "o": { "email": "a@x.io", "deep": { "x": "y" } }, "l": ["p", "q"], "z": null });
        assert_eq!(lookup(&body, &path("u")), Found::Text("ana"));
        assert_eq!(lookup(&body, &path("o[email]")), Found::Text("a@x.io"));
        assert_eq!(lookup(&body, &path("o[deep][x]")), Found::Text("y"));
        assert_eq!(lookup(&body, &path("u[email]")), Found::Text("ana"), "a string before the end of the path is returned");
        assert_eq!(lookup(&body, &path("o")), Found::Nothing, "an object at the end of the path");
        assert_eq!(lookup(&body, &path("l")), Found::Nothing, "an array at the end of the path");
        assert_eq!(lookup(&body, &path("l[1]")), Found::Text("q"));
        assert_eq!(lookup(&body, &path("l[2]")), Found::Nothing);
        assert_eq!(lookup(&body, &path("l[length]")), Found::Nothing);
        assert_eq!(lookup(&body, &path("l[01]")), Found::Nothing);
        assert_eq!(lookup(&body, &path("n")), Found::Nothing, "numbers are missing");
        assert_eq!(lookup(&body, &path("b")), Found::Nothing);
        assert_eq!(lookup(&body, &path("e")), Found::Nothing, "empty is missing");
        assert_eq!(lookup(&body, &path("z")), Found::Nothing);
        assert_eq!(lookup(&body, &path("z[x]")), Found::Nothing, "null in the middle does not panic");
        assert_eq!(lookup(&body, &path("absent")), Found::Nothing);
        assert_eq!(lookup(&Value::Null, &path("u")), Found::Nothing);
        assert_eq!(lookup(&json!("text"), &path("u")), Found::Nothing);
        assert_eq!(lookup(&body, &path("")), Found::Nothing);
    }

    #[test]
    fn options_are_validated() {
        assert!(Resolved::new(&LocalStrategyOptions::default()).is_ok());
        assert!(Resolved::new(&LocalStrategyOptions::default().username_field("")).is_err());
        assert!(Resolved::new(&LocalStrategyOptions::default().password_field("")).is_err());
        assert!(Resolved::new(&LocalStrategyOptions::default().max_password_bytes(0)).is_err());
        assert!(!Resolved::new(&LocalStrategyOptions::default()).unwrap().allow_query);
        assert!(Resolved::new(&LocalStrategyOptions::default().allow_query_credentials()).unwrap().allow_query);
    }
}
