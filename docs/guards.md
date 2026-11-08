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
`use_global_guards` and `Provider::app_guard` protect the controllers' routes. They also run on WebSocket gateway messages (see [WebSocket guards](/websockets/guards)), but not on the `upgrade` listeners of `corvis::ws`, to static files, or to the Swagger or Scalar routes. A static file with the same path as a protected endpoint is served before the endpoint. Protect those surfaces separately (middleware, `verify_client`, a different static root).
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

A typical setup installs an `AuthGuard` as a global guard (`examples/cats-app` does). Public routes are marked with `#[public]`, which stores the metadata entry `PUBLIC_METADATA` (`"isPublic"`) with the value `true`. The guard reads it with `Reflector`:

```rust
use corvis::PUBLIC_METADATA;

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let is_public: bool =
            self.reflector.get_all_and_override(PUBLIC_METADATA, &[ctx.get_handler(), ctx.get_class()]).unwrap_or(false);
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
#[public]
fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> { ... }
```

`request.set_user(..)` stores the payload as JSON; it is read with `request.user_as::<JwtPayload>()`.

`#[public]` goes on a route method inside `#[routes]` or below `#[controller]` (the whole controller is public). It takes no arguments and does not change how requests are handled: it only stores the metadata entry. A route's value wins over the controller's, so `#[set_metadata(PUBLIC_METADATA, false)]` on a route makes it private again inside a public controller. Applications that already mark routes with `#[set_metadata(IS_PUBLIC_KEY, true)]` and `IS_PUBLIC_KEY = "isPublic"` keep working: it is the same key.

## Require a guard on every route

Forgetting `#[use_guards(...)]` is silent: the route works and is open to everyone. Ask the application to check at startup that every controller route has a guard, is marked `#[public]` or is exempt:

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.require_route_posture();
app.listen(3000).await?;
```

In this mode `init()` (so `listen()` too) fails before the server binds and before any module hook runs, with an error that lists every route that has no guard:

```text
Route posture check failed: 2 routes have no guard and are not marked #[public].
  GET   /api/rooms                 RoomsController::list
  POST  /api/rooms/:room/messages  RoomsController::post
Add #[use_guards(...)] to the route or its controller (or register a global guard), mark it #[public] if it is meant to be open, or add an exemption with a reason. Guards that do not control access (ThrottlerGuard, ZodGuard) do not count.
```

Guards count at every level the router runs them: `use_global_guards`, `Provider::app_guard`, the controller and the route. Each path alias and URI version of a route is checked on its own.

To adopt it step by step, use warn mode, which logs the same list once and starts normally. Routes that are protected outside the framework (a reverse proxy, a private network) can be exempted, always with a reason:

```rust
use corvis::{RequestMethod, RoutePostureOptions};

app.set_route_posture(
    RoutePostureOptions::warn()
        .exempt(RequestMethod::Get, "/metrics", "scraped from the private network only"),
);
```

- The exemption pattern is compared literally with the route's full path template (global prefix and version included), segment by segment: a literal segment matches the same literal (ignoring case), `:id` or `{id}` matches only a `:param` segment (so `/files/:id` does not exempt `/files/export-all`), `*` as a whole segment is exactly one segment (`/api/*/internal/*`), `**` is zero or more segments (`/metrics/**`), and `*` inside a segment (`/health*`) never crosses a `/`. `RequestMethod::All` matches any method; a route declared with `#[all]` is matched only by an `All` exemption.
- A blank reason makes `init()` fail. An exemption that matches no route logs a warning, and so does one that matches only routes that already have a guard or are public (it would silently accept them if the guard were removed).
- A guard that only limits or validates requests does not decide who may call a route. `ThrottlerGuard` and `ZodGuard` do not count. Your own guard counts by default; override `controls_access()` to return `false` when it lets every request through unless configured (a roles guard without roles, for example). Only the declaration changes, not how the guard runs.
- `app.route_posture()` returns, after `init()` and in every mode (also off), the posture of every route: method, path, controller and handler, guards, and whether it is protected, public (and whether the marker is on the handler or the controller), exempt (with its reason) or unprotected. It also lists the guards that counted as access control (`access_guards()`) and the redundant exemptions. It implements `Display` (a table) and `serde::Serialize`.
- Call `require_route_posture()` or `set_route_posture()` before `init()` or `listen()`; they have no effect afterwards.
- `#[sse]` routes are checked like any other route. Gateway message handlers are not checked yet.

Counting a guard is a declaration, not proof that it authenticates: the check cannot tell a real authentication guard from one that always returns `true`.

### Pass-through global guards

The risk is a global guard that lets everything through when it has nothing to check. A roles guard registered with `Provider::app_guard::<RolesGuard>()` that returns `Ok(true)` when the route has no `roles` metadata is the typical case; so are tenant, CSRF, maintenance-mode and feature-flag guards. By default it counts as access control, so every route looks protected and the check passes although nobody is authenticated. Override `controls_access()` to return `false` in every guard that does not deny by default:

```rust
#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> { ... }

    fn controls_access(&self) -> bool {
        false
    }
}
```

To spot the case, in warn and enforce mode the check logs one line with the guards it counted, their scope and how many routes each covers, for example `Route posture: access guards counted: RolesGuard (global, 42 routes), AuthGuard (controller CatsController, 5 routes)`. A global guard that covers every route and is not your authentication guard is the one to review. The same data is in `app.route_posture().unwrap().access_guards()`.

### What the check does not see

The check covers controller routes only. These HTTP surfaces are outside it, so protect them on their own:

- the Swagger and Scalar UI and their JSON document (they are mounted as middleware);
- `ServeStaticModule` and `serve_static` (static files);
- any middleware from `use_fn` or `configure` that answers without calling `next`;
- raw `upgrade` listeners and WebSocket handshakes (gateways are covered by a later version).

## Guards in tests

`Test::create_testing_module().override_guard::<RolesGuard>(AllowAll)` replaces a guard with a test double. See [Testing](/fundamentals/testing).
