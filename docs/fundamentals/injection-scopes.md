---
status: partial
---

# Injection scopes

A provider's scope decides how many instances the container creates and when. Corvis has two:

| Scope | Instances | When it is created |
|---|---|---|
| `Scope::Default` (singleton) | One per module that registers the provider, shared by all its consumers. | When the application starts. |
| `Scope::Transient` | A new one for each consumer that injects it. | Every time it is resolved. |

There is no request scope. Below you will see how to carry request data without it.

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

Each provider that has an `Arc<OperationTimer>` field receives its own instance, and `ModuleRef::resolve::<OperationTimer>()` creates a new one on each call.

Consequences worth knowing:

- **They are not created at startup.** The container does not instantiate transient providers in advance; it builds them when injecting them.
- **They are not stored.** That is why `ModuleRef::get::<T>()` and `app.get::<T>()`, which only return existing instances, don't find a transient provider. Use `module_ref.resolve::<T>().await`.
- **They don't receive lifecycle hooks.** Hooks (`on_module_init`, `on_application_bootstrap`, the shutdown ones) run on the stored instances, so a transient provider doesn't receive them.
- **The scope doesn't propagate.** A singleton that depends on a transient receives a single instance when it is created and keeps it.

## Per-request data

Asking for `#[injectable(scope = "request")]` is a compile error:

```text
request-scoped providers are not supported; use transient or default scope
```

The alternative is to store the request's data on the request itself. `Request` has a per-type extension store: a guard or a middleware does `req.insert(value)` and any later piece reads it with `req.get::<T>()`. This is what Tertulia's `AuthGuard` does with the authenticated user:

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

- The `REQUEST` scope and injecting the current request into a provider.
- Durable providers and context identification strategies.
- Automatic scope propagation to consumers.
