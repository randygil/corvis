---
status: partial
---

# Discovery service

A discovery service lets a library walk the application's providers and controllers at runtime and find those that carry a certain mark: methods annotated as event handlers, controllers with a piece of metadata, and so on.

::: warning There is no public `DiscoveryService`
Corvis does not currently expose an injectable service to enumerate providers or walk their methods. Discovery exists, but it is internal and limited to the framework's own attributes. This page describes what is available.
:::

## What Corvis discovers for you: `#[handlers]`

The method attributes that `#[handlers]` recognizes are discovered automatically when `app.init()` is called. Corvis walks all the provider and controller instances, collects their handlers, and connects them to the corresponding module:

| Attribute | Connects to |
|---|---|
| `#[on_event("cat.created")]` | `EventEmitter` (see [Events](/application/events)) |
| `#[cron(expr)]`, `#[interval(ms)]`, `#[timeout(ms)]` | The scheduler (see [Task scheduling](/application/task-scheduling)) |
| `#[subscribe_message("event")]` | The WebSocket gateway (see [Gateways](/websockets/gateways)) |

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

It is enough for the type to be registered as a provider in some module. The set of attributes is fixed: new handler types cannot be added without modifying the framework.

## Walking controllers and routes: `modules_info`

`CorvisApplication::modules_info()` returns the module graph with its controllers and each route's metadata. It doesn't need `app.init()`. It is what `corvis-swagger` uses to generate the OpenAPI document.

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

| Type | Useful fields |
|---|---|
| `ModuleInfo` | `name`, `type_id`, `global`, `internal`, `imports` (indexes into the same list), `controllers` |
| `ControllerInfo` | `name`, `type_id`, `metadata` (`ControllerMetadata`), `routes` |
| `ControllerMetadata` | `paths`, `host`, `version`, `metadata` (key/value pairs from `#[set_metadata]` and similar) |
| `RouteMeta` | `method`, `paths`, `handler_name`, `params`, `http_code`, `headers`, `redirect`, `metadata`, `version`, `sse` |

Together with `#[set_metadata]` or a `CustomDecorator`, this lets you build your own "discovery" over routes. For example, listing the routes marked as public:

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

Other application introspection functions: `global_prefix()`, `global_prefix_exclude()`, `versioning_options()`, `effective_route_version(..)`, `resolve_route_paths(..)`, and `resolve_route_paths_with(.., global_prefix)`.

## Getting instances

To reach a specific instance:

- `app.get::<T>()` and `app.get_token::<T>("TOKEN")` look in the container (see [Module reference](/fundamentals/module-ref)).
- `app.get_instance::<T>(token)` returns `Option<Arc<T>>`, looking for the instance across the whole application.
- A `ModuleRef` field in a provider gives the same access from the inside.

## What is missing

- The **providers** of a module cannot be enumerated: `ModuleInfo` only lists controllers.
- There is no way to walk a provider's methods or to read metadata from methods that are not routes.
- The types the internal discovery uses (`corvis::discovery::DiscoveredHandler`, `HandlerKind`) are public, but the function that collects them is not.

In the meantime, the practical alternative is for your library to define a trait and for the interested providers to register themselves in an `on_module_init` hook, injecting a shared registry:

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
