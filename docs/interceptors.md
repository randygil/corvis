---
status: documented
---

# Interceptors

An interceptor wraps the handler's execution. It can do work before and after, transform the response, transform or catch errors, or respond without ever calling the handler. It is the tool for cross-cutting concerns such as measuring time, wrapping responses in a common format, caching, or applying timeouts.

## The `CorvisInterceptor` trait

```rust
#[async_trait]
pub trait CorvisInterceptor: Send + Sync + 'static {
    async fn intercept(&self, context: &ExecutionContext, next: CallHandler) -> HandlerResult;
}
```

- `context` is the request's [execution context](/fundamentals/execution-context).
- `next` represents the rest of the chain: the following interceptors, the pipes, and the handler. `next.handle().await` runs them and returns a `HandlerResult` (`Result<Response, HttpException>`). `handle` consumes `next`, so it can only be called once.
- What `intercept` returns is what continues on its way to the client (or to the outer interceptor).

::: info No reactive streams
In Corvis `next.handle()` returns a future, not an observable. You can transform, time, cache, and apply timeouts, but not compose operators over a stream.
:::

## Before and after

```rust
use std::time::Instant;

use corvis::prelude::*;

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
```

(`examples/cats-app/src/common/interceptors.rs`)

## Transforming the response

`Response::map_data` applies a function to the response body. JSON bodies arrive as they are, text ones as a JSON string, and empty ones as `null`; binary or streaming ones are left untouched.

```rust
#[injectable]
pub struct TransformInterceptor;

#[async_trait]
impl CorvisInterceptor for TransformInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        Ok(next.handle().await?.map_data(|data| json!({ "data": data })))
    }
}
```

With this interceptor, a handler that returns `[1, 2, 3]` responds `{"data":[1,2,3]}`.

Besides `map_data`, on the `Response` you have `json_body()` to read the JSON body, `status()`, `set_status(..)`, `set_header(..)`, `headers()`, and `set_body(..)`.

## Transforming errors

The error also goes through the interceptor, so you can replace it:

```rust
#[injectable]
pub struct ErrorsInterceptor;

#[async_trait]
impl CorvisInterceptor for ErrorsInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        next.handle().await.map_err(|e| {
            if e.is_internal() { BadGatewayException::empty() } else { e }
        })
    }
}
```

## Responding without calling the handler

If the interceptor doesn't call `next.handle()`, the handler doesn't run. This is the basis of a cache:

```rust
#[async_trait]
impl CorvisInterceptor for CachedInterceptor {
    async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        let key = ctx.request().url();
        if let Some(body) = self.store.get(&key) {
            return Ok(Response::json(body));
        }
        let res = next.handle().await?;
        if let Some(body) = res.json_body() {
            self.store.insert(key, body.clone());
        }
        Ok(res)
    }
}
```

Corvis already includes a complete `CacheInterceptor` with per-route TTL (see [Caching](/data/caching)).

## Built-in interceptors

| Interceptor | Use |
|---|---|
| `TimeoutInterceptor::new(Duration)` / `TimeoutInterceptor::millis(ms)` | Responds `408 Request Timeout` if the handler takes longer; `Default` is 5 s |
| `LoggingInterceptor` | Logs `GET /cats 200 - 3ms` with the `LoggingInterceptor` context |
| `CacheInterceptor` | Caches `GET` responses (requires `CacheModule`) |
| `z::ZodSerializerInterceptor` | Validates and shapes the output with a schema (see [Serialization](/application/serialization)) |
| `FileInterceptor`, `FilesInterceptor`, ... | Read `multipart` bodies (see [File upload](/http/file-upload)) |

```rust
app.use_global_interceptors(TimeoutInterceptor::millis(10_000));
```

## Applying interceptors

| Scope | Form |
|---|---|
| One route | `#[use_interceptors(TransformInterceptor)]` on the method |
| One controller | `#[use_interceptors(TimingInterceptor)]` below `#[controller]` |
| Global, instance | `app.use_global_interceptors(LoggingInterceptor)` |
| Global, with DI | `Provider::app_interceptor::<MyInterceptor>()` in `providers` |

```rust
#[controller("cats")]
#[use_interceptors(TimingInterceptor)]
pub struct CatsController {
    cats_service: Arc<CatsService>,
}

#[routes]
impl CatsController {
    #[get]
    #[use_interceptors(TransformInterceptor)]
    fn find_all(&self, #[query] query: ListCatsQuery) -> Vec<Cat> {
        self.cats_service.find_all(&query)
    }
}
```

As with guards and pipes, in the attribute you can pass a type (resolved with DI in the controller's module) or an expression (`#[use_interceptors(TimeoutInterceptor::millis(500))]`).

### Order

Interceptors wrap one another: the first in the list is the outermost.

1. Globals registered as a provider (`app_interceptor`).
2. Globals from `use_global_interceptors`.
3. The controller's.
4. The route's.

The "before" code runs in that order and the "after" code in reverse order. Interceptors run after guards and before pipes, so a pipe's validation error also goes through them.

## Reading the route's status

Inside an interceptor, `ctx.switch_to_http().status_code()` returns the status the response will have if the handler doesn't change it: the one from `#[http_code]`, `201` on `POST`, or `200`. It is useful, for example, to store `{ status, body }` in a cache.
