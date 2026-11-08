---
status: documented
---

# Providers personalizados

Poner un tipo en `providers = [RoomsService]` es la forma corta de registrar un provider: el token es el propio tipo y el contenedor construye el valor con el constructor que genera `#[injectable]`. Cuando necesitas otra cosa (un valor ya construido, una fábrica, un token de texto, una implementación detrás de un trait o un alias), usas los constructores de `Provider`.

Todos devuelven un `Provider`, que se pone tal cual en la lista `providers` de un `#[module]` o se pasa a `DynamicModule::provider(...)`.

| Constructor | Token | El consumidor pide |
|---|---|---|
| `Provider::class::<T>()` | `T` | `Arc<T>` |
| `Provider::use_class::<T>(token)` | `token` | `Arc<T>` |
| `Provider::use_class_as::<T, dyn Trait>(token, \|r\| r)` | `token` | `Arc<dyn Trait>` |
| `Provider::use_value(token, valor)` | `token` | `Arc<T>` (o `T` si es `Clone`) |
| `Provider::value(valor)` | el tipo del valor | `Arc<T>` |
| `Provider::use_value_arc(token, arc)` | `token` | `Arc<T>`, también `Arc<dyn Trait>` |
| `Provider::use_factory(token, \|ctx\| async move { .. })` | `token` | `Arc<T>` |
| `Provider::use_factory_arc(token, \|ctx\| async move { .. })` | `token` | `Arc<T>`, también `Arc<dyn Trait>` |
| `Provider::use_existing(alias, existente)` | `alias` | lo mismo que `existente` |

El `token` puede ser un `&str`/`String` (token de texto) o un `Token` (`Token::of::<T>()`, `Token::named("...")`).

## Valores: `use_value`

Registra un valor que ya existe. Sirve para constantes, configuración y para sustituir un servicio por un doble en las pruebas.

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
    prefix: String,          // se clona del Arc<String> registrado
}
```

El consumidor puede pedir el valor como `Arc<String>` o, si el tipo es `Clone`, directamente como `String`.

`Provider::value(valor)` es un atajo que usa el tipo del valor como token: `Provider::value(AppConfig { .. })` se inyecta en un campo `Arc<AppConfig>`.

## Fábricas: `use_factory`

Una fábrica es una función asíncrona que construye el valor. Recibe un `ResolveCtx`, con el que pide las dependencias que necesita (el equivalente de la lista `inject` de otros contenedores):

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

La fábrica devuelve `corvis::Result<T>` (un `anyhow::Result`), así que `?` funciona con cualquier error. Si falla, el arranque se detiene con `Error while instantiating DB_URL: <causa>`.

Lo que ofrece `ResolveCtx`:

| Método | Uso |
|---|---|
| `ctx.get::<T>().await?` | Un provider por tipo. |
| `ctx.get_token::<T>("TOKEN").await?` | Un provider por token. |
| `ctx.get_optional::<T>("TOKEN").await?` | `Option<Arc<T>>`: `None` si el token no es visible. |
| `ctx.module_ref()` | Un [`ModuleRef`](/es/fundamentals/module-ref) situado en el módulo de la fábrica. |
| `ctx.module_name()` | El nombre del módulo que está resolviendo. |

Las dependencias se buscan con las mismas reglas de encapsulación que un campo `Arc<T>`: desde el módulo donde está registrada la fábrica (ver [Módulos](/es/modules#encapsulacion)).

Las fábricas también admiten `async` real: abrir una conexión, leer un archivo o llamar a otra API antes de que arranque la aplicación. Ver [Providers asíncronos](/es/fundamentals/async-providers).

## Clases con otro token: `use_class`

`Provider::use_class::<T>(token)` construye `T` con su constructor de `#[injectable]` (inyectando sus campos), pero lo registra bajo otro token:

```rust
providers = [Provider::use_class::<SmtpMailer>("MAILER")]

#[injectable]
pub struct NotificationsService {
    #[inject("MAILER")]
    mailer: Arc<SmtpMailer>,
}
```

El consumidor sigue pidiendo el tipo concreto. Para elegir la implementación según el entorno, combina un trait con `use_class_as`.

## Implementaciones detrás de un trait: `use_class_as`

Rust no tiene clases abstractas, pero sí trait objects. `use_class_as` construye `T` y lo entrega como `Arc<dyn Trait>`:

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

El segundo argumento convierte `Arc<MemoryUserRepo>` en `Arc<dyn UserRepo>`; con `|r| r` basta. Para cambiar de implementación solo cambias esa línea (o la eliges en un [módulo dinámico](/es/fundamentals/dynamic-modules)).

`use_class_as` se comporta como `use_class` respecto de `T`: conserva su scope, y sus hooks del ciclo de vida, los `#[handlers]` (`#[on_event]`, `#[cron]`...) y los metadatos de gateway se ejecutan sobre la instancia concreta. Si `MemoryUserRepo` implementa `OnModuleInit`, el hook se ejecuta una vez y el estado que fija es visible a través del `Arc<dyn UserRepo>`. Igual que con `use_class`, los providers transient no tienen hooks de ciclo de vida.

Con un valor ya construido, usa `use_value_arc`; con una fábrica, `use_factory_arc`:

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

(`MemoryUserRepo::default()` requiere `#[derive(Default)]` en el struct; `PgUserRepo` es ilustrativo.)

## Alias: `use_existing`

`Provider::use_existing(alias, existente)` registra un segundo token que entrega **la misma instancia** que otro provider:

```rust
providers = [
    CatsService,
    Provider::use_existing("CATS", Token::of::<CatsService>()),
]

#[injectable]
pub struct CatsReport {
    #[inject("CATS")]
    cats: Arc<CatsService>,   // el mismo Arc que recibe quien pide Arc<CatsService>
}
```

El token existente se busca desde el módulo donde está registrado el alias, con las reglas habituales de visibilidad.

## Exportar providers personalizados

Un provider personalizado se exporta por su token:

```rust
#[module(
    providers = [Provider::use_value("API_KEY", "secret".to_string()), CatsService],
    exports = ["API_KEY", CatsService],
)]
pub struct CatsModule;
```

Para un token de tipo, usa el tipo (`exports = [CatsService]`) o `Token::of::<T>()`.

## Scope

Cualquier provider puede cambiar de scope con `.scope(...)`:

```rust
Provider::use_class::<RequestIdGenerator>("ID_GEN").scope(Scope::Transient)
```

`use_class` y `use_class_as` heredan el scope declarado en `#[injectable(scope = "transient")]`. Ver [Scopes de inyección](/es/fundamentals/injection-scopes).

## Enhancers globales como providers

Cuatro constructores registran un guard, interceptor, pipe o filtro **global** que se construye con inyección de dependencias:

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

`Provider::app_guard::<T>()`, `app_interceptor::<T>()`, `app_pipe::<T>()` y `app_filter::<T>()` exigen que `T` sea `#[injectable]` e implemente el trait correspondiente. A diferencia de `app.use_global_guards(...)`, que recibe una instancia ya creada, aquí el contenedor construye el enhancer y le inyecta sus dependencias (`AuthGuard` recibe `Arc<JwtService>`).

Orden: los enhancers registrados como providers se aplican **antes** que los de `use_global_*` (los envuelven), y los filtros registrados así tienen prioridad sobre los de `use_global_filters`. Igual que los `use_global_*`, solo se aplican a las rutas HTTP, no a los gateways de WebSockets.
