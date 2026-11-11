//! Authentication with every strategy of `corvis::auth` in one small application: local login with hashed passwords, JWT
//! access tokens, an API key for machine clients, sessions, and an OAuth2 login against a bundled mock provider.
//!
//! `create_app(settings)` builds the application the binary runs and the tests drive. See the README for the routes.

pub mod app_module;
pub mod auth;
pub mod controllers;
pub mod mock_provider;
pub mod same_origin;
pub mod settings;
pub mod users;

use corvis::prelude::*;

pub use app_module::AppModule;
pub use settings::Settings;

/// Creates the application. A route with no guard that controls access and no `#[public]` mark stops `init()`.
pub async fn create_app(settings: Settings) -> corvis::Result<CorvisApplication> {
    // The modules read the settings while the application is built: `SettingsModule` hands them out.
    let mut app = settings::current::SETTINGS.scope(settings, CorvisFactory::create::<AppModule>()).await?;
    app.require_route_posture();
    Ok(app)
}
