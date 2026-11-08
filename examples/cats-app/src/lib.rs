//! Cats API: a tour of the framework (modules, guards, pipes, interceptors, events, SSE,
//! WebSockets, versioning and an OpenAPI document).
//!
//! The application is created by [`create_app`], shared by `main` and the tests.

pub mod app_controller;
pub mod app_module;
pub mod app_service;
pub mod auth;
pub mod cats;
pub mod chat;
pub mod common;
pub mod tasks;
pub mod users;

use corvis::prelude::*;
use corvis_swagger::{DocumentBuilder, SwaggerModule, cleanup_open_api_doc};

pub use app_module::AppModule;
use common::filters::HttpExceptionFilter;

/// Creates the application with the same configuration as `main` (the tests use it too).
pub async fn create_app() -> corvis::Result<CorvisApplication> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.set_global_prefix("api");
    app.use_global_pipes(ValidationPipe::new().whitelist(true).forbid_non_whitelisted(true));
    app.use_global_filters(HttpExceptionFilter);
    app.enable_cors(CorsOptions::default());
    // Every route needs a guard that controls access or `#[public]` (see docs/security/route-posture.md).
    app.require_route_posture();
    app.enable_versioning(VersioningOptions::uri().default_version("1"));
    // The document reads the routes, so it is built after the prefix and versioning are set.
    // `/docs` and `/docs-json` are mounted outside the global prefix and guards: do not expose
    // them as is in production.
    let config = DocumentBuilder::new().set_title("Cats").set_version("1.0").add_bearer_auth().build();
    let document = cleanup_open_api_doc(SwaggerModule::create_document(&app, &config));
    SwaggerModule::setup("docs", &mut app, document);
    Ok(app)
}
