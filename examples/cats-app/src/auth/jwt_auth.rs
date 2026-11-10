use corvis::auth::jwt::{ExtractJwt, JwtAlgorithm, JwtKey, JwtStrategyOptions, JwtValidator};
use corvis::prelude::*;

use crate::common::decorators::JwtPayload;

/// The `jwt` strategy (`class JwtStrategy extends PassportStrategy(Strategy)`): bearer tokens signed with `JWT_SECRET`
/// (the secret `AuthService` signs with). `validate` turns the verified payload into the user of the request.
#[injectable]
pub struct JwtAuth {
    config: Arc<ConfigService>,
}

#[async_trait]
impl JwtValidator for JwtAuth {
    type User = JwtPayload;

    fn options(&self) -> JwtStrategyOptions {
        // `JwtModule` refuses to start without `JWT_SECRET` before this runs; an empty value would not start either.
        let secret = self.config.get::<String>("JWT_SECRET").unwrap_or_default();
        JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
            .secret_or_key(JwtKey::secret(secret))
            .algorithms([JwtAlgorithm::HS256])
    }

    async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<JwtPayload>, HttpException> {
        Ok(corvis::serde_json::from_value(payload).ok())
    }
}
