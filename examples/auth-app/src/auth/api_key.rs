use corvis::auth::api_key::{ApiKey, ApiKeyValidator, ExpectedApiKey};
use corvis::prelude::*;
use serde::Serialize;

use crate::settings::Settings;

/// The API key of the services that call the machine routes. Only its digest is kept in memory.
pub struct ServiceKeys {
    expected: ExpectedApiKey,
}

impl ServiceKeys {
    /// Builds the key from the settings. A key shorter than 32 bytes stops the application, and the message never
    /// repeats it.
    pub fn provider() -> Provider {
        Provider::use_factory(Token::of::<ServiceKeys>(), |ctx| async move {
            let settings = ctx.get::<Settings>().await?;
            let expected = ExpectedApiKey::new(settings.service_api_key.clone())
                .map_err(|e| corvis::anyhow::anyhow!("SERVICE_API_KEY is not usable: {e}"))?;
            Ok(ServiceKeys { expected })
        })
    }
}

/// Who a request with the service key is.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceCaller {
    pub service: &'static str,
}

/// The validator of the `api-key` strategy (`passport-headerapikey`): the key arrives in the `X-Api-Key` header and
/// is compared as a digest in constant time, never with `==`.
#[injectable]
pub struct ServiceKeyAuth {
    keys: Arc<ServiceKeys>,
}

#[async_trait]
impl ApiKeyValidator for ServiceKeyAuth {
    type User = ServiceCaller;

    async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<ServiceCaller>, HttpException> {
        // `then_some` answers `None` for a wrong key: a validator that returned `Some(false)` would say "found".
        Ok(self.keys.expected.matches(key).then_some(ServiceCaller { service: "reports" }))
    }
}
