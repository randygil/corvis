use corvis::prelude::*;
use corvis::{ContextType, JwtService, WsException, extract_bearer_token};

use super::user::ChatUser;

/// Metadata read by [`RolesGuard`]: `#[set_metadata(ROLES_KEY, ["moderator"])]`.
pub const ROLES_KEY: &str = "roles";

/// One guard for both transports:
/// - HTTP: verifies `Authorization: Bearer <jwt>` and attaches the [`ChatUser`] to the request.
/// - WebSocket: the gateway authenticated the socket on connect; the guard only checks that
///   the client carries a user (a socket rejected on connect never reaches a handler).
#[injectable]
pub struct AuthGuard {
    jwt: Arc<JwtService>,
}

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        if ctx.get_type() == ContextType::Ws {
            return match ChatUser::of(ctx) {
                Some(_) => Ok(true),
                None => Err(WsException::new("Unauthorized")),
            };
        }
        let request = ctx.switch_to_http().get_request();
        let token = extract_bearer_token(request).ok_or_else(|| UnauthorizedException::new("Missing bearer token"))?;
        let user: ChatUser = self.jwt.verify_or_unauthorized(&token)?;
        request.insert(user);
        Ok(true)
    }
}

/// `@Roles('moderator')` for HTTP routes and `#[subscribe_message]` handlers alike.
#[injectable]
pub struct RolesGuard {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let roles: Option<Vec<String>> = self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
        let Some(roles) = roles else { return Ok(true) };
        Ok(ChatUser::of(ctx).is_some_and(|u| roles.contains(&u.role)))
    }
}
