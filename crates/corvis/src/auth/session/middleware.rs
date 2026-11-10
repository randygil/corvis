//! `SessionMiddleware`: loads the session before the guards and saves it after the handler.

use std::sync::Arc;

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::BoxFuture;

use super::manager::SessionManager;
use super::session::{PASSPORT_KEY, Session, USER_KEY};
use crate::auth::AuthenticatedUser;
use crate::common::middleware::{CorvisMiddleware, Next};
use crate::di::{DiError, Injectable, ResolveCtx};
use crate::http::{HandlerResult, HttpException, Request};

/// The user the session restored for this request, read by the `"session"` strategy. It lives next to the request's
/// user, not in it: a guard that replaces or clears the request's user leaves it alone.
#[derive(Clone)]
pub(crate) struct SessionUser(pub(crate) AuthenticatedUser);

/// The middleware of [`SessionModule`](super::SessionModule), the port of `express-session` with `passport.session()`:
///
/// 1. reads the session cookie, verifies its signature and loads the session (no store access without a valid
///    cookie);
/// 2. restores the logged-in user through the application's `SessionSerializer`, before any guard;
/// 3. runs the rest of the chain;
/// 4. writes the session to the store (awaited before the response is returned) and sets the cookie when needed.
///
/// `SessionModule` installs it for every route. It is public so it can also be registered with
/// `app.use_middleware(SessionMiddleware::new(manager))` to run before other global middleware.
pub struct SessionMiddleware {
    manager: Arc<SessionManager>,
}

impl SessionMiddleware {
    pub fn new(manager: Arc<SessionManager>) -> Self {
        SessionMiddleware { manager }
    }
}

impl Injectable for SessionMiddleware {
    fn construct(ctx: ResolveCtx) -> BoxFuture<'static, Result<Self, DiError>> {
        async move { Ok(SessionMiddleware { manager: ctx.get::<SessionManager>().await? }) }.boxed()
    }
}

/// Destroys the identifiers a request queued when the request is cancelled (the client went away) before its
/// session could be committed: a logout must not be lost because the connection dropped.
struct CommitGuard {
    session: Session,
    done: bool,
}

impl Drop for CommitGuard {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let ids = self.session.state().destroy.clone();
        if ids.is_empty() {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(self.session.manager().clone().destroy_queued(ids));
        }
    }
}

#[async_trait]
impl CorvisMiddleware for SessionMiddleware {
    async fn use_(&self, req: Request, next: Next) -> HandlerResult {
        let manager = &self.manager;
        // The original does nothing for a request outside the cookie's path, and so does this; it also leaves alone a
        // request that already has a session (the middleware was registered twice).
        if !req.path().starts_with(manager.options.cookie.path.as_str()) || req.get::<Session>().is_some() {
            return next.run(req).await;
        }
        let session = manager.load(&req).await?;
        req.insert(session.clone());
        let mut guard = CommitGuard { session: session.clone(), done: false };
        restore_user(manager, &session, &req).await?;

        let extensions = req.extensions.clone();
        let result = next.run(req).await;

        // A guard login is kept or discarded by what the pipeline did, not by the status of the response: a filter can
        // turn a denial into a redirect or a page with any status. An `Err` that reaches this point never became a
        // response, so it is a rejection too.
        let status = result.as_ref().ok().map(|response| response.status_code());
        let rejected = result.is_err() || crate::common::context::RequestRejected::is_marked(&extensions);
        let cookie = manager.commit(&session, status, rejected).await?;
        guard.done = true;
        let mut response = result?;
        if let Some(cookie) = cookie {
            response.append_header("set-cookie", cookie);
        }
        Ok(response)
    }
}

/// Deserializes `passport.user` and puts the user on the request, as `passport.session()` does. A user that is no
/// longer there (or an empty value) is removed from the session and the request goes on anonymous; an error of the
/// serializer is the response.
async fn restore_user(manager: &SessionManager, session: &Session, req: &Request) -> Result<(), HttpException> {
    let Some(codec) = manager.codec() else { return Ok(()) };
    let Some(stored) = take_stored_user(session) else { return Ok(()) };
    let lookup = tokio::time::timeout(manager.options.store_timeout, codec.deserialize(req, stored)).await;
    match lookup {
        Err(_) => Err(manager.fault("the session user lookup did not answer in time", None)),
        Ok(Err(e)) => Err(e),
        // An empty user (null, false, 0, "") is not a user, whatever the serializer says.
        Ok(Ok(Some(user))) if !user.is_empty() => {
            user.install(req);
            req.insert(SessionUser(user));
            Ok(())
        }
        Ok(Ok(_)) => {
            session.remove_passport_user();
            Ok(())
        }
    }
}

/// The stored user of the session, when there is one worth deserializing. An empty stored value (`null`, `false`, `0`,
/// `""`) is dropped from the session instead.
fn take_stored_user(session: &Session) -> Option<serde_json::Value> {
    let stored = session.passport_user()?;
    if super::login::is_empty_json(&stored) {
        session.remove_passport_user();
        return None;
    }
    Some(stored)
}

impl Session {
    /// The value under `passport.user`, if `passport` is an object that has it.
    pub(crate) fn passport_user(&self) -> Option<serde_json::Value> {
        self.state().data.get(PASSPORT_KEY).and_then(|p| p.get(USER_KEY)).cloned()
    }

    /// Removes `passport.user`, leaving the rest of `passport`.
    pub(crate) fn remove_passport_user(&self) {
        if let Some(serde_json::Value::Object(passport)) = self.state().data.get_mut(PASSPORT_KEY) {
            passport.shift_remove(USER_KEY);
        }
    }
}
