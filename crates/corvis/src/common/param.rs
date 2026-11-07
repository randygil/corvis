//! Custom parameter decorators (`createParamDecorator`).
//!
//! Any handler parameter without a decorator attribute is resolved through
//! [`FromContext`]. Implement it for your own types to create custom decorators:
//!
//! ```ignore
//! pub struct CurrentUser(pub User);
//!
//! #[async_trait]
//! impl FromContext for CurrentUser {
//!     async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
//!         ctx.request().get::<User>().map(CurrentUser).ok_or(UnauthorizedException.into())
//!     }
//! }
//!
//! #[get("profile")]
//! async fn profile(&self, user: CurrentUser) -> User { user.0 }
//! ```

use async_trait::async_trait;

use super::context::ExecutionContext;
use crate::di::ModuleRef;
use crate::http::{HttpException, InternalServerErrorException, Request};

#[async_trait]
pub trait FromContext: Sized + Send {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException>;
}

#[async_trait]
impl FromContext for Request {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        Ok(ctx.request().clone())
    }
}

/// `@Res()` / `@Res({ passthrough: true })` (declare the parameter with `#[res]`).
#[async_trait]
impl FromContext for crate::http::Res {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        ctx.request().get::<crate::http::Res>().ok_or_else(|| {
            InternalServerErrorException::new("The response object is only available to handlers that declare it with #[res]")
        })
    }
}

#[async_trait]
impl FromContext for ExecutionContext {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        Ok(ctx.clone())
    }
}

#[async_trait]
impl FromContext for ModuleRef {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        ctx.module_ref().cloned().ok_or_else(|| InternalServerErrorException::new("ModuleRef not available"))
    }
}

#[async_trait]
impl<T: FromContext> FromContext for Option<T> {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        Ok(T::from_context(ctx).await.ok())
    }
}
