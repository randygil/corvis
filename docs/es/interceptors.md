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
| `LoggingInterceptor` | Registra `GET /cats 200 - 3ms` con el contexto `LoggingInterceptor`. Los parámetros de query que un guard marcó como credenciales se ocultan: `GET /data?page=2&api_key=REDACTED 200 - 3ms` (ver abajo) |
| `CacheInterceptor` | Cachea respuestas `GET` (requiere `CacheModule`) |
| `z::ZodSerializerInterceptor` | Valida y da forma a la salida con un schema (ver [Serialización](/es/application/serialization)) |
| `FileInterceptor`, `FilesInterceptor`, ... | Leen cuerpos `multipart` (ver [Subida de archivos](/es/http/file-upload)) |

```rust
app.use_global_interceptors(TimeoutInterceptor::millis(10_000));
```

### Ocultar credenciales en la URL registrada

`LoggingInterceptor` registra `request.redacted_url()`, no `request.url()`. Las estrategias que leen una credencial del query string (la [estrategia de API key](/es/recipes/passport#api-keys), la estrategia JWT con `from_url_query_parameter`, la estrategia local con `allow_query_credentials`) registran los nombres de los parámetros al arrancar la aplicación, y el valor de cada aparición se reemplaza por `REDACTED` en toda petición, corra o no la estrategia para ella (también las grafías con percent-encoding y, con el parser de query extendido, `api_key[x]=`). Una estrategia propia lista sus nombres en `Strategy::credential_query_params`; un guard propio marca uno para la petición actual con `request.redact_query_param("api_key")`, que conserva como máximo 16 nombres por petición (pasado ese límite se ocultan todos los valores del query). Si no hay nada registrado ni marcado, `redacted_url()` es exactamente `url()`, y la aplicación no paga nada por ello. Un query con más de 1000 pares tiene todos los valores ocultos, sin analizarlo. La línea no se construye cuando el nivel `log` está desactivado.

Solo se cubre esta línea de log. `request.url()` no cambia (tus handlers, el mensaje del `404` de una ruta sin coincidencia y un middleware que registre la URL antes de los guards siguen viendo la clave), y lo mismo ocurre con todo lo que queda fuera de Corvis: los logs de acceso del reverse proxy y del CDN, el historial del navegador y las cabeceras `Referer`. Prefiere una cabecera para las credenciales.

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

## Poner cabeceras de respuesta

Un interceptor puede añadir una cabecera a la respuesta, también después de que corra el handler, con `ctx.switch_to_http().set_response_header(nombre, valor)`:

```rust
let result = next.handle().await;
ctx.switch_to_http().set_response_header("X-Elapsed-Ms", &started.elapsed().as_millis().to_string());
result
```

La cabecera se mezcla en la respuesta final en éxito, en errores y en la respuesta de un filtro de excepciones, y nunca reemplaza una cabecera que la respuesta ya tiene. Devuelve `false` y no registra nada con un nombre o valor inválido. Ver [Guards](/es/guards#poner-cabeceras-de-respuesta).
