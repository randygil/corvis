//! `AuthGuard`: the port of `AuthGuard('name')` of the original passport integration over `passport.authenticate`.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::authenticator::{AuthFailure, AuthResult, Authenticator, internal};
use super::strategy::AuthenticateOptions;
use crate::common::context::{ContextType, ExecutionContext};
use crate::common::guard::CanActivate;
use crate::common::reflector::Reflector;
use crate::http::{HttpException, UnauthorizedException};

type FailureFn = dyn Fn(&[AuthFailure]) -> HttpException + Send + Sync;

enum Names {
    /// One or several names given to `new` or `chain`.
    Explicit(Vec<String>),
    /// The default strategies of `PassportModule::register`.
    Default,
}

/// Authenticates a request with named strategies, as `@UseGuards(AuthGuard('jwt'))` does:
///
/// - the strategies run in order; the first success stores the user (see [`User`](super::User)) and lets the route
///   run;
/// - when every strategy fails the answer is `401 {"message":"Unauthorized","statusCode":401}` through the
///   exception filters, whatever status or challenge the strategies attached to their failure;
/// - an error of a strategy is passed on unchanged; a redirect ends the request with the redirect.
///
/// Unlike the original, the strategy names are checked when the application starts. It is an instance guard:
/// `#[use_guards(AuthGuard::new("jwt"))]` or `app.use_global_guards(AuthGuard::new("jwt"))`. It does not work in a
/// WebSocket context, where it denies the message.
///
/// ```ignore
/// AuthGuard::new("jwt")                        // one strategy
/// AuthGuard::chain(["jwt", "apikey"])          // tried in order
/// AuthGuard::default()                         // the default of PassportModule::register
///     .optional()                              // no user instead of 401
///     .on_failure(|_| UnauthorizedException::new("Token required"))
///     .skip_public()                           // let #[public] routes through
/// ```
pub struct AuthGuard {
    names: Names,
    optional: bool,
    skip_public: bool,
    on_failure: Option<Arc<FailureFn>>,
    options: AuthenticateOptions,
    #[cfg(feature = "auth-session")]
    log_in: bool,
}

impl Default for AuthGuard {
    /// A guard for the default strategy configured with `PassportModule::register` (`AuthGuard()` in the original).
    /// Initialization fails when none is configured.
    fn default() -> Self {
        AuthGuard {
            names: Names::Default,
            optional: false,
            skip_public: false,
            on_failure: None,
            options: AuthenticateOptions::new(),
            #[cfg(feature = "auth-session")]
            log_in: false,
        }
    }
}

impl AuthGuard {
    /// `AuthGuard('jwt')`.
    pub fn new(strategy: impl Into<String>) -> Self {
        Self::chain([strategy.into()])
    }

    /// `AuthGuard(['jwt', 'apikey'])`: the strategies are tried in order, the first success wins.
    pub fn chain<I, S>(strategies: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        AuthGuard { names: Names::Explicit(strategies.into_iter().map(Into::into).collect()), ..Self::default() }
    }

    /// Lets the route run without a user when every strategy fails (the usual `handleRequest` override that returns
    /// `user ?? null`). Errors are still passed on. An optional guard does not count as access control for route
    /// posture.
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// Replaces the 401 of a failed authentication with the exception `f` builds from the failures (the usual
    /// `handleRequest` override that throws another exception).
    pub fn on_failure<F>(mut self, f: F) -> Self
    where
        F: Fn(&[AuthFailure]) -> HttpException + Send + Sync + 'static,
    {
        self.on_failure = Some(Arc::new(f));
        self
    }

    /// Lets a public route through without running a strategy: the handler's boolean `PUBLIC_METADATA` value, else
    /// the controller's (what `#[public]` sets). Without it, public routes are authenticated like any other, as the
    /// original does.
    pub fn skip_public(mut self) -> Self {
        self.skip_public = true;
        self
    }

    /// After a successful authentication, logs the user in to the session before the handler runs (the login guard of
    /// the original: `super.canActivate(context)` followed by `super.logIn(request)`): a new session identifier is
    /// generated, the old session is destroyed and the user is stored for later requests. Use it on the login route
    /// with a credential strategy (`AuthGuard::new("local").log_in()`). Without it a guard never touches the session.
    ///
    /// The application must register a `SessionStrategy`, and the guard cannot name the `"session"` strategy itself
    /// (it would log in again on every request); both stop `init()`.
    ///
    /// Deliberate difference from the original: the login is kept only when the request is not rejected. If a later
    /// guard denies, a pipe fails, or the handler returns an error or panics, the login is discarded: the session the
    /// request arrived with (if any) is left as it was and no logged-in cookie is issued. The rejection is noted
    /// before any exception filter or interceptor can change the response, so what a filter answers (a redirect to a
    /// second step, a page with status 200) does not matter. A handler that returns normally keeps the login whatever
    /// status it sets: a handler that implements a second step must call [`Authenticator::log_in`] after the last
    /// step instead of using `.log_in()` on the first step's guard. A login made by a handler with
    /// [`Authenticator::log_in`] stands whatever the response is.
    #[cfg(feature = "auth-session")]
    pub fn log_in(mut self) -> Self {
        self.log_in = true;
        self
    }

