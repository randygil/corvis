use corvis::auth::api_key::ApiKeyStrategy;
use corvis::auth::jwt::JwtStrategy;
use corvis::auth::local::{LocalStrategy, LocalStrategyOptions, PasswordHasher, login_tracker};
use corvis::auth::oauth2::OAuth2Strategy;
use corvis::auth::session::SessionStrategy;
use corvis::prelude::*;
use corvis::{JwtModule, JwtModuleOptions, ThrottlerStorage};

use crate::auth::api_key::{ServiceKeyAuth, ServiceKeys};
use crate::auth::jwt::JwtAuth;
use crate::auth::local::LocalAuth;
use crate::auth::provider::ProviderAuth;
use crate::auth::roles::RolesGuard;
use crate::auth::session::{UserSerializer, session_module};
use crate::controllers::{AuthController, HomeController, ProviderController, SessionController};
use crate::settings::{Settings, SettingsModule};
use crate::users::UsersService;

/// Two limits for the login routes: the `default` throttler counts per client address, and `login` counts per account
/// (`login_tracker` reads the username the `local` strategy will read). The routes pick their own numbers.
fn throttler() -> DynamicModule {
    ThrottlerModule::for_root_storage(
        ThrottlerStorage::new(vec![
            // Per client, a minute. The shorter window is checked first, so a client over its limit does not add to the
            // account counter.
            ThrottlerOptions::new(60_000, 100),
            // Per account, five minutes.
            ThrottlerOptions { name: "login".into(), ttl: 300_000, limit: 100 },
        ])
        .with_named_tracker("login-account", login_tracker(LocalStrategyOptions::default())),
    )
}

#[module(
    imports = [
        SettingsModule::module(),
        throttler(),
        session_module(),
        JwtModule::register_async(true, |ctx| async move {
            let settings = ctx.get::<Settings>().await?;
            // Access tokens are short-lived: a stolen one works until `exp`.
            Ok(JwtModuleOptions::new(settings.jwt_secret.clone()).expires_in("15m"))
        }),
    ],
    controllers = [HomeController, AuthController, SessionController, ProviderController],
    providers = [
        // A global guard registered through the container runs before every `AuthGuard`.
        Provider::app_guard::<ThrottlerGuard>(),
        Provider::use_factory(Token::of::<UsersService>(), |_ctx| async move { Ok(UsersService::seeded()) }),
        Provider::class::<PasswordHasher>(),
        RolesGuard,
        // jwt
        JwtAuth,
        Provider::strategy::<JwtStrategy<JwtAuth>>(),
        // local
        LocalAuth,
        Provider::strategy::<LocalStrategy<LocalAuth>>(),
        // api-key
        ServiceKeys::provider(),
        ServiceKeyAuth,
        Provider::strategy::<ApiKeyStrategy<ServiceKeyAuth>>(),
        // session
        UserSerializer,
        Provider::strategy::<SessionStrategy<UserSerializer>>(),
        // provider (OAuth2)
        ProviderAuth,
        Provider::strategy::<OAuth2Strategy<ProviderAuth>>(),
    ],
)]
pub struct AppModule;
