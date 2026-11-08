---
status: documented
---

# Modules

A **module** groups controllers and providers that work together and decides which of them the rest of the application can use. Every application has a root module, the one passed to `CorvisFactory::create::<AppModule>()`, and from it Corvis walks the `imports` to build the complete graph.

## Declaring a module

A module is a `struct` with the `#[module(...)]` attribute:

```rust
use corvis::prelude::*;

#[module(
    controllers = [RoomsController, StatsController],
    providers = [ChatGateway, RoomsService, FirehoseService],
    exports = [ChatGateway, RoomsService],
)]
pub struct ChatModule;
```

The attribute accepts five keys, all optional:

| Key | Contents |
|---|---|
| `imports` | Other modules whose exports this module needs: types (`AuthModule`) or dynamic modules (`ConfigModule::for_root(...)`). |
| `controllers` | The controllers this module mounts on the router. |
| `providers` | The providers the container creates in this module: `#[injectable]` types or `Provider` expressions (`Provider::use_value(...)`, see [Custom providers](/fundamentals/custom-providers)). |
| `exports` | What this module lets its importers use: provider types, text tokens (`"API_KEY"`), or imported modules (re-export). |
| `global` | Marks the module as global: `global`, `global = true`. |

Each key takes a bracketed list or a single element (`imports = AuthModule`). An unknown key is a compile error.

## Tertulia's root module

```rust
#[module(
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
        EventEmitterModule::for_root(EventEmitterOptions::default()),
        JwtModule::register_async(true, |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let secret = config.get::<String>("JWT_SECRET").filter(|s| !s.trim().is_empty());
            let secret = secret.ok_or_else(|| corvis::anyhow::anyhow!("JWT_SECRET is required"))?;
            Ok(JwtModuleOptions::new(secret).expires_in(config.get_or("JWT_EXPIRES_IN", "12h".to_string())))
        }),
        OrmModule::for_root_async(true, |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let url: String = config.get_or("DATABASE_URL", "file:./tertulia.db".to_string());
            Ok(OrmModuleOptions::new(url).push::<(Room, ChatMessage)>())
        }),
        AuthModule,
        ChatModule,
    ],
)]
pub struct AppModule;
```

The root module has no controllers or providers of its own: it composes the application from four infrastructure modules (configuration, events, JWT, and database, all four global: `EventEmitterOptions::default()` already comes with `global: true`) and two feature modules, `AuthModule` and `ChatModule`. This is the recommended way to organize an application: one module per area, with its controller, its services, and what it exposes.

```text
AppModule
├── ConfigModule        (global)  exports ConfigService
├── EventEmitterModule  (global)  exports EventEmitter
├── JwtModule           (global)  exports JwtService
├── OrmModule           (global)  exports OrmService
├── AuthModule                    AuthController · AuthService → exports AuthService
└── ChatModule                    RoomsController, StatsController · ChatGateway, RoomsService,
                                  FirehoseService → exports ChatGateway, RoomsService
```

## Encapsulation

When a provider or a controller asks for a dependency, the container looks for it **starting from the module that registers it** and only in these places, in this order:

1. The module's own providers.
2. What the modules in its `imports` export, including what those modules re-export.
3. What the global modules export.

Nothing else. In particular:

- A module **does not see** the providers of the module that imports it. `ChatModule` cannot inject anything that `AppModule` registers in its `providers` unless it receives it through an import or a global module.
- A module does not see the providers of a sibling module unless it imports it. `ChatModule` does not import `AuthModule`, so it could not ask for `Arc<AuthService>`; it doesn't need to, because it authenticates with `JwtService`, which comes from `JwtModule` (global).
- Registering a provider does **not** expose it. Without `exports`, a provider is private to its module.
- A global module doesn't expose its providers just by being registered either: only what it puts in `exports` counts.

With these rules, in Tertulia:

| Who asks | What | Where it finds it |
|---|---|---|
| `RoomsController` (`ChatModule`) | `Arc<RoomsService>`, `Arc<ChatGateway>` | Providers of `ChatModule` itself |
| `StatsController` (`ChatModule`) | `Arc<ConfigService>` | Export of `ConfigModule`, which is global |
| `RoomsService` (`ChatModule`) | `Arc<OrmService>` | Export of `OrmModule` (global, thanks to the `true` in `for_root_async`) |
| `RoomsService` (`ChatModule`) | `Arc<EventEmitter>` | Export of `EventEmitterModule`, which is global by default (`EventEmitterOptions::default()`) |
| `AuthService` (`AuthModule`) | `Arc<JwtService>`, `Arc<ConfigService>` | Exports of two global modules |

If a dependency isn't visible, startup fails and the message names the module where it was looked up:

```text
Corvis can't resolve dependencies of the NeedsCats (?). Please make sure that the argument
CatsService at index [0] is available in the BrokenModule context.
```

## Sharing providers

