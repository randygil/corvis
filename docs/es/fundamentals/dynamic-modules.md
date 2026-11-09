---
status: documented
---

# Módulos dinámicos

Un módulo estático (`#[module(...)]` y nada más) siempre registra lo mismo. Un **módulo dinámico** completa su definición en el momento de importarlo: recibe opciones y, con ellas, añade providers, imports, controladores o exports. Todos los módulos configurables de Corvis funcionan así:

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

Cada una de esas llamadas devuelve un `DynamicModule`, y `imports` acepta tanto tipos de módulo como valores `DynamicModule`.

## Escribir un módulo dinámico

Un módulo dinámico tiene dos partes: la definición estática con `#[module]` y un método asociado que devuelve `DynamicModule`. Las dos se **combinan**: lo que añade el método se suma a lo que declara el atributo.

Un módulo de notificaciones que necesita un remitente y un host configurables:

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

// Parte estática: siempre registra y exporta MailerService.
#[module(providers = [MailerService], exports = [MailerService])]
pub struct MailerModule;

// Parte dinámica: añade las opciones.
impl MailerModule {
    pub fn for_root(options: MailerOptions) -> DynamicModule {
        DynamicModule::new::<Self>().provider(Provider::use_value(MAILER_OPTIONS, options))
    }
}
```

Y se importa así:

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

`MailerService` recibe `Arc<MailerOptions>` porque las opciones son un provider del mismo módulo. Quien importa `MailerModule::for_root(...)` recibe `MailerService` porque está en los `exports` de la parte estática. Las opciones quedan privadas del módulo, salvo que también las exportes.

Si la parte estática y el `DynamicModule` registran un provider con el mismo token, gana el que añade el `DynamicModule`. Así `#[module]` puede declarar un valor por defecto que `for_root` reemplaza.

## La API de `DynamicModule`

| Método | Efecto |
|---|---|
| `DynamicModule::new::<M>()` | Parte de la definición de `#[module]` de `M`. |
| `.provider(p)` / `.providers(vec![..])` | Añade providers. |
| `.import(m)` / `.imports(vec![..])` | Añade imports: un `DynamicModule` o `ModuleImport::of::<OtroModulo>()`. |
| `.controller(ControllerDef::of::<C>())` | Añade un controlador. |
| `.export(e)` / `.exports(vec![..])` | Añade exports: `Token::of::<T>()`, un token de texto o `&provider`. |
| `.global(bool)` | Hace global (o no) esta instancia del módulo. Sin llamarlo, se usa lo que diga `#[module]`. |

Como los métodos reciben valores, puedes decidir en tiempo de ejecución qué registrar:

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

## Opciones asíncronas

Cuando las opciones dependen de otro provider (casi siempre `ConfigService`), el método recibe una fábrica en lugar de un valor y la registra con `Provider::use_factory`. Es el patrón de `for_root_async` y `register_async`:

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

::: info ¿Desde dónde resuelve la fábrica?
La fábrica se ejecuta en el contexto del módulo dinámico (`MailerModule`), no en el del módulo que lo importa. `ctx.get::<ConfigService>()` funciona porque `ConfigModule` es global. Si la dependencia viene de un módulo no global, añádelo a los imports del módulo dinámico con `.import(...)`; es lo que hace `ScheduleModule::for_root_async(imports, factory)` con su primer argumento.
:::

## Convención de las variantes asíncronas

Toda variante asíncrona se llama como su método síncrono más `_async` (`for_root` pasa a `for_root_async`, `register` a `register_async`). Sus argumentos son `(is_global, fábrica)` cuando el módulo tiene una opción `is_global`, y `(fábrica)` cuando el módulo es siempre global o nunca lo es. Los imports extra para la fábrica van en el `DynamicModule` devuelto con `.import(...)`.

`is_global` es un argumento, no un campo de las opciones que devuelve la fábrica: la globalidad se decide antes de que corra cualquier fábrica, así que un `is_global` dentro de las opciones devueltas se ignora y manda el argumento.

```rust
CacheModule::register_async(true, |ctx| async move {
    let config = ctx.get::<ConfigService>().await?;
    Ok(CacheOptions { ttl: config.try_get::<u64>("CACHE_TTL")?, ..Default::default() })
})
.import(SettingsModule::for_root()) // solo si la fábrica necesita un módulo no global
```

Si la fábrica devuelve un error, la aplicación no arranca y el mensaje es `Error while instantiating <token>: <causa>`. `ScheduleModule::for_root_async(imports, fábrica)` es anterior a la convención y es la única excepción: recibe sus imports como primer argumento.

## Los módulos dinámicos incluidos

| Módulo | Métodos |
|---|---|
| `ConfigModule` | `for_root(ConfigModuleOptions)`, `for_root_async(is_global, fábrica)`, `for_feature(register_as(..))` |
| `JwtModule` | `register(JwtModuleOptions)`, `register_async(global, fábrica)` |
| `EventEmitterModule` | `for_root(EventEmitterOptions)` |
| `ScheduleModule` | `for_root()`, `for_root_with(ScheduleModuleOptions)`, `for_root_async(imports, fábrica)` |
| `CacheModule` | `register(CacheOptions)`, `register_async(is_global, fábrica)` |
| `ThrottlerModule` | `for_root(vec![ThrottlerOptions])`, `for_root_storage(ThrottlerStorage)`, `for_root_async(fábrica)` |
| `ServeStaticModule` | `for_root(vec![ServeStaticOptions])` |
| `MulterModule` | `register(MulterOptions)`, `register_async(fábrica)` |
| `OrmModule` (`corvis-orm`) | `for_root(OrmModuleOptions)`, `for_root_async(global, fábrica)`, `for_root_from_config(global)` |

Los nombres siguen una convención: `for_root` para algo que se configura una vez para toda la aplicación, `register` para un módulo que se configura donde se importa y `for_feature` para añadir una parte a algo configurado con `for_root`. Es solo una convención; el contenedor los trata igual.

## Cada llamada es una instancia nueva

Importar dos veces el mismo módulo estático da un único nodo en el grafo. Con los dinámicos no ocurre: **cada** `DynamicModule` que aparece en un `imports` crea un nodo nuevo, con sus propias instancias. Si `UsersModule` y `BillingModule` llaman cada uno a `MailerModule::for_root(...)`, habrá dos `MailerService`.

Para una configuración compartida, llama al método una sola vez (en el módulo raíz o en un módulo de infraestructura) y hazlo global con `.global(true)`, o re-exporta el módulo dinámico desde un módulo que el resto importe.

Un detalle de la implementación: si un módulo importa el tipo estático (`MailerModule`) y ese tipo ya se importó antes en forma dinámica, el import estático reutiliza el nodo dinámico existente en lugar de crear otro. No dependas de ese orden; importa siempre la forma configurada.

Por la misma razón, dos módulos dinámicos no pueden importarse mutuamente: cada import crearía una instancia nueva del otro, sin fin. Corvis lo informa al arrancar con un error `CircularImport`. Ver [Los módulos dinámicos son la excepción](/es/fundamentals/circular-dependency#los-modulos-dinamicos-son-la-excepcion).
