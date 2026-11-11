//! What the strategies share when they run an application's callback: the deadline, the one `503` of the module and
//! the outcome of a validator's answer.

use crate::http::HttpException;

/// The `503 Service Unavailable` every part of the authentication module answers with, the same bytes for every cause:
/// `{"statusCode":503,"message":"Service Unavailable"}`. It is the plain body the API key strategy answers for any
/// status of 500 or more, so every `503` of the module has its form.
pub(crate) fn service_unavailable() -> HttpException {
    HttpException::new((), http::StatusCode::SERVICE_UNAVAILABLE)
}

#[cfg(any(feature = "jwt", feature = "auth-local", feature = "auth-api-key"))]
pub(crate) use with_validator::{user_outcome, with_deadline};

#[cfg(any(feature = "jwt", feature = "auth-local", feature = "auth-api-key"))]
mod with_validator {
    use std::future::Future;
    use std::time::Duration;

    use serde::Serialize;
    use serde_json::Value;

    use super::service_unavailable;
    use crate::auth::{AuthenticatedUser, StrategyOutcome};
    use crate::http::HttpException;

    /// Runs `future` within `timeout`. Past it the future is dropped (which cancels it), `warn` writes the strategy's
    /// rate-limited warning, and the outcome is the `503` for every caller.
    pub(crate) async fn with_deadline<T>(
        timeout: Duration,
        future: impl Future<Output = T>,
        warn: impl FnOnce(),
    ) -> Result<T, StrategyOutcome> {
        match tokio::time::timeout(timeout, future).await {
            Ok(value) => Ok(value),
            Err(_) => {
                warn();
                Err(StrategyOutcome::Error(service_unavailable()))
            }
        }
    }

    /// The outcome of a validator's answer: an exception is passed on as it is, `None` is a failure without a challenge,
    /// and a user is a success with the auth info `info` gives for it (a user that does not serialize is an error).
    pub(crate) fn user_outcome<U: Serialize + Clone + Send + Sync + 'static>(
        result: Result<Option<U>, HttpException>,
        info: impl FnOnce(&U) -> Option<Value>,
    ) -> StrategyOutcome {
        match result {
            Err(e) => StrategyOutcome::Error(e),
            Ok(None) => StrategyOutcome::Fail { challenge: None, status: None },
            Ok(Some(user)) => {
                let info = info(&user);
                match AuthenticatedUser::new(user) {
                    Ok(user) => StrategyOutcome::Success { user, info },
                    Err(e) => StrategyOutcome::Error(e),
                }
            }
        }
    }
}
