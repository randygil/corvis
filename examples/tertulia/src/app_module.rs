use corvis::prelude::*;
use corvis::{JwtModule, JwtModuleOptions};
use corvis_orm::{OrmModule, OrmModuleOptions};

use crate::auth::AuthModule;
use crate::chat::ChatModule;
use crate::models::{ChatMessage, Room};

#[module(
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
        EventEmitterModule::for_root(EventEmitterOptions::default()),
        JwtModule::register_async(true, |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            Ok(JwtModuleOptions::new(config.get_or("JWT_SECRET", "tertulia-dev-secret".to_string()))
                .expires_in(config.get_or("JWT_EXPIRES_IN", "12h".to_string())))
        }),
        OrmModule::for_root_async(true, |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let url: String = config.get_or("DATABASE_URL", "file:./tertulia.db".to_string());
            Ok(OrmModuleOptions::new(url).log_queries(config.get_or("LOG_QUERIES", false)).push::<(Room, ChatMessage)>())
        }),
        AuthModule,
        ChatModule,
    ],
)]
pub struct AppModule;
