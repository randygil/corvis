use std::time::Instant;

use corvis::prelude::*;

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
