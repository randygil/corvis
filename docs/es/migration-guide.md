---
status: documented
---

# Guía de migración

Corvis está en su primera versión (`0.1.0`), así que todavía no hay cambios incompatibles entre versiones que documentar. Cuando se publique una versión que los tenga, esta página explicará cómo actualizar.

Mientras tanto, esta guía sirve a quien llega desde un framework de Node.js con TypeScript y decoradores organizado en módulos, controladores y providers: el modelo es el mismo, así que la migración consiste sobre todo en traducir la sintaxis. Si tu punto de partida es una aplicación de Express o de Fastify sin esa estructura, empieza por [Migrar desde Express](/es/recipes/express-migration) o [Migrar desde Fastify](/es/recipes/fastify-migration).

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
| `Scope.REQUEST` | `#[injectable(scope = "request")]`, consumido con `Scoped<T>` o `ModuleRef::resolve_for` |
| `@Inject(REQUEST)` | un campo `Request` en un provider con scope por petición (`ResolveCtx::request()` en una factory) |
| Guard, pipe, interceptor o filtro global registrado como provider | `Provider::app_guard::<G>()`, `app_pipe`, `app_interceptor`, `app_filter` |

Ver [Módulos](/es/modules), [Providers](/es/providers) y [Providers personalizados](/es/fundamentals/custom-providers).

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
| Una estrategia de Passport y su guard (`AuthGuard('jwt')`) | Un validador, `Provider::strategy::<JwtStrategy<V>>()` y `#[use_guards(AuthGuard::new("jwt"))]`; ver [Migrar desde Passport](/es/recipes/passport-migration) |

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

- **Scope por petición sin propagación.** `#[injectable(scope = "request")]` da una instancia por petición, pero un singleton, controller o gateway que depende de ella es un error de arranque en lugar de volverse él mismo por petición: recibe un parámetro `Scoped<T>` en el handler (o usa `ModuleRef::resolve_for`) en cada petición. Para datos simples por petición también puedes usar las extensiones de la request (`req.insert`, `req.get`) o `FromContext`. Ver [Scopes de inyección](/es/fundamentals/injection-scopes).
- **Sin `forwardRef`.** Una dependencia circular entre providers es un error al arrancar. Ver [Dependencias circulares](/es/fundamentals/circular-dependency).
- **Interceptors sin observables.** `CallHandler::handle()` devuelve un `Future`: puedes transformar, medir y cachear la respuesta, pero no componer operadores de flujos.
- **Pipes sobre JSON.** Los pipes reciben y devuelven `serde_json::Value`; el resultado se deserializa en el tipo del parámetro.
- **Enhancers globales y gateways.** Como en el original, los guards, pipes e interceptors globales se aplican a los handlers de los gateways de WebSockets, y los filtros de excepción globales no. Es un cambio de comportamiento respecto a versiones anteriores de Corvis, donde un guard global protegía solo las rutas HTTP: marca los gateways públicos con metadatos, comprueba `ctx.get_type()` en el guard o llama a `app.apply_global_enhancers_to_gateways(false)`. `#[use_filters]` funciona en gateways y handlers. Ver [Gateways](/es/websockets/gateways#enhancers-globales).
- **Orden de los filtros dentro de un `#[use_filters]`.** Como en el original, el último filtro de un mismo `#[use_filters(A, B)]` se prueba primero, en rutas, controladores, gateways y handlers de mensajes. Las versiones anteriores de Corvis probaban `A` primero. Si dos filtros de un mismo atributo atrapan la misma excepción, ahora responde el otro: intercambia los nombres para conservar el comportamiento anterior. Los `#[use_filters]` apilados mantienen primero el de arriba. Ver [Filtros de excepción](/es/exception-filters#que-filtro-responde).
- **Un handler por evento en un gateway.** Con dos handlers para el mismo mensaje, el adaptador `ws` del original conserva el último y su adaptador de socket.io los ejecuta todos, en ambos casos sin avisar. Corvis se niega a arrancar, así que un handler con guard nunca queda tapado por uno sin guard. Elimina el handler muerto o fusiona los dos en uno. Ver [Gateways](/es/websockets/gateways#handlers-y-subscribe-message).
- **Los resultados `null` y `undefined` responden un cuerpo vacío.** Como en el original, un handler sin valor que devolver (`None`, `Value::Null`, un `Serialize` cuyo JSON es `null`) responde el estado por defecto con cuerpo vacío, sin `Content-Type` ni `ETag`. Es un cambio de comportamiento respecto a versiones anteriores de Corvis, que respondían el texto `null` para un `Option<Dto>`. Devuelve `Json(None::<T>)` si un cliente necesita un `null` literal. Ver [Controladores](/es/controllers#respuestas).
- **Sin GraphQL ni microservicios** por ahora.