    /// Options handed to every strategy this guard runs (`AuthenticateOptions::get`), the options object of
    /// `AuthGuard('x')` subclasses. Anything but a JSON object is ignored.
    pub fn with_options(mut self, options: Value) -> Self {
        if let Value::Object(map) = options {
            self.options.options = map;
        }
        self
    }

    /// The names this guard runs, or why it cannot run: an empty chain, no default configured.
    fn names<'a>(&'a self, authenticator: &'a Authenticator) -> Result<&'a [String], String> {
        match &self.names {
            Names::Explicit(names) if names.is_empty() => Err("AuthGuard::chain was given no strategy names".to_string()),
            Names::Explicit(names) => Ok(names.as_slice()),
            Names::Default => match authenticator.default_names() {
                Some(names) if !names.is_empty() => Ok(names),
                _ => Err("AuthGuard::default() is used but no default strategy is configured: import PassportModule::register(PassportModuleOptions::new().default_strategy(..))".to_string()),
            },
        }
    }

    /// What is wrong with this guard at startup, if anything.
    pub(crate) fn startup_problem(&self, authenticator: &Authenticator) -> Option<String> {
        let names = match self.names(authenticator) {
            Ok(names) => names,
            Err(problem) => return Some(problem),
        };
        let missing: Vec<String> = names.iter().filter(|n| !authenticator.has_strategy(n)).map(|n| format!("\"{n}\"")).collect();
        #[cfg(feature = "auth-session")]
        if self.log_in {
            if names.iter().any(|n| n == super::session::SESSION_STRATEGY) {
                return Some(
                    "log_in() cannot be combined with the \"session\" strategy: it would log in again on every request".to_string(),
                );
            }
            if !authenticator.has_strategy(super::session::SESSION_STRATEGY) {
                return Some(
                    "log_in() needs a registered session strategy: add Provider::strategy::<SessionStrategy<YourSerializer>>() and import SessionModule"
                        .to_string(),
                );
            }
        }
        if missing.is_empty() {
            return None;
        }
        let which = if matches!(self.names, Names::Default) { "the default strategy" } else { "the strategy" };
        Some(format!("{which} {} is not registered", missing.join(", ")))
    }

    /// The names of the strategies the guard was built with, for logs. Default guards list `default`.
    #[cfg(feature = "websockets")]
    pub(crate) fn describe(&self) -> String {
        match &self.names {
            Names::Explicit(names) => names.join(", "),
            Names::Default => "default".to_string(),
        }
    }

    fn is_public(&self, ctx: &ExecutionContext) -> bool {
        Reflector.get_all_and_override::<bool>(crate::PUBLIC_METADATA, &[ctx.get_handler(), ctx.get_class()]) == Some(true)
    }
}

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        // The handshake request of a connection is shared by all its messages: a strategy must not store a user on
        // it, and a redirect means nothing on a socket.
        if ctx.get_type() != ContextType::Http {
            return Ok(false);
        }
        if self.skip_public && self.is_public(ctx) {
            return Ok(true);
        }
        let authenticator = ctx
            .module_ref()
            .and_then(|m| m.get::<Authenticator>().ok())
            .ok_or_else(|| internal("the Authenticator provider was not found"))?;
        let names = self.names(&authenticator).map_err(internal)?;
        let req = ctx.request();
        let failures = match authenticator.authenticate(ctx, names, &self.options).await? {
            // The user is stored only when it is a real value; the authenticator already turns an empty one into a
            // failure, and this refuses it again for a result built by other means.
            result @ AuthResult::Success { .. } => {
                if result.store_user(req) {
                    #[cfg(feature = "auth-session")]
                    if self.log_in
                        && let AuthResult::Success { user, .. } = &result
                    {
                        authenticator.log_in_from_guard(req, user).await?;
                    }
                    return Ok(true);
                }
                Vec::new()
            }
            AuthResult::Pass => {
                // Nothing of an earlier guard survives a guard that passed: not the user, not the info either.
                req.clear_user();
                req.remove::<super::AuthInfo>();
                return Ok(true);
            }
            AuthResult::Redirected => return Ok(true),
            AuthResult::Failed { failures } => failures,
        };
        if self.optional {
            req.clear_user();
            return Ok(true);
        }
        Err(match &self.on_failure {
            Some(f) => f(&failures),
            None => UnauthorizedException::empty(),
        })
    }

    fn controls_access(&self) -> bool {
        !self.optional
    }

    fn guard_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}
