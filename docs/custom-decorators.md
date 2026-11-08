---
status: partial
---

# Custom decorators

Rust has no decorators; Corvis uses procedural attributes. You cannot create a new attribute without writing your own macro crate, but you can cover the usual uses of custom decorators with three tools:

| Need | Tool |
|---|---|
| A parameter computed from the request (`@User()`) | Implement `FromContext` for a type |
| Mark routes or controllers with a piece of data (`@Roles('admin')`, `@Public()`) | A constant + `#[set_metadata(KEY, value)]` |
| Group several metadata entries under one name | `CustomDecorator` + `#[apply_decorators(...)]` |

## Parameter decorators: `FromContext`

Every handler parameter **without** an attribute is resolved with the `FromContext` trait:

```rust
#[async_trait]
pub trait FromContext: Sized + Send {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException>;
}
```

Implementing it for your own type is the equivalent of a parameter decorator. This example from `examples/tertulia` returns the user that `AuthGuard` left on the request (or on the socket, if the context is a WebSocket one):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatUser {
    pub username: String,
    pub role: String,
}

impl ChatUser {
    /// The user of an HTTP request or a WebSocket message.
    pub fn of(ctx: &ExecutionContext) -> Option<ChatUser> {
        match ctx.get_type() {
            corvis::ContextType::Ws => ctx.switch_to_ws().get_client::<WsClient>().and_then(|c| c.get::<ChatUser>()),
            _ => ctx.request().get::<ChatUser>(),
        }
    }
}

#[async_trait]
impl FromContext for ChatUser {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        ChatUser::of(ctx).ok_or_else(|| UnauthorizedException::new("Unauthorized"))
    }
}
```

Usage in the controller, without any attribute:

```rust
#[post]
#[use_guards(AuthGuard)]
async fn create(&self, #[body] dto: CreateRoomDto, user: ChatUser) -> HttpResult<Room> {
    Ok(self.rooms.create(dto, &user.username).await?)
}
```

Details:

- If `from_context` returns `Err`, the request ends with that exception and the handler does not run.
- Declaring the parameter as `Option<ChatUser>` makes it optional: an error becomes `None`.
- A *newtype* gives the same data a different meaning: `pub struct CurrentUser(pub JwtPayload);` in `examples/cats-app/src/common/decorators.rs`.
- Corvis already ships implementations for `Request`, `Res`, `ExecutionContext`, `ModuleRef`, and `Option<T>`.
- The `#[req]`, `#[request]`, and `#[ctx]` attributes also resolve through `FromContext`; you can use them to document intent.

::: warning No pipes
Parameters resolved with `FromContext` do not go through pipes (neither global nor route-level). Validate inside `from_context` if you need to.
:::

### Reading data with arguments

A parameter decorator that takes an argument (for example, "give me only the `email` field of the user") has no direct equivalent: `FromContext` receives no parameters. Use one type per variant, or return the whole object and pick the field in the handler.

## Metadata: `#[set_metadata]`

To "mark" a handler or a controller, define a key and use it with `#[set_metadata]`. A guard or an interceptor reads it later with `Reflector`.

Corvis defines one marker for you: `#[public]` stores `PUBLIC_METADATA` (`"isPublic"`) with the value `true`, and the route posture check reads it (see [Route posture](/security/route-posture#marking-routes-public)). Your own keys work the same way with `#[set_metadata]`:

```rust
pub const ROLES_KEY: &str = "roles";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Admin,
}
```

```rust
#[get]
#[public]
fn find_all(&self, #[query] query: ListCatsQuery) -> Vec<Cat> { ... }

#[delete(":id")]
#[http_code(HttpStatus::NO_CONTENT)]
#[set_metadata(ROLES_KEY, [Role::Admin])]
fn remove(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<()> { ... }
```

(`examples/cats-app/src/cats/cats_controller.rs`)

The value can be anything that implements `Serialize`: it is stored as JSON. Whoever reads it chooses the type to deserialize it into:

```rust
let required: Option<Vec<Role>> =
    self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
```

`#[set_metadata]` can be used on a `#[routes]` method, below `#[controller]`, below `#[websocket_gateway]`, or on a `#[subscribe_message]`.

## Composing metadata: `CustomDecorator`

`CustomDecorator` is a reusable list of `(key, value)` pairs. It is built with a function and applied with `#[apply_decorators(...)]`:

```rust
use corvis::CustomDecorator;

fn admin_only() -> CustomDecorator {
    CustomDecorator::new()
        .set_metadata(ROLES_KEY, [Role::Admin])
        .set_metadata("audit", true)
}

#[routes]
impl UsersController {
    #[delete(":id")]
    #[apply_decorators(admin_only())]
    fn remove(&self, #[param("id")] id: String) { ... }
}
```

- `CustomDecorator::new().set_metadata(k, v)` adds a pair; `.and(other)` concatenates another decorator.
- `apply_decorators([a(), b()])` combines several into one.
- `#[apply_decorators(a(), b())]` accepts several expressions and can be used on a handler or below `#[controller]`.
- Entries keep their declaration order.

`CustomDecorator::new().set_metadata(PUBLIC_METADATA, true)` composes the public marker with other metadata, with the same effect as `#[public]`:

```rust
use corvis::{CustomDecorator, PUBLIC_METADATA};

fn public_health_check() -> CustomDecorator {
    CustomDecorator::new().set_metadata(PUBLIC_METADATA, true).set_metadata("audit", false)
}
```

A `CustomDecorator` can also set the route's status code with the `HTTP_CODE_METADATA` key, as `#[http_code]` would:

```rust
fn created() -> CustomDecorator {
    CustomDecorator::new().set_metadata(corvis::HTTP_CODE_METADATA, 201)
}
```

This is how the `#[api_*]` attributes of `corvis-swagger` work: each `#[api_tags("cats")]` is translated into the `api_tags!("cats")` macro, which produces a `CustomDecorator`. Any attribute whose name starts with `api_` follows that convention, so you can publish your own `api_*!` macros that return a `CustomDecorator`.

::: warning Current limit
A `CustomDecorator` only carries **metadata** (and the status code). It cannot add guards, interceptors, pipes, or filters. To combine, for example, a role with its guard, write both attributes (`#[set_metadata(ROLES_KEY, ...)]` and `#[use_guards(RolesGuard)]`) or register the guard as global and let it read the metadata.
:::
