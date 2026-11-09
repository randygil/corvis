---
status: partial
---

# Injection scopes

A provider's scope decides how many instances the container creates and when. Corvis has three:

| Scope | Instances | When it is created |
|---|---|---|
| `Scope::Default` (singleton) | One per module that registers the provider, shared by all its consumers. | When the application starts. |
| `Scope::Transient` | A new one for each consumer that injects it. | Every time it is resolved. |
| `Scope::Request` | At most one per HTTP request and per module that registers the provider. | The first time it is resolved during a request. |

## Singleton (default)

It is the scope of every provider that doesn't say otherwise and the right one in the vast majority of cases: stateless services, database clients, caches, configuration. Since the instance is shared and concurrent, its mutable state goes behind a `Mutex`, `RwLock`, or atomics.

## Transient

Declare a transient provider in the attribute:

```rust
#[injectable(scope = "transient")]
pub struct OperationTimer {
    #[init(std::time::Instant::now())]
    started: std::time::Instant,
}
```

or when registering it, on any `Provider`:

```rust
providers = [Provider::use_class::<OperationTimer>("TIMER").scope(Scope::Transient)]
```

Each injection point (every `Arc<OperationTimer>` field, even two in the same struct) receives its own instance, and `ModuleRef::resolve::<OperationTimer>()` creates a new one on each call.

Consequences worth knowing:

- **They are not created at startup.** The container does not instantiate transient providers in advance; it builds them when injecting them. Their dependencies are checked at startup, though: a transient class provider (or an alias of one) that asks for something its module cannot see fails startup with the usual "can't resolve dependencies" error, even if nothing injects it yet. Optional (`Option<Arc<T>>`) fields are skipped, and factory providers are not checked because their dependencies are code: the error of a transient factory appears the first time it is resolved. Prefer class providers when early validation matters.
- **They are not stored.** That is why `ModuleRef::get::<T>()` and `app.get::<T>()`, which only return existing instances, don't find a transient provider. Use `module_ref.resolve::<T>().await`.
- **They don't receive lifecycle hooks.** Hooks (`on_module_init`, `on_application_bootstrap`, the shutdown ones) run on the stored instances, so a transient provider doesn't receive them.
- **The scope doesn't propagate.** A singleton that depends on a transient receives a single instance when it is created and keeps it.

## Request scope

Declare a request-scoped provider in the attribute:

```rust
#[injectable(scope = "request")]
pub struct RequestContext {
    req: Request,            // the request being handled
    clock: Arc<Clock>,       // a singleton, as usual
}
```

or when registering it: `Provider::use_class::<RequestContext>("CTX").scope(Scope::Request)`. The attribute accepts
exactly `"default"`, `"transient"` and `"request"` (or the `Scope::*` paths); anything else is a compile error.

The instance is created the first time something resolves it during a request, and every later resolution in that
request (from a middleware, a guard, the handler, or another request-scoped provider) returns the same one. Two
requests never share an instance, even when they run at the same time. When the request ends (response sent, error,
panic, or the client disconnecting) the instances are released.

### `Scoped<T>`

You consume a request-scoped provider per request. In a handler, declare a `Scoped<T>` parameter, with no attribute:

```rust
#[controller("cats")]
pub struct CatsController;

#[routes]
impl CatsController {
    #[get("request-id")]
    #[public]
    fn request_id(&self, ctx: Scoped<RequestContext>) -> Value {
        json!({ "requestId": ctx.request_id() })
    }
}
```

`Scoped<T>` dereferences to `T` (`into_inner()` gives the `Arc<T>`) and resolves the provider registered under
`Token::of::<T>()` from the module of the route's controller, with the usual visibility rules. It works for a provider
of **any** scope: a request-scoped one is the instance of this request, a transient one is a new instance, a singleton
is the shared one. Changing a provider's scope does not change its consumers, but the name says "resolved for this
request", not "T is request-scoped".

Guards, interceptors and route-level exception filters take it from their context, and get the same instance as the
handler:

```rust
#[async_trait]
impl CorvisInterceptor for RequestIdInterceptor {
    async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        let request_context = Scoped::<RequestContext>::from_context(ctx).await?;
        let mut response = next.handle().await?;
        response.set_header("X-Request-Id", request_context.request_id());
        Ok(response)
    }
}
```

The `cats-app` example does exactly this: a request-scoped `RequestContext` (id from `X-Request-Id` or generated), a
handler with `Scoped<RequestContext>` and an interceptor that echoes the id.

