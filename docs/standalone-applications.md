---
status: documented
---

# Standalone applications

Not every application needs an HTTP server. A worker that processes scheduled tasks, a data migration script, or a console tool can still take advantage of modules, dependency injection, and configuration. For that, you create the application without calling `listen`.

## Creating the application without a server

```rust
use corvis::prelude::*;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.init().await?;

    let reports = app.get::<ReportsService>()?;
    reports.generate_monthly().await?;

    app.close().await
}
```

| Step | What happens |
|---|---|
| `CorvisFactory::create::<M>()` | Walks the module graph and creates all providers (except transient ones) |
| `app.init()` | Registers the `#[on_event]`, `#[cron]`, `#[interval]`, and `#[timeout]` handlers, and runs `on_module_init` and `on_application_bootstrap`. It also prepares the routes, but opens no port |
| `app.get::<T>()` | Returns an already created provider |
| `app.close()` | Stops scheduled tasks and runs `on_module_destroy`, `before_application_shutdown`, and `on_application_shutdown` |

## Getting providers

| Method | Use |
|---|---|
| `app.get::<T>()` | A provider by its type: `app.get::<ConfigService>()?` |
| `app.get_token::<T>("TOKEN")` | A provider registered with a token: `app.get_token::<String>("API_KEY")?` |
| `app.module_ref()` | A `ModuleRef` of the root module, with `get`, `get_strict`, `resolve`, and `create` (see [Module reference](/fundamentals/module-ref)) |

`app.get` looks first in the root module's context and then across the whole application, so it finds providers from modules the root does not import directly. If you care about a specific provider of a module, use `ModuleRef::get_strict` from that module.

## A worker

An application without HTTP that only runs scheduled tasks needs to stay alive. With `ScheduleModule` and `#[handlers]` (see [Task scheduling](/application/task-scheduling)):

```rust
#[injectable]
pub struct CleanupService {
    #[init(Logger::new("Cleanup"))]
    logger: Logger,
}

#[handlers]
impl CleanupService {
    #[cron(CronExpression::EVERY_HOUR)]
    async fn purge(&self) {
        self.logger.log("Purging expired sessions");
    }
}

#[module(imports = [ScheduleModule::for_root()], providers = [CleanupService])]
pub struct WorkerModule;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<WorkerModule>().await?;
    app.init().await?;
    corvis::tokio::signal::ctrl_c().await?;
    app.close().await
}
```

## `create_application_context`

`CorvisFactory::create_application_context::<M>()` is a shortcut that creates the application and runs the initialization hooks (`on_module_init`, `on_application_bootstrap`) without preparing routes or gateways.

::: warning
`create_application_context` does not register the `#[on_event]`, `#[cron]`, `#[interval]`, or `#[timeout]` handlers: that registration happens in `app.init()`. If your standalone application uses events or scheduled tasks, create the application with `CorvisFactory::create` and call `app.init()`, as in the examples above.
:::

## Creation options

`CorvisFactory::create_with_options` accepts `CorvisApplicationOptions`. It is useful in scripts to silence the startup log:

```rust
use corvis::{CorvisApplicationOptions, LoggerOption};

let app = CorvisFactory::create_with_options::<AppModule>(CorvisApplicationOptions {
    logger: LoggerOption::Disabled,
    ..Default::default()
})
.await?;
```

`LoggerOption` also supports `Levels(vec![LogLevel::Error, LogLevel::Warn])` and `Custom(Box::new(my_logger))`. With `abort_on_error: true`, an error while creating the application ends the process with exit code 1 instead of being returned.

## See also

- [Commander](/recipes/commander): a console tool with arguments.
- [Testing](/fundamentals/testing): `Test::create_testing_module()` to test modules in isolation.
