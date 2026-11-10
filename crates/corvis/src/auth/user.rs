//! The `User<T>` parameter extractor.

use std::ops::Deref;

use async_trait::async_trait;

use crate::common::context::ExecutionContext;
use crate::common::param::FromContext;
use crate::http::{HttpException, UnauthorizedException};

/// The authenticated user of the request as a handler parameter, the equivalent of a `@User()` parameter decorator
/// over `req.user`. It reads the typed value an [`AuthGuard`](super::AuthGuard) stored, and answers the
/// `401 Unauthorized` of a failed authentication when there is none (no guard on the route, or an optional guard
/// that found no credentials).
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
        ctx.request().get::<T>().map(User).ok_or_else(UnauthorizedException::empty)
    }
}
