---
status: documented
---

# Lifecycle events

A Corvis application goes through three phases: **startup** (the container is created and providers are instantiated), **running** (it serves requests), and **shutdown**. At the boundaries between phases Corvis calls *hooks* that your providers, controllers, and modules can implement to prepare or release resources.

## The hooks

Each hook is an async trait:

| Trait | Method | When it is called |
|---|---|---|
| `OnModuleInit` | `on_module_init(&self)` | On initialization, with all dependencies already resolved |
| `OnApplicationBootstrap` | `on_application_bootstrap(&self)` | After all the `on_module_init`, before accepting connections |
| `OnModuleDestroy` | `on_module_destroy(&self)` | When the application closes (`app.close()` or a signal) |
| `BeforeApplicationShutdown` | `before_application_shutdown(&self, signal: Option<String>)` | After all the `on_module_destroy` |
| `OnApplicationShutdown` | `on_application_shutdown(&self, signal: Option<String>)` | At the end of shutdown |

All of them return `corvis::Result<()>`. Implementing the trait is enough: `#[injectable]`, `#[controller]`, and `#[module]` detect the implementation at compile time, with no additional registration.

```rust
use corvis::prelude::*;

#[injectable]
pub struct RoomsService {
    db: Arc<OrmService>,
    events: Arc<EventEmitter>,
}

#[async_trait]
impl OnApplicationBootstrap for RoomsService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        for (name, topic) in DEFAULT_ROOMS {
            if self.find(name).await?.is_none() {
                self.db
                    .room()
                    .create(json!({ "data": { "name": name, "topic": topic, "createdBy": "tertulia" } }))
                    .await?;
            }
        }
        Ok(())
    }
}
```

(`examples/tertulia/src/rooms/rooms_service.rs`: creates the default rooms at startup.)

In `examples/tertulia/src/firehose.rs`, `FirehoseService` uses `on_application_bootstrap` to register an `upgrade` listener on the HTTP server: at that point the container is already complete and the application is not accepting connections yet.

## The full sequence

### Startup

1. `CorvisFactory::create::<AppModule>()` walks the module graph and creates all providers and controllers (except those with `Transient` scope, which are created when injected). If something fails here, `create` returns the error.
2. `app.init()` (which `listen` calls for you if you didn't) registers the routes, middleware, gateways, and event handlers, and then:
   1. calls `on_module_init` on all modules;
   2. mounts the scheduled tasks (`#[cron]`, `#[interval]`, `#[timeout]`);
   3. calls `on_application_bootstrap` on all modules.
3. `listen` opens the socket and starts serving requests.

After a successful `init()`, calling it again (directly, or through `listen`, `handle` or `TestingModule::init`) does nothing: the hooks are not repeated.

After a **failed** `init()` the application cannot be started again. The first call returns the original error; every later call returns `corvis::InitFailedError`, whose message says the application failed to initialize earlier, quotes the first error and asks you to create a new application. Hooks that already ran (open connections, spawned tasks) cannot be undone, so retrying on the same application would either repeat them or skip them. Create a new application with `CorvisFactory::create` instead. You can recognize the error with `err.downcast_ref::<corvis::InitFailedError>()`. A `listen` that fails to bind after a successful `init()` is not an init failure: you can call it again on another address.

### Shutdown

`app.close().await` (or a signal, see below) runs:

1. `on_module_destroy` on all modules;
2. stops the scheduled tasks;
3. `before_application_shutdown(signal)`;
4. `on_application_shutdown(signal)`.

`close()` is also idempotent.

### Order between modules

Hooks are walked module by module, starting with those **farthest** from the root module (the most deeply imported) and ending at the root. Within each module the order is: controllers, providers, and finally the module's own class. Shutdown follows the same order as startup.

If a hook returns `Err`, the walk stops and the error comes out of `init()`, `listen()`, or `close()`. After an `init()` failure, see [the startup sequence](#startup) for what happens on a retry.

### Which instances receive hooks

- Providers with the default scope (singleton) and controllers.
- The module class (`#[module]` is also a provider): a module can implement `OnModuleInit` for configuration tasks.
- `Transient` providers do **not** receive them (there is no single instance to notify), nor do guards, pipes, interceptors, or filters that Corvis builds on demand without being in `providers`.

## System signals

By default the shutdown hooks only run if you call `app.close()`. For the application to react to system signals, enable it before listening:

```rust
#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = tertulia::create_app().await?;
    app.enable_shutdown_hooks();
    let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 5150);
    app.listen(format!("127.0.0.1:{port}")).await
}
```

With `enable_shutdown_hooks()`, on receiving the signal:

1. the server stops accepting connections and waits for the ones in progress to finish;
2. the shutdown sequence runs, with `signal` equal to `Some("SIGINT")` or `Some("SIGTERM")`;
3. `listen` returns.

On Unix, `SIGINT` (Ctrl+C) and `SIGTERM` are listened for. On Windows only Ctrl+C, which is reported as `"SIGINT"`. When you close with `app.close()`, `signal` is `None`.

```rust
#[async_trait]
impl BeforeApplicationShutdown for JobsService {
    async fn before_application_shutdown(&self, signal: Option<String>) -> corvis::Result<()> {
        self.logger.log(format!("Shutting down ({})", signal.as_deref().unwrap_or("manual")));
        self.flush().await
    }
}
```

::: info Connections during shutdown
When shutdown is started by a signal, the HTTP connections have already finished before `on_module_destroy` is called. Hooks are for closing your own resources (queues, database connections, files), not for continuing to serve requests.
:::

## Contexts without a server

`CorvisFactory::create_application_context::<M>()` creates the container and runs `on_module_init` and `on_application_bootstrap` without registering routes or opening sockets. It is the basis of scripts and workers (see [Standalone applications](/standalone-applications)). Close with `app.close().await` so the shutdown hooks run.

`create_application_context` also registers the `#[handlers]` handlers (`#[on_event]`, `#[cron]`, `#[interval]`, `#[timeout]`) before `on_module_init` and starts the scheduled ones before `on_application_bootstrap`, so a worker receives its events and runs its jobs. `app.close().await` stops them. Calling `app.init()` afterwards never registers them twice.

In tests, `TestingModule::init()` and `TestingModule::close()` trigger the same phases (see [Testing](/fundamentals/testing)).
