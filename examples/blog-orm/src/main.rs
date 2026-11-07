//! Blog API with corvis + corvis-orm (SQLite).
//!
//! ```text
//! cargo run -p blog-orm
//! curl -X POST http://127.0.0.1:3000/users -H 'content-type: application/json' -d '{"email":"ana@corvis.dev","name":"Ana"}'
//! curl 'http://127.0.0.1:3000/posts?search=hola&page=1&limit=10'
//! ```
//!
//! Environment: `DATABASE_URL` (default `file:./dev.db`), `PORT` (default 3000),
//! `LOG_QUERIES=true` to print every SQL query.

mod app_module;
mod models;
mod posts;
mod users;

use corvis::prelude::*;
use corvis_orm::OrmExceptionFilter;

use app_module::AppModule;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    // DTOs carry zod-like schemas (`zod_dto!`), validated by `ZodValidationPipe`.
    app.use_global_pipes(ZodValidationPipe::auto());
    // P2002 -> 409, P2025 -> 404, validation -> 400.
    app.use_global_filters(OrmExceptionFilter::new());
    app.enable_shutdown_hooks();

    let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 3000);
    // Always bind to the loopback interface.
    app.listen(format!("127.0.0.1:{port}")).await
}
