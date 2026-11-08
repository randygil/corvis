---
status: documented
---

# Custom providers

Putting a type in `providers = [RoomsService]` is the short way to register a provider: the token is the type itself and the container builds the value with the constructor that `#[injectable]` generates. When you need something else (an already built value, a factory, a text token, an implementation behind a trait, or an alias), you use the `Provider` constructors.

All of them return a `Provider`, which goes as is in the `providers` list of a `#[module]` or is passed to `DynamicModule::provider(...)`.

| Constructor | Token | The consumer asks for |
|---|---|---|
| `Provider::class::<T>()` | `T` | `Arc<T>` |
| `Provider::use_class::<T>(token)` | `token` | `Arc<T>` |
| `Provider::use_class_as::<T, dyn Trait>(token, \|r\| r)` | `token` | `Arc<dyn Trait>` |
| `Provider::use_value(token, value)` | `token` | `Arc<T>` (or `T` if `Clone`) |
| `Provider::value(value)` | the value's type | `Arc<T>` |
| `Provider::use_value_arc(token, arc)` | `token` | `Arc<T>`, also `Arc<dyn Trait>` |
| `Provider::use_factory(token, \|ctx\| async move { .. })` | `token` | `Arc<T>` |
| `Provider::use_factory_arc(token, \|ctx\| async move { .. })` | `token` | `Arc<T>`, also `Arc<dyn Trait>` |
| `Provider::use_existing(alias, existing)` | `alias` | the same as `existing` |

The `token` can be a `&str`/`String` (text token) or a `Token` (`Token::of::<T>()`, `Token::named("...")`).

## Values: `use_value`

Registers a value that already exists. It is useful for constants, configuration, and for replacing a service with a test double.

```rust
#[module(
    controllers = [CatsController],
    providers = [CatsService, Provider::use_value("PREFIX", "Sir ".to_string())],
    exports = [CatsService],
)]
pub struct CatsModule;

#[injectable]
pub struct CatsService {
    #[inject("PREFIX")]
    prefix: String,          // cloned from the registered Arc<String>
}
```

The consumer can ask for the value as `Arc<String>` or, if the type is `Clone`, directly as `String`.

`Provider::value(value)` is a shortcut that uses the value's type as the token: `Provider::value(AppConfig { .. })` is injected into an `Arc<AppConfig>` field.

## Factories: `use_factory`

A factory is an async function that builds the value. It receives a `ResolveCtx`, with which it asks for the dependencies it needs (the equivalent of the `inject` list in other containers):

```rust
#[module(
    imports = [ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() })],
    providers = [
        Provider::use_factory("DB_URL", |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            Ok(config.get_or("DATABASE_URL", "sqlite::memory:".to_string()))
        }),
        ReportsService,
    ],
)]
pub struct ReportsModule;
```

The factory returns `corvis::Result<T>` (an `anyhow::Result`), so `?` works with any error. If it fails, startup stops with `Error while instantiating DB_URL: <cause>`.

What `ResolveCtx` offers:

| Method | Use |
|---|---|
| `ctx.get::<T>().await?` | A provider by type. |
| `ctx.get_token::<T>("TOKEN").await?` | A provider by token. |
| `ctx.get_optional::<T>("TOKEN").await?` | `Option<Arc<T>>`: `None` if the token isn't visible. |
| `ctx.module_ref()` | A [`ModuleRef`](/fundamentals/module-ref) located in the factory's module. |
| `ctx.module_name()` | The name of the module being resolved. |

