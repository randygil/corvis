---
status: documented
---

# Async providers

Sometimes a provider can't be created until an asynchronous operation finishes: opening a connection pool, downloading a public key, reading a configuration file. In Corvis the whole container construction is asynchronous, so nothing special is needed: a `Provider::use_factory` factory is an `async` function and startup waits for it to finish.

```rust
use corvis::prelude::*;

#[module(
    providers = [
        Provider::use_factory("JWKS", |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let path: String = config.get_or("JWKS_PATH", "./jwks.json".to_string());
            let raw = corvis::tokio::fs::read_to_string(&path).await?;
            let keys: Value = corvis::serde_json::from_str(&raw)?;
            Ok(keys)
        }),
        KeysService,
    ],
)]
pub struct KeysModule;

#[injectable]
pub struct KeysService {
    #[inject("JWKS")]
    jwks: Arc<Value>,
}
```

Points to keep in mind:

- **It is resolved once.** A factory with the default scope runs only once, at startup, and all consumers receive the same `Arc`.
- **Dependencies wait.** If `KeysService` asks for `"JWKS"`, the container runs the factory first. The order of the `providers` list doesn't matter.
- **Errors stop startup.** An `Err` from the factory makes `CorvisFactory::create` return the error, wrapped as `Error while instantiating JWKS: ...`, and logs it with the `ExceptionHandler` logger. The application never starts listening.
- **Don't block the runtime.** The factory runs inside Tokio: use `tokio::fs`, async clients, or `tokio::task::spawn_blocking` for blocking work.

## A real example: Tertulia's database

`OrmModule::for_root_async` takes an async factory that builds the connection options from `ConfigService`. The module registers those options with `Provider::use_factory` and, from them, creates `OrmService`:

```rust
OrmModule::for_root_async(true, |ctx| async move {
    let config = ctx.get::<ConfigService>().await?;
    let url: String = config.get_or("DATABASE_URL", "file:./tertulia.db".to_string());
    Ok(OrmModuleOptions::new(url)
        .log_queries(config.get_or("LOG_QUERIES", false))
        .push::<(Room, ChatMessage)>())
})
```

`JwtModule::register_async` follows the same pattern to build `JwtService` with the secret that comes from configuration. How to write your own `*_async` methods is covered in [Dynamic modules](/fundamentals/dynamic-modules#async-options).

## Async work after creating the provider

If the provider can be built right away and the asynchronous part is later preparation, implement it in a lifecycle hook instead of in the factory. Tertulia's `RoomsService` creates the default rooms in `on_application_bootstrap`, when all the dependencies already exist:

```rust
#[async_trait]
impl OnApplicationBootstrap for RoomsService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        for (name, topic) in DEFAULT_ROOMS {
            if self.find(name).await?.is_none() {
                // create the room
            }
        }
        Ok(())
    }
}
```

See [Lifecycle events](/fundamentals/lifecycle-events).
