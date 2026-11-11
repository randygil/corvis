use corvis::JwtService;
use corvis::auth::jwt::{ExtractJwt, JwtAlgorithm, JwtKey, JwtStrategyOptions, JwtValidator};
use corvis::prelude::*;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;
use crate::users::{Account, Role};

/// What the access token carries. `exp` is added when it is signed: the strategy refuses a token without one.
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: u32,
    username: String,
    roles: Vec<Role>,
}

/// The validator of the `jwt` strategy (`passport-jwt`): bearer tokens signed with `JWT_SECRET`.
///
/// `validate` receives the payload of a token whose signature, algorithm and expiry were already checked. It returns
/// `None`, never a user that would serialize to `null`, `false`, `0` or `""`, for a payload it does not understand.
#[injectable]
pub struct JwtAuth {
    settings: Arc<Settings>,
}

#[async_trait]
impl JwtValidator for JwtAuth {
    type User = Account;

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
            .secret_or_key(JwtKey::secret(self.settings.jwt_secret.clone()))
            .algorithms([JwtAlgorithm::HS256])
    }

    async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Account>, HttpException> {
        // A token is a statement the application signed earlier: a user deleted since keeps a valid token until it
        // expires, which is why access tokens are short-lived.
        let claims: Option<Claims> = corvis::serde_json::from_value(payload).ok();
        Ok(claims.map(|c| Account { id: c.sub, username: c.username, roles: c.roles }))
    }
}

/// Signs the access token of a user the `local` strategy authenticated. `JWT_EXPIRES_IN` of the module sets `exp`.
pub fn issue_token(jwt: &JwtService, account: &Account) -> HttpResult<String> {
    let claims = Claims { sub: account.id, username: account.username.clone(), roles: account.roles.clone() };
    jwt.sign(&claims).map_err(HttpException::from_anyhow)
}
