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

::: warning La exclusión se aplica por ruta de controlador
Cada elemento de la lista se compara con la **ruta del controlador** (el argumento de
`#[controller]`), no con la ruta completa de cada handler, y no distingue métodos HTTP. El ejemplo
deja sin prefijo a un `#[controller("health")]`, pero no a un handler `#[get("health")]` dentro de un
controlador sin ruta. Si necesitas excluir un solo endpoint, muévelo a su propio controlador.
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
