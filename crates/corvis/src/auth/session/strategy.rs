//! `SessionSerializer` and the `"session"` strategy: the port of `serializeUser`, `deserializeUser` and
//! `passport.session()`.

use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::manager::{SessionManager, UserCodec};
use super::middleware::SessionUser;
use crate::auth::{AuthenticateOptions, AuthenticatedUser, Strategy, StrategyOutcome};
use crate::common::context::ExecutionContext;
use crate::di::{DiError, Injectable, ResolveCtx};
use crate::http::{HttpException, Request};

/// How the session stores the user and gets it back: what `passport.serializeUser` and `passport.deserializeUser` do.
/// Store something small and stable (the user's id), and look the user up in `deserialize_user`, so a user deleted or
/// disabled after logging in is noticed on the next request.
///
/// ```ignore
/// #[injectable]
/// pub struct UserSerializer { users: Arc<UsersService> }
///
/// #[async_trait]
/// impl SessionSerializer for UserSerializer {
///     type User = User;
///     async fn serialize_user(&self, user: &User) -> Result<Value, HttpException> { Ok(json!(user.id)) }
///     async fn deserialize_user(&self, _: &Request, stored: Value) -> Result<Option<User>, HttpException> {
///         match stored.as_u64() {
///             Some(id) => self.users.find(id).await,
///             None => Ok(None),
///         }
///     }
/// }
/// // providers = [UsersService, Provider::strategy::<SessionStrategy<UserSerializer>>()]
/// ```
///
/// The stored value must not be `null`, `false`, `0` or `""` (such a login is refused), and a user whose JSON form is
/// one of those is never restored.
#[async_trait]
pub trait SessionSerializer: Injectable {
    /// The user handed to the handler (`User<Self::User>`) and to `Authenticator::log_in`.
    type User: Serialize + DeserializeOwned + Clone + Send + Sync + 'static;

    /// The value saved under `passport.user`.
    async fn serialize_user(&self, user: &Self::User) -> Result<Value, HttpException>;

    /// The user for a stored value, `None` when there is no such user any more (the session is then anonymous and the
    /// stored user is removed), or an exception that becomes the response. Runs on every request that carries a
    /// session with a user, before the guards.
    async fn deserialize_user(&self, req: &Request, stored: Value) -> Result<Option<Self::User>, HttpException>;
}

struct Codec<S: SessionSerializer>(Arc<S>);

#[async_trait]
impl<S: SessionSerializer> UserCodec for Codec<S> {
    async fn serialize(&self, user: &AuthenticatedUser) -> Result<Value, HttpException> {
        // The typed value when it is a `S::User`, its JSON form otherwise (a user another strategy built).
        let typed = match user.typed::<S::User>() {
            Some(u) => u.clone(),
            None => serde_json::from_value::<S::User>(user.json().clone())
                .map_err(|_| crate::auth::authenticator::internal("the user cannot be converted to the type of the session serializer"))?,
        };
        self.0.serialize_user(&typed).await
    }

    async fn deserialize(&self, req: &Request, stored: Value) -> Result<Option<AuthenticatedUser>, HttpException> {
        match self.0.deserialize_user(req, stored).await? {
            Some(user) => AuthenticatedUser::new(user).map(Some),
            None => Ok(None),
        }
    }
}

/// The strategy named `"session"`, and the registration of the application's [`SessionSerializer`]: register it with
/// `Provider::strategy::<SessionStrategy<S>>()`. A second one stops the application at startup.
///
/// `AuthGuard::new("session")` lets a request through when the session restored a user and answers the plain
/// `401 {"message":"Unauthorized","statusCode":401}` otherwise (what the usual `isAuthenticated()` guard written with
/// `UnauthorizedException` answers; the `session` strategy of the original always passes).
pub struct SessionStrategy<S: SessionSerializer> {
    _marker: PhantomData<fn() -> S>,
}

impl<S: SessionSerializer> std::fmt::Debug for SessionStrategy<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionStrategy")
    }
}

impl<S: SessionSerializer> Injectable for SessionStrategy<S> {
    fn construct(ctx: ResolveCtx) -> BoxFuture<'static, Result<Self, DiError>> {
        async move {
            let serializer = ctx.get_or_construct::<S>().await?;
            let manager = ctx.get::<SessionManager>().await?;
            if !manager.set_codec(Arc::new(Codec(serializer))) {
                return Err(DiError::Factory {
                    token: format!("SessionStrategy ({})", std::any::type_name::<S>()),
                    cause: Arc::new(anyhow::anyhow!("a session strategy is registered already: an application has one SessionSerializer")),
                });
            }
            Ok(SessionStrategy { _marker: PhantomData })
        }
        .boxed()
    }
}

#[async_trait]
impl<S: SessionSerializer> Strategy for SessionStrategy<S> {
    fn name(&self) -> &str {
        SESSION_STRATEGY
    }

    async fn authenticate(&self, ctx: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
        match ctx.request().get::<SessionUser>() {
            Some(SessionUser(user)) => StrategyOutcome::Success { user, info: None },
            None => StrategyOutcome::Fail { challenge: None, status: None },
        }
    }
}

/// The name of the strategy `SessionStrategy` registers.
pub const SESSION_STRATEGY: &str = "session";
