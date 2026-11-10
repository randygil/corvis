use corvis::prelude::*;

use crate::app_controller::AppController;
use crate::app_service::AppService;
use crate::auth::{AuthGuard, AuthModule, session_module};
use crate::cats::CatsModule;
use crate::chat::ChatGateway;
use crate::common::guards::RolesGuard;
use crate::common::middleware::LoggerMiddleware;
use crate::tasks::TasksService;
use crate::users::UsersModule;

#[module(
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
        EventEmitterModule::for_root(EventEmitterOptions::default()),
        ScheduleModule::for_root(),
        CacheModule::register(CacheOptions { is_global: true, ..Default::default() }),
        ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 100)]),
        TerminusModule,
        session_module(),
        AuthModule,
        UsersModule,
        CatsModule,
    ],
    controllers = [AppController],
    providers = [
        AppService,
        TasksService,
        ChatGateway,
        // Global guards with dependency injection (APP_GUARD).
        Provider::app_guard::<ThrottlerGuard>(),
        Provider::app_guard::<AuthGuard>(),
        Provider::app_guard::<RolesGuard>(),
    ],
)]
pub struct AppModule;

impl CorvisModule for AppModule {
    fn configure(&self, consumer: &mut MiddlewareConsumer) {
        consumer.apply::<LoggerMiddleware>().for_routes("*");
    }
}
