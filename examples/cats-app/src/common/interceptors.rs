use std::time::Instant;

use corvis::prelude::*;

use super::request_context::RequestContext;

/// Sets the `X-Request-Id` response header from the request-scoped [`RequestContext`]. It is a singleton, so it takes
/// the context per request with `Scoped` instead of injecting it.
#[injectable]
pub struct RequestIdInterceptor;

#[async_trait]
impl CorvisInterceptor for RequestIdInterceptor {
    async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        let request_context = Scoped::<RequestContext>::from_context(ctx).await?;
        let mut response = next.handle().await?;
        response.set_header("X-Request-Id", request_context.request_id());
        Ok(response)
    }
}

/// Wraps every response in `{ "data": ... }` (the `TransformInterceptor` from the docs).
#[injectable]
pub struct TransformInterceptor;

#[async_trait]
impl CorvisInterceptor for TransformInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        Ok(next.handle().await?.map_data(|data| json!({ "data": data })))
    }
}

/// Logs `Before...` / `After... Xms` (the `LoggingInterceptor` from the docs).
#[injectable]
pub struct TimingInterceptor;

#[async_trait]
impl CorvisInterceptor for TimingInterceptor {
    async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        let logger = Logger::new("TimingInterceptor");
        logger.debug(format!("Before {}.{}", ctx.get_class().name, ctx.get_handler().name));
        let now = Instant::now();
        let res = next.handle().await;
        logger.debug(format!("After... {}ms", now.elapsed().as_millis()));
        res
    }
}
