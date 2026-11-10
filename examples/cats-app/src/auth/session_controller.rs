use corvis::auth::{AuthGuard as PassportGuard, Authenticator, User};
use corvis::prelude::*;
use corvis_swagger::*;

use super::local_auth::LoginUser;
use super::session_auth::Member;
use super::sign_in_dto::SignInDto;

/// Login with a session cookie instead of a token: the same `local` strategy, then the server keeps the user in a
/// session. The global guard authenticates with a JWT, so these routes are `#[public]` and carry their own guard.
#[controller("session")]
#[api_tags("session")]
pub struct SessionController {
    authenticator: Arc<Authenticator>,
}

#[routes]
impl SessionController {
    /// Checks the credentials, then logs in to the session: the response sets a new session cookie.
    #[post("login")]
    #[http_code(HttpStatus::OK)]
    #[public]
    #[use_guards(PassportGuard::new("local").log_in())]
    // Every login creates a session in the store: the same limit as the token login.
    #[apply_decorators(throttle("default").limit(5).ttl(60_000))]
    fn login(&self, #[body] _dto: SignInDto, user: User<LoginUser>) -> Value {
        json!({ "username": user.username })
    }

    /// The user of the session, or the plain 401 for a request without one.
    #[get("me")]
    #[public]
    #[use_guards(PassportGuard::new("session"))]
    fn me(&self, user: User<Member>) -> Member {
        user.0
    }

    /// Ends the session: the record is destroyed and the old cookie stops working.
    #[post("logout")]
    #[http_code(HttpStatus::NO_CONTENT)]
    #[public]
    #[use_guards(PassportGuard::new("session"))]
    async fn logout(&self, req: Request) -> HttpResult<()> {
        self.authenticator.log_out(&req).await
    }
}
