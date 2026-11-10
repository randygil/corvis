use corvis::auth::local::{LocalValidator, PasswordHasher};
use corvis::prelude::*;
use serde::Serialize;

use crate::common::decorators::Role;
use crate::users::UsersService;

/// The user a successful login hands to the route: no password hash.
#[derive(Debug, Clone, Serialize)]
pub struct LoginUser {
    pub user_id: u32,
    pub username: String,
    pub roles: Vec<Role>,
}

/// The validator of the `local` strategy (`LocalStrategy` of `passport-local`). It verifies the password against the
/// stored argon2id hash, and for an unknown username it verifies against a dummy hash, so the response time does not
/// say which usernames exist.
#[injectable]
pub struct LocalAuth {
    users_service: Arc<UsersService>,
    hasher: Arc<PasswordHasher>,
}

#[async_trait]
impl LocalValidator for LocalAuth {
    type User = LoginUser;

    async fn validate(&self, _req: &Request, username: &str, password: &str) -> Result<Option<LoginUser>, HttpException> {
        let user = self.users_service.find_one(username).await;
        let ok = self
            .hasher
            .verify_or_dummy(password, user.as_ref().map(|u| u.password_hash.as_str()))
            .await
            .map_err(|_| InternalServerErrorException::empty())?;
        Ok(user.filter(|_| ok).map(|u| LoginUser { user_id: u.user_id, username: u.username, roles: u.roles }))
    }
}
