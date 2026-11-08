---
status: documented
---

# Exception filters

When a handler, a guard, a pipe, or an interceptor fails, the error ends up in the exceptions layer. That layer decides what response the client receives. Corvis ships a default filter that produces JSON bodies in the usual format, and lets you replace it with your own filters wherever you need.

## `HttpException`

In Corvis every pipeline error is an `HttpException`. It carries an HTTP status, a JSON body and, optionally, the original cause.

```rust
#[get(":id")]
fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> {
    self.cats_service
        .find_one(id)
        .ok_or_else(|| NotFoundException::new(format!("Cat #{id} not found")))
}
```

```json
{ "message": "Cat #42 not found", "error": "Not Found", "statusCode": 404 }
```

`HttpResult<T>` is an alias of `Result<T, HttpException>`. The `?` operator works with any error that converts into `HttpException`.

### Building an exception

`HttpException::new(body, status)` accepts as the body a `&str`/`String`, a `Vec` of messages, a JSON `Value`, or `()`; and as the status a `u16`, a `StatusCode`, or an `HttpStatus` constant:

```rust
HttpException::new("Forbidden", 403);
// {"statusCode":403,"message":"Forbidden"}

HttpException::new(json!({ "status": 418, "error": "I am a teapot" }), HttpStatus::I_AM_A_TEAPOT);
// the object is sent as is
```

Useful methods:

| Method | Use |
|---|---|
| `get_status()` / `status()` | Status as `u16` / `StatusCode` |
| `get_response()` | JSON body (`&Value`) |
| `message()` | Message as text |
| `with_description("...")` | Replaces the `error` field of the body |
| `with_cause(err)` | Attaches the original cause (it is not sent to the client) |
| `cause()`, `cause_is::<E>()`, `downcast_cause::<E>()` | Inspect the cause |
| `is_internal()` | `true` for unexpected errors (see below) |
| `HttpException::from_anyhow(e)` / `into_anyhow()` | Conversion with `anyhow::Error` |

### Built-in exceptions

Each one has `::new(body)` (which adds `error` with the status phrase) and `::empty()`. In addition, the unit type converts into `HttpException` with `.into()` or with `?`:

| Type | Status |
|---|---|
| `BadRequestException` | 400 |
| `UnauthorizedException` | 401 |
| `ForbiddenException` | 403 |
| `NotFoundException` | 404 |
| `MethodNotAllowedException` | 405 |
| `NotAcceptableException` | 406 |
| `RequestTimeoutException` | 408 |
| `ConflictException` | 409 |
| `GoneException` | 410 |
| `PreconditionFailedException` | 412 |
| `PayloadTooLargeException` | 413 |
| `UnsupportedMediaTypeException` | 415 |
| `ImATeapotException` | 418 |
| `MisdirectedException` | 421 |
| `UnprocessableEntityException` | 422 |
| `TooManyRequestsException` | 429 |
| `InternalServerErrorException` | 500 |
| `NotImplementedException` | 501 |
| `BadGatewayException` | 502 |
| `ServiceUnavailableException` | 503 |
| `GatewayTimeoutException` | 504 |
| `HttpVersionNotSupportedException` | 505 |

```rust
NotFoundException::empty();                    // {"message":"Not Found","statusCode":404}
NotFoundException::new("Cat not found");       // {"message":"Cat not found","error":"Not Found","statusCode":404}
BadRequestException::new("x").with_description("desc");
let token = extract_bearer_token(request).ok_or(UnauthorizedException)?;
```

### Unexpected errors

If a handler propagates with `?` an error that is not an `HttpException` (a `std::io::Error`, a database error, an `anyhow::Error`), Corvis wraps it in an **internal** exception:

```json
{ "statusCode": 500, "message": "Internal server error" }
```

The client does not see the original message. The default filter logs it with the `ExceptionsHandler` context, including the chain of causes. The same happens with a `panic` inside a handler: it is caught and answered as `500`.

`HttpException::http_error(status, message)` represents an error with its own status that is also not intentional, like the `413` or `415` of the body parser: it responds `{ statusCode, message }` with that status, but `is_internal()` is `true`, so a custom catch-all filter treats it as an unknown error.

