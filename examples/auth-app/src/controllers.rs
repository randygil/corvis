//! The routes. There is no global `AuthGuard`: every route states its own guard, and `require_route_posture()` stops the
//! application at startup when one has none and is not marked `#[public]`.

use corvis::JwtService;
use corvis::auth::{AuthGuard, Authenticator, User};
use corvis::prelude::*;

use crate::auth::jwt::issue_token;
use crate::auth::roles::RolesGuard;
use crate::same_origin::SameOriginGuard;
use crate::users::{Account, ROLES_KEY, Role, UsersService};

#[controller]
pub struct HomeController {
    users: Arc<UsersService>,
}

#[routes]
impl HomeController {
    /// A public route: `#[public]` is the mark route posture accepts for a route with no guard.
    #[get]
    #[public]
    fn index(&self) -> Value {
        json!({ "name": "auth-app", "login": "POST /auth/login", "docs": "see README.md" })
    }

    /// Optional authentication: everyone gets an answer, and a user gets a personal one. An optional guard is not
    /// access control, so the route is marked `#[public]` too. The chain accepts either credential.
    #[get("feed")]
    #[public]
    #[use_guards(AuthGuard::chain(["jwt", "session"]).optional())]
    fn feed(&self, req: Request) -> Value {
        match req.user_as::<Account>() {
            Some(user) => json!({ "message": format!("Hello, {}", user.username) }),
            None => json!({ "message": "Hello, stranger" }),
        }
    }

    /// The token route: a bearer token, and the user as a typed parameter.
    #[get("me")]
    #[use_guards(AuthGuard::new("jwt"))]
    fn me(&self, user: User<Account>) -> Account {
        user.0
    }

    /// A route for people and for machines: a user's token, or the service key in `X-Api-Key`. The first strategy that
    /// succeeds decides who the caller is.
    #[get("reports")]
    #[use_guards(AuthGuard::chain(["jwt", "api-key"]))]
    fn reports(&self, req: Request) -> Value {
        json!({ "caller": req.user() })
    }

    /// Authorization after authentication: `RolesGuard` is listed after the `AuthGuard` whose user it reads.
    #[get("admin/users")]
    #[set_metadata(ROLES_KEY, [Role::Admin])]
    #[use_guards(AuthGuard::chain(["jwt", "session"]), RolesGuard)]
    fn admin_users(&self) -> Vec<Account> {
        self.users.accounts()
    }
}

#[controller("auth")]
pub struct AuthController {
    jwt: Arc<JwtService>,
}

#[routes]
impl AuthController {
    /// Local login for a token: the guard checks the credentials, the handler signs an access token with an expiry.
    // Per client address (10 a minute) and per account from any address (5 every five minutes). `ThrottlerGuard` is a
    // global guard (`Provider::app_guard`), so it runs before the route's `AuthGuard`: a throttled request costs no
    // password hash.
    #[post("login")]
    #[http_code(HttpStatus::OK)]
    #[apply_decorators(throttle("default").limit(10), throttle("login").limit(5).tracker("login-account"))]
    #[use_guards(AuthGuard::new("local"))]
    fn login(&self, user: User<Account>) -> HttpResult<Value> {
        Ok(json!({ "access_token": issue_token(&self.jwt, &user)? }))
    }
}

/// Login with a session cookie instead of a token: the same `local` strategy, then the server keeps the user in a
/// session.
#[controller("session")]
pub struct SessionController {
    authenticator: Arc<Authenticator>,
}

#[routes]
impl SessionController {
    /// `SameOriginGuard` runs first: `SameSite=Lax` does not stop login CSRF (a cross-site form `POST` is answered and
    /// its cookie stored), so a request that comes from another site is refused before the credentials are checked.
    #[post("login")]
    #[http_code(HttpStatus::OK)]
    #[apply_decorators(throttle("default").limit(10), throttle("login").limit(5).tracker("login-account"))]
    #[use_guards(SameOriginGuard::new(), AuthGuard::new("local").log_in())]
    fn login(&self, user: User<Account>) -> Value {
        json!({ "username": user.username })
    }

    #[get("me")]
    #[use_guards(AuthGuard::new("session"))]
    fn me(&self, user: User<Account>) -> Account {
        user.0
    }

    /// Ends the session: the record is destroyed and the old cookie stops working.
    #[post("logout")]
    #[http_code(HttpStatus::NO_CONTENT)]
    #[use_guards(SameOriginGuard::new(), AuthGuard::new("session"))]
    async fn logout(&self, req: Request) -> HttpResult<()> {
        self.authenticator.log_out(&req).await
    }
}

/// "Sign in with a provider". Both routes run the same guard: the one that starts a login and the callback must
/// authenticate the same way, so a login started by one user cannot be completed as another (this matters most when
/// the flow links an account to a signed-in user).
#[controller("auth/provider")]
pub struct ProviderController;

#[routes]
impl ProviderController {
    /// Redirects to the provider. The guard answers with the redirect; the handler never runs.
    #[get]
    #[apply_decorators(throttle("default").limit(10))]
    #[use_guards(AuthGuard::new("provider").log_in())]
    fn start(&self) -> Value {
        json!({})
    }

    /// Where the provider sends the browser back. The login is made by the guard, which creates the session.
    #[get("callback")]
    #[apply_decorators(throttle("default").limit(10))]
    #[use_guards(AuthGuard::new("provider").log_in())]
    fn callback(&self, user: User<Account>) -> Value {
        json!({ "username": user.username })
    }
}
