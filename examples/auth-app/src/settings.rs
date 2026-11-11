//! The configuration of the application: four secrets, the OAuth2 client and the `--dev` switch.
//!
//! There are no default secrets. `Settings::from_env()` fails when one is missing or shorter than 32 bytes, and the
//! error names the variable, never its value. `Settings::dev(port)` generates the secrets for one run and starts the
//! bundled mock OAuth2 provider on `127.0.0.1`.

use std::fmt;
use std::sync::Arc;

use corvis::prelude::*;
use rand::RngCore;

use crate::mock_provider::MockProvider;

/// The shortest secret the framework accepts for an HMAC key, a signed cookie or an API key.
pub const MIN_SECRET_BYTES: usize = 32;

/// The application's OAuth2 client: where the provider is and who this application is for it.
#[derive(Clone)]
pub struct OAuthClient {
    pub authorize_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub client_id: String,
    pub client_secret: String,
    /// The absolute URL of `GET /auth/provider/callback`, as registered with the provider.
    pub redirect_uri: String,
}

#[derive(Clone)]
pub struct Settings {
    pub jwt_secret: String,
    pub session_secret: String,
    /// The API key of the services that call `GET /reports`.
    pub service_api_key: String,
    /// Signs the cookie that holds the state of an OAuth2 login. Not the session secret: the two cookies must not
    /// verify each other's values.
    pub oauth_state_secret: String,
    pub oauth: OAuthClient,
    /// `--dev`: plain-HTTP cookies and the mock provider. Never true in a deployment.
    pub dev: bool,
    mock: Option<Arc<MockProvider>>,
}

// Secrets never reach a log through `{:?}`.
impl fmt::Debug for Settings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Settings").field("dev", &self.dev).field("oauth_authorize_url", &self.oauth.authorize_url).finish_non_exhaustive()
    }
}

impl Settings {
    /// Reads the process environment. See `.env.example` for the names.
    pub fn from_env() -> corvis::Result<Settings> {
        Settings::from_lookup(|name| std::env::var(name).ok())
    }

    /// The same, from any source of variables (the tests use a map instead of the process environment).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> corvis::Result<Settings> {
        let secret = |name: &str| -> corvis::Result<String> {
            let value = get(name).filter(|v| !v.is_empty()).ok_or_else(|| corvis::anyhow::anyhow!("{name} is required"))?;
            if value.len() < MIN_SECRET_BYTES {
                // The message holds the length, never the value.
                corvis::anyhow::bail!(
                    "{name} must have at least {MIN_SECRET_BYTES} bytes, it has {} (generate one with `openssl rand -hex 32`)",
                    value.len()
                );
            }
            Ok(value)
        };
        let plain = |name: &str| -> corvis::Result<String> {
            get(name).filter(|v| !v.is_empty()).ok_or_else(|| corvis::anyhow::anyhow!("{name} is required"))
        };
        let secrets = [
            ("JWT_SECRET", secret("JWT_SECRET")?),
            ("SESSION_SECRET", secret("SESSION_SECRET")?),
            ("SERVICE_API_KEY", secret("SERVICE_API_KEY")?),
            ("OAUTH_STATE_SECRET", secret("OAUTH_STATE_SECRET")?),
        ];
        // One secret per purpose: with the same value for the token key and the service key, whoever holds the key can sign
        // tokens for any user. The error names the variables, never the value.
        for (i, (name, value)) in secrets.iter().enumerate() {
            if let Some((other, _)) = secrets[..i].iter().find(|(_, v)| v == value) {
                corvis::anyhow::bail!("{name} and {other} have the same value: each secret must be different");
            }
        }
        let [(_, jwt_secret), (_, session_secret), (_, service_api_key), (_, oauth_state_secret)] = secrets;
        Ok(Settings {
            jwt_secret,
            session_secret,
            service_api_key,
            oauth_state_secret,
            oauth: OAuthClient {
                authorize_url: plain("OAUTH_AUTHORIZE_URL")?,
                token_url: plain("OAUTH_TOKEN_URL")?,
                userinfo_url: plain("OAUTH_USERINFO_URL")?,
                client_id: plain("OAUTH_CLIENT_ID")?,
                client_secret: plain("OAUTH_CLIENT_SECRET")?,
                redirect_uri: plain("OAUTH_REDIRECT_URI")?,
            },
            dev: false,
            mock: None,
        })
    }

    /// `--dev`: secrets generated for this run, the mock provider started on a free loopback port and pointed at, and
    /// the callback registered for the application on `127.0.0.1:{port}`. The mock provider stops when the last clone
    /// of the settings is dropped.
    pub async fn dev(port: u16) -> corvis::Result<Settings> {
        let mock = MockProvider::start().await?;
        let oauth = mock.client(format!("http://127.0.0.1:{port}/auth/provider/callback"));
        Ok(Settings {
            jwt_secret: random_secret(),
            session_secret: random_secret(),
            service_api_key: random_secret(),
            oauth_state_secret: random_secret(),
            oauth,
            dev: true,
            mock: Some(Arc::new(mock)),
        })
    }

    /// The mock provider of a `--dev` run.
    pub fn mock(&self) -> Option<&MockProvider> {
        self.mock.as_deref()
    }
}

/// 32 random bytes as hex, from the operating system's generator.
pub fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The settings of the application being created: `create_app` sets them while the modules are built, and the
/// provider below hands them to everything that asks for `Settings`.
pub(crate) mod current {
    use super::Settings;

    corvis::tokio::task_local! {
        pub static SETTINGS: Settings;
    }
}

/// A global module that provides `Settings` to every other module.
#[module]
pub struct SettingsModule;

impl SettingsModule {
    pub fn module() -> DynamicModule {
        DynamicModule::new::<SettingsModule>()
            .provider(Provider::use_factory(Token::of::<Settings>(), |_ctx| async move {
                current::SETTINGS
                    .try_with(|settings| settings.clone())
                    .map_err(|_| corvis::anyhow::anyhow!("no Settings: build the application with create_app"))
            }))
            .export(Token::of::<Settings>())
            .global(true)
    }
}
