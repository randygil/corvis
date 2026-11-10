//! Authentication with named strategies: the model of `passport` and its integration for the original framework.
//!
//! A [`Strategy`] turns the credentials of a request into a user and is registered by name with
//! [`Provider::strategy`](crate::di::Provider::strategy). An [`AuthGuard`] on a route (or global) runs strategies by
//! name through the [`Authenticator`], stores the user for the handler (`User<T>`, `Request::user`) and answers the
//! original's `401 {"message":"Unauthorized","statusCode":401}` when every strategy fails.
//! [`PassportModule::register`] sets the default strategy of `AuthGuard::default()`.
//!
//! Strategies can end a request with a redirect and set cookies through
//! [`HttpArgumentsHost::respond_with`](crate::common::context::HttpArgumentsHost::respond_with) and
//! [`append_response_header`](crate::common::context::HttpArgumentsHost::append_response_header).
//!
//! The module is always compiled and has no dependencies of its own; the strategy kinds that need crates (password
//! hashing, ...) come with their own features. The JWT strategy, `corvis::auth::jwt`, comes with the `jwt` feature (a default one).
//!
//! ```ignore
//! #[module(
//!     imports = [PassportModule::register(PassportModuleOptions::new().default_strategy("header"))],
//!     providers = [Provider::strategy::<HeaderStrategy>()],
//!     controllers = [CatsController],
//! )]
//! pub struct AppModule;
//!
//! #[controller("cats")]
//! #[use_guards(AuthGuard::default())]
//! pub struct CatsController;
//! ```

#[cfg(feature = "auth-api-key")]
pub mod api_key;
mod authenticator;
mod guard;
#[cfg(feature = "auth-local")]
pub mod local;
mod module;
#[cfg(feature = "auth-oauth2")]
pub mod oauth2;
#[cfg(feature = "auth-session")]
pub mod session;
mod strategy;
mod user;
mod warn_limit;

#[cfg(feature = "jwt")]
pub mod jwt;

pub use authenticator::{AuthFailure, AuthResult, Authenticator};
pub use guard::AuthGuard;
pub use module::{PassportModule, PassportModuleOptions};
pub use strategy::{AuthInfo, AuthenticateOptions, AuthenticatedUser, Strategy, StrategyOutcome};
pub use user::User;

pub(crate) use authenticator::StrategyRegistry;
pub(crate) use strategy::AuthUserSlot;

use std::collections::HashMap;
use std::sync::Arc;

use crate::di::container::Container;
use crate::router::RouterTable;

/// Collects the strategies registered with `Provider::strategy` and the default strategy of the
/// `PassportModule` registrations into the registry of the core `Authenticator`. Fails on a duplicate name and on two
/// registrations with different defaults.
pub(crate) fn build_registry(container: &Container) -> crate::Result<()> {
    let Some(authenticator) = container.instances_of::<Authenticator>().first().cloned() else { return Ok(()) };
    let mut by_name: HashMap<String, (String, Arc<dyn Strategy>)> = HashMap::new();
    for (module, provider) in container.all_providers() {
        let Some(tag) = &provider.strategy else { continue };
        let token = provider.token.display_name();
        let strategy = match container.instance(module, &provider.token).as_ref().and_then(tag.cast) {
            Some(s) => s,
            None => anyhow::bail!("strategy {token} could not be registered: the provider has no instance or its value is not a strategy"),
        };
        let name = strategy.name().to_string();
        if let Err(problem) = check_name(&name) {
            anyhow::bail!("strategy {token} has an invalid name: {problem}");
        }
        if let Some((first, _)) = by_name.get(&name) {
            anyhow::bail!("two strategies are named \"{name}\": {first} and {token}");
        }
        by_name.insert(name, (token, strategy));
    }
    let mut default: Option<Vec<String>> = None;
    for options in container.instances_of::<PassportModuleOptions>() {
        let names = options.default_strategies.clone();
        match &default {
            Some(existing) if *existing != names => {
                anyhow::bail!(
                    "PassportModule::register was called with different default strategies: [{}] and [{}]",
                    existing.join(", "),
                    names.join(", ")
                );
            }
            _ => default = Some(names),
        }
    }
    authenticator.set_registry(StrategyRegistry { by_name: by_name.into_iter().map(|(k, (_, s))| (k, s)).collect(), default });
    Ok(())
}

/// The query parameters the registered strategies read credentials from, for the log lines of every request.
///
/// A name with a `%` stops the start: the parsers of the application and the log disagree on how a name that is not
/// valid percent-encoding reads, so its value could reach the strategy and still be printed.
pub(crate) fn credential_query_params(container: &Container) -> crate::Result<Vec<String>> {
    let Some(authenticator) = container.instances_of::<Authenticator>().first().cloned() else { return Ok(Vec::new()) };
    let Some(registry) = authenticator.registry() else { return Ok(Vec::new()) };
    let mut names = Vec::new();
    for (strategy, instance) in &registry.by_name {
        for name in instance.credential_query_params() {
            if name.contains('%') {
                anyhow::bail!(
                    "the strategy \"{strategy}\" reads a credential from the query parameter {name:?}: a name with `%` cannot be hidden reliably in the request log; use a name without it"
                );
            }
            names.push(name);
        }
    }
    Ok(names)
}

