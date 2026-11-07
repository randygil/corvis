use std::time::Instant;

use corvis::prelude::*;

use crate::auth::ChatUser;

/// Logs every chat message: who sent it, the handler, the outcome and how long it took.
#[injectable]
pub struct WsLoggingInterceptor {
    #[init(Logger::new("ChatGateway"))]
    logger: Logger,
}

#[async_trait]
impl CorvisInterceptor for WsLoggingInterceptor {
    async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        let started = Instant::now();
        let user = ChatUser::of(ctx).map(|u| u.username).unwrap_or_else(|| "-".to_string());
        let handler = ctx.get_handler().name.clone();
        let result = next.handle().await;
        let outcome = if result.is_ok() { "ok" } else { "error" };
        self.logger.verbose(format!("{user} -> {handler}: {outcome} ({} µs)", started.elapsed().as_micros()));
        result
    }
}
