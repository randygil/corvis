use corvis::JwtService;
use corvis::prelude::*;

use super::local_auth::LoginUser;
use super::sign_in_dto::TokenDto;
use crate::common::decorators::JwtPayload;

#[injectable]
pub struct AuthService {
    jwt_service: Arc<JwtService>,
}

impl AuthService {
    /// Signs the access token of a user the `local` strategy authenticated.
    pub fn sign_in(&self, user: LoginUser) -> HttpResult<TokenDto> {
        let payload = JwtPayload { sub: user.user_id, username: user.username, roles: user.roles };
        let token = self.jwt_service.sign(&payload).map_err(HttpException::from_anyhow)?;
        Ok(TokenDto { access_token: token })
    }
}
