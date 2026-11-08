---
status: documented
---

# Filtros de excepción

Los gateways soportan filtros de excepción. `#[use_filters(...)]` va debajo de `#[websocket_gateway]` (todos los handlers del gateway) o sobre un método `#[subscribe_message]` (ese handler), y los filtros implementan el mismo trait `ExceptionFilter` que en las rutas HTTP, así que un mismo tipo de filtro sirve para ambos transportes.

Los filtros globales (`app.use_global_filters`, `Provider::app_filter`) **no** se aplican a los gateways, igual que en el framework original. Un filtro escrito para rutas HTTP (por ejemplo uno que responde con un estado y un cuerpo de error JSON) no cambia los frames de un gateway a menos que lo declares ahí.

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

`#[use_filters]` sobre otro tipo de método dentro de `#[handlers]` (`#[on_event]`, `#[cron]`, ...) es un error de compilación, como los demás enhancers.

## Orden

Cuando falla un guard, un interceptor, un pipe o el handler (incluido un guard que devuelve `false` y un handler que hace `panic`), Corvis prueba los filtros del handler, después los del gateway, y usa el primero cuyo `catches` devuelve `true`. Los filtros de un mismo `#[use_filters(A, B)]` se prueban en el orden en que están escritos, como en las rutas HTTP. Si ninguno atrapa la excepción, responde el mapeo integrado de abajo.

El framework original prueba los filtros de un mismo `@UseFilters(A, B)` empezando por el último. Corvis mantiene una sola regla para ambos transportes, así que `A` va antes que `B` aquí y en las rutas.

## De la respuesta al frame

El `host` que recibe un filtro es el contexto de ejecución del mensaje: `host.get_type()` es `ContextType::Ws`, y `host.switch_to_ws()` da el cliente y los datos del mensaje. La `Response` que devuelve el filtro se convierte en el frame `exception`:

| Cuerpo de la respuesta | Lo que recibe el cliente |
|---|---|
| JSON | `{ "event": "exception", "data": <el JSON> }` |
| Texto o HTML | `{ "event": "exception", "data": "<el texto>" }` |
| Vacío | nada: el filtro ya respondió (por ejemplo con `client.emit`) |
| Bytes o stream | el frame integrado de la excepción original, y se registra un error con el nombre del filtro |

El estado y las cabeceras de la respuesta se ignoran. Un filtro que hace `panic` se registra y se responde con el frame integrado de error interno. Si el frame del cliente traía `id`, el frame lo repite (ver [acuses de recibo](/es/websockets/gateways#acuses-de-recibo-acks)).

Un filtro que quiere enviar otro evento lo emite y devuelve una respuesta vacía:

```rust
async fn catch(&self, _exception: HttpException, host: &ArgumentsHost) -> Response {
    let client = host.switch_to_ws().get_client::<WsClient>().expect("un cliente WebSocket");
    client.emit("error", json!({ "retry": true }));
    Response::new()
}
```

::: warning No filtres errores internos
Un filtro que atrapa todo (el `catches` por defecto devuelve `true` para cualquier excepción) y copia `exception.message()` en el frame envía al cliente el texto de los errores internos (mensajes de la base de datos, rutas de archivos), porque el mapeo integrado que lo oculta ya no corre. Comprueba `exception.is_internal()` y responde con un mensaje genérico en esos casos.
:::

Un filtro cuyo `catches` hace `panic` se trata como si no atrapara la excepción, y el `panic` se registra.

## El mapeo integrado

Cuando ningún filtro atrapa la excepción, cualquier error de un guard, interceptor, pipe o handler se convierte en un frame `exception` con una forma fija.

| Error | `data` del frame `exception` |
|---|---|
| `WsException::new("msg")` | `{ "status": "error", "message": "msg" }` |
| `WsException::object(obj)` | `obj`, tal cual |
| Otra `HttpException` | `{ "status": "error", "message", ...resto de su cuerpo }` |
| Error desconocido o `panic` | `{ "status": "error", "message": "Internal server error" }` (y se registra) |

Los errores internos se registran solo cuando responde este mapeo: un filtro que los atrapa asume la responsabilidad.

## Alternativas

Para controlar el formato de los errores sin un filtro:

- **Construye el error con su forma final** usando `WsException::object(json!({ ... }))`. Tertulia lo hace para incluir códigos como `ROOM_FULL` o `NOT_IN_ROOM` que el cliente puede interpretar.
- **Traduce los errores en un interceptor**. Un interceptor declarado en el gateway ve el `Err` que devuelve `next.handle().await` y puede reemplazarlo por una `WsException`:

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

Los interceptors corren después de los guards, así que los errores lanzados por un guard no pasan por ellos; los filtros sí los ven.
