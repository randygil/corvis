//! ```text
//! cargo run -p tertulia
//! # open http://127.0.0.1:5150
//! ```
//!
//! Environment: `PORT` (default 5150), `DATABASE_URL` (default `file:./tertulia.db`),
//! `JWT_SECRET` (required), `MODERATORS` (default `admin`), `STATS_INTERVAL_MS` (default 2000).
//!
//! The login is a password-less demo: do not expose this example as is.

use corvis::prelude::*;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = tertulia::create_app().await?;
    app.enable_shutdown_hooks();
    let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 5150);
    app.listen(format!("127.0.0.1:{port}")).await
}
