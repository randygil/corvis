use corvis::prelude::*;
use corvis_swagger::*;

use super::auth_service::AuthService;
use super::sign_in_dto::{SignInDto, TokenDto};
use crate::common::decorators::{CurrentUser, JwtPayload};

#[controller("auth")]
#[api_tags("auth")]
pub struct AuthController {
    auth_service: Arc<AuthService>,
}

#[routes]
impl AuthController {
    #[post("login")]
    #[http_code(HttpStatus::OK)]
    #[public]
    // The global throttler allows 100 requests per minute; password guessing gets 5 per minute and client.
    #[apply_decorators(throttle("default").limit(5).ttl(60_000))]
    async fn sign_in(&self, #[body] dto: SignInDto) -> HttpResult<TokenDto> {
        self.auth_service.sign_in(&dto.username, &dto.password).await
    }

    #[get("profile")]
    #[api_bearer_auth]
    fn get_profile(&self, user: CurrentUser) -> JwtPayload {
        user.0
    }
}
