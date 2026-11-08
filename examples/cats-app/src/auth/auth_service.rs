use corvis::JwtService;
use corvis::prelude::*;

use super::sign_in_dto::TokenDto;
use crate::common::decorators::JwtPayload;
use crate::users::UsersService;

#[injectable]
pub struct AuthService {
    users_service: Arc<UsersService>,
    jwt_service: Arc<JwtService>,
}

impl AuthService {
    pub async fn sign_in(&self, username: &str, pass: &str) -> HttpResult<TokenDto> {
        let user = self.users_service.find_one(username).await;
        let Some(user) = user.filter(|u| u.password == pass) else {
            return Err(UnauthorizedException.into());
        };
        let payload = JwtPayload { sub: user.user_id, username: user.username, roles: user.roles };
        let token = self.jwt_service.sign(&payload).map_err(HttpException::from_anyhow)?;
        Ok(TokenDto { access_token: token })
    }
}
