use corvis::prelude::*;

use super::decorators::{JwtPayload, ROLES_KEY, Role};

/// Role based access control (`RolesGuard`).
#[injectable]
pub struct RolesGuard {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let required: Option<Vec<Role>> = self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
        let Some(required) = required else { return Ok(true) };
        let user = ctx.switch_to_http().get_request().user_as::<JwtPayload>();
        Ok(user.map(|u| required.iter().any(|r| u.roles.contains(r))).unwrap_or(false))
    }

    /// It lets every request through when the route declares no roles, so it must not count as access control
    /// for the route posture check (`AuthGuard` is the guard that decides who may call a route).
    fn controls_access(&self) -> bool {
        false
    }
}
