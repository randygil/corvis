use corvis::prelude::*;

use super::auth_service::AuthService;
use super::sign_in_dto::SignInDto;
use crate::common::decorators::{CurrentUser, IS_PUBLIC_KEY, JwtPayload};

#[controller("auth")]
pub struct AuthController {
    auth_service: Arc<AuthService>,
}

#[routes]
impl AuthController {
    #[post("login")]
    #[http_code(HttpStatus::OK)]
    #[set_metadata(IS_PUBLIC_KEY, true)]
    async fn sign_in(&self, #[body] dto: SignInDto) -> HttpResult<Value> {
        self.auth_service.sign_in(&dto.username, &dto.password).await
    }

    #[get("profile")]
    fn get_profile(&self, user: CurrentUser) -> JwtPayload {
        user.0
    }
}