/// The longest strategy name accepted.
const MAX_NAME_BYTES: usize = 128;

/// A strategy name is not empty, at most 128 bytes and holds no control character (names are written to logs and
/// startup errors, and an empty one could be matched by a guard built from a blank string).
fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("the name is empty".to_string());
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(format!("the name is longer than {MAX_NAME_BYTES} bytes"));
    }
    if name.chars().any(char::is_control) {
        return Err("the name contains a control character".to_string());
    }
    Ok(())
}

/// The `AuthGuard` inside a guard, found through `CanActivate::as_any`.
fn auth_guard_of(guard: &dyn crate::common::guard::CanActivate) -> Option<&AuthGuard> {
    guard.as_any().and_then(|a| a.downcast_ref::<AuthGuard>())
}

/// How deep guards may nest through `inner_guards` before startup fails (a guard that returns itself would otherwise
/// recurse until the stack overflows).
const MAX_GUARD_DEPTH: usize = 32;

/// The `AuthGuard`s of a guard and, through `CanActivate::inner_guards`, of the guards it delegates to. `Err` when
/// the nesting is deeper than [`MAX_GUARD_DEPTH`].
fn auth_guards_in(guard: &dyn crate::common::guard::CanActivate, depth: usize) -> Result<Vec<&AuthGuard>, ()> {
    if depth > MAX_GUARD_DEPTH {
        return Err(());
    }
    let mut found: Vec<&AuthGuard> = auth_guard_of(guard).into_iter().collect();
    for inner in guard.inner_guards() {
        found.extend(auth_guards_in(inner, depth + 1)?);
    }
    Ok(found)
}

/// `true` when the guard is a `ThrottlerGuard` or delegates to one through `CanActivate::inner_guards`.
fn holds_throttler(guard: &dyn crate::common::guard::CanActivate, depth: usize) -> bool {
    if depth > MAX_GUARD_DEPTH {
        return false;
    }
    guard.as_any().is_some_and(|a| a.is::<crate::modules::throttler::ThrottlerGuard>())
        || guard.inner_guards().into_iter().any(|inner| holds_throttler(inner, depth + 1))
}

/// Logs one warning for a route whose guards run an `AuthGuard` and, later, a `ThrottlerGuard`: the rate limit is
/// checked after the password hash, the store read or the provider request it was meant to protect. A warning, not an
/// error: limiting authenticated users after authenticating them is legitimate. A wrapper guard that does not expose
/// the guards it holds through `as_any` or `inner_guards` hides both, and so the warning.
fn warn_throttler_after_auth(route: &crate::router::CompiledRoute) {
    let mut seen_auth = false;
    for guard in &route.guards {
        if seen_auth && holds_throttler(guard.as_ref(), 0) {
            crate::logger::Logger::new("AuthGuard").warn(format!(
                "AuthGuard runs before ThrottlerGuard on {} {}: the rate limit is checked after authentication. \
                 Register ThrottlerGuard as a global guard (Provider::app_guard) or list it first",
                route.method.as_str(),
                route.path
            ));
            return;
        }
        if auth_guards_in(guard.as_ref(), 0).is_ok_and(|found| !found.is_empty()) {
            seen_auth = true;
        }
    }
}

/// Startup check of the strategy names of every compiled route guard (global, controller and route level). One
/// error lists every problem with its route.
pub(crate) fn check_routes(container: &Container, table: &RouterTable) -> crate::Result<()> {
    let Some(authenticator) = container.instances_of::<Authenticator>().first().cloned() else { return Ok(()) };
    let mut problems: Vec<String> = Vec::new();
    for route in &table.routes {
        warn_throttler_after_auth(route);
        for guard in &route.guards {
            let Ok(auth_guards) = auth_guards_in(guard.as_ref(), 0) else {
                let line = format!(
                    "{} {}: guards are nested more than {MAX_GUARD_DEPTH} levels deep through inner_guards (does a guard return itself?)",
                    route.method.as_str(),
                    route.path
                );
                if !problems.contains(&line) {
                    problems.push(line);
                }
                continue;
            };
            for auth_guard in auth_guards {
                if let Some(problem) = auth_guard.startup_problem(&authenticator) {
                    let line = format!("{} {}: {problem}", route.method.as_str(), route.path);
                    if !problems.contains(&line) {
                        problems.push(line);
                    }
                }
            }
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    anyhow::bail!("AuthGuard cannot be used by {} route(s):\n  {}", problems.len(), problems.join("\n  "))
}

/// Warns once per gateway that an `AuthGuard` reaches its messages, where it denies every one.
#[cfg(feature = "websockets")]
pub(crate) fn warn_gateways(gateways: &[crate::posture::GatewayPosture]) {
    let log = crate::logger::Logger::new("AuthGuard");
    for gateway in gateways {
        let found = gateway.handlers.iter().flat_map(|h| h.guards.iter()).find_map(|g| auth_guard_of(g.as_ref()));
        if let Some(guard) = found {
            log.warn(format!(
                "Gateway {} ({}) runs an AuthGuard ({}): it denies every WebSocket message. Authenticate in handle_connection, \
                 or call apply_global_enhancers_to_gateways(false) if the guard is global \
                 (that removes every global enhancer from gateways, not only this guard)",
                gateway.name,
                gateway.path,
                guard.describe()
            ));
        }
    }
}
