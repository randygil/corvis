use corvis::prelude::*;

use super::auth_guard::AuthGuard;
use super::auth_service::AuthService;
use super::dto::LoginDto;
use super::user::ChatUser;

#[controller("auth")]
pub struct AuthController {
    auth_service: Arc<AuthService>,
}

#[routes]
impl AuthController {
    /// `POST /api/auth/login { "username": "ana" }` -> `{ accessToken, user }`.
    #[post("login")]
    #[http_code(200)]
    fn login(&self, #[body] dto: LoginDto) -> HttpResult<Value> {
        self.auth_service.login(&dto.username)
    }

    #[get("me")]
    #[use_guards(AuthGuard)]
    fn me(&self, user: ChatUser) -> ChatUser {
        user
    }
}
