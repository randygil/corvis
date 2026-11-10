use corvis::auth::{AuthGuard as PassportGuard, User};
use corvis::prelude::*;
use corvis_swagger::*;

use super::auth_service::AuthService;
use super::local_auth::LoginUser;
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
    #[use_guards(PassportGuard::new("local"))]
    // The global throttler allows 100 requests per minute; password guessing gets 5 per minute and client.
    #[apply_decorators(throttle("default").limit(5).ttl(60_000))]
    // The guard has already checked the credentials; the body parameter validates and documents the shape.
    fn sign_in(&self, #[body] _dto: SignInDto, user: User<LoginUser>) -> HttpResult<TokenDto> {
        self.auth_service.sign_in(user.0)
    }

    #[get("profile")]
    #[api_bearer_auth]
    fn get_profile(&self, user: CurrentUser) -> JwtPayload {
        user.0
    }
}