Errors do not reach the client with internals: if a factory provider fails with an `HttpException` (for example a `400`
for a missing tenant header), that exception is returned unchanged; any other failure is a generic `500`, and the
`DiError` message, which names the provider, is logged under `Scoped`. In a WebSocket handler `Scoped<T>` always fails
(request scope is HTTP only). Pipes cannot use it, because they receive no execution context, and `Scoped` resolves by
type: for a provider registered under a string token use `ModuleRef::resolve_token_for`.

### `ModuleRef::resolve_for`

When you need the instance outside a handler parameter, inject a `ModuleRef` and pass it the `Request`:

```rust
#[controller("cats")]
pub struct CatsController {
    module_ref: ModuleRef,
}

#[routes]
impl CatsController {
    #[get("me")]
    async fn me(&self, #[req] req: Request) -> String {
        let ctx = self.module_ref.resolve_for::<RequestContext>(&req).await.unwrap();
        ctx.user_id()
    }
}
```

`resolve_for::<T>(&req)` works for a provider of any scope and follows the usual visibility rules (only a
request-scoped provider needs a request that is being dispatched; the others resolve as with `resolve`);
`resolve_token_for::<T>("TOKEN", &req)` does the same for a custom token. A factory provider reads the request with
`ctx.request()?`:

```rust
Provider::use_factory("TENANT", |ctx| async move {
    Ok(Tenant(ctx.request()?.header("x-tenant").unwrap_or("none").to_string()))
})
.scope(Scope::Request)
```

What may depend on what:

- A request-scoped provider may depend on singletons, transient providers and other request-scoped providers, and may
  have a `Request` field. A transient provider built for it may do the same.
- A **singleton, a controller or a gateway cannot depend on a request-scoped provider** or on a `Request` (directly,
  through an alias, or through transient providers). Corvis does not turn the consumer into a request-scoped one,
  which would rebuild a whole controller graph per request; it fails at startup with `DiError::RequestScopedDependency`,
  naming the consumer, the dependency and the chain, and pointing at `Scoped<T>` and `resolve_for`.
- Global enhancers (`Provider::app_guard` and its siblings) and enhancers referenced by type (`#[use_guards(T)]`)
  cannot be request-scoped, because they are created once. Resolve the request-scoped provider inside them with
  `ModuleRef::resolve_for`.
- The declared dependencies of request-scoped class providers are validated at startup like those of transient ones
  (missing dependency, cycles), without creating them. Factory providers are not checked.

Consequences worth knowing:

- **They are not created at startup, not stored, and receive no lifecycle hooks.** `ModuleRef::get::<T>()` and
  `app.get::<T>()` return `NotFound` for them.
- **Outside a request they cannot be resolved.** In a gateway message handler or handshake, a standalone application
  context, a lifecycle hook, a scheduled job, an event listener, a singleton factory, or a spawned task that outlives the
  response, resolving one fails with `DiError::OutsideRequest`, naming the provider. A `Request` you cloned and kept
  after the response no longer resolves anything either.
- **A provider first resolved in a middleware sees a request without route parameters.** Routing has not matched yet;
  the extensions are shared and always current, so a value a guard inserts later is visible to it.
- **A test override can hide a scope error.** `override_provider::<T>().use_value(..)` replaces a request-scoped provider
  with a singleton value, so a singleton consumer starts in the test and fails in production.
- **Applications without request-scoped providers pay nothing per request.**

## Per-request data

For plain data you do not need a provider: store it on the request itself. `Request` has a per-type extension store: a guard or a middleware does `req.insert(value)` and any later piece reads it with `req.get::<T>()`. This is what Tertulia's `AuthGuard` does with the authenticated user:

```rust
#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let request = ctx.switch_to_http().get_request();
        let token = extract_bearer_token(request).ok_or_else(|| UnauthorizedException::new("Missing bearer token"))?;
        let user: ChatUser = self.jwt.verify_or_unauthorized(&token)?;
        request.insert(user);
        Ok(true)
    }
}
```

To receive that data as a handler parameter, implement `FromContext` (see [Custom decorators](/custom-decorators)):

```rust
#[async_trait]
impl FromContext for ChatUser {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        ChatUser::of(ctx).ok_or_else(|| UnauthorizedException::new("Unauthorized"))
    }
}

#[get("me")]
#[use_guards(AuthGuard)]
fn me(&self, user: ChatUser) -> ChatUser {
    user
}
```

In WebSocket gateways, the equivalent is the client store: `client.insert(value)` and `client.get::<T>()` (see [Gateways](/websockets/gateways)).

## What is not available

- Durable providers and context identification strategies.
- Automatic scope propagation to consumers: a singleton that depends on a request-scoped provider is a startup error.
- `INQUIRER`: a transient provider cannot ask which class injected it.