## Custom filters

A filter implements `ExceptionFilter`:

```rust
#[async_trait]
pub trait ExceptionFilter: Send + Sync + 'static {
    fn catches(&self, _exception: &HttpException) -> bool { true }
    async fn catch(&self, exception: HttpException, host: &ArgumentsHost) -> Response;
}
```

- `catches` decides which exceptions the filter handles. By default, all of them.
- `catch` builds the response. `host` is the [execution context](/fundamentals/execution-context): `host.switch_to_http().get_request()` gives the request.
- If the `Response` you return has no status, the exception's status is used.

This filter from `examples/cats-app` adds `timestamp` and `path`, and only handles intentional exceptions:

```rust
use corvis::prelude::*;

#[injectable]
pub struct HttpExceptionFilter;

#[async_trait]
impl ExceptionFilter for HttpExceptionFilter {
    fn catches(&self, exception: &HttpException) -> bool {
        // Only exceptions thrown on purpose, not unexpected errors.
        !exception.is_internal()
    }

    async fn catch(&self, exception: HttpException, host: &ArgumentsHost) -> Response {
        let request = host.switch_to_http().get_request();
        let status = exception.get_status();
        let message = exception.get_response().get("message").cloned().unwrap_or(json!(exception.message()));
        Response::json(json!({
            "statusCode": status,
            "message": message,
            "timestamp": unix_timestamp(),
            "path": request.url(),
        }))
        .with_status(status)
    }
}
```

Since there is no exception class hierarchy, `catches` filters by what you do have at hand: the status (`get_status() == 404`), whether it is internal (`is_internal()`), or the type of the cause (`cause_is::<sqlx::Error>()`, for example).

### Reusing the default filter

`BaseExceptionFilter::handle(&exception)` produces the standard response (and logs internal errors). Use it for cases your filter doesn't want to treat specially:

```rust
#[async_trait]
impl ExceptionFilter for AuditFilter {
    async fn catch(&self, exception: HttpException, host: &ArgumentsHost) -> Response {
        if exception.get_status() == 403 {
            self.audit.record(host.request().path());
        }
        BaseExceptionFilter::handle(&exception)
    }
}
```

## Applying filters

| Scope | Form |
|---|---|
| One route | `#[use_filters(HttpExceptionFilter)]` on the method |
| One controller | `#[use_filters(HttpExceptionFilter)]` below `#[controller]` |
| Global, instance | `app.use_global_filters(HttpExceptionFilter)` |
| Global, with DI | `Provider::app_filter::<HttpExceptionFilter>()` in a module's `providers` |

```rust
#[get("filtered/missing")]
#[use_filters(HttpExceptionFilter)]
async fn filtered(&self) -> HttpResult<()> {
    Err(NotFoundException.into())
}
```

In `#[use_filters(...)]` you can pass a type (resolved with dependency injection in the controller's module, so it must be `#[injectable]`) or an expression that builds the instance (`#[use_filters(MyFilter::new("x"))]`).

`use_global_filters` takes an instance: it cannot inject dependencies. If the filter needs them, register it with `Provider::app_filter::<T>()`:

```rust
#[module(
    providers = [Provider::app_filter::<HttpExceptionFilter>()],
)]
pub struct AppModule;
```

### Which filter responds

For each exception, Corvis walks the filters in this order and uses **the first one whose `catches` returns `true`**:

1. The route's filters.
2. The controller's.
3. The global ones, last registered first: first those from `use_global_filters`, from the last call to the first, then those registered as a provider (`app_filter`), from the last to the first.

   Guards, interceptors and pipes run in registration order, but exception filters are tried in reverse, so a filter registered later takes precedence over an earlier one. The order was checked against the original framework.

If none handles it, the default filter responds.

Exceptions that occur outside a route (in a middleware, the `404` of a nonexistent route, a body parser error) only go through the global filters.

::: info Gateways
Global filters do not apply to WebSocket gateways; declare `#[use_filters]` on the gateway or handler instead. See [WebSockets: exception filters](/websockets/exception-filters).
:::
