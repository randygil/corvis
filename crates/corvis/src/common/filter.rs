//! Exception filters (`ExceptionFilter`) and the default `BaseExceptionFilter`.

use async_trait::async_trait;

use super::context::ArgumentsHost;
use crate::http::{HttpException, Response};
use crate::logger::Logger;

/// Exception filters let you take full control of the exceptions layer.
///
/// `catches()` plays the role of `@Catch(...)`: return `true` for the exceptions
/// this filter handles (default: every exception, like `@Catch()`).
///
/// ```ignore
/// #[injectable]
/// pub struct HttpExceptionFilter;
///
/// #[async_trait]
/// impl ExceptionFilter for HttpExceptionFilter {
///     async fn catch(&self, exception: HttpException, host: &ArgumentsHost) -> Response {
///         let req = host.switch_to_http().get_request();
///         Response::json(json!({
///             "statusCode": exception.get_status(),
///             "timestamp": chrono::Utc::now().to_rfc3339(),
///             "path": req.url(),
///         }))
///         .with_status(exception.get_status())
///     }
/// }
/// ```
#[async_trait]
pub trait ExceptionFilter: Send + Sync + 'static {
    fn catches(&self, _exception: &HttpException) -> bool {
        true
    }

    async fn catch(&self, exception: HttpException, host: &ArgumentsHost) -> Response;
}

/// The built-in global exception filter. Extend it by delegating to it
/// (the equivalent of `super.catch(exception, host)`).
#[derive(Debug, Default, Clone, Copy)]
pub struct BaseExceptionFilter;

impl BaseExceptionFilter {
    pub fn handle(exception: &HttpException) -> Response {
        if exception.is_internal() {
            let logger = Logger::new("ExceptionsHandler");
            let mut msg = exception.message().to_string();
            if let Some(cause) = exception.cause() {
                let mut src = cause.source();
                while let Some(s) = src {
                    msg.push_str(&format!("\n    caused by: {s}"));
                    src = s.source();
                }
            }
            logger.error(msg);
        }
        Response::json(exception.get_response().clone()).with_status(exception.status())
    }
}

#[async_trait]
impl ExceptionFilter for BaseExceptionFilter {
    async fn catch(&self, exception: HttpException, _host: &ArgumentsHost) -> Response {
        Self::handle(&exception)
    }
}
