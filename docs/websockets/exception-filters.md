---
status: documented
---

# Exception filters

Gateways support exception filters. `#[use_filters(...)]` goes below `#[websocket_gateway]` (every handler of the gateway) or on a `#[subscribe_message]` method (that handler), and the filters implement the same `ExceptionFilter` trait as on HTTP routes, so one filter type can serve both transports.

Global filters (`app.use_global_filters`, `Provider::app_filter`) do **not** apply to gateways, like in the original framework. A filter written for HTTP routes (for example one that answers with a status and a JSON error body) therefore never changes the frames of a gateway unless you declare it there.

```rust
#[injectable]
pub struct RoomFilter;

#[async_trait]
impl ExceptionFilter for RoomFilter {
    fn catches(&self, exception: &HttpException) -> bool {
        WsException::get_error(exception).is_some()
    }

    async fn catch(&self, exception: HttpException, host: &ArgumentsHost) -> Response {
        let room = host.switch_to_ws().get_data().cloned();
        Response::json(json!({ "code": "ROOM_ERROR", "room": room, "message": exception.message() }))
    }
}

#[websocket_gateway(path = "/chat")]
#[use_filters(RoomFilter)]
pub struct ChatGateway {
    #[websocket_server]
    server: WsServer,
}
```

`#[use_filters]` on another kind of method inside `#[handlers]` (`#[on_event]`, `#[cron]`, ...) is a compile error, like the other enhancers.

## Order

When a guard, interceptor, pipe or handler fails (a guard that returns `false` and a handler that panics included), Corvis tries the handler's filters, then the gateway's, and uses the first one whose `catches` returns `true`. Filters inside one `#[use_filters(A, B)]` are tried in the order they are written, as on HTTP routes. If none catches the exception, the built-in mapping below answers.

The original framework tries the filters of one `@UseFilters(A, B)` last first. Corvis keeps one rule for both transports, so `A` goes before `B` here and on routes.

## From response to frame

The `host` a filter receives is the message's execution context: `host.get_type()` is `ContextType::Ws`, and `host.switch_to_ws()` gives the client and the message data. The `Response` the filter returns becomes the `exception` frame:

| Body of the response | What the client receives |
|---|---|
| JSON | `{ "event": "exception", "data": <the JSON> }` |
| Text or HTML | `{ "event": "exception", "data": "<the text>" }` |
| Empty | nothing: the filter already answered (for example with `client.emit`) |
| Bytes or stream | the built-in frame of the original exception, and an error naming the filter is logged |

The status and headers of the response are ignored. A filter that panics is logged and answered with the built-in internal-error frame. If the client's frame carried an `id`, the frame repeats it (see [acknowledgements](/websockets/gateways#acknowledgements-acks)).

A filter that wants to send a different event emits it and returns an empty response:

```rust
async fn catch(&self, _exception: HttpException, host: &ArgumentsHost) -> Response {
    let client = host.switch_to_ws().get_client::<WsClient>().expect("a WebSocket client");
    client.emit("error", json!({ "retry": true }));
    Response::new()
}
```

::: warning Do not leak internal errors
A catch-all filter (the default `catches` returns `true` for everything) that copies `exception.message()` into the frame sends internal error text (database messages, file paths) to the client, because the built-in mapping that hides it no longer runs. Check `exception.is_internal()` and answer with a generic message for those.
:::

A filter whose `catches` panics is treated as not catching the exception, and the panic is logged.

## The built-in mapping

When no filter catches the exception, any error from a guard, interceptor, pipe, or handler is turned into an `exception` frame with a fixed shape.

| Error | `data` of the `exception` frame |
|---|---|
| `WsException::new("msg")` | `{ "status": "error", "message": "msg" }` |
| `WsException::object(obj)` | `obj`, as is |
| Another `HttpException` | `{ "status": "error", "message", ...rest of its body }` |
| Unknown error or `panic` | `{ "status": "error", "message": "Internal server error" }` (and it is logged) |

Internal errors are logged only when this mapping answers: a filter that catches them takes the responsibility.

## Alternatives

To control the format of errors without a filter:

- **Build the error with its final shape** using `WsException::object(json!({ ... }))`. Tertulia does this to include codes like `ROOM_FULL` or `NOT_IN_ROOM` that the client can interpret.
- **Translate errors in an interceptor**. An interceptor declared on the gateway sees the `Err` returned by `next.handle().await` and can replace it with a `WsException`:

```rust
#[injectable]
pub struct WsErrorsInterceptor;

#[async_trait]
impl CorvisInterceptor for WsErrorsInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        next.handle().await.map_err(|e| {
            if e.get_status() == 404 {
                WsException::object(json!({ "status": "error", "code": "NOT_FOUND", "message": e.message() }))
            } else {
                e
            }
        })
    }
}
```

Interceptors run after guards, so errors thrown by a guard do not go through them; filters do see them.
