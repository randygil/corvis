---
status: partial
---

# Overview

Observing an application in production means being able to answer what it is doing, how fast, and why it fails. Corvis does not ship an observability platform (tracing SDK, metrics, dashboard or APM provider integration). What it does offer are the basic pieces to build your own on top of:

| Need | What Corvis has |
|---|---|
| Structured logs | `ConsoleLogger::new().json(true)` writes one JSON line per entry; `LoggerService` lets you send the logs wherever you want (see [Logging](/application/logger)) |
| Duration of each request | `corvis::LoggingInterceptor` records method, URL, status and milliseconds; or your own [interceptor](/interceptors) that measures and publishes the metric |
| Unhandled errors | A global [exception filter](/exception-filters) sees every error before a response is sent; `exception.is_internal()` tells unexpected errors apart |
| Instance health | `TerminusModule` and `HealthCheckService` (see [Health checks](/reliability/terminus)) |

The other pages in this section describe features that do not exist in Corvis yet and, where applicable, how to cover the need with these pieces.

## Example: a metrics interceptor

```rust
use std::time::Instant;

#[injectable]
pub struct MetricsInterceptor {
    #[init(Logger::new("Metrics"))]
    logger: Logger,
}

#[async_trait]
impl CorvisInterceptor for MetricsInterceptor {
    async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        let started = Instant::now();
        let route = format!("{}.{}", ctx.get_class().name, ctx.get_handler().name);
        let result = next.handle().await;
        let status = match &result {
            Ok(r) => r.status().map(|s| s.as_u16()).unwrap_or(200),
            Err(e) => e.get_status(),
        };
        self.logger.log(format!("{route} {status} {}us", started.elapsed().as_micros()));
        result
    }
}
```

Register it as global with `Provider::app_interceptor::<MetricsInterceptor>()` in the root module. Instead of writing a log, you can publish the value to the metrics system you use.
