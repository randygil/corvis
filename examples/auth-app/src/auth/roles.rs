use corvis::prelude::*;

use crate::users::{Account, ROLES_KEY, Role};

/// Role based access control: a route marked with `#[set_metadata(ROLES_KEY, [Role::Admin])]` is for those users only.
///
/// It reads the typed user an `AuthGuard` stored (`Request::typed_user`, no JSON to deserialize), so it is listed
/// **after** one: `#[use_guards(AuthGuard::..., RolesGuard)]`. Without an `AuthGuard` before it, the typed user is
/// whatever the session middleware restored from the cookie, which nothing about the request itself has checked.
#[injectable]
pub struct RolesGuard {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let required: Option<Vec<Role>> = self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
        let Some(required) = required else { return Ok(true) };
        let user = ctx.switch_to_http().get_request().typed_user::<Account>();
        Ok(user.is_some_and(|u| required.iter().any(|r| u.roles.contains(r))))
    }

    /// It lets a request through when the route declares no roles, so it must not count as access control for the
    /// route posture check: `AuthGuard` is the guard that decides who is authenticated.
    fn controls_access(&self) -> bool {
        false
    }
}
