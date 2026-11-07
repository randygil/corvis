//! Guards (`CanActivate`).

use async_trait::async_trait;

use super::context::ExecutionContext;
use crate::http::HttpException;

/// A guard determines whether a given request will be handled by the route handler.
/// Returning `Ok(false)` produces `403 Forbidden resource`; returning an `Err`
/// lets you throw a custom exception (e.g. `UnauthorizedException`).
///
/// ```ignore
/// #[injectable]
/// pub struct AuthGuard;
///
/// #[async_trait]
/// impl CanActivate for AuthGuard {
///     async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
///         Ok(ctx.switch_to_http().get_request().header("authorization").is_some())
///     }
/// }
/// ```
#[async_trait]
pub trait CanActivate: Send + Sync + 'static {
    async fn can_activate(&self, context: &ExecutionContext) -> Result<bool, HttpException>;
}
