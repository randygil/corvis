---
status: documented
---

# Authorization

Authentication answers *who* is making the request; authorization decides *what* they can do. In
Corvis the tool for the second is the same as for the first: a [guard](/guards) that reads
route metadata with `Reflector` and compares it with the user data left by the authentication
guard.

## Role-based access control

Define the roles and a metadata key. Since metadata is stored as JSON, any
`Serialize + Deserialize` type works as a value:

```rust
use serde::{Deserialize, Serialize};

pub const ROLES_KEY: &str = "roles";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtPayload {
    pub sub: u32,
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
#[injectable]
pub struct RolesGuard {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let required: Option<Vec<Role>> =
            self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
        let Some(required) = required else { return Ok(true) };
        let user = ctx.switch_to_http().get_request().user_as::<JwtPayload>();
        Ok(user.map(|u| required.iter().any(|r| u.roles.contains(r))).unwrap_or(false))
    }
}
```

`get_all_and_override` looks for the key first on the handler and then on the class, and returns the
first value it finds; this way a method can override what its controller declares. If the
route declares no roles, the guard lets the request through.

When a guard returns `Ok(false)`, the client receives:

```json
{ "message": "Forbidden resource", "error": "Forbidden", "statusCode": 403 }
```

If you want another status or message, return an error instead of `false`, for example
`Err(ForbiddenException::new("Admins only"))`.

## Registration and order

The roles guard needs the user that the authentication guard attaches, so it must run
after it. Guards registered with `Provider::app_guard` run in the order in which they appear in
the modules:

```rust
#[module(
    imports = [AuthModule, UsersModule, CatsModule],
    providers = [
        Provider::app_guard::<ThrottlerGuard>(),
        Provider::app_guard::<AuthGuard>(),
        Provider::app_guard::<RolesGuard>(),
    ],
)]
pub struct AppModule;
```

You can also apply them only where needed, on the controller or on the method. In a list, they
run from left to right:

```rust
#[post(":room/announcements")]
#[set_metadata(ROLES_KEY, [MODERATOR])]
#[use_guards(AuthGuard, RolesGuard)]
async fn announce(&self, #[param("room")] room: String, #[body] dto: AnnouncementDto, user: ChatUser) -> HttpResult<Value> {
    // ...
}
```

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
the same rule protects a REST endpoint and a message:

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
