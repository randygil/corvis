//! Server-side sessions with a signed cookie: the port of `express-session` with `passport.session()`, `req.logIn` and
//! `req.logOut`. Available with the `auth-session` cargo feature.
//!
//! [`SessionModule::for_root`] installs [`SessionMiddleware`], which reads the signed session cookie
//! (`connect.sid`, the value format of `express-session`), loads the session before the guards, exposes it as
//! `Request::session()` and, after the handler, saves it to a [`SessionStore`] and sets the cookie when needed.
//! [`SessionStrategy`] with the application's [`SessionSerializer`] restores the logged-in user on later requests
//! and registers the strategy named `"session"`; `AuthGuard::new("local").log_in()` logs in to the session.
//!
//! What is stricter than the original on purpose: the cookie is `HttpOnly; SameSite=Lax; Secure` by default,
//! identifiers come from the operating system's generator and are regenerated on login and logout, idle and absolute
//! timeouts are enforced by the middleware, a destroyed session cannot be re-created by a slower request, and the
//! memory store is bounded.
//!
//! A session cookie is an ambient credential: the browser sends it with every request to the site, so routes that
//! change state need protection against cross-site requests. `SameSite=Lax` stops the cookie on cross-site `POST`
//! requests in current browsers, and nothing else here does: no CSRF token is provided.

#[cfg(test)]
mod fuzz;
mod login;
mod manager;
mod memory;
mod middleware;
mod module;
mod options;
#[allow(clippy::module_inception)]
mod session;
mod store;
mod strategy;

pub use manager::SessionManager;
pub use memory::{DEFAULT_MAX_SESSIONS, DEFAULT_MAX_SESSIONS_PER_USER, DEFAULT_MAX_TOTAL_BYTES, MemorySessionStore};
pub use middleware::SessionMiddleware;
pub use module::SessionModule;
pub use options::{
    DEFAULT_ABSOLUTE_TIMEOUT, DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_SESSION_BYTES, DEFAULT_STORE_TIMEOUT, MAX_TIMEOUT, MIN_IDLE_TIMEOUT,
    SameSite, SessionCookie, SessionOptions, SessionOptionsError,
};
pub use session::{Session, SessionError};
pub use store::{SessionStore, SessionStoreError};
pub use strategy::{SESSION_STRATEGY, SessionSerializer, SessionStrategy};
