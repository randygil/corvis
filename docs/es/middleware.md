---
status: documented
---

# Middleware

Un middleware es una función que se ejecuta **antes** de que el router elija el handler. Recibe la petición y un `Next` que representa el resto de la cadena: puede inspeccionar o modificar la petición, cortar la cadena devolviendo su propia respuesta, o llamar a `next.run(req)` y trabajar sobre la respuesta que vuelve.

## Middleware como clase

Un middleware con dependencias es un `#[injectable]` que implementa `CorvisMiddleware`:

```rust
use std::time::Instant;

use corvis::prelude::*;

/// Registra cada petición: `GET /api/v1/cats 200 - 3ms`.
#[injectable]
pub struct LoggerMiddleware {
    #[init(Logger::new("HTTP"))]
    logger: Logger,
}

#[async_trait]
impl CorvisMiddleware for LoggerMiddleware {
    async fn use_(&self, req: Request, next: Next) -> HandlerResult {
        let start = Instant::now();
        let line = format!("{} {}", req.method(), req.url());
        let res = next.run(req).await;
        let status = match &res {
            Ok(r) => r.status_code(),
            Err(e) => e.get_status(),
        };
        self.logger.log(format!("{line} {status} - {}ms", start.elapsed().as_millis()));
        res
    }
}
```

(`examples/cats-app/src/common/middleware.rs`)

La firma es:

```rust
#[async_trait]
pub trait CorvisMiddleware: Send + Sync + 'static {
    async fn use_(&self, req: Request, next: Next) -> HandlerResult;
}
```

- `req` llega por valor. Si quieres agregar datos para los handlers, usa las extensiones de la petición: `req.insert(valor)` y, más adelante, `req.get::<T>()`.
- `next.run(req).await` ejecuta el siguiente middleware o, al final, el handler. Devuelve un `HandlerResult` (`Result<Response, HttpException>`).
- Para cortar la cadena, no llames a `next` y devuelve tu propia respuesta (`Ok(Response::json(...))`) o un error (`Err(UnauthorizedException::new("..."))`).
- Para modificar la respuesta, trabaja sobre lo que devuelve `next.run`:

```rust
#[injectable]
pub struct HeaderMiddleware;

#[async_trait]
impl CorvisMiddleware for HeaderMiddleware {
    async fn use_(&self, req: Request, next: Next) -> HandlerResult {
        let mut res = next.run(req).await?;
        res.set_header("x-middleware", "yes");
        Ok(res)
    }
}
```

::: info Diferencia con Express
No hay un objeto `res` mutable que el middleware escriba antes de llamar a `next`: la respuesta se modifica sobre el resultado de `next.run(req)`. El objeto `res` de Express solo existe en los handlers (`#[res]`).
:::

## Aplicar middleware en un módulo

Los middleware de clase se registran en el método `configure` del trait `CorvisModule`, implementado sobre el módulo:

```rust
#[module(
    imports = [/* ... */],
    controllers = [AppController],
    providers = [AppService],
)]
pub struct AppModule;

impl CorvisModule for AppModule {
    fn configure(&self, consumer: &mut MiddlewareConsumer) {
        consumer.apply::<LoggerMiddleware>().for_routes("*");
    }
}
```

`#[module]` detecta la implementación de `CorvisModule` por sí solo; no hace falta declararla en ningún otro lado.

El middleware se resuelve con inyección de dependencias **en el contexto de ese módulo**: si no está en `providers`, Corvis lo construye y lo cachea igual, pero sus dependencias tienen que ser visibles desde el módulo.

### `MiddlewareConsumer`

| Método | Uso |
|---|---|
| `apply::<M>()` | Un middleware de clase (con DI) |
| `apply_instance(m)` | Una instancia ya construida |
| `apply_fn(\|req, next\| async move { ... })` | Una función |

Los tres devuelven un objeto de configuración con:

