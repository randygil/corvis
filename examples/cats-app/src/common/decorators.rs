//! Custom decorators: metadata keys (used with `#[set_metadata]`) and param decorators. Public routes use the
//! built-in `#[public]` marker.

use corvis::prelude::*;
use serde::{Deserialize, Serialize};

/// `export const Roles = (...roles: Role[]) => SetMetadata(ROLES_KEY, roles);`
/// Usage: `#[set_metadata(ROLES_KEY, [Role::Admin])]`.
pub const ROLES_KEY: &str = "roles";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Admin,
}

/// The JWT payload attached to the request by `AuthGuard` (`req.user`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtPayload {
    pub sub: u32,
    pub username: String,
    pub roles: Vec<Role>,
}

/// `@CurrentUser()` param decorator (`createParamDecorator`).
pub struct CurrentUser(pub JwtPayload);

#[async_trait]
impl FromContext for CurrentUser {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        ctx.request().user_as::<JwtPayload>().map(CurrentUser).ok_or_else(|| UnauthorizedException.into())
    }
}
