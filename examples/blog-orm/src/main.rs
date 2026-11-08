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

use corvis::prelude::*;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = blog_orm::create_app().await?;
    app.enable_shutdown_hooks();

    let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 3000);
    // Always bind to the loopback interface.
    app.listen(format!("127.0.0.1:{port}")).await
}
