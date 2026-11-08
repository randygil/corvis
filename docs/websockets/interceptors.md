---
status: documented
---

# Interceptors

[Interceptors](/interceptors) wrap the execution of a message handler just like that of an HTTP route: they run code before, call `next.handle().await`, and can inspect or transform the result. They are declared with `#[use_interceptors(...)]` below `#[websocket_gateway]` (all handlers) or on top of a `#[subscribe_message]` (only that one). Global interceptors (`app.use_global_interceptors`, `Provider::app_interceptor`) apply to gateways too and wrap the gateway's and the handler's; they see WebSocket contexts, so check `ctx.get_type()` before reading the HTTP request. `app.apply_global_enhancers_to_gateways(false)` keeps them HTTP-only (see [Gateways](/websockets/gateways#global-enhancers)). The built-in `CacheInterceptor` never caches gateway messages (it passes them through), and `LoggingInterceptor` logs `WS <Gateway>.<handler>` instead of the handshake URL, which can carry a token.

The order within a message is: guards → interceptors (before) → pipes → handler → interceptors (after).

## Example: logging every message

`examples/tertulia/src/chat/ws_logging_interceptor.rs` measures how long each handler takes and who called it:

```rust
use std::time::Instant;

use corvis::prelude::*;

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
```

```rust
#[websocket_gateway(path = "/chat")]
#[use_interceptors(WsLoggingInterceptor)]
pub struct ChatGateway { /* ... */ }
```

## What the interceptor sees

`next.handle().await` returns a `HandlerResult`, that is, a `Response` or an `HttpException`. In a gateway, the JSON body of that `Response` represents what the handler returned:

| The handler returned | Body the interceptor sees |
|---|---|
| A `Serialize` value | The value |
| `WsResponse::new(event, data)` | `{ "event": event, "data": data }` |
| Nothing (`()` or `None`) | `null` |

If the interceptor transforms the body (for example with `response.map_data(|v| ...)`), the client receives the transformed value with the same shape: an `{ event, data }` is still a response with its own event and any other value is sent with the original event.

As in HTTP, an interceptor can return an error before calling `next` to cut the execution short; the client receives it as an `exception` frame.
