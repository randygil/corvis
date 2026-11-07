mod app_controller;
mod app_module;
mod app_service;
mod auth;
mod cats;
mod chat;
mod common;
mod tasks;
mod users;

use corvis::prelude::*;

use app_module::AppModule;
use common::filters::HttpExceptionFilter;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.set_global_prefix("api");
    app.use_global_pipes(ValidationPipe::new().whitelist(true).forbid_non_whitelisted(true));
    app.use_global_filters(HttpExceptionFilter);
    app.enable_cors(CorsOptions::default());
    app.enable_versioning(VersioningOptions::uri().default_version("1"));
    app.enable_shutdown_hooks();

    let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 3000);
    app.listen(port).await
}
