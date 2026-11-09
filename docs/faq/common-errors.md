---
status: documented
---

# Common errors

This page gathers the errors that show up most often when starting with Corvis, with their exact message and
how to resolve them.

## "Corvis can't resolve dependencies of the ..."

```text
Corvis can't resolve dependencies of the CatsService (?, ConfigService). Please make sure that the
argument API_KEY at index [0] is available in the CatsModule context.
```

It appears at startup, when a provider or controller asks for something its module cannot see. The `?` marks the
missing argument, and the message names the token (`API_KEY` or the type name) and the module.

Common causes:

- The provider is in another module that does not **export** it, or that module is not in the `imports` of the
  one asking for it. A module only sees its own providers, the `exports` of its `imports` and those of the
  global modules. See [Modules](/modules).
- A global module (`#[module(global, ...)]`) does not export the provider: global modules also have to
  export what they share.
- You asked for `Arc<T>` of a type that is not registered in the `providers` of any module.
- The token in `#[inject("TOKEN")]` does not match the one in `Provider::use_value("TOKEN", ...)`.
- The provider is transient and nothing injects it yet: the dependencies of transient providers are checked at startup,
  so the error appears even before the first resolution.

If the dependency is optional, declare it as `Option<Arc<T>>`: you will receive `None` instead of the error.

## "Corvis could not find X element"

```text
Corvis could not find CatsService element (this provider does not exist in the current context)
```

It is returned by a programmatic lookup (`app.get::<T>()`, `ModuleRef`, `ctx.get::<T>()` inside a
factory) when the token does not exist in the context being queried.

## Circular dependencies

```text
A circular dependency has been detected: AService -> BService -> AService. Break the cycle: move the shared
logic into a third provider, resolve one side lazily with ModuleRef (not in its constructor), or communicate
through EventEmitter. See "Circular dependencies" in the docs.
```

Two providers ask for each other (directly or indirectly). The chain can also pass through a `ModuleRef` call (`resolve` or `create`) made inside a constructor or factory. The cycle has to be broken (Corvis has no `forwardRef`). Options:

- Extract the shared logic into a third provider that both depend on.
- Connect one of the sides with events (`EventEmitter`) instead of a direct call.
- Resolve the dependency lazily with `ModuleRef` (injecting `module_ref: ModuleRef` and
  calling `self.module_ref.get::<T>()` when needed, not in the constructor).

See [Circular dependencies](/fundamentals/circular-dependency).

## "A circular import between dynamic modules has been detected"

```text
A circular import between dynamic modules has been detected: AModule -> BModule -> AModule. Each dynamic import
(for example for_root()) creates a new module instance, so this cycle never ends. Move what both modules need
into a separate module that both import, or remove one of the two imports.
```

