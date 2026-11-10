use corvis::auth::oauth2;
use corvis::auth::{AuthGuard as PassportGuard, User};
use corvis::prelude::*;
use corvis_swagger::*;

use super::auth_service::AuthService;
use super::local_auth::LoginUser;

/// "Sign in with GitHub": `login` sends the browser to GitHub, `callback` is where GitHub sends it back. Both routes are
/// `#[public]` and carry the guard of the `github` strategy, which checks the `state` and the PKCE verifier before
/// anything is sent to GitHub.
#[controller("auth/github")]
#[api_tags("auth")]
pub struct GithubController {
    auth_service: Arc<AuthService>,
}

#[routes]
impl GithubController {
    /// Redirects to GitHub. `?return_to=/cats` (a relative path) comes back after the callback.
    #[get("login")]
    #[public]
    #[use_guards(PassportGuard::new("github"))]
    // Every login start is a request a visitor can make: the same limit as the other logins.
    #[apply_decorators(throttle("default").limit(10).ttl(60_000))]
    fn login(&self) -> Value {
        // The guard answers with the redirect; the handler never runs.
        json!({})
    }

    /// The callback: the account is found or created by `(provider, id)` and the response carries the application's token,
    /// as the other login does. A browser application would redirect to `return_to` with the token in a cookie of its own;
    /// a token in a redirect URL would end up in logs and history.
    #[get("callback")]
    #[public]
    #[use_guards(PassportGuard::new("github"))]
    #[apply_decorators(throttle("default").limit(10).ttl(60_000))]
    fn callback(&self, req: Request, user: User<LoginUser>) -> HttpResult<Value> {
        let token = self.auth_service.sign_in(user.0)?;
        Ok(json!({ "access_token": token.access_token, "return_to": oauth2::return_to(&req) }))
    }
}
