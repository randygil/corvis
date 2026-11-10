//! API key authentication: [`ApiKeyStrategy`] reads a key from the places the application configures and hands it to
//! an [`ApiKeyValidator`], with the helpers to compare keys in constant time and keep them out of logs. Needs the
//! `auth-api-key` cargo feature.
//!
//! ```ignore
//! #[injectable]
//! pub struct ServiceKeyAuth { expected: Arc<ExpectedKeys> }
//!
//! #[async_trait]
//! impl ApiKeyValidator for ServiceKeyAuth {
//!     type User = Service;
//!
//!     async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<Service>, HttpException> {
//!         Ok(self.expected.lookup(key).cloned())
//!     }
//! }
//!
//! #[module(providers = [Provider::class::<ServiceKeyAuth>(), Provider::strategy::<ApiKeyStrategy<ServiceKeyAuth>>()])]
//! pub struct MachineModule;
//!
//! #[get("data")]
//! #[use_guards(AuthGuard::new("api-key"))]
//! fn data(&self, service: User<Service>) -> Value { ... }
//! ```
//!
//! What is stricter than `passport-headerapikey` 1.2.2, on purpose (the docs list each one):
//!
//! - the key is read only from the configured sources, in the configuration order and all of them; the query string
//!   is a source only when configured, with a startup warning, and its parameter is hidden in the URL that
//!   `LoggingInterceptor` logs;
//! - a prefix is a literal string, not a regular expression;
//! - two sources that carry different keys fail the attempt, and so does a source whose value is present but
//!   unusable (too long, control characters, a repeated query parameter): no source can be made to look empty so
//!   that another one decides;
//! - keys longer than `max_key_bytes` (512) or with control characters never reach the validator;
//! - a validator that fails with a status of 500 or above is answered with a body no key can influence, and one that
//!   runs longer than `validator_timeout` is a 503.

mod key;
mod options;
mod strategy;

pub use key::{ApiKey, ApiKeyConfigError, ApiKeySet, ExpectedApiKey, MAX_SECRET_BYTES, MIN_SECRET_BYTES, constant_time_eq};
pub use options::{
    ApiKeySource, ApiKeyStrategyOptions, DEFAULT_MAX_KEY_BYTES, DEFAULT_VALIDATOR_TIMEOUT, MAX_KEY_BYTES_LIMIT, MAX_SOURCES,
};
pub use strategy::{ApiKeyStrategy, ApiKeyValidator};
