use corvis::prelude::*;
use corvis::{JwtService, extract_bearer_token};

use crate::common::decorators::JwtPayload;

/// Global JWT guard; routes marked with `#[public]` are skipped.
#[injectable]
pub struct AuthGuard {
    jwt_service: Arc<JwtService>,
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let is_public: bool = self.reflector.get_all_and_override(PUBLIC_METADATA, &[ctx.get_handler(), ctx.get_class()]).unwrap_or(false);
        if is_public {
            return Ok(true);
        }
        let request = ctx.switch_to_http().get_request();
        let token = extract_bearer_token(request).ok_or(UnauthorizedException)?;
        let payload: JwtPayload = self.jwt_service.verify_or_unauthorized(&token)?;
        // 💡 We're assigning the payload to the request object here
        // so that we can access it in our route handlers.
        request.set_user(&payload);
        Ok(true)
    }
}
