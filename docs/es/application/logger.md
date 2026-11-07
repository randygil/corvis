---
status: documentado
---

# Logging

Corvis trae un logger de consola que usan el propio framework (arranque, rutas mapeadas, errores no controlados) y tu código. Puedes cambiar los niveles que se imprimen, pasar a salida JSON o reemplazarlo por tu propia implementación.

## Escribir logs

`Logger` es un valor ligero con un contexto (el texto entre corchetes). Lo habitual es guardarlo como campo de un provider, inicializado con `#[init(...)]`:

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

(De `examples/cats-app`.) `Logger` no es un provider: no se inyecta como `Arc<Logger>`, se construye. `Logger::for_type::<T>()` usa el nombre del tipo como contexto.

Niveles disponibles: `log`, `error`, `warn`, `debug`, `verbose` y `fatal`. También hay funciones sin instancia: `Logger::log_with(mensaje, contexto)`, `Logger::error_with` y `Logger::warn_with`.

La línea de consola tiene este formato:

```text
[Corvis] 18072  - 11/07/2026, 1:20:03 AM     LOG [CatCreatedListener] New cat registered: Tom +2ms
```

El sufijo `+Nms` es el tiempo transcurrido desde la línea anterior. `error` y `fatal` van a `stderr`; el resto, a `stdout`. Los colores se desactivan si existe la variable `NO_COLOR`.

## Configurar el logger al crear la aplicación

`CorvisFactory::create_with_options` acepta `CorvisApplicationOptions`, cuyo campo `logger` es un `LoggerOption`:

| Variante | Efecto |
|---|---|
| `LoggerOption::Enabled` | Logger por defecto con todos los niveles |
| `LoggerOption::Disabled` | Sin logs |
| `LoggerOption::Levels(vec![...])` | Solo los niveles indicados |
| `LoggerOption::Custom(Box::new(logger))` | Tu implementación de `LoggerService` |

```rust
use corvis::{CorvisApplicationOptions, LogLevel, LoggerOption};

let app = CorvisFactory::create_with_options::<AppModule>(CorvisApplicationOptions {
    logger: LoggerOption::Levels(vec![LogLevel::Error, LogLevel::Warn, LogLevel::Log]),
    ..Default::default()
})
.await?;
```

Configúralo aquí y no después de crear la aplicación si quieres que los mensajes del arranque (`InstanceLoader`, `RoutesResolver`, ...) ya usen esa configuración.

## Salida JSON

`ConsoleLogger` es la implementación por defecto y se puede ajustar:

```rust
use corvis::ConsoleLogger;

let logger = ConsoleLogger::new()
    .prefix("Tertulia")  // reemplaza "[Corvis]"
    .colors(false)
    .json(true);         // una línea JSON por entrada

let app = CorvisFactory::create_with_options::<AppModule>(CorvisApplicationOptions {
    logger: LoggerOption::Custom(Box::new(logger)),
    ..Default::default()
})
.await?;
```

En modo JSON cada entrada es `{"level":"log","pid":...,"timestamp":...,"message":"...","context":"..."}`, apta para agregadores de logs.

## Logger propio

Implementa el trait `LoggerService`, que tiene un solo método:

```rust
use corvis::{LogLevel, LoggerService};

struct StderrLogger;

impl LoggerService for StderrLogger {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        eprintln!("{} {} {}", level.label(), context.unwrap_or("-"), message);
    }
}
```

Regístralo con `LoggerOption::Custom(Box::new(StderrLogger))` o, después de crear la aplicación, con `app.use_logger(StderrLogger)`.

::: info Estado global
La configuración del logger (implementación, niveles, activado o no) es global al proceso, no a cada aplicación. Si creas varias aplicaciones en el mismo proceso, por ejemplo en las pruebas, la última configuración aplicada es la que rige.
:::

## Otras piezas relacionadas

- `app.silence_route_logs()` evita imprimir las líneas `Mapped {/ruta, GET} route` durante `init()`.
- `corvis::LoggingInterceptor` es un interceptor incluido que registra cada petición con su estado y duración (`GET /cats 200 - 3ms`, contexto `LoggingInterceptor`): `app.use_global_interceptors(corvis::LoggingInterceptor)`.
- En modo texto, `ConsoleLogger` escapa los saltos de línea y los caracteres de control del mensaje y del contexto, así que un dato del usuario no puede inventar líneas de log ni enviar secuencias ANSI a la terminal. El modo JSON no los modifica (el JSON ya los escapa).
