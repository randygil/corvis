---
status: parcial
---

# Servicio de descubrimiento

Un servicio de descubrimiento permite que una librería recorra los providers y controladores de la aplicación en tiempo de ejecución y encuentre los que llevan cierta marca: los métodos anotados como handlers de un evento, los controladores con un metadato, etc.

::: warning No hay un `DiscoveryService` público
Corvis no expone hoy un servicio inyectable para enumerar providers ni para recorrer sus métodos. El descubrimiento existe, pero es interno y está limitado a los atributos del propio framework. Esta página describe lo que sí está disponible.
:::

## Lo que Corvis descubre por ti: `#[handlers]`

Los atributos de método que reconoce `#[handlers]` se descubren solos al llamar a `app.init()`. Corvis recorre todas las instancias de providers y controladores, recoge sus handlers y los conecta con el módulo correspondiente:

| Atributo | Se conecta con |
|---|---|
| `#[on_event("cat.created")]` | `EventEmitter` (ver [Eventos](/application/events)) |
| `#[cron(expr)]`, `#[interval(ms)]`, `#[timeout(ms)]` | El planificador (ver [Tareas programadas](/application/task-scheduling)) |
| `#[subscribe_message("event")]` | El gateway de WebSocket (ver [Gateways](/websockets/gateways)) |

```rust
#[injectable]
pub struct TasksService {
    events: Arc<EventEmitter>,
}

#[handlers]
impl TasksService {
    #[cron(CronExpression::EVERY_30_SECONDS)]
    async fn report(&self) {
        self.events.emit("report.ready", json!({ "at": "now" }));
    }

    #[on_event("cat.*")]
    async fn on_cat(&self, payload: Value) { /* ... */ }
}
```

Basta con que el tipo esté registrado como provider en algún módulo. El conjunto de atributos es fijo: no se pueden agregar tipos de handler nuevos sin modificar el framework.

## Recorrer controladores y rutas: `modules_info`

`CorvisApplication::modules_info()` devuelve el grafo de módulos con sus controladores y los metadatos de cada ruta. No necesita `app.init()`. Es lo que usa `corvis-swagger` para generar el documento OpenAPI.

```rust
let app = CorvisFactory::create::<AppModule>().await?;

for module in app.modules_info().iter().filter(|m| !m.internal) {
    for controller in &module.controllers {
        for route in &controller.routes {
            let paths = app.resolve_route_paths(&controller.metadata, route);
            println!("{} {:?} -> {}.{}", route.method.as_str(), paths, controller.name, route.handler_name);
        }
    }
}
```

| Tipo | Campos útiles |
|---|---|
| `ModuleInfo` | `name`, `type_id`, `global`, `internal`, `imports` (índices en la misma lista), `controllers` |
| `ControllerInfo` | `name`, `type_id`, `metadata` (`ControllerMetadata`), `routes` |
| `ControllerMetadata` | `paths`, `host`, `version`, `metadata` (pares clave/valor de `#[set_metadata]` y similares) |
| `RouteMeta` | `method`, `paths`, `handler_name`, `params`, `http_code`, `headers`, `redirect`, `metadata`, `version`, `sse` |

Junto con `#[set_metadata]` o un `CustomDecorator`, esto permite construir un "descubrimiento" propio sobre las rutas. Por ejemplo, listar las rutas marcadas como públicas:

```rust
let public: Vec<String> = app
    .modules_info()
    .iter()
    .flat_map(|m| m.controllers.iter())
    .flat_map(|c| c.routes.iter().map(move |r| (c, r)))
    .filter(|(_, r)| r.metadata.iter().any(|(k, v)| k == IS_PUBLIC_KEY && v == &json!(true)))
    .map(|(c, r)| format!("{}.{}", c.name, r.handler_name))
    .collect();
```

Otras funciones de introspección de la aplicación: `global_prefix()`, `global_prefix_exclude()`, `versioning_options()`, `effective_route_version(..)`, `resolve_route_paths(..)` y `resolve_route_paths_with(.., global_prefix)`.

## Obtener instancias

Para llegar a una instancia concreta:

- `app.get::<T>()` y `app.get_token::<T>("TOKEN")` buscan en el contenedor (ver [Referencia al módulo](/fundamentals/module-ref)).
- `app.get_instance::<T>(token)` devuelve `Option<Arc<T>>` buscando la instancia en toda la aplicación.
- Un campo `ModuleRef` en un provider da el mismo acceso desde dentro.

## Lo que falta

- No se pueden enumerar los **providers** de un módulo: `ModuleInfo` solo lista controladores.
- No hay forma de recorrer los métodos de un provider ni de leer metadatos de métodos que no sean rutas.
- Los tipos que usa el descubrimiento interno (`corvis::discovery::DiscoveredHandler`, `HandlerKind`) son públicos, pero la función que los recolecta no lo es.

Mientras tanto, la alternativa práctica es que tu librería defina un trait y que los providers interesados se registren a sí mismos en un hook `on_module_init`, inyectando un registro compartido:

```rust
#[injectable]
pub struct ReportRegistry {
    #[default]
    reports: std::sync::Mutex<Vec<String>>,
}

#[injectable]
pub struct SalesReport {
    registry: Arc<ReportRegistry>,
}

#[async_trait]
impl OnModuleInit for SalesReport {
    async fn on_module_init(&self) -> corvis::Result<()> {
        self.registry.reports.lock().unwrap().push("sales".into());
        Ok(())
    }
}
```
