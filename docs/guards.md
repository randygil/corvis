---
status: documented
---

# Guards

A guard decides whether a request can reach the handler. It is the natural place for authentication and authorization: unlike a middleware, a guard runs **after** the route is chosen, so it knows which handler and which controller are about to run and can read their metadata.

## The `CanActivate` trait

```rust
#[async_trait]
pub trait CanActivate: Send + Sync + 'static {
    async fn can_activate(&self, context: &ExecutionContext) -> Result<bool, HttpException>;
}
```

| Result | Effect |
|---|---|
| `Ok(true)` | The request continues: next guard, interceptors, pipes, and handler |
| `Ok(false)` | It is cut short with `403` `{"message":"Forbidden resource","error":"Forbidden","statusCode":403}` |
| `Err(exception)` | It is cut short with that exception, for example `UnauthorizedException` |

The `ExecutionContext` gives access to the request (`context.switch_to_http().get_request()`), to the handler and the class (`get_handler()`, `get_class()`), and to the transport type (`get_type()`). See [Execution context](/fundamentals/execution-context).

## An authentication guard

This guard from `examples/tertulia` verifies a JWT and leaves the user in the request extensions for handlers to use:

```rust
use corvis::prelude::*;
use corvis::{ContextType, JwtService, WsException, extract_bearer_token};

#[injectable]
pub struct AuthGuard {
    jwt: Arc<JwtService>,
}

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        if ctx.get_type() == ContextType::Ws {
            return match ChatUser::of(ctx) {
                Some(_) => Ok(true),
                None => Err(WsException::new("Unauthorized")),
            };
        }
        let request = ctx.switch_to_http().get_request();
        let token = extract_bearer_token(request).ok_or_else(|| UnauthorizedException::new("Missing bearer token"))?;
        let user: ChatUser = self.jwt.verify_or_unauthorized(&token)?;
        request.insert(user);
        Ok(true)
    }
}
```

- A guard is an `#[injectable]`, so it receives dependencies like any provider (`jwt: Arc<JwtService>`).
- `request.insert(value)` stores a piece of data on the request; the handler retrieves it with `req.get::<T>()` or, more conveniently, with a `FromContext` parameter (see [Custom decorators](/custom-decorators)).
- The same guard works for HTTP and for WebSockets by checking `ctx.get_type()`.

## Applying guards

```rust
#[get("me")]
#[use_guards(AuthGuard)]
fn me(&self, user: ChatUser) -> ChatUser {
    user
}
```

| Scope | Form |
|---|---|
| One route | `#[use_guards(AuthGuard)]` on the method |
| One controller | `#[use_guards(AuthGuard)]` below `#[controller]` |
| Several at once | `#[use_guards(AuthGuard, RolesGuard)]` (they run in that order) |
| Global, instance | `app.use_global_guards(MyGuard)` |
| Global, with DI | `Provider::app_guard::<AuthGuard>()` in a module's `providers` |

In `#[use_guards(...)]` you can pass a type or an expression. A type is resolved with dependency injection **in the controller's module**: if it is in `providers`, that instance is used; if not, Corvis builds it and caches it for that module. In both cases its dependencies must be visible from that module. An expression (`#[use_guards(RolesGuard::new("admin"))]`) builds an instance without injection.

`app.use_global_guards` also takes an instance. For a global guard that needs providers, register it with `Provider::app_guard`:

```rust
#[module(
    imports = [/* ... */ AuthModule, UsersModule, CatsModule],
    controllers = [AppController],
    providers = [
        AppService,
        // Global guards with dependency injection.
        Provider::app_guard::<ThrottlerGuard>(),
        Provider::app_guard::<AuthGuard>(),
        Provider::app_guard::<RolesGuard>(),
    ],
)]
pub struct AppModule;
```

(`examples/cats-app/src/app_module.rs`)

### Order

For each route, guards run as follows:

1. Globals registered as a provider (`app_guard`), in the order they appear.
2. Globals from `use_global_guards`.
3. The controller's.
4. The route's.

The first one that returns `false` or an error cuts the chain short. Guards run after all middleware and before interceptors.

::: warning What global guards do not cover
`use_global_guards` and `Provider::app_guard` protect the controllers' routes. They do not apply to WebSocket gateways (declare them on the gateway), to the `upgrade` listeners of `corvis::ws`, to static files, or to the Swagger or Scalar routes. A static file with the same path as a protected endpoint is served before the endpoint. Protect those surfaces separately (middleware, `verify_client`, a different static root).
:::

## Role-based authorization with metadata

A guard can read metadata that the handler or the controller declares with `#[set_metadata(key, value)]`. Reading is done with `Reflector`, which is injected like any provider:

```rust
pub const ROLES_KEY: &str = "roles";

#[injectable]
pub struct RolesGuard {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let roles: Option<Vec<String>> = self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
        let Some(roles) = roles else { return Ok(true) };
        Ok(ChatUser::of(ctx).is_some_and(|u| roles.contains(&u.role)))
    }
}
```

```rust
#[post(":room/announcements")]
#[set_metadata(ROLES_KEY, [MODERATOR])]
#[use_guards(AuthGuard, RolesGuard)]
async fn announce(&self, #[param("room")] room: String, #[body] dto: AnnouncementDto, user: ChatUser) -> HttpResult<Value> {
    ...
}
```

- `get_all_and_override(key, &[handler, class])` returns the first defined value: the handler's takes priority over the class's.
- The value is deserialized into the type you ask for (`Vec<String>`, an `enum` with `serde`, `bool`, ...). If it doesn't exist or doesn't fit, it returns `None`.
- The other `Reflector` methods (`get`, `get_all`, `get_all_and_merge`) are described in [Execution context](/fundamentals/execution-context).

## Public routes with a global guard

`examples/cats-app` installs `AuthGuard` as a global guard and marks public routes with a metadata entry:

```rust
pub const IS_PUBLIC_KEY: &str = "isPublic";

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let is_public: bool =
            self.reflector.get_all_and_override(IS_PUBLIC_KEY, &[ctx.get_handler(), ctx.get_class()]).unwrap_or(false);
        if is_public {
            return Ok(true);
        }
        let request = ctx.switch_to_http().get_request();
        let token = extract_bearer_token(request).ok_or(UnauthorizedException)?;
        let payload: JwtPayload = self.jwt_service.verify_or_unauthorized(&token)?;
        request.set_user(&payload);
        Ok(true)
    }
}
```

```rust
#[get(":id")]
#[set_metadata(IS_PUBLIC_KEY, true)]
fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> { ... }
```

`request.set_user(..)` stores the payload as JSON; it is read with `request.user_as::<JwtPayload>()`.

## Guards in tests

`Test::create_testing_module().override_guard::<RolesGuard>(AllowAll)` replaces a guard with a test double. See [Testing](/fundamentals/testing).
