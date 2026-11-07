use corvis::prelude::*;
use corvis_orm::{OrmModule, OrmModuleOptions};

use crate::models::{Post, User};
use crate::posts::PostsModule;
use crate::users::UsersModule;

#[module(
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
        // Async options: the URL comes from `ConfigService`.
        OrmModule::for_root_async(true, |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let url: String = config.get_or("DATABASE_URL", "file:./dev.db".to_string());
            Ok(OrmModuleOptions::new(url)
                .log_queries(config.get_or("LOG_QUERIES", false))
                // Synchronize the tables on startup (`db push`, a development convenience).
                .push::<(User, Post)>())
        }),
        UsersModule,
        PostsModule,
    ],
)]
pub struct AppModule;
