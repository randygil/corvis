---
status: documented
---

# Authorization

Authentication answers *who* is making the request; authorization decides *what* they can do. In
Corvis the tool for the second is the same as for the first: a [guard](/guards) that reads
route metadata with `Reflector` and compares it with the user that
[`corvis::auth::AuthGuard`](/security/authentication) left on the request. The samples on this page come from
`examples/auth-app`.

## Role-based access control

Define the roles and a metadata key. Since metadata is stored as JSON, any
`Serialize + Deserialize` type works as a value. The user is whatever your validators return; in the example, every
strategy returns an `Account` that carries its roles:

```rust
// examples/auth-app/src/users.rs
pub const ROLES_KEY: &str = "roles";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Admin,
}

// ...
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: u32,
    pub username: String,
    pub roles: Vec<Role>,
}
```

Mark routes with `#[set_metadata]`:

```rust
#[delete(":id")]
#[http_code(HttpStatus::NO_CONTENT)]
#[set_metadata(ROLES_KEY, [Role::Admin])]
fn remove(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<()> {
    self.cats_service.remove(id)
}
```

And write the guard:

```rust
// examples/auth-app/src/auth/roles.rs
#[injectable]
pub struct RolesGuard {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let required: Option<Vec<Role>> = self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
        let Some(required) = required else { return Ok(true) };
        let user = ctx.switch_to_http().get_request().user_as::<Account>();
        Ok(user.is_some_and(|u| required.iter().any(|r| u.roles.contains(r))))
    }

    // ...
    fn controls_access(&self) -> bool {
        false
    }
}
```

`get_all_and_override` looks for the key first on the handler and then on the class, and returns the
first value it finds; this way a method can override what its controller declares. If the
route declares no roles, the guard lets the request through, which is why it returns `false` from
`controls_access`: it is not what keeps anonymous requests out, so [route posture](/security/route-posture) must not
count it as access control. A guard written for HTTP reads the user with `Request::user_as::<T>()`; a handler reads it
with the `User<T>` parameter.

::: warning A roles guard needs an `AuthGuard` before it
`Request::user()` is not always what a guard established. The session middleware restores the user from the cookie on
**every** route, including routes with no `AuthGuard` and `#[public]` ones, before any guard runs. A roles guard that
reads the user on a route that has no `AuthGuard` before it is therefore authenticated by the cookie alone, with nothing
checked about the request itself (a cross-site form `POST` carries the cookie). Always list an `AuthGuard` that names the
credentials the route accepts before the roles guard, as the examples below do. See
[the session user](/security/authentication-threat-model#the-session-user-on-routes-without-an-authguard) and
[CSRF protection](/security/csrf).
:::

When a guard returns `Ok(false)`, the client receives:

```json
{ "message": "Forbidden resource", "error": "Forbidden", "statusCode": 403 }
```

If you want another status or message, return an error instead of `false`, for example
`Err(ForbiddenException::new("Admins only"))`.

## Registration and order

The roles guard needs the user that the authentication guard attaches, so it must run
after it. Apply both on the route or on the controller; in a list they run from left to right. The `AuthGuard`
comes from `corvis::auth` and names the strategies the route accepts:

```rust
// examples/auth-app/src/controllers.rs
    #[get("admin/users")]
    #[set_metadata(ROLES_KEY, [Role::Admin])]
    #[use_guards(AuthGuard::chain(["jwt", "session"]), RolesGuard)]
    fn admin_users(&self) -> Vec<Account> {
        self.users.accounts()
    }
```

An anonymous request gets `401` from the `AuthGuard`, and a signed-in user without the role gets `403` from the roles guard.

Mind the order when you register guards globally. Guards registered with `Provider::app_guard` run before
the ones passed to `app.use_global_guards(..)`, whatever the order of the calls, so a roles guard registered as
`Provider::app_guard::<RolesGuard>()` runs *before* a global `AuthGuard::new("jwt")` given to `use_global_guards` and
would see no user. Either apply the roles guard on the controllers and routes that need it (as above), or register the
authentication guard through the container too (a type that wraps the `AuthGuard` and is registered with
`Provider::app_guard`, listed before the roles guard, as `examples/cats-app` does).

The full order among global, controller and method guards is described in
[Request lifecycle](/faq/request-lifecycle).

## Grouping metadata in a decorator

To avoid repeating `#[set_metadata(ROLES_KEY, ...)]`, group the metadata in a `CustomDecorator` and
apply it with `#[apply_decorators]`:

```rust
use corvis::CustomDecorator;

fn admin_only() -> CustomDecorator {
    CustomDecorator::new().set_metadata(ROLES_KEY, [Role::Admin])
}

#[delete(":id")]
#[apply_decorators(admin_only())]
fn remove(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<()> {
    self.cats_service.remove(id)
}
```

A `CustomDecorator` only carries metadata: it cannot register guards or interceptors. The guard is still
applied with `#[use_guards]` or globally.

## Per-resource rules

When the decision depends on the resource (for example, "only the author can edit their post"), the guard
does not always have the necessary information. There are two approaches:

- Read the route parameter in the guard (`ctx.request().param("id")`) and query a service
  injected into the guard itself.
- Do the check in the service and return `ForbiddenException` from there.

```rust
pub async fn update(&self, id: u32, user: &JwtPayload, dto: UpdatePostDto) -> HttpResult<Post> {
    let post = self.find_one(id).await?;
    if post.author_id != user.sub && !user.roles.contains(&Role::Admin) {
        return Err(ForbiddenException::new("You can only edit your own posts"));
    }
    // ...
}
```

## Authorization in WebSockets

Guards also work in a gateway's handlers. `RolesGuard` in `examples/tertulia`
gets the user from the HTTP request or from the WebSocket client depending on `ctx.get_type()`, so
the same rule protects a REST endpoint and a message. (`corvis::auth::AuthGuard` denies WebSocket messages, so a gateway
authenticates in `handle_connection` and a guard of your own reads the user stored on the client; see
[Authentication](/security/authentication#websockets).)

```rust
#[subscribe_message("clear")]
#[set_metadata(ROLES_KEY, [MODERATOR])]
#[use_guards(RolesGuard, RoomMemberGuard)]
async fn clear(&self, #[message_body] dto: RoomDto, #[connected_socket] client: WsClient) -> HttpResult<()> {
    // ...
}
```

Global guards also run on gateway messages; mark public gateways with metadata or check `ctx.get_type()`. See
[WebSocket guards](/websockets/guards).

## What Corvis does not include

There is no policies or permissions module (CASL-style) and no rules language. Guards with
`Reflector` cover the cases of roles and *claims*; for more complex rules, encapsulate them in an
injectable service and call it from the guard.
