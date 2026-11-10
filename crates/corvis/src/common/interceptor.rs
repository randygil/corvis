//! Interceptors (`CorvisInterceptor`) and the `CallHandler`.

use std::time::Duration;

use async_trait::async_trait;
use futures::future::BoxFuture;

use super::context::ExecutionContext;
use crate::http::{HandlerResult, RequestTimeoutException};

/// Interceptors can run logic before / after the handler, transform the result,
/// transform exceptions, or completely override the handler.
///
/// ```ignore
/// #[injectable]
/// pub struct TransformInterceptor;
///
/// #[async_trait]
/// impl CorvisInterceptor for TransformInterceptor {
///     async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
///         Ok(next.handle().await?.map_data(|data| json!({ "data": data })))
///     }
/// }
/// ```
#[async_trait]
pub trait CorvisInterceptor: Send + Sync + 'static {
    async fn intercept(&self, context: &ExecutionContext, next: CallHandler) -> HandlerResult;
}

/// Gives access to the response stream (`next.handle()`).
pub struct CallHandler {
    next: Box<dyn FnOnce() -> BoxFuture<'static, HandlerResult> + Send>,
}

impl CallHandler {
    pub fn new(next: impl FnOnce() -> BoxFuture<'static, HandlerResult> + Send + 'static) -> Self {
        CallHandler { next: Box::new(next) }
    }

    /// Invokes the route handler (and the remaining interceptors).
    pub async fn handle(self) -> HandlerResult {
        (self.next)().await
    }
}

/// Built-in interceptor that throws `RequestTimeoutException` when the handler takes too long.
#[derive(Debug, Clone, Copy)]
pub struct TimeoutInterceptor {
    pub duration: Duration,
}

impl TimeoutInterceptor {
    pub fn new(duration: Duration) -> Self {
        TimeoutInterceptor { duration }
    }
    pub fn millis(ms: u64) -> Self {
        TimeoutInterceptor { duration: Duration::from_millis(ms) }
    }
}

impl Default for TimeoutInterceptor {
    fn default() -> Self {
        TimeoutInterceptor { duration: Duration::from_secs(5) }
    }
}

#[async_trait]
impl CorvisInterceptor for TimeoutInterceptor {
    async fn intercept(&self, _context: &ExecutionContext, next: CallHandler) -> HandlerResult {
        match tokio::time::timeout(self.duration, next.handle()).await {
            Ok(r) => r,
            Err(_) => Err(RequestTimeoutException::empty()),
        }
    }
}

/// Built-in interceptor that logs every request with its duration
/// (`[Router] GET /cats 200 +3ms`).
#[derive(Debug, Clone, Copy, Default)]
pub struct LoggingInterceptor;

#[async_trait]
impl CorvisInterceptor for LoggingInterceptor {
    async fn intercept(&self, context: &ExecutionContext, next: CallHandler) -> HandlerResult {
        let start = std::time::Instant::now();
        // Not built when the level is off: the URL of a request is client controlled and can be long.
        let line = if !crate::logger::level_enabled(crate::logger::LogLevel::Log) {
            String::new()
        } else if context.get_type() == super::context::ContextType::Http {
            // In a gateway the request is the handshake, whose URL may carry a token in its query: log
            // the handler instead.
            let req = context.request();
            // The URL with the values of the parameters marked as credentials hidden.
            format!("{} {}", req.method(), req.redacted_url())
        } else {
            format!("WS {}.{}", context.get_class().name, context.get_handler().name)
        };
        let result = next.handle().await;
        let status = match &result {
            Ok(r) => r.status().map(|s| s.as_u16()).unwrap_or(200),
            Err(e) => e.get_status(),
        };
        crate::logger::Logger::new("LoggingInterceptor").log(format!("{line} {status} - {}ms", start.elapsed().as_millis()));
        result
    }
}
