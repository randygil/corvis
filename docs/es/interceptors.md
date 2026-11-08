---
status: documented
---

# Interceptors

Un interceptor envuelve la ejecución del handler. Puede hacer trabajo antes y después, transformar la respuesta, transformar o capturar errores, o responder sin llegar a llamar al handler. Es la herramienta para preocupaciones transversales como medir tiempos, envolver respuestas en un formato común, cachear o aplicar timeouts.

## El trait `CorvisInterceptor`

```rust
#[async_trait]
pub trait CorvisInterceptor: Send + Sync + 'static {
    async fn intercept(&self, context: &ExecutionContext, next: CallHandler) -> HandlerResult;
}
```

- `context` es el [contexto de ejecución](/es/fundamentals/execution-context) de la petición.
- `next` representa el resto de la cadena: los interceptors siguientes, los pipes y el handler. `next.handle().await` los ejecuta y devuelve un `HandlerResult` (`Result<Response, HttpException>`). `handle` consume `next`, así que solo se puede llamar una vez.
- Lo que devuelve `intercept` es lo que sigue su camino hacia el cliente (o hacia el interceptor exterior).

::: info Sin streams reactivos
En Corvis `next.handle()` devuelve un futuro, no un observable. Puedes transformar, cronometrar, cachear y aplicar timeouts, pero no componer operadores sobre un flujo.
:::

## Antes y después

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

## Transformar la respuesta

`Response::map_data` aplica una función al cuerpo de la respuesta. Los cuerpos JSON llegan tal cual, los de texto como string JSON y los vacíos como `null`; los binarios o en streaming no se tocan. Un resultado `null` de la función se guarda como cuerpo vacío, así que `map_data(|data| data)` mantiene vacía una respuesta vacía.

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

Con este interceptor, un handler que devuelve `[1, 2, 3]` responde `{"data":[1,2,3]}`.

Además de `map_data`, sobre la `Response` tienes `json_body()` para leer el cuerpo JSON, `status()`, `set_status(..)`, `set_header(..)`, `headers()` y `set_body(..)`.

## Transformar errores

El error también pasa por el interceptor, así que puedes reemplazarlo:

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

## Responder sin llamar al handler

Si el interceptor no llama a `next.handle()`, el handler no se ejecuta. Es la base de una caché:

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

Corvis ya incluye un `CacheInterceptor` completo con TTL por ruta (ver [Caché](/es/data/caching)).

## Interceptors incluidos

| Interceptor | Uso |
|---|---|
| `TimeoutInterceptor::new(Duration)` / `TimeoutInterceptor::millis(ms)` | Responde `408 Request Timeout` si el handler tarda más; `Default` es 5 s |
| `LoggingInterceptor` | Registra `GET /cats 200 - 3ms` con el contexto `LoggingInterceptor` |
| `CacheInterceptor` | Cachea respuestas `GET` (requiere `CacheModule`) |
| `z::ZodSerializerInterceptor` | Valida y da forma a la salida con un schema (ver [Serialización](/es/application/serialization)) |
| `FileInterceptor`, `FilesInterceptor`, ... | Leen cuerpos `multipart` (ver [Subida de archivos](/es/http/file-upload)) |

```rust
app.use_global_interceptors(TimeoutInterceptor::millis(10_000));
```

## Aplicar interceptors

| Alcance | Forma |
|---|---|
| Una ruta | `#[use_interceptors(TransformInterceptor)]` sobre el método |
| Un controlador | `#[use_interceptors(TimingInterceptor)]` debajo de `#[controller]` |
| Global, instancia | `app.use_global_interceptors(LoggingInterceptor)` |
| Global, con DI | `Provider::app_interceptor::<MiInterceptor>()` en `providers` |

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

Como con guards y pipes, en el atributo puedes pasar un tipo (resuelto con DI en el módulo del controlador) o una expresión (`#[use_interceptors(TimeoutInterceptor::millis(500))]`).

### Orden

Los interceptors se anidan: el primero de la lista es el más externo.

1. Globales registrados como provider (`app_interceptor`).
2. Globales de `use_global_interceptors`.
3. Los del controlador.
4. Los de la ruta.

El código "antes" se ejecuta en ese orden y el código "después", en el orden inverso. Los interceptors corren después de los guards y antes de los pipes, así que un error de validación de un pipe también pasa por ellos.

## Leer el estado de la ruta

Dentro de un interceptor, `ctx.switch_to_http().status_code()` devuelve el estado que tendrá la respuesta si el handler no lo cambia: el de `#[http_code]`, `201` en `POST` o `200`. Sirve, por ejemplo, para guardar `{ status, body }` en una caché.
