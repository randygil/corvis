---
status: documented
---

# Logging

Corvis ships a console logger used by the framework itself (startup, mapped routes, unhandled errors) and by your code. You can change which levels are printed, switch to JSON output or replace it with your own implementation.

## Writing logs

`Logger` is a lightweight value with a context (the text in brackets). The usual approach is to keep it as a field of a provider, initialized with `#[init(...)]`:

```rust
#[injectable]
pub struct CatCreatedListener {
    #[init(Logger::new("CatCreatedListener"))]
    logger: Logger,
}

impl CatCreatedListener {
    fn report(&self, name: &str) {
        self.logger.log(format!("New cat registered: {name}"));
    }
}
```

(From `examples/cats-app`.) `Logger` is not a provider: it is not injected as `Arc<Logger>`, it is constructed. `Logger::for_type::<T>()` uses the type name as the context.

Available levels: `log`, `error`, `warn`, `debug`, `verbose` and `fatal`. There are also functions that need no instance: `Logger::log_with(message, context)`, `Logger::error_with` and `Logger::warn_with`.

The console line has this format:

```text
[Corvis] 18072  - 11/07/2026, 1:20:03 AM     LOG [CatCreatedListener] New cat registered: Tom +2ms
```

The `+Nms` suffix is the time elapsed since the previous line. `error` and `fatal` go to `stderr`; the rest go to `stdout`. Colors are disabled if the `NO_COLOR` variable exists.

## Configuring the logger when creating the application

`CorvisFactory::create_with_options` accepts `CorvisApplicationOptions`, whose `logger` field is a `LoggerOption`:

| Variant | Effect |
|---|---|
| `LoggerOption::Enabled` | Default logger with all levels |
| `LoggerOption::Disabled` | No logs |
| `LoggerOption::Levels(vec![...])` | Only the levels given |
| `LoggerOption::Custom(Box::new(logger))` | Your implementation of `LoggerService` |

```rust
use corvis::{CorvisApplicationOptions, LogLevel, LoggerOption};

let app = CorvisFactory::create_with_options::<AppModule>(CorvisApplicationOptions {
    logger: LoggerOption::Levels(vec![LogLevel::Error, LogLevel::Warn, LogLevel::Log]),
    ..Default::default()
})
.await?;
```

Configure it here and not after creating the application if you want the startup messages (`InstanceLoader`, `RoutesResolver`, ...) to already use that configuration.

## JSON output

`ConsoleLogger` is the default implementation and can be adjusted:

```rust
use corvis::ConsoleLogger;

let logger = ConsoleLogger::new()
    .prefix("Tertulia")  // replaces "[Corvis]"
    .colors(false)
    .json(true);         // one JSON line per entry

let app = CorvisFactory::create_with_options::<AppModule>(CorvisApplicationOptions {
    logger: LoggerOption::Custom(Box::new(logger)),
    ..Default::default()
})
.await?;
```

In JSON mode each entry is `{"level":"log","pid":...,"timestamp":...,"message":"...","context":"..."}`, suitable for log aggregators.

## Custom logger

Implement the `LoggerService` trait, which has a single method:

```rust
use corvis::{LogLevel, LoggerService};

struct StderrLogger;

impl LoggerService for StderrLogger {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        eprintln!("{} {} {}", level.label(), context.unwrap_or("-"), message);
    }
}
```

Register it with `LoggerOption::Custom(Box::new(StderrLogger))` or, after creating the application, with `app.use_logger(StderrLogger)`.

::: info Global state
The logger configuration (implementation, levels, enabled or not) is global to the process, not per application. If you create several applications in the same process, for example in tests, the last configuration applied is the one in effect.
:::

## Other related pieces

- `app.silence_route_logs()` avoids printing the `Mapped {/path, GET} route` lines during `init()`.
- `corvis::LoggingInterceptor` is a built-in interceptor that logs each request with its status and duration (`GET /cats 200 - 3ms`, context `LoggingInterceptor`): `app.use_global_interceptors(corvis::LoggingInterceptor)`.
- In text mode, `ConsoleLogger` escapes line breaks and control characters in the message and the context, so user data cannot fake log lines or send ANSI sequences to the terminal. JSON mode does not modify them (JSON already escapes them).
