//! Blog API with corvis + corvis-orm (SQLite).
//!
//! The application is created by [`create_app`], shared by `main` and the tests.

pub mod app_module;
pub mod models;
pub mod posts;
pub mod users;

use corvis::prelude::*;
use corvis_orm::OrmExceptionFilter;

pub use app_module::AppModule;

/// Creates the application with the same configuration as `main` (the tests use it too).
pub async fn create_app() -> corvis::Result<CorvisApplication> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    // DTOs carry zod-like schemas (`zod_dto!`), validated by `ZodValidationPipe`.
    app.use_global_pipes(ZodValidationPipe::auto());
    // P2002 -> 409, P2025 -> 404, validation -> 400.
    app.use_global_filters(OrmExceptionFilter::new());
    Ok(app)
}
