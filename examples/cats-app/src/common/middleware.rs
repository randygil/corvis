use std::time::Instant;

use corvis::prelude::*;

/// Logs every request: `GET /api/v1/cats 200 - 3ms`.
#[injectable]
pub struct LoggerMiddleware {
    #[init(Logger::new("HTTP"))]
    logger: Logger,
}

#[async_trait]
impl CorvisMiddleware for LoggerMiddleware {
    async fn use_(&self, req: Request, next: Next) -> HandlerResult {
        let start = Instant::now();
        // `redacted_url` hides the query parameters a strategy reads credentials from (the OAuth callback's `code` and `state`).
        let line = format!("{} {}", req.method(), req.redacted_url());
        let res = next.run(req).await;
        let status = match &res {
            Ok(r) => r.status_code(),
            Err(e) => e.get_status(),
        };
        self.logger.log(format!("{line} {status} - {}ms", start.elapsed().as_millis()));
        res
    }
}
