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

    /// `true` (default) when this guard decides who may call the handler. Return `false` for guards that only
    /// limit or validate requests (rate limiting, payload validation), or that let every request through when
    /// they are not configured: they do not count for route posture
    /// ([`CorvisApplication::require_route_posture`](crate::CorvisApplication::require_route_posture)).
    /// It does not change how the guard runs.
    fn controls_access(&self) -> bool {
        true
    }

    /// Name shown in route posture errors and reports (default: the type name).
    fn guard_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// The guard as [`Any`](std::any::Any), so startup checks can look inside it. Only guards that opt in return
    /// `Some(self)`: [`AuthGuard`](crate::auth::AuthGuard) does, so the application can verify its strategy
    /// names when it initializes.
    ///
    /// A guard that wraps another one should not return the inner guard here (that would also make the
    /// [`AuthGuard`](crate::auth::AuthGuard) gateway warning fire for a wrapper that skips public targets); it
    /// lists it in [`inner_guards`](CanActivate::inner_guards) instead.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// The guards this one delegates to. The startup check of strategy names looks inside them, so a composite
    /// guard that wraps a [`corvis::auth::AuthGuard`](crate::auth::AuthGuard) fails at startup on a name that no
    /// strategy has, instead of answering 500 on every request. Default: none.
    fn inner_guards(&self) -> Vec<&dyn CanActivate> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct OpenGuard;

    #[async_trait]
    impl CanActivate for OpenGuard {
        async fn can_activate(&self, _: &ExecutionContext) -> Result<bool, HttpException> {
            Ok(true)
        }
    }

    #[test]
    fn a_guard_controls_access_by_default_and_reports_its_type_name() {
        let guard: Arc<dyn CanActivate> = Arc::new(OpenGuard);
        assert!(guard.controls_access());
        assert!(guard.guard_name().ends_with("OpenGuard"));
    }

    #[test]
    fn zod_guard_does_not_control_access() {
        let guard: Arc<dyn CanActivate> = Arc::new(crate::z::ZodGuard::new(crate::z::ZodGuardSource::Body, crate::z::string()));
        assert!(!guard.controls_access());
        assert!(guard.guard_name().ends_with("ZodGuard"));
    }
}
