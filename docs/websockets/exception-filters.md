---
status: partial
---

# Exception filters

Gateways **do not support custom exception filters yet**. `#[use_filters(...)]` below `#[websocket_gateway]` produces a compile error (`#[use_filters] is not supported on WebSocket gateways`), and global filters (`app.use_global_filters`, `Provider::app_filter`) only apply to HTTP routes.

What does exist is the built-in filter: any error from a guard, interceptor, pipe, or handler is turned into an `exception` frame with a fixed shape.

| Error | `data` of the `exception` frame |
|---|---|
| `WsException::new("msg")` | `{ "status": "error", "message": "msg" }` |
| `WsException::object(obj)` | `obj`, as is |
| Another `HttpException` | `{ "status": "error", "message", ...rest of its body }` |
| Unknown error or `panic` | `{ "status": "error", "message": "Internal server error" }` (and it is logged) |

If the client's frame carried an `id`, the `exception` frame repeats it (see [acknowledgements](/websockets/gateways#acknowledgements-acks)).

## Alternatives

To control the format of errors:

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

Interceptors run after guards, so errors thrown by a guard do not go through them.
