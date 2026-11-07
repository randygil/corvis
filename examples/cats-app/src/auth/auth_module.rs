use corvis::prelude::*;
use corvis::{JwtModule, JwtModuleOptions};

use super::auth_controller::AuthController;
use super::auth_service::AuthService;
use crate::users::UsersModule;

#[module(
    imports = [
        UsersModule,
        JwtModule::register_async(true, |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            Ok(JwtModuleOptions::new(config.get_or("JWT_SECRET", "change-me".to_string()))
                .expires_in(config.get_or("JWT_EXPIRES_IN", "1h".to_string())))
        }),
    ],
    controllers = [AuthController],
    providers = [AuthService],
    exports = [AuthService],
)]
pub struct AuthModule;
