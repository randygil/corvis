//! Tertulia: a multi-room real-time chat that exercises Corvis' WebSocket support.
//!
//! - `/chat`: a `#[websocket_gateway]` with rooms, presence, typing indicators, JWT auth on
//!   connect, guards, zod-validated payloads, an interceptor, acknowledgements and
//!   `WsException` errors.
//! - `/api/...`: REST endpoints that read the live WebSocket state (members, online users)
//!   or push to it (announcements, new rooms).
//! - `/api/stats/stream`: Server-Sent Events, for contrast.
//! - `/ws/firehose`: a plain `corvis::ws` WebSocket (no `{ event, data }` protocol) that
//!   streams every chat message to moderators.
//! - `/`: a small browser client (`public/`).

pub mod app_module;
pub mod auth;
pub mod chat;
pub mod firehose;
pub mod models;
pub mod rooms;
pub mod stats;

use corvis::prelude::*;
use corvis_orm::OrmExceptionFilter;

pub use app_module::AppModule;

/// Directory of the browser client.
pub const PUBLIC_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/public");

/// Creates the application with the same configuration as `main` (the tests use it too).
pub async fn create_app() -> corvis::Result<CorvisApplication> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.set_global_prefix("api");
    // DTOs carry zod schemas (`zod_dto!`). Global pipes do not apply to gateways: the chat
    // gateway declares its own `#[use_pipes(ZodValidationPipe::auto())]`.
    app.use_global_pipes(ZodValidationPipe::auto());
    app.use_global_filters(OrmExceptionFilter::new());
    app.use_middleware(corvis::serve_static(PUBLIC_DIR));
    Ok(app)
}
