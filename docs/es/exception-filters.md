---
status: documented
---

# Filtros de excepción

Cuando un handler, un guard, un pipe o un interceptor fallan, el error termina en la capa de excepciones. Esa capa decide qué respuesta recibe el cliente. Corvis trae un filtro por defecto que produce cuerpos JSON con el formato habitual, y te deja reemplazarlo por tus propios filtros donde lo necesites.

## `HttpException`

En Corvis todos los errores del pipeline son una `HttpException`. Lleva un estado HTTP, un cuerpo JSON y, opcionalmente, la causa original.

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

`HttpResult<T>` es un alias de `Result<T, HttpException>`. El operador `?` funciona con cualquier error que se convierta en `HttpException`.

### Construir una excepción

`HttpException::new(cuerpo, estado)` acepta como cuerpo un `&str`/`String`, un `Vec` de mensajes, un `Value` JSON o `()`; y como estado un `u16`, un `StatusCode` o una constante de `HttpStatus`:

```rust
HttpException::new("Forbidden", 403);
// {"statusCode":403,"message":"Forbidden"}

HttpException::new(json!({ "status": 418, "error": "Soy una tetera" }), HttpStatus::I_AM_A_TEAPOT);
// el objeto se envía tal cual
```

Métodos útiles:

| Método | Uso |
|---|---|
| `get_status()` / `status()` | Estado como `u16` / `StatusCode` |
| `get_response()` | Cuerpo JSON (`&Value`) |
| `message()` | Mensaje como texto |
| `with_description("...")` | Reemplaza el campo `error` del cuerpo |
| `with_cause(err)` | Adjunta la causa original (no se envía al cliente) |
| `cause()`, `cause_is::<E>()`, `downcast_cause::<E>()` | Inspeccionan la causa |
| `is_internal()` | `true` para errores inesperados (ver más abajo) |
| `HttpException::from_anyhow(e)` / `into_anyhow()` | Conversión con `anyhow::Error` |

### Excepciones incluidas

Cada una tiene `::new(cuerpo)` (que agrega `error` con la frase del estado) y `::empty()`. Además, el tipo unitario se convierte en `HttpException` con `.into()` o con `?`:

| Tipo | Estado |
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

### Errores inesperados

Si un handler propaga con `?` un error que no es una `HttpException` (un `std::io::Error`, un error de base de datos, un `anyhow::Error`), Corvis lo envuelve en una excepción **interna**:

```json
{ "statusCode": 500, "message": "Internal server error" }
```

El cliente no ve el mensaje original. El filtro por defecto lo registra en el log con el contexto `ExceptionsHandler`, incluida la cadena de causas. Lo mismo ocurre con un `panic` dentro de un handler: se captura y se responde como `500`.

`HttpException::http_error(estado, mensaje)` representa un error con estado propio que tampoco es intencional, como el `413` o el `415` del body parser: responde `{ statusCode, message }` con ese estado, pero `is_internal()` es `true`, así que un filtro propio que captura todo lo trata como un error desconocido.

## Filtros propios

Un filtro implementa `ExceptionFilter`:

```rust
#[async_trait]
pub trait ExceptionFilter: Send + Sync + 'static {
    fn catches(&self, _exception: &HttpException) -> bool { true }
    async fn catch(&self, exception: HttpException, host: &ArgumentsHost) -> Response;
}
```

- `catches` decide qué excepciones atiende el filtro. Por defecto, todas.
- `catch` arma la respuesta. `host` es el [contexto de ejecución](/es/fundamentals/execution-context): `host.switch_to_http().get_request()` da la petición.
- Si la `Response` que devuelves no tiene estado, se usa el de la excepción.

Este filtro de `examples/cats-app` agrega `timestamp` y `path`, y solo atiende excepciones intencionales:

```rust
use corvis::prelude::*;

#[injectable]
pub struct HttpExceptionFilter;

#[async_trait]
impl ExceptionFilter for HttpExceptionFilter {
    fn catches(&self, exception: &HttpException) -> bool {
        // Solo las excepciones lanzadas a propósito, no los errores inesperados.
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

Como no hay jerarquía de clases de excepción, `catches` filtra por lo que sí tienes a mano: el estado (`get_status() == 404`), si es interna (`is_internal()`) o el tipo de la causa (`cause_is::<sqlx::Error>()`, por ejemplo).

### Reutilizar el filtro por defecto

`BaseExceptionFilter::handle(&exception)` produce la respuesta estándar (y registra los errores internos). Úsalo para casos que tu filtro no quiere tratar de forma especial:

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

## Aplicar filtros

| Alcance | Forma |
|---|---|
| Una ruta | `#[use_filters(HttpExceptionFilter)]` sobre el método |
| Un controlador | `#[use_filters(HttpExceptionFilter)]` debajo de `#[controller]` |
| Global, instancia | `app.use_global_filters(HttpExceptionFilter)` |
| Global, con DI | `Provider::app_filter::<HttpExceptionFilter>()` en `providers` de un módulo |

```rust
#[get("filtered/missing")]
#[use_filters(HttpExceptionFilter)]
async fn filtered(&self) -> HttpResult<()> {
    Err(NotFoundException.into())
}
```

En `#[use_filters(...)]` puedes pasar un tipo (se resuelve con inyección de dependencias en el módulo del controlador, así que debe ser `#[injectable]`) o una expresión que construya la instancia (`#[use_filters(MiFiltro::new("x"))]`).

`use_global_filters` recibe una instancia: no puede inyectar dependencias. Si el filtro las necesita, regístralo con `Provider::app_filter::<T>()`:

```rust
#[module(
    providers = [Provider::app_filter::<HttpExceptionFilter>()],
)]
pub struct AppModule;
```

### Qué filtro responde

Para cada excepción, Corvis recorre los filtros en este orden y usa **el primero cuyo `catches` devuelve `true`**:

1. Los filtros de la ruta.
2. Los del controlador.
3. Los globales, el último registrado primero: antes los de `use_global_filters`, de la última llamada a la primera, y después los registrados como provider (`app_filter`), del último al primero.

   Los guards, interceptors y pipes se ejecutan en orden de registro, pero los filtros de excepción se prueban al revés, así que un filtro registrado después tiene prioridad sobre uno anterior. El orden se verificó contra el framework original.

Si ninguno la atiende, responde el filtro por defecto.

Las excepciones que se producen fuera de una ruta (en un middleware, el `404` de una ruta inexistente, un error del body parser) solo pasan por los filtros globales.

::: info Gateways
Los filtros globales no se aplican a los gateways de WebSocket; declara `#[use_filters]` en el gateway o en el handler. Ver [WebSockets: filtros de excepción](/es/websockets/exception-filters).
:::