Two modules import each other through `for_root()` (or any other `DynamicModule`), directly or through other modules. Every dynamic import creates a new instance, so the graph would never finish. Move what both modules need into a third module that both import, or remove one of the two imports. See [Dynamic modules are the exception](/fundamentals/circular-dependency#dynamic-modules-are-the-exception).

## "Corvis cannot export a provider/module that is not a part of the currently processed module"

A module lists in `exports` something that is not in its `providers` and that none of its
`imports` exports, or it re-exports a module it does not import. Add it to `providers` or import the
corresponding module.

## "The provider X was found but it is not of the requested type Y"

The token exists, but the registered value is of another type. This happens with string tokens: for example,
`Provider::use_value("PORT", 3000)` registers an `i32`, and a field `#[inject("PORT")] port: Arc<u16>`
asks for a `u16`. Use the same type on both sides (`Provider::use_value("PORT", 3000u16)`).

## "Error while instantiating X: ..."

A factory (`Provider::use_factory`, `register_async`, `for_root_async`) returned an error. The
message includes the cause. A typical case is a required environment variable that is missing, such as
`JWT_SECRET` in `examples/tertulia`.

## "X depends on Y, which is request-scoped"

```text
CatsController depends on RequestContext, which is request-scoped (CatsController -> AuditLog -> RequestContext). A singleton, a controller or a gateway is created once and would keep the instance of the first request. ...
```

A singleton provider, a controller or a gateway has a field (directly, through an alias or through a transient
provider) that is a request-scoped provider, or a `Request`. Corvis does not turn the consumer into a request-scoped
one: it stops at startup. Take a `ModuleRef` and resolve the provider per request with
`module_ref.resolve_for::<Y>(&req).await?`, or make the consumers in the chain request-scoped too. See
[Request scope](/fundamentals/injection-scopes#request-scope).

## "X is request-scoped but was resolved outside an HTTP request"

A request-scoped provider (or a `Request` field) was resolved where no HTTP request is being handled: a singleton
factory, a gateway message handler, a lifecycle hook, a scheduled job, an event listener, a standalone application
context, or a task that kept a `Request` after the response was sent. The message names the provider and the chain
that led to it. Resolve it inside the request with `ModuleRef::resolve_for(&req)`, or make it a singleton or transient
provider.

## "global guard X cannot be request-scoped"

Global enhancers (`Provider::app_guard`, `app_interceptor`, `app_pipe`, `app_filter`) and guards, interceptors, pipes,
filters and middleware referenced by type are created once at startup, so they cannot be request-scoped. Resolve the
request-scoped provider inside the enhancer with `ModuleRef::resolve_for(&req)`.

## Attribute compile errors

| Message | Cause |
|---|---|
| `#[get] must be used on a method inside a #[routes] impl block` | The controller's `impl` does not carry `#[routes]` |
| `#[use_guards] must be placed below #[controller] or on a method inside #[routes]` | Class attributes go **below** `#[controller]` (Rust processes attributes in order) |
| `unknown scope "requested"; expected scope = "default", scope = "transient" or scope = "request"` | `#[injectable(...)]` takes only `scope = "default"`, `"transient"` or `"request"`, spelled exactly. See [Injection scopes](/fundamentals/injection-scopes) |
| `unknown #[module] key; expected imports, controllers, providers, exports or global` | A misspelled key in `#[module(...)]` |
| `#[corvis::main] requires an async fn` | `async` is missing on `main` |
| `#[optional] has no effect: declare the field as Option<Arc<T>> to make the dependency optional` | `#[optional]` on a field. Write `cats: Option<Arc<CatsService>>` instead |
| `` `purge` has #[use_guards] but no route attribute (#[get], #[post], #[put], #[patch], #[delete], #[options], #[head], #[search], #[all] or #[sse]); add one or remove #[use_guards] `` | A method in a `#[routes]` block has an enhancer or another route attribute (`#[http_code]`, `#[set_metadata]`, `#[version]`, `#[api_*]`, ...) but no verb attribute, so it would be dropped. Add `#[get]`, `#[post]`, ... or remove the attribute |
| `` `kick` has #[use_guards] but no #[subscribe_message] attribute; enhancers and metadata in `#[handlers]` only apply to message handlers `` | A method in a `#[handlers]` block has `#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]` or `#[set_metadata]` but no `#[subscribe_message]`. Add it or remove the enhancer |

A field of an `#[injectable]` that is not `Arc<T>`, `Option<Arc<T>>` or `ModuleRef` is initialized with
`Default`. If its type does not implement `Default`, the compiler error mentions it: initialize it with
`#[init(expr)]`.

## "Route posture check failed: ..."

```text
Route posture check failed: 1 route has no guard and is not marked #[public].
  GET  /api/rooms  RoomsController::list
Add #[use_guards(...)] to the route or its controller (or register a global guard), mark it #[public] if it is meant to be open, or add an exemption with a reason. Guards that do not control access (ThrottlerGuard, ZodGuard) do not count.
```

The application called `require_route_posture()` and the listed routes, `SSE` streams or `WS` message handlers have no guard that controls access, are not public and match no exemption. It fails before the server binds. Every unprotected entry is listed, with its method, full path and `Controller::handler`.

Fix each entry in one of three ways:

- add `#[use_guards(...)]` to the route or its controller, or register a global guard (a rate limiter such as `ThrottlerGuard` does not count);
- mark it `#[public]` if it is meant to be open;
- if something outside the framework protects it, add an exemption with a reason.

See [Route posture](/security/route-posture), or run `corvis posture` to list every route (see [`corvis posture`](/cli/usages#corvis-posture)).

## The port is busy

```text
ERROR [CorvisApplication] listen EADDRINUSE / bind error on 0.0.0.0:3000: ...
```

Another process (often a previous run of the same application) is listening on that port. Stop it
or use another port.

## `404 Cannot GET /route`

```json
{ "message": "Cannot GET /cats", "error": "Not Found", "statusCode": 404 }
```

- Check the global prefix (`set_global_prefix("api")` moves everything to `/api/...`) and URI
  versioning (`/api/v1/...`). The `Mapped {...}` logs at startup show the actual routes.
- Check that the controller is in the `controllers` of a module reachable from the root module.
- With header or media type versioning, a request without a version only reaches routes without a version
  or `VERSION_NEUTRAL`. See [Versioning](/http/versioning).

## `413`, `415` and `400` from the body parser

| Response | Cause |
|---|---|
| `413 request entity too large` | The JSON or urlencoded body exceeds 100 kb. Raise it with `app.use_body_parser("json", bytes)` |
| `415 unsupported content encoding` | The body is compressed (`gzip`, `br`...): it is not supported |
| `415 unsupported charset "..."` | A `charset` other than UTF-8 (or latin1 in urlencoded) |
| `400` with a `JSON.parse` message | Malformed JSON, or nested more than 256 levels deep |

## The handler receives `null` in `#[body]`

The body was not parsed because the `Content-Type` is neither `application/json` nor
`application/x-www-form-urlencoded`. Check the client's header, or read the bytes with
`req.raw_body()` (see [Raw body](/faq/raw-body)).
