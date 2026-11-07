---
status: unavailable
---

# Resilience

Corvis does not include resilience utilities such as retries with exponential backoff, circuit breakers or bulkheads.

## Alternative

- **Maximum time per request**: `corvis::TimeoutInterceptor` cuts the handler off when it exceeds a deadline and responds `408 Request Timeout`. The default is 5 seconds; `TimeoutInterceptor::millis(2_000)` or `TimeoutInterceptor::new(Duration)` change it. It is applied with `#[use_interceptors(...)]` or as a global interceptor (see [Interceptors](/interceptors)).
- **Server timeouts**: `ServerOptions` limits how long to wait for the headers and the body of each request (see [Keep-alive connections](/faq/keep-alive-connections)).
- **Retries**: write them in your service with `tokio::time::sleep` or with a Rust retry crate; if the operation is a database call, combine it with a [corvis-orm](/data/corvis-orm) transaction.
