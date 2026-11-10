use corvis::auth::api_key::{ApiKey, ApiKeyValidator, ExpectedApiKey};
use corvis::prelude::*;
use serde::Serialize;

/// The API key of the services that call the machine routes, read once at startup.
pub struct ServiceKeys {
    expected: Option<ExpectedApiKey>,
}

impl ServiceKeys {
    /// Builds the key from `SERVICE_API_KEY`. The variable is optional: without it no key works and the machine route
    /// answers 401 to everyone. A value shorter than 32 bytes stops the application, and the message never repeats it.
    pub fn provider() -> Provider {
        Provider::use_factory(Token::of::<ServiceKeys>(), |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let secret = config.get::<String>("SERVICE_API_KEY").filter(|s| !s.is_empty());
            let expected = secret
                .map(|s| {
                    ExpectedApiKey::new(s).map_err(|e| {
                        corvis::anyhow::anyhow!("SERVICE_API_KEY is not usable: {e} (generate one with `openssl rand -hex 32`)")
                    })
                })
                .transpose()?;
            Ok(ServiceKeys { expected })
        })
    }
}

/// Who a request with a valid service key is.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceCaller {
    pub service: &'static str,
}

/// The validator of the `api-key` strategy (`ApiKeyStrategy` of the framework): the key arrives in the `X-Api-Key`
/// header and is compared with the configured one in constant time. With no key configured, the comparison runs
/// against a dummy digest and always fails.
#[injectable]
pub struct ServiceKeyAuth {
    keys: Arc<ServiceKeys>,
}

#[async_trait]
impl ApiKeyValidator for ServiceKeyAuth {
    type User = ServiceCaller;

    async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<ServiceCaller>, HttpException> {
        Ok(ExpectedApiKey::matches_or_dummy(self.keys.expected.as_ref(), key).then_some(ServiceCaller { service: "reports" }))
    }
}
