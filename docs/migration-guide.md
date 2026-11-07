---
status: documentado
---

# Guía de migración

Corvis está en su primera versión (`0.1.0`), así que todavía no hay cambios incompatibles entre versiones que documentar. Cuando se publique una versión que los tenga, esta página explicará cómo actualizar.

Mientras tanto, esta guía sirve a quien llega desde un framework de Node.js con TypeScript y decoradores organizado en módulos, controladores y providers: el modelo es el mismo, así que la migración consiste sobre todo en traducir la sintaxis. Si tu punto de partida es una aplicación de Express o de Fastify sin esa estructura, empieza por [Migrar desde Express](/recipes/express-migration) o [Migrar desde Fastify](/recipes/fastify-migration).

## Equivalencias de estructura

| En TypeScript | En Corvis |
|---|---|
| Un módulo con `imports`, `controllers`, `providers` y `exports` | `#[module(imports = [...], controllers = [...], providers = [...], exports = [...])]` sobre un `struct` |
| Módulo global | `#[global]` encima de `#[module]`, o `#[module(global, ...)]` |
| Método estático que devuelve un módulo dinámico (`forRoot`) | `fn for_root(...) -> DynamicModule`, construido con `DynamicModule::new::<Self>()` |
| Clase marcada como inyectable | `#[injectable]` sobre un `struct` |
| Dependencia en el constructor | Campo `Arc<T>`: el constructor se genera solo |
| Dependencia opcional | Campo `Option<Arc<T>>` |
| Inyección por token | `#[inject("TOKEN")]` sobre el campo |
| Providers de valor, clase, factory y alias | `Provider::use_value`, `use_class`, `use_factory`, `use_existing` |
| Scope transitorio | `#[injectable(scope = "transient")]` |
| Guard, pipe, interceptor o filtro global registrado como provider | `Provider::app_guard::<G>()`, `app_pipe`, `app_interceptor`, `app_filter` |

Ver [Módulos](/modules), [Providers](/providers) y [Providers personalizados](/fundamentals/custom-providers).

## Equivalencias en controladores

| En TypeScript | En Corvis |
|---|---|
| Decorador de controlador con ruta | `#[controller("cats")]` |
| Decoradores de método HTTP | `#[get]`, `#[post(":id")]`, `#[put]`, `#[patch]`, `#[delete]`, `#[all]`, `#[options]`, `#[head]` dentro de un `#[routes] impl` |
| Código de estado, cabecera, redirección, versión | `#[http_code(204)]`, `#[header("Cache-Control", "none")]`, `#[redirect("url", 301)]`, `#[version("2")]` |
| Parámetros del handler | `#[body]`, `#[param("id", ParseIntPipe)]`, `#[query]`, `#[headers("authorization")]`, `#[ip]`, `#[req]`, `#[res]` |
| Decorador de parámetro propio | `impl FromContext for MiTipo` y el parámetro sin atributo |
| Metadata | `#[set_metadata("roles", ["admin"])]` |
| Guards, interceptors, pipes y filtros por ruta | `#[use_guards(...)]`, `#[use_interceptors(...)]`, `#[use_pipes(...)]`, `#[use_filters(...)]` |
| Lanzar una excepción HTTP | Devolver `Err(NotFoundException::new("..."))` en un `HttpResult` |

Los atributos de clase (`#[use_guards]`, `#[set_metadata]`...) van **debajo** de `#[controller]`, porque Rust procesa los atributos en orden.

## Equivalencias en el bootstrap

| En TypeScript | En Corvis |
|---|---|
| Crear la aplicación desde el módulo raíz | `CorvisFactory::create::<AppModule>().await?` |
| Prefijo global | `app.set_global_prefix("api")` |
| Pipes, guards, interceptors y filtros globales | `app.use_global_pipes(...)` y sus equivalentes |
| CORS | `app.enable_cors(CorsOptions::default())` |
| Versionado | `app.enable_versioning(VersioningOptions::uri().default_version("1"))` |
| Hooks de apagado | `app.enable_shutdown_hooks()` |
| Obtener un provider | `app.get::<CatsService>()?` |
| Escuchar | `app.listen(3000).await` |

## Diferencias que vale la pena conocer

- **Sin scope por petición.** Solo hay providers singleton y transient. Para datos por petición usa las extensiones de la request (`req.insert`, `req.get`) o `FromContext`. Ver [Scopes de inyección](/fundamentals/injection-scopes).
- **Sin `forwardRef`.** Una dependencia circular entre providers es un error al arrancar. Ver [Dependencias circulares](/fundamentals/circular-dependency).
- **Interceptors sin observables.** `CallHandler::handle()` devuelve un `Future`: puedes transformar, medir y cachear la respuesta, pero no componer operadores de flujos.
- **Pipes sobre JSON.** Los pipes reciben y devuelven `serde_json::Value`; el resultado se deserializa en el tipo del parámetro.
- **Enhancers globales y gateways.** Los guards, pipes e interceptors globales no se aplican a los gateways de WebSockets. Ver [Gateways](/websockets/gateways#enhancers-globales).
- **Sin GraphQL ni microservicios** por ahora.
