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

use std::sync::Arc;

use super::context::{ContextType, ExecutionContext};
use crate::di::{DiError, ModuleRef};
use crate::http::exception::HttpExceptionError;
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

/// A provider resolved for the request being handled, as a handler parameter.
///
/// `Scoped<T>` resolves the provider registered under `Token::of::<T>()` from the module of the route's controller,
/// as [`ModuleRef::resolve_for`] does, and dereferences to `T`. It works for a provider of any scope (request-scoped,
/// transient or singleton), so changing a provider's scope does not change its consumers. Within one request every
/// `Scoped` of a request-scoped provider is the same instance.
///
/// ```ignore
/// #[injectable(scope = "request")]
/// pub struct RequestContext { req: Request }
///
/// #[get("me")]
/// async fn me(&self, ctx: Scoped<RequestContext>) -> String {
///     ctx.request_id()
/// }
/// ```
///
/// A guard, an interceptor or a route-level exception filter takes it from its context:
///
/// ```ignore
/// async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
///     let request_ctx = Scoped::<RequestContext>::from_context(ctx).await?;
///     Ok(request_ctx.is_allowed())
/// }
/// ```
///
/// Errors: the `HttpException` of a factory provider that fails with one is returned unchanged; any other failure is a
/// `500` with the default body, and the `DiError` is logged under `Scoped`. In a WebSocket context it always fails
/// (request scope is HTTP only).
pub struct Scoped<T: ?Sized>(Arc<T>);

impl<T: ?Sized> Scoped<T> {
    /// The shared instance.
    pub fn into_inner(self) -> Arc<T> {
        self.0
    }
}

impl<T: ?Sized> std::ops::Deref for Scoped<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: ?Sized + std::fmt::Debug> std::fmt::Debug for Scoped<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Scoped").field(&&*self.0).finish()
    }
}

#[async_trait]
impl<T: ?Sized + Send + Sync + 'static> FromContext for Scoped<T> {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        let name = crate::di::token::short_type_name(std::any::type_name::<T>());
        let logger = crate::logger::Logger::new("Scoped");
        if ctx.get_type() != ContextType::Http {
            logger.error(format!("Scoped<{name}> is not available here: request scope is HTTP only"));
            return Err(InternalServerErrorException::empty());
        }
        let Some(module_ref) = ctx.module_ref() else {
            let error = DiError::OutsideRequest { token: name.clone(), chain: String::new() };
            logger.error(format!("Scoped<{name}> could not be resolved: {error}"));
            return Err(InternalServerErrorException::empty());
        };
        match module_ref.resolve_for::<T>(ctx.request()).await {
            Ok(instance) => Ok(Scoped(instance)),
            Err(DiError::Factory { cause, .. }) if cause.downcast_ref::<HttpExceptionError>().is_some() => {
                Err(cause.downcast_ref::<HttpExceptionError>().map(|e| e.0.clone()).expect("checked by the guard"))
            }
            Err(error) => {
                logger.error(format!("Scoped<{name}> could not be resolved: {error}"));
                Err(InternalServerErrorException::empty())
            }
        }
    }
}

#[async_trait]
impl<T: FromContext> FromContext for Option<T> {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        Ok(T::from_context(ctx).await.ok())
    }
}
