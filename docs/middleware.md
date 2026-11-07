---
status: documented
---

# Middleware

A middleware is a function that runs **before** the router chooses the handler. It receives the request and a `Next` that represents the rest of the chain: it can inspect or modify the request, cut the chain short by returning its own response, or call `next.run(req)` and work on the response that comes back.

## Class middleware

A middleware with dependencies is an `#[injectable]` that implements `CorvisMiddleware`:

```rust
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
        let line = format!("{} {}", req.method(), req.url());
        let res = next.run(req).await;
        let status = match &res {
            Ok(r) => r.status_code(),
            Err(e) => e.get_status(),
        };
        self.logger.log(format!("{line} {status} - {}ms", start.elapsed().as_millis()));
        res
    }
}
```

(`examples/cats-app/src/common/middleware.rs`)

The signature is:

```rust
#[async_trait]
pub trait CorvisMiddleware: Send + Sync + 'static {
    async fn use_(&self, req: Request, next: Next) -> HandlerResult;
}
```

- `req` arrives by value. If you want to add data for the handlers, use the request extensions: `req.insert(value)` and, later, `req.get::<T>()`.
- `next.run(req).await` runs the next middleware or, at the end, the handler. It returns a `HandlerResult` (`Result<Response, HttpException>`).
- To cut the chain short, don't call `next` and return your own response (`Ok(Response::json(...))`) or an error (`Err(UnauthorizedException::new("..."))`).
- To modify the response, work on what `next.run` returns:

```rust
#[injectable]
pub struct HeaderMiddleware;

#[async_trait]
impl CorvisMiddleware for HeaderMiddleware {
    async fn use_(&self, req: Request, next: Next) -> HandlerResult {
        let mut res = next.run(req).await?;
        res.set_header("x-middleware", "yes");
        Ok(res)
    }
}
```

::: info Difference from Express
There is no mutable `res` object that the middleware writes to before calling `next`: the response is modified on the result of `next.run(req)`. Express's `res` object only exists in handlers (`#[res]`).
:::

## Applying middleware in a module

Class middleware is registered in the `configure` method of the `CorvisModule` trait, implemented on the module:

```rust
#[module(
    imports = [/* ... */],
    controllers = [AppController],
    providers = [AppService],
)]
pub struct AppModule;

impl CorvisModule for AppModule {
    fn configure(&self, consumer: &mut MiddlewareConsumer) {
        consumer.apply::<LoggerMiddleware>().for_routes("*");
    }
}
```

`#[module]` detects the `CorvisModule` implementation on its own; it doesn't need to be declared anywhere else.

The middleware is resolved with dependency injection **in the context of that module**: if it isn't in `providers`, Corvis builds and caches it anyway, but its dependencies must be visible from the module.

### `MiddlewareConsumer`

| Method | Use |
|---|---|
| `apply::<M>()` | A class middleware (with DI) |
| `apply_instance(m)` | An already built instance |
| `apply_fn(\|req, next\| async move { ... })` | A function |

All three return a configuration object with:

| Method | Use |
|---|---|
| `.and::<M>()`, `.and_instance(m)`, `.and_fn(f)` | Adds another middleware to the same rule; they run in the order written |
| `.exclude(routes)` | Routes the rule does not cover |
| `.for_routes(routes)` | Routes the rule covers; closes the rule and returns the consumer so you can chain another |

### Route selection

`for_routes` and `exclude` accept:

- A `&str` or `String` with a route pattern: `"cats"`, `"cats/:id"`, `"cats/*"`, `"*"`.
- `RouteInfo { path: "cats", method: RequestMethod::Get }` or `RouteSpec::path("cats", RequestMethod::Post)`, for a single verb.
- `RouteSpec::controller::<CatsController>()`: all the routes of that controller, with their final paths (global prefix and version included).
- An array or `Vec` of any of the above.

```rust
impl CorvisModule for CatsModule {
    fn configure(&self, consumer: &mut MiddlewareConsumer) {
        consumer
            .apply::<HeaderMiddleware>()
            .for_routes(RouteSpec::controller::<CatsController>());

        consumer
            .apply::<AuditMiddleware>()
            .exclude(RouteInfo { path: "admin/health", method: RequestMethod::Get })
            .for_routes(["admin/*", "reports"]);
    }
}
```

Pattern rules:

- The global prefix (`set_global_prefix`) is added automatically: write `"cats"`, not `"api/cats"`. The patterns `"*"`, `"(.*)"`, `"*path"`, and `"{*path}"` cover any route, with or without a prefix.
- `:name` and `{name}` cover one segment. `*` as a segment covers zero or more segments.
- A `*` inside a segment (`"admin*"`) covers the rest of the path, like `/admin(.*)` in Express: `/api/admin`, `/api/ADMINpanel`, and `/api/admin/users/1` all match.
- Comparison is case-insensitive, the same rule the router uses, so a middleware can't be skipped by changing the capitalization of the URL.

::: warning URI versioning
Text patterns do not include the version segment (`/v1`). If the application uses URI versioning, cover the routes with `RouteSpec::controller::<...>()` or with a wildcard.
:::

## Functional middleware

When the middleware doesn't need dependencies, a function is enough:

```rust
consumer
    .apply_fn(|req: Request, next: Next| async move {
        corvis::Logger::new("Request").log(format!("{} {}", req.method(), req.path()));
        next.run(req).await
    })
    .for_routes("*");
```

There is also the `FnMiddleware(f)` type, which wraps a function as a `CorvisMiddleware`.

## Global middleware

For a middleware that applies to all requests, register it on the application:

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.use_middleware(corvis::serve_static(PUBLIC_DIR));          // an instance
app.use_fn(|req, next| async move { next.run(req).await });   // a function
```

(`examples/tertulia/src/lib.rs` mounts the web client's static files this way.)

`use_middleware` takes an instance, so it has no dependency injection. If a global middleware needs providers, register it in the root module with `consumer.apply::<M>().for_routes("*")`.

## Execution order

1. CORS answers `OPTIONS` preflights before anything else.
2. Global middleware (`use_middleware`, `use_fn`), in the order they were registered.
3. Module middleware whose rules cover the request, in the order the modules are walked and, within each one, in the order of `configure`.
4. Router: guards, interceptors, pipes, and handler (see [Request lifecycle](/faq/request-lifecycle)).

Middleware runs **before** the route is looked up: it also runs for requests that end in a `404`.

An error returned by a middleware (or a `panic` inside it) goes through the **global filters**; filters declared on a controller or a route do not see it.

Body parser errors (malformed JSON, body too large) are answered before reaching the middleware.
