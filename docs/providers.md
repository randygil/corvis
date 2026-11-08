---
status: documented
---

# Providers

A **provider** is any value that the dependency injection container knows how to create and hand to whoever needs it: services, repositories, clients for other APIs, configurations, factories. Controllers don't create their dependencies; they declare them, and Corvis resolves them when the application starts.

On this page you will see the most common form, a `struct` marked with `#[injectable]`. The variants (values, factories, aliases, trait objects) are in [Custom providers](/fundamentals/custom-providers).

## A service

In Tertulia, `RoomsService` keeps the rooms and the message history. It is used by both the WebSocket gateway and the REST controller:

```rust
use corvis::prelude::*;
use corvis_orm::{OrmError, OrmService};

#[injectable]
pub struct RoomsService {
    db: Arc<OrmService>,
    events: Arc<EventEmitter>,
}

impl RoomsService {
    pub async fn find(&self, name: &str) -> Result<Option<Room>, OrmError> {
        self.db.room().find_unique(json!({ "where": { "name": name } })).await
    }

    pub async fn create(&self, dto: CreateRoomDto, created_by: &str) -> Result<Room, OrmError> {
        let room: Room = self.db.room().create(json!({ "data": { /* ... */ } })).await?;
        self.events.emit("room.created", room.clone());
        Ok(room)
    }
}
```

`#[injectable]` generates the implementation of the `Injectable` trait: an async constructor that asks the container for each field. You don't write `new()`; the fields **are** the list of dependencies.

For the container to know about it, the service is registered in a module's `providers`:

```rust
#[module(
    controllers = [RoomsController, StatsController],
    providers = [ChatGateway, RoomsService, FirehoseService],
    exports = [ChatGateway, RoomsService],
)]
pub struct ChatModule;
```

## How each field is resolved

The generated constructor looks at the type of each field:

| Field | What it receives |
|---|---|
| `Arc<T>` | The provider registered with the token `T`. If it isn't visible from the module, startup fails with an error that names the field. |
| `Option<Arc<T>>` | The provider if it is visible; `None` if not. This is how you declare an optional dependency. |
| `ModuleRef` | A reference to the container, located in the module that owns the provider. See [Module reference](/fundamentals/module-ref). |
| `#[inject("TOKEN")] field: Arc<T>` | The provider registered with the text token `"TOKEN"`. |
| `#[inject("TOKEN")] field: T` | The same, but cloning the value (`T: Clone`). Useful for small values like a `String` or a `u16`. |
| `#[init(expr)] field: T` | The result of evaluating `expr`. It doesn't go through the container. |
| `#[default] field: T` | `T::default()`. |
| Any other type | `T::default()`, if `T` implements `Default`; otherwise it is a compile error. |

An example that combines several forms (`ChatGateway`, from Tertulia):

```rust
#[websocket_gateway(path = "/chat")]
pub struct ChatGateway {
    #[websocket_server]
    server: WsServer,
    jwt: Arc<JwtService>,              // by type, comes from JwtModule (global)
    rooms: Arc<RoomsService>,          // by type, from the same module
    firehose: Arc<FirehoseService>,
    #[init(Logger::new("ChatGateway"))]
    logger: Logger,                    // its own value, no container
}
```

And one with a text token and internal state:

```rust
#[injectable]
pub struct CatsService {
    #[default]
    cats: Mutex<Vec<Cat>>,
    #[inject("PREFIX")]
    prefix: String,                    // Provider::use_value("PREFIX", "Sir ".to_string())
}
```

::: tip Mutable state
A default provider is a singleton shared among all its consumers through `Arc`, so it receives `&self`. State that changes goes inside a `Mutex`, `RwLock`, atomics, or an `Arc` of shared state, like `cats: Mutex<Vec<Cat>>` in the example above.
:::

::: info `#[optional]`
The `#[optional]` attribute is accepted on fields but changes nothing: what makes a dependency optional is the `Option<Arc<T>>` type.
:::

## Tokens

Each provider is identified by a `Token`:

- `Token::of::<T>()`: a type's token. It is the one used by `providers = [RoomsService]` and the `Arc<RoomsService>` fields.
- `Token::named("API_KEY")`: a text token. `&str` and `String` convert into a `Token` on their own, which is why `Provider::use_value("API_KEY", ...)` and `#[inject("API_KEY")]` work without writing `Token::named`.

Two type tokens are equal if the type is the same; two text tokens, if the text is the same. A type token never matches a text one.

## Registration and lifecycle

- **When they are created.** `CorvisFactory::create` walks the module graph and creates all the instances before returning the application (except *transient* providers, which are created when injected). A missing dependency or a cycle is detected at startup, not on the first request.
- **How many instances.** One per provider and per module that registers it. If two modules put `RoomsService` in their `providers`, each will have its own. To share a single one, register it in one module and export it (see [Modules](/modules)).
- **Scopes.** Singleton by default. `#[injectable(scope = "transient")]` creates one instance per consumer. There is no request scope. See [Injection scopes](/fundamentals/injection-scopes).
- **Hooks.** If the struct implements `OnModuleInit`, `OnApplicationBootstrap`, `OnModuleDestroy`, `BeforeApplicationShutdown`, or `OnApplicationShutdown`, Corvis detects it without registering it anywhere. `RoomsService` uses `OnApplicationBootstrap` to create the default rooms:

```rust
#[async_trait]
impl OnApplicationBootstrap for RoomsService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        for (name, topic) in DEFAULT_ROOMS {
            if self.find(name).await?.is_none() {
                self.db.room().create(json!({ "data": { "name": name, "topic": topic, "createdBy": "tertulia" } })).await?;
            }
        }
        Ok(())
    }
}
```

More details in [Lifecycle events](/fundamentals/lifecycle-events).

## When a dependency is missing

If an `Arc<T>` field can't be resolved, startup fails with a message that indicates the provider, the argument's position (marked with `?`), and the module:

```text
Corvis can't resolve dependencies of the NeedsCats (?). Please make sure that the argument
CatsService at index [0] is available in the BrokenModule context.
```

There are three usual causes: the provider is not in the module's `providers`, it is in another module that doesn't export it, or that other module is not in the `imports`. Fields with `#[init]` or `#[default]` appear as `-` in the list.

## Getting a provider outside injection

From `main` or from a test you can ask the application for an already created instance:

```rust
let app = CorvisFactory::create::<AppModule>().await?;
let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 5150);
```

`app.get::<T>()` looks up by type and `app.get_token::<T>("TOKEN")` by text token. Inside a provider, use a `ModuleRef` field (see [Module reference](/fundamentals/module-ref)).

## Guards, pipes, and interceptors are providers too

Guards, pipes, interceptors, filters, and middleware that you reference by type (`#[use_guards(AuthGuard)]`) are built with the same mechanism, so they can have `Arc<T>` fields. If they are registered as providers of the module, that instance is used; if not, Corvis builds them in the context of the controller's module and keeps the instance. That is why Tertulia's `AuthGuard` can receive `Arc<JwtService>`:

```rust
#[injectable]
pub struct AuthGuard {
    jwt: Arc<JwtService>,
}
```
