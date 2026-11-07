---
status: documentado
---

# Interceptors

Los [interceptors](/es/interceptors) envuelven la ejecución de un handler de mensajes igual que la de una ruta HTTP: corren código antes, llaman a `next.handle().await` y pueden inspeccionar o transformar el resultado. Se declaran con `#[use_interceptors(...)]` debajo de `#[websocket_gateway]` (todos los handlers) o encima de un `#[subscribe_message]` (solo ese). Los interceptors globales no se aplican a los gateways.

El orden dentro de un mensaje es: guards → interceptors (antes) → pipes → handler → interceptors (después).

## Ejemplo: registrar cada mensaje

`examples/tertulia/src/chat/ws_logging_interceptor.rs` mide cuánto tarda cada handler y quién lo llamó:

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

## Qué ve el interceptor

`next.handle().await` devuelve un `HandlerResult`, es decir, un `Response` o un `HttpException`. En un gateway el cuerpo JSON de ese `Response` representa lo que devolvió el handler:

| El handler devolvió | Cuerpo que ve el interceptor |
|---|---|
| Un valor `Serialize` | El valor |
| `WsResponse::new(evento, datos)` | `{ "event": evento, "data": datos }` |
| Nada (`()` o `None`) | `null` |

Si el interceptor transforma el cuerpo (por ejemplo con `response.map_data(|v| ...)`), el cliente recibe el valor transformado con la misma forma: un `{ event, data }` sigue siendo una respuesta con evento propio y cualquier otro valor se envía con el evento original.

Como en HTTP, un interceptor puede devolver un error antes de llamar a `next` para cortar la ejecución; el cliente lo recibe como frame `exception`.