| Método | Uso |
|---|---|
| `.and::<M>()`, `.and_instance(m)`, `.and_fn(f)` | Agrega otro middleware a la misma regla; se ejecutan en el orden escrito |
| `.exclude(rutas)` | Rutas que la regla no cubre |
| `.for_routes(rutas)` | Rutas que la regla cubre; cierra la regla y devuelve el consumer para encadenar otra |

### Selección de rutas

`for_routes` y `exclude` aceptan:

- Un `&str` o `String` con un patrón de ruta: `"cats"`, `"cats/:id"`, `"cats/*"`, `"*"`.
- `RouteInfo { path: "cats", method: RequestMethod::Get }` o `RouteSpec::path("cats", RequestMethod::Post)`, para un solo verbo.
- `RouteSpec::controller::<CatsController>()`: todas las rutas de ese controlador, con sus rutas finales (prefijo global y versión incluidos).
- Un array o `Vec` de cualquiera de los anteriores.

```rust
impl CorvisModule for CatsModule {
    fn configure(&self, consumer: &mut MiddlewareConsumer) {
        consumer
            .apply::<HeaderMiddleware>()
            .for_routes(RouteSpec::controller::<CatsController>());

        consumer
            .apply::<AuditMiddleware>()
            .exclude(RouteInfo { path: "admin/health", method: RequestMethod::Get })
            .for_routes(["admin/*", "reports"]);
    }
}
```

Reglas de los patrones:

- El prefijo global (`set_global_prefix`) se agrega solo: escribe `"cats"`, no `"api/cats"`. Los patrones `"*"`, `"(.*)"`, `"*path"` y `"{*path}"` cubren cualquier ruta, con o sin prefijo.
- `:nombre` y `{nombre}` cubren un segmento. `*` como segmento cubre cero o más segmentos.
- Un `*` dentro de un segmento (`"admin*"`) cubre el resto de la ruta, igual que `/admin(.*)` en Express: `/api/admin`, `/api/ADMINpanel` y `/api/admin/users/1` coinciden.
- La comparación no distingue mayúsculas, la misma regla que usa el router, así que no se puede saltar un middleware cambiando la capitalización de la URL.

::: warning Versionado por URI
Los patrones de texto no incluyen el segmento de versión (`/v1`). Si la aplicación usa versionado por URI, cubre las rutas con `RouteSpec::controller::<...>()` o con un comodín.
:::

## Middleware funcional

Cuando el middleware no necesita dependencias, una función basta:

```rust
consumer
    .apply_fn(|req: Request, next: Next| async move {
        corvis::Logger::new("Request").log(format!("{} {}", req.method(), req.path()));
        next.run(req).await
    })
    .for_routes("*");
```

También existe el tipo `FnMiddleware(f)`, que envuelve una función como `CorvisMiddleware`.

## Middleware global

Para un middleware que se aplica a todas las peticiones, regístralo en la aplicación:

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.use_middleware(corvis::serve_static(PUBLIC_DIR));          // una instancia
app.use_fn(|req, next| async move { next.run(req).await });   // una función
```

(`examples/tertulia/src/lib.rs` monta así los archivos estáticos del cliente web.)

`use_middleware` recibe una instancia, así que no tiene inyección de dependencias. Si un middleware global necesita providers, regístralo en el módulo raíz con `consumer.apply::<M>().for_routes("*")`.

## Orden de ejecución

1. CORS responde los preflight `OPTIONS` antes que nada.
2. Middleware globales (`use_middleware`, `use_fn`), en el orden en que se registraron.
3. Middleware de módulo cuyas reglas cubren la petición, en el orden en que se recorren los módulos y, dentro de cada uno, en el orden de `configure`.
4. Router: guards, interceptors, pipes y handler (ver [Ciclo de vida de la petición](/es/faq/request-lifecycle)).

Los middleware corren **antes** de buscar la ruta: también se ejecutan para peticiones que terminan en `404`.

Un error que devuelve un middleware (o un `panic` dentro de él) pasa por los **filtros globales**; los filtros declarados en un controlador o en una ruta no lo ven.

Los errores del body parser (JSON mal formado, cuerpo demasiado grande) se responden antes de llegar a los middleware.
