---
status: documented
---

# Dynamic modules

A static module (`#[module(...)]` and nothing else) always registers the same thing. A **dynamic module** completes its definition at the time it is imported: it receives options and, with them, adds providers, imports, controllers, or exports. All of Corvis's configurable modules work this way:

```rust
#[module(
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
        JwtModule::register_async(true, |ctx| async move { /* ... */ }),
        OrmModule::for_root_async(true, |ctx| async move { /* ... */ }),
    ],
)]
pub struct AppModule;
```

Each of those calls returns a `DynamicModule`, and `imports` accepts both module types and `DynamicModule` values.

## Writing a dynamic module

A dynamic module has two parts: the static definition with `#[module]` and an associated method that returns `DynamicModule`. The two are **combined**: what the method adds is added to what the attribute declares.

A notifications module that needs a configurable sender and host:

```rust
use corvis::prelude::*;

#[derive(Clone)]
pub struct MailerOptions {
    pub from: String,
    pub host: String,
}

pub const MAILER_OPTIONS: &str = "MAILER_OPTIONS";

#[injectable]
pub struct MailerService {
    #[inject(MAILER_OPTIONS)]
    options: Arc<MailerOptions>,
}

impl MailerService {
    pub fn sender(&self) -> &str {
        &self.options.from
    }
}

// Static part: always registers and exports MailerService.
#[module(providers = [MailerService], exports = [MailerService])]
pub struct MailerModule;

// Dynamic part: adds the options.
impl MailerModule {
    pub fn for_root(options: MailerOptions) -> DynamicModule {
        DynamicModule::new::<Self>().provider(Provider::use_value(MAILER_OPTIONS, options))
    }
}
```

And it is imported like this:

```rust
#[module(
    imports = [MailerModule::for_root(MailerOptions {
        from: "no-reply@tertulia.dev".into(),
        host: "smtp.local".into(),
    })],
    providers = [SignupService],
)]
pub struct UsersModule;
```

`MailerService` receives `Arc<MailerOptions>` because the options are a provider of the same module. Whoever imports `MailerModule::for_root(...)` receives `MailerService` because it is in the static part's `exports`. The options remain private to the module, unless you export them too.

If the static part and the `DynamicModule` register a provider with the same token, the one added by the `DynamicModule` wins. That makes it possible for `#[module]` to declare a default that `for_root` replaces.

## The `DynamicModule` API

| Method | Effect |
|---|---|
| `DynamicModule::new::<M>()` | Starts from the `#[module]` definition of `M`. |
| `.provider(p)` / `.providers(vec![..])` | Adds providers. |
| `.import(m)` / `.imports(vec![..])` | Adds imports: a `DynamicModule` or `ModuleImport::of::<AnotherModule>()`. |
| `.controller(ControllerDef::of::<C>())` | Adds a controller. |
| `.export(e)` / `.exports(vec![..])` | Adds exports: `Token::of::<T>()`, a text token, or `&provider`. |
| `.global(bool)` | Makes this instance of the module global (or not). If not called, whatever `#[module]` says is used. |

Since the methods receive values, you can decide at runtime what to register:

```rust
impl MailerModule {
    pub fn for_root(options: MailerOptions, global: bool) -> DynamicModule {
        let mut module = DynamicModule::new::<Self>()
            .provider(Provider::use_value(MAILER_OPTIONS, options.clone()))
            .global(global);
        if options.host == "debug" {
            module = module.controller(ControllerDef::of::<MailerDebugController>());
        }
        module
    }
}
```

## Async options

When the options depend on another provider (almost always `ConfigService`), the method receives a factory instead of a value and registers it with `Provider::use_factory`. This is the pattern of `for_root_async` and `register_async`:

```rust
use std::future::Future;
use corvis::ResolveCtx;

impl MailerModule {
    pub fn for_root_async<F, Fut>(factory: F) -> DynamicModule
    where
        F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = corvis::Result<MailerOptions>> + Send + 'static,
    {
        DynamicModule::new::<Self>().provider(Provider::use_factory(MAILER_OPTIONS, factory))
    }
}
```

```rust
#[module(
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
        MailerModule::for_root_async(|ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            Ok(MailerOptions {
                from: config.get_or("MAIL_FROM", "no-reply@tertulia.dev".to_string()),
                host: config.try_get::<String>("SMTP_HOST")?,
            })
        }),
    ],
)]
pub struct AppModule;
```

::: info Where does the factory resolve from?
The factory runs in the context of the dynamic module (`MailerModule`), not that of the module that imports it. `ctx.get::<ConfigService>()` works because `ConfigModule` is global. If the dependency comes from a non-global module, add it to the dynamic module's imports with `.import(...)`; this is what `ScheduleModule::for_root_async(imports, factory)` does with its first argument.
:::

## The built-in dynamic modules

| Module | Methods |
|---|---|
| `ConfigModule` | `for_root(ConfigModuleOptions)`, `for_feature(register_as(..))` |
| `JwtModule` | `register(JwtModuleOptions)`, `register_async(global, factory)` |
| `EventEmitterModule` | `for_root(EventEmitterOptions)` |
| `ScheduleModule` | `for_root()`, `for_root_with(ScheduleModuleOptions)`, `for_root_async(imports, factory)` |
| `CacheModule` | `register(CacheOptions)` |
| `ThrottlerModule` | `for_root(vec![ThrottlerOptions])`, `for_root_storage(ThrottlerStorage)` |
| `ServeStaticModule` | `for_root(vec![ServeStaticOptions])` |
| `MulterModule` | `register(MulterOptions)`, `register_async(factory)` |
| `OrmModule` (`corvis-orm`) | `for_root(OrmModuleOptions)`, `for_root_async(global, factory)`, `for_root_from_config(global)` |

The names follow a convention: `for_root` for something configured once for the whole application, `register` for a module configured where it is imported, and `for_feature` to add a part to something configured with `for_root`. It is only a convention; the container treats them all the same.

## Each call is a new instance

Importing the same static module twice gives a single node in the graph. With dynamic ones this doesn't happen: **each** `DynamicModule` that appears in an `imports` creates a new node, with its own instances. If `UsersModule` and `BillingModule` each call `MailerModule::for_root(...)`, there will be two `MailerService`s.

For a shared configuration, call the method only once (in the root module or in an infrastructure module) and make it global with `.global(true)`, or re-export the dynamic module from a module that the rest import.

One implementation detail: if a module imports the static type (`MailerModule`) and that type was already imported earlier in dynamic form, the static import reuses the existing dynamic node instead of creating another. Don't depend on that order; always import the configured form.

For the same reason, two dynamic modules cannot import each other: each import would create a new instance of the other, forever. Corvis reports it at startup with a `CircularImport` error. See [Dynamic modules are the exception](/fundamentals/circular-dependency#dynamic-modules-are-the-exception).
