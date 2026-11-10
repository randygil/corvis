use corvis::auth::api_key::ApiKeyStrategy;
use corvis::auth::jwt::JwtStrategy;
use corvis::auth::local::{LocalStrategy, PasswordHasher};
use corvis::auth::session::SessionStrategy;
use corvis::prelude::*;
use corvis::{JwtModule, JwtModuleOptions};

use super::auth_controller::AuthController;
use super::auth_service::AuthService;
use super::jwt_auth::JwtAuth;
use super::local_auth::LocalAuth;
use super::service_key_auth::{ServiceKeyAuth, ServiceKeys};
use super::session_auth::MemberSerializer;
use super::session_controller::SessionController;
use crate::users::UsersModule;

#[module(
    imports = [
        UsersModule,
        JwtModule::register_async(true, |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            // There is no default secret: a well-known one would let anyone forge tokens.
            let secret = config.get::<String>("JWT_SECRET").filter(|s| !s.trim().is_empty());
            let secret = secret.ok_or_else(|| corvis::anyhow::anyhow!("JWT_SECRET is required (set it in the environment or in .env)"))?;
            Ok(JwtModuleOptions::new(secret).expires_in(config.get_or("JWT_EXPIRES_IN", "1h".to_string())))
        }),
    ],
    controllers = [AuthController, SessionController],
    providers = [
        AuthService,
        JwtAuth,
        Provider::strategy::<JwtStrategy<JwtAuth>>(),
        Provider::class::<PasswordHasher>(),
        LocalAuth,
        Provider::strategy::<LocalStrategy<LocalAuth>>(),
        ServiceKeys::provider(),
        ServiceKeyAuth,
        Provider::strategy::<ApiKeyStrategy<ServiceKeyAuth>>(),
        MemberSerializer,
        Provider::strategy::<SessionStrategy<MemberSerializer>>(),
    ],
    exports = [AuthService],
)]
pub struct AuthModule;
