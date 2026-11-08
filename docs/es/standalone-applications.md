---
status: documented
---

# Aplicaciones standalone

No toda aplicación necesita un servidor HTTP. Un worker que procesa tareas programadas, un script de migración de datos o una herramienta de consola pueden aprovechar igual los módulos, la inyección de dependencias y la configuración. Para eso se crea la aplicación sin llamar a `listen`.

## Crear la aplicación sin servidor

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

| Paso | Qué ocurre |
|---|---|
| `CorvisFactory::create::<M>()` | Recorre el grafo de módulos y crea todos los providers (salvo los transient) |
| `app.init()` | Registra los handlers `#[on_event]`, `#[cron]`, `#[interval]` y `#[timeout]`, y ejecuta `on_module_init` y `on_application_bootstrap`. También prepara las rutas, pero no abre ningún puerto |
| `app.get::<T>()` | Devuelve un provider ya creado |
| `app.close()` | Detiene las tareas programadas y ejecuta `on_module_destroy`, `before_application_shutdown` y `on_application_shutdown` |

## Obtener providers

| Método | Uso |
|---|---|
| `app.get::<T>()` | Un provider por su tipo: `app.get::<ConfigService>()?` |
| `app.get_token::<T>("TOKEN")` | Un provider registrado con un token: `app.get_token::<String>("API_KEY")?` |
| `app.module_ref()` | Un `ModuleRef` del módulo raíz, con `get`, `get_strict`, `resolve` y `create` (ver [Referencia al módulo](/es/fundamentals/module-ref)) |

`app.get` busca primero en el contexto del módulo raíz y después en toda la aplicación, así que encuentra providers de módulos que el raíz no importa directamente. Si te interesa un provider concreto de un módulo, usa `ModuleRef::get_strict` desde ese módulo.

## Un worker

Una aplicación sin HTTP que solo ejecuta tareas programadas necesita mantenerse viva. Con `ScheduleModule` y `#[handlers]` (ver [Tareas programadas](/es/application/task-scheduling)):

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

`CorvisFactory::create_application_context::<M>()` es un atajo para un worker o script que no necesita rutas ni gateways. Crea la aplicación, registra los handlers `#[on_event]`, `#[cron]`, `#[interval]` y `#[timeout]`, ejecuta `on_module_init`, arranca las tareas programadas y ejecuta `on_application_bootstrap`, en el mismo orden que `app.init()`. `app.close()` detiene las tareas.

```rust
#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create_application_context::<WorkerModule>().await?;
    corvis::tokio::signal::ctrl_c().await?;
    app.close().await
}
```

Llamar a `app.init()` sobre esa aplicación está permitido y nunca registra los handlers dos veces.

Un script de una sola ejecución que importa un módulo con handlers `#[cron]`, `#[interval]` o `#[timeout]` también los ejecutará mientras viva. Para evitarlo, desactiva un tipo con `ScheduleModule::for_root_with(ScheduleModuleOptions { cron_jobs: false, ..Default::default() })` (los campos son `cron_jobs`, `intervals` y `timeouts`), o usa un módulo raíz que no importe `ScheduleModule`.

## Opciones de creación

`CorvisFactory::create_with_options` acepta `CorvisApplicationOptions`. Es útil en scripts para silenciar el log de arranque:

```rust
use corvis::{CorvisApplicationOptions, LoggerOption};

let app = CorvisFactory::create_with_options::<AppModule>(CorvisApplicationOptions {
    logger: LoggerOption::Disabled,
    ..Default::default()
})
.await?;
```

`LoggerOption` también admite `Levels(vec![LogLevel::Error, LogLevel::Warn])` y `Custom(Box::new(mi_logger))`. Con `abort_on_error: true`, un error al crear la aplicación termina el proceso con código 1 en lugar de devolverse.

## Ver también

- [Commander](/es/recipes/commander): una herramienta de consola con argumentos.
- [Testing](/es/fundamentals/testing): `Test::create_testing_module()` para probar módulos aislados.