A provider instance belongs to the module that registers it. All the modules that import that module and receive the provider through its `exports` share **the same** instance. In Tertulia, the `RoomsService` used by `ChatGateway` is the same object used by `RoomsController`.

The opposite is also true: if two modules put the same type in their `providers`, each one creates its own instance. To have just one, register it in one module, export it, and import that module wherever you need it.

```rust
#[module(providers = [UsersService], exports = [UsersService])]
pub struct UsersModule;

#[module(imports = [UsersModule], controllers = [AuthController], providers = [AuthService], exports = [AuthService])]
pub struct AuthModule;   // AuthService receives the UsersService from UsersModule
```

Importing the same static module from several places does not create copies: the graph has a single node per module type, so `UsersModule` is instantiated only once even if both `AuthModule` and `AppModule` import it.

## What can be exported

```rust
#[module(
    imports = [CommonModule],
    providers = [
        CatsService,
        Provider::use_value("API_KEY", "secret".to_string()),
    ],
    exports = [
        CatsService,     // one of its own providers, by type
        "API_KEY",       // one of its own providers, by text token
        CommonModule,    // re-export: whoever imports this module sees the exports of CommonModule
    ],
)]
pub struct CatsModule;
```

- A **type** or a **token** must be a provider of the module itself or something one of its `imports` already exports (this is how a single provider is re-exported).
- A **module** can only be exported if it is in the same module's `imports`.

Exporting something that doesn't meet these rules makes startup fail:

```text
Corvis cannot export a provider/module that is not a part of the currently processed module
(CatsModule). Please verify whether the exported X is available in this particular context.
```

Expressions that convert into `ExportItem` are also accepted, such as `Token::of::<CatsService>()` or a `String` with the token name.

## Global modules

A global module makes its exports visible in **all** the application's modules without them having to import it. It is useful for infrastructure that almost everything uses (configuration, database, JWT); for everything else, explicit imports document dependencies better.

```rust
#[module(global, providers = [AuditService], exports = [AuditService])]
pub struct AuditModule;

// equivalent:
#[module(global = true, providers = [AuditService], exports = [AuditService])]
pub struct AuditModule2;

#[module(providers = [AuditService], exports = [AuditService])]
#[global]
pub struct AuditModule3;
```

::: warning Position of `#[global]`
`#[global]` must go **below** `#[module(...)]`. Above it, Rust processes it before the `#[module]` attribute and compilation fails with `cannot find attribute 'global' in this scope`.
:::

Two conditions for a global module to work:

1. **It must be in the graph.** A global module is registered when some module imports it (usually the root). Being global avoids repeating the import, not the first one.
2. **It must export.** Like any module, it only shares what is in `exports`.

Dynamic modules decide whether they are global with `.global(bool)`. The built-in modules expose it as an option: `ConfigModuleOptions { is_global: true, .. }`, `CacheOptions { is_global: true, .. }`, `JwtModule::register_async(true, ..)`, `OrmModule::for_root_async(true, ..)`. `ScheduleModule` is always global.

## The module is also a provider

The module's `struct` is built with the same rules as an `#[injectable]`: its `Arc<T>` fields are injected and it can implement lifecycle hooks. In addition, if it implements `CorvisModule`, its `configure` method registers middleware for the application's routes:

```rust
#[module(imports = [/* ... */], controllers = [AppController], providers = [AppService])]
pub struct AppModule;

impl CorvisModule for AppModule {
    fn configure(&self, consumer: &mut MiddlewareConsumer) {
        consumer.apply::<LoggerMiddleware>().for_routes("*");
    }
}
```

See [Middleware](/middleware).

## Dynamic modules

A dynamic module is a module whose list of providers is completed at the time it is imported, usually from options: `ConfigModule::for_root(...)`, `JwtModule::register_async(...)`, `OrmModule::for_root_async(...)`. It is written with a method that returns `DynamicModule`, and is explained in [Dynamic modules](/fundamentals/dynamic-modules).

Unlike static modules, **each call** to `for_root(...)` or `register(...)` in an `imports` creates a new node in the graph, with its own instances. Call those methods once (in the root module, or in an infrastructure module) and share the result by making it global or exporting it.

## Inspecting the graph

`app.modules_info()` returns the application's modules with their name, whether they are global, the indexes of their imports, and their controllers with the routes. It doesn't need `app.init()`:

```rust
let app = CorvisFactory::create::<AppModule>().await?;
for m in app.modules_info().iter().filter(|m| !m.internal) {
    println!("{} global={} controllers={}", m.name, m.global, m.controllers.len());
}
```

The list includes an internal module (`InternalCoreModule`, with `internal: true`) that provides `Reflector` and `HttpAdapterHost` to the whole application.

## Initialization order

Providers are created at startup, module by module, and the log shows one line per module:

```text
LOG [InstanceLoader] AppModule dependencies initialized +1ms
```

Lifecycle hooks (`on_module_init`, `on_application_bootstrap`, and the shutdown ones) run starting with the modules farthest from the root. See [Lifecycle events](/fundamentals/lifecycle-events).
