use corvis::auth::AuthGuard as JwtGuard;
use corvis::prelude::*;

/// Global guard: authenticates with the `jwt` strategy ([`super::JwtAuth`]) and lets `#[public]` routes through.
///
/// It wraps `corvis::auth::AuthGuard` to keep the chat gateway public: the framework's guard denies every WebSocket
/// message, while this one skips public targets (gateway handlers included) before anything else.
#[injectable]
pub struct AuthGuard {
    reflector: Arc<Reflector>,
    #[init(JwtGuard::new("jwt"))]
    jwt: JwtGuard,
}

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let is_public: bool = self.reflector.get_all_and_override(PUBLIC_METADATA, &[ctx.get_handler(), ctx.get_class()]).unwrap_or(false);
        if is_public {
            return Ok(true);
        }
        // The strategy stores the payload as the request's user, so that route handlers can read it.
        self.jwt.can_activate(ctx).await
    }
}
