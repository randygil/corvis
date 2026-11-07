---
status: parcial
---

# Filtros de excepción

Los gateways **no admiten filtros de excepción propios todavía**. `#[use_filters(...)]` debajo de `#[websocket_gateway]` produce un error de compilación (`#[use_filters] is not supported on WebSocket gateways`), y los filtros globales (`app.use_global_filters`, `Provider::app_filter`) solo se aplican a las rutas HTTP.

Lo que sí existe es el filtro integrado: cualquier error de un guard, interceptor, pipe o handler se convierte en un frame `exception` con una forma fija.

| Error | `data` del frame `exception` |
|---|---|
| `WsException::new("msg")` | `{ "status": "error", "message": "msg" }` |
| `WsException::object(obj)` | `obj`, tal cual |
| Otro `HttpException` | `{ "status": "error", "message", ...resto de su cuerpo }` |
| Error desconocido o `panic` | `{ "status": "error", "message": "Internal server error" }` (y se registra) |

Si el frame del cliente traía `id`, el frame `exception` lo repite (ver [acuses de recibo](/es/websockets/gateways#acuses-de-recibo-acks)).

## Alternativas

Para controlar el formato de los errores:

- **Construye el error con la forma final** usando `WsException::object(json!({ ... }))`. Tertulia lo hace para incluir códigos como `ROOM_FULL` o `NOT_IN_ROOM` que el cliente puede interpretar.
- **Traduce errores en un interceptor**. Un interceptor declarado en el gateway ve el `Err` que devuelve `next.handle().await` y puede reemplazarlo por un `WsException`:

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

Los interceptors corren después de los guards, así que los errores que lanza un guard no pasan por ellos.
