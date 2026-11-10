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

mod authenticator;
mod guard;
mod module;
mod strategy;
mod user;

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

/// The `AuthGuard` inside a guard, found through `CanActivate::as_any`.
fn auth_guard_of(guard: &dyn crate::common::guard::CanActivate) -> Option<&AuthGuard> {
    guard.as_any().and_then(|a| a.downcast_ref::<AuthGuard>())
}

/// The `AuthGuard`s of a guard and, through `CanActivate::inner_guards`, of the guards it delegates to.
fn auth_guards_in(guard: &dyn crate::common::guard::CanActivate) -> Vec<&AuthGuard> {
    let mut found: Vec<&AuthGuard> = auth_guard_of(guard).into_iter().collect();
    for inner in guard.inner_guards() {
        found.extend(auth_guards_in(inner));
    }
    found
}

/// Startup check of the strategy names of every compiled route guard (global, controller and route level). One
/// error lists every problem with its route.
pub(crate) fn check_routes(container: &Container, table: &RouterTable) -> crate::Result<()> {
    let Some(authenticator) = container.instances_of::<Authenticator>().first().cloned() else { return Ok(()) };
    let mut problems: Vec<String> = Vec::new();
    for route in &table.routes {
        for guard in &route.guards {
            for auth_guard in auth_guards_in(guard.as_ref()) {
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
