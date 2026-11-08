---
status: documented
---

# Prefijo global

Para que todas las rutas cuelguen de un mismo prefijo (por ejemplo `/api`), llama a
`set_global_prefix` antes de `listen`:

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.set_global_prefix("api");
app.listen(3000).await
```

Con eso, un `#[controller("cats")]` con `#[get(":id")]` responde en `GET /api/cats/:id`. Si además
activas el [versionado por URI](/es/http/versioning), la versión va después del prefijo:
`/api/v1/cats/:id`.

## Excluir controladores del prefijo

`set_global_prefix_with_exclude` recibe una lista de rutas que quedan fuera del prefijo:

```rust
app.set_global_prefix_with_exclude("api/v1", &["health"]);
```

Cada elemento se compara con la **ruta completa de cada handler** (la ruta del controlador unida a la
del handler, sin el prefijo y sin el segmento de versión), y no distingue métodos HTTP. Así un solo
endpoint queda fuera del prefijo:

```rust
// `GET /status` y `GET /api/users`; `/api/status` y `/users` responden 404.
app.set_global_prefix_with_exclude("api", &["status"]);
```

Para dejar sin prefijo todas las rutas de un controlador, termina el elemento con un comodín:

```rust
// `GET /health` y `GET /health/live`.
app.set_global_prefix_with_exclude("api", &["health/*"]);
```

::: warning Un elemento que nombra solo el controlador excluye solo su ruta
`"health"` deja fuera del prefijo a `GET /health` (un `#[get]` en `#[controller("health")]`), pero no a
`GET /api/health/live`. Las versiones anteriores excluían con ese elemento todas las rutas del
controlador; escribe `"health/*"` para eso.
:::

Los patrones aceptan comodines (`"admin/*"`), con las mismas reglas que `for_routes` del
[middleware](/es/middleware).

## Lo que el prefijo no cubre

El prefijo solo afecta a las rutas de los controladores. No cambia:

- las rutas de los `#[websocket_gateway(path = "...")]`;
- los archivos servidos con `corvis::serve_static` o `ServeStaticModule`;
- las rutas que montan `SwaggerModule::setup` o Scalar, que reciben su propia ruta (en Swagger,
  `SwaggerCustomOptions { use_global_prefix: true, .. }` la antepone; ver
  [OpenAPI](/es/openapi/introduction)).

`app.global_prefix()` devuelve el prefijo configurado.