Dependencies are looked up with the same encapsulation rules as an `Arc<T>` field: from the module where the factory is registered (see [Modules](/modules#encapsulation)).

Factories also support real `async`: opening a connection, reading a file, or calling another API before the application starts. See [Async providers](/fundamentals/async-providers).

## Classes with another token: `use_class`

`Provider::use_class::<T>(token)` builds `T` with its `#[injectable]` constructor (injecting its fields), but registers it under another token:

```rust
providers = [Provider::use_class::<SmtpMailer>("MAILER")]

#[injectable]
pub struct NotificationsService {
    #[inject("MAILER")]
    mailer: Arc<SmtpMailer>,
}
```

The consumer still asks for the concrete type. To choose the implementation depending on the environment, combine a trait with `use_class_as`.

## Implementations behind a trait: `use_class_as`

Rust has no abstract classes, but it does have trait objects. `use_class_as` builds `T` and hands it over as `Arc<dyn Trait>`:

```rust
pub trait UserRepo: Send + Sync {
    fn find(&self, id: u32) -> Option<String>;
}

#[injectable]
pub struct MemoryUserRepo {
    #[default]
    users: std::sync::RwLock<Vec<String>>,
}

impl UserRepo for MemoryUserRepo {
    fn find(&self, id: u32) -> Option<String> {
        self.users.read().unwrap().get(id as usize).cloned()
    }
}

#[injectable]
pub struct UsersService {
    #[inject("USER_REPO")]
    repo: Arc<dyn UserRepo>,
}

#[module(
    providers = [
        Provider::use_class_as::<MemoryUserRepo, dyn UserRepo>("USER_REPO", |r| r),
        UsersService,
    ],
    exports = [UsersService],
)]
pub struct UsersModule;
```

The second argument converts `Arc<MemoryUserRepo>` into `Arc<dyn UserRepo>`; `|r| r` is enough. To switch implementations you only change that line (or choose it in a [dynamic module](/fundamentals/dynamic-modules)).

::: warning Hooks and `use_class_as`
`use_class_as` keeps `T`'s scope but does not register its lifecycle hooks: if `MemoryUserRepo` implements `OnModuleInit`, that hook does not run when it is registered with `use_class_as`. Use `use_class` or `Provider::class` if you need the hooks.
:::

With an already built value, use `use_value_arc`; with a factory, `use_factory_arc`:

```rust
Provider::use_value_arc("USER_REPO", Arc::new(FakeUserRepo::default()) as Arc<dyn UserRepo>)

Provider::use_factory_arc("USER_REPO", |ctx| async move {
    let config = ctx.get::<ConfigService>().await?;
    let repo: Arc<dyn UserRepo> = if config.get_or("IN_MEMORY", false) {
        Arc::new(MemoryUserRepo::default())
    } else {
        Arc::new(PgUserRepo::connect(&config).await?)
    };
    Ok(repo)
})
```

(`MemoryUserRepo::default()` requires `#[derive(Default)]` on the struct; `PgUserRepo` is illustrative.)

## Aliases: `use_existing`

`Provider::use_existing(alias, existing)` registers a second token that hands over **the same instance** as another provider:

```rust
providers = [
    CatsService,
    Provider::use_existing("CATS", Token::of::<CatsService>()),
]

#[injectable]
pub struct CatsReport {
    #[inject("CATS")]
    cats: Arc<CatsService>,   // the same Arc that whoever asks for Arc<CatsService> receives
}
```

The existing token is looked up from the module where the alias is registered, with the usual visibility rules.

## Exporting custom providers

A custom provider is exported by its token:

```rust
#[module(
    providers = [Provider::use_value("API_KEY", "secret".to_string()), CatsService],
    exports = ["API_KEY", CatsService],
)]
pub struct CatsModule;
```

For a type token, use the type (`exports = [CatsService]`) or `Token::of::<T>()`.

## Scope

Any provider can change scope with `.scope(...)`:

```rust
Provider::use_class::<RequestIdGenerator>("ID_GEN").scope(Scope::Transient)
```

`use_class` and `use_class_as` inherit the scope declared in `#[injectable(scope = "transient")]`. See [Injection scopes](/fundamentals/injection-scopes).

## Global enhancers as providers

Four constructors register a **global** guard, interceptor, pipe, or filter that is built with dependency injection:

```rust
#[module(
    imports = [/* ... */],
    providers = [
        Provider::app_guard::<ThrottlerGuard>(),
        Provider::app_guard::<AuthGuard>(),
        Provider::app_guard::<RolesGuard>(),
    ],
)]
pub struct AppModule;
```

`Provider::app_guard::<T>()`, `app_interceptor::<T>()`, `app_pipe::<T>()`, and `app_filter::<T>()` require `T` to be `#[injectable]` and implement the corresponding trait. Unlike `app.use_global_guards(...)`, which takes an already created instance, here the container builds the enhancer and injects its dependencies (`AuthGuard` receives `Arc<JwtService>`).

Order: enhancers registered as providers are applied **before** those from `use_global_*` (they wrap them), and filters registered this way take priority over those from `use_global_filters`. Like the `use_global_*` ones, they only apply to HTTP routes, not to WebSocket gateways.
