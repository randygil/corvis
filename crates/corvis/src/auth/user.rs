//! The `User<T>` parameter extractor.

use std::ops::Deref;

use async_trait::async_trait;

use crate::common::context::ExecutionContext;
use crate::common::param::FromContext;
use crate::http::{HttpException, UnauthorizedException};

/// The authenticated user of the request as a handler parameter, the equivalent of a `@User()` parameter decorator
/// over `req.user`. It reads the typed user's own slot, which holds what an [`AuthGuard`](super::AuthGuard) stored, or
/// what the session middleware restored from the session (or `Authenticator::log_in` stored), and nothing else: a
/// request extension of the same type put there by another guard is not a user, and `Request::set_user` drops it. It
/// answers the `401 Unauthorized` of a failed authentication when there is none (an optional guard that found no
/// credentials, or no user at all).
///
/// On a route without an `AuthGuard`, `User<T>` is the session's user when the session restored one, as
/// `Request::user()` is: nothing about the request itself checked it. Put an `AuthGuard` on every route that needs an
/// authenticated user. Guards read the same slot with [`Request::typed_user`](crate::http::Request::typed_user).
///
/// ```ignore
/// #[get("me")]
/// #[use_guards(AuthGuard::new("jwt"))]
/// fn me(&self, user: User<Claims>) -> Value { json!({ "id": user.id }) }
/// ```
#[derive(Debug, Clone)]
pub struct User<T>(pub T);

impl<T> Deref for User<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

#[async_trait]
impl<T: Clone + Send + Sync + 'static> FromContext for User<T> {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        super::strategy::AuthUserSlot::read::<T>(ctx.request()).map(User).ok_or_else(UnauthorizedException::empty)
    }
}
