use corvis::auth::jwt::JwtStrategy;
use corvis::prelude::*;
use corvis::{JwtModule, JwtModuleOptions};

use super::auth_controller::AuthController;
use super::auth_service::AuthService;
use super::jwt_auth::JwtAuth;
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
    controllers = [AuthController],
    providers = [AuthService, JwtAuth, Provider::strategy::<JwtStrategy<JwtAuth>>()],
    exports = [AuthService],
)]
pub struct AuthModule;
