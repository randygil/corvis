---
status: documented
---

# Migration guide

Corvis is at its first version (`0.1.0`), so there are no breaking changes between versions to document yet. When a version that has them is released, this page will explain how to upgrade.

In the meantime, this guide is for those coming from a decorator-based Node.js framework written in TypeScript and organized into modules, controllers, and providers: the model is the same, so migration is mostly a matter of translating syntax. If your starting point is an Express or Fastify application without that structure, start with [Migrating from Express](/recipes/express-migration) or [Migrating from Fastify](/recipes/fastify-migration).

## Structure equivalences

| In TypeScript | In Corvis |
|---|---|
| A module with `imports`, `controllers`, `providers`, and `exports` | `#[module(imports = [...], controllers = [...], providers = [...], exports = [...])]` on a `struct` |
| Global module | `#[global]` above `#[module]`, or `#[module(global, ...)]` |
| Static method that returns a dynamic module (`forRoot`) | `fn for_root(...) -> DynamicModule`, built with `DynamicModule::new::<Self>()` |
| Class marked as injectable | `#[injectable]` on a `struct` |
| Constructor dependency | `Arc<T>` field: the constructor is generated for you |
| Optional dependency | `Option<Arc<T>>` field |
| Injection by token | `#[inject("TOKEN")]` on the field |
| Value, class, factory, and alias providers | `Provider::use_value`, `use_class`, `use_factory`, `use_existing` |
| Transient scope | `#[injectable(scope = "transient")]` |
| `Scope.REQUEST` | `#[injectable(scope = "request")]`, consumed with `Scoped<T>` or `ModuleRef::resolve_for` |
| `@Inject(REQUEST)` | a `Request` field in a request-scoped provider (`ResolveCtx::request()` in a factory) |
| Global guard, pipe, interceptor, or filter registered as a provider | `Provider::app_guard::<G>()`, `app_pipe`, `app_interceptor`, `app_filter` |

See [Modules](/modules), [Providers](/providers), and [Custom providers](/fundamentals/custom-providers).

## Controller equivalences

| In TypeScript | In Corvis |
|---|---|
| Controller decorator with a path | `#[controller("cats")]` |
| HTTP method decorators | `#[get]`, `#[post(":id")]`, `#[put]`, `#[patch]`, `#[delete]`, `#[all]`, `#[options]`, `#[head]` inside a `#[routes] impl` |
| Status code, header, redirect, version | `#[http_code(204)]`, `#[header("Cache-Control", "none")]`, `#[redirect("url", 301)]`, `#[version("2")]` |
| Handler parameters | `#[body]`, `#[param("id", ParseIntPipe)]`, `#[query]`, `#[headers("authorization")]`, `#[ip]`, `#[req]`, `#[res]` |
| Custom parameter decorator | `impl FromContext for MyType` and the parameter without an attribute |
| Metadata | `#[set_metadata("roles", ["admin"])]` |
| Per-route guards, interceptors, pipes, and filters | `#[use_guards(...)]`, `#[use_interceptors(...)]`, `#[use_pipes(...)]`, `#[use_filters(...)]` |
| Throwing an HTTP exception | Returning `Err(NotFoundException::new("..."))` in an `HttpResult` |
| A Passport strategy and its guard (`AuthGuard('jwt')`) | A validator, `Provider::strategy::<JwtStrategy<V>>()` and `#[use_guards(AuthGuard::new("jwt"))]`; see [Migrating from Passport](/recipes/passport-migration) |

Class attributes (`#[use_guards]`, `#[set_metadata]`...) go **below** `#[controller]`, because Rust processes attributes in order.

## Bootstrap equivalences

| In TypeScript | In Corvis |
|---|---|
| Creating the application from the root module | `CorvisFactory::create::<AppModule>().await?` |
| Global prefix | `app.set_global_prefix("api")` |
| Global pipes, guards, interceptors, and filters | `app.use_global_pipes(...)` and its equivalents |
| CORS | `app.enable_cors(CorsOptions::default())` |
| Versioning | `app.enable_versioning(VersioningOptions::uri().default_version("1"))` |
| Shutdown hooks | `app.enable_shutdown_hooks()` |
| Getting a provider | `app.get::<CatsService>()?` |
| Listening | `app.listen(3000).await` |

## Differences worth knowing

- **Request scope without bubbling.** `#[injectable(scope = "request")]` gives one instance per request, but a singleton, controller or gateway that depends on it is a startup error instead of becoming request-scoped itself: take a `Scoped<T>` handler parameter (or use `ModuleRef::resolve_for`) per request. For plain per-request data you can also use the request extensions (`req.insert`, `req.get`) or `FromContext`. See [Injection scopes](/fundamentals/injection-scopes).
- **No `forwardRef`.** A circular dependency between providers is a startup error. See [Circular dependencies](/fundamentals/circular-dependency).
- **Interceptors without observables.** `CallHandler::handle()` returns a `Future`: you can transform, time, and cache the response, but not compose stream operators.
- **Pipes over JSON.** Pipes receive and return `serde_json::Value`; the result is deserialized into the parameter's type.
- **Global enhancers and gateways.** As in the original, global guards, pipes, and interceptors apply to WebSocket gateway handlers, and global exception filters do not. This is a behavior change from earlier Corvis releases, where a global guard protected HTTP routes only: mark public gateways with metadata, check `ctx.get_type()` in the guard, or call `app.apply_global_enhancers_to_gateways(false)`. `#[use_filters]` works on gateways and handlers. See [Gateways](/websockets/gateways#global-enhancers).
- **Filter order inside one `#[use_filters]`.** As in the original, the last filter listed in one `#[use_filters(A, B)]` is tried first, on routes, controllers, gateways and message handlers. Earlier Corvis releases tried `A` first. If two filters in one attribute both catch the same exception, the other one answers now: swap the names to keep the old behavior. Stacked `#[use_filters]` attributes keep the upper one first. See [Exception filters](/exception-filters#which-filter-responds).
- **One handler per event in a gateway.** With two handlers for the same message, the original's `ws` adapter keeps the last one and its socket.io adapter runs all of them, silently in both cases. Corvis refuses to start instead, so a guarded handler can never be shadowed by an unguarded one. Remove the dead handler, or merge the two into one. See [Gateways](/websockets/gateways#handlers-and-subscribe-message).
- **`null` and `undefined` results answer an empty body.** As in the original, a handler with no value to return (`None`, `Value::Null`, a `Serialize` whose JSON is `null`) answers the default status with an empty body, no `Content-Type` and no `ETag`. This is a behavior change from earlier Corvis releases, which answered the text `null` for an `Option<Dto>`. Return `Json(None::<T>)` if a client needs a literal `null`. See [Controllers](/controllers#responses).
- **No GraphQL or microservices** for now.
