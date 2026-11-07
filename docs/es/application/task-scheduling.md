---
status: documentado
---

# Tareas programadas

`ScheduleModule` ejecuta métodos de tus providers en momentos fijos: con una expresión cron, cada cierto intervalo o una sola vez tras un retardo. El cálculo de fechas sigue al paquete `cron` 4 de Node.js: misma sintaxis, mismas zonas horarias y mismos mensajes de error.

## Registrar el módulo

```rust
#[module(imports = [ScheduleModule::for_root()])]
pub struct AppModule;
```

El módulo es global y registra `SchedulerRegistry`. Variantes:

- `ScheduleModule::for_root_with(ScheduleModuleOptions { cron_jobs, intervals, timeouts })` desactiva tipos de handlers (todos están activos por defecto).
- `ScheduleModule::for_root_async(imports, |ctx| async move { ... })` calcula esas opciones con una fábrica asíncrona que puede leer otros providers.

## Declarar tareas

Los handlers se declaran en un bloque `#[handlers]` de un provider registrado:

```rust
#[injectable]
pub struct TasksService {
    #[init(Logger::new("TasksService"))]
    logger: Logger,
}

#[handlers]
impl TasksService {
    #[cron(CronExpression::EVERY_MINUTE, name = "heartbeat")]
    async fn heartbeat(&self) {
        self.logger.debug("Called every minute");
    }

    #[timeout(1_000)]
    async fn warmup(&self) {
        self.logger.log("Warm-up finished (1s after bootstrap)");
    }
}
```

(De `examples/cats-app/src/tasks.rs`.)

| Atributo | Cuándo se ejecuta |
|---|---|
| `#[cron(expr, ...)]` | Según la expresión cron |
| `#[interval(ms)]` / `#[interval("nombre", ms)]` | Cada `ms` milisegundos, empezando tras el primer intervalo |
| `#[timeout(ms)]` / `#[timeout("nombre", ms)]` | Una vez, `ms` milisegundos después del arranque |

El método puede ser `async` o síncrono y devolver `()` o un `Result`. Un `Err` o un `panic` se registran con el logger `Scheduler` y la tarea sigue programada.

### Expresiones cron

Se aceptan expresiones de 5 o 6 campos (`[segundo] minuto hora día-mes mes día-semana`), con rangos, listas, pasos, alias (`jan`..`dec`, `sun`..`sat`), día de la semana `0` a `7` (`0` y `7` son domingo) y presets como `@daily` o `@weekdays`. Si restringes a la vez el día del mes y el día de la semana, basta con que coincida uno de los dos.

`CronExpression` trae constantes listas para usar: `EVERY_SECOND`, `EVERY_30_SECONDS`, `EVERY_MINUTE`, `EVERY_HOUR`, `EVERY_DAY_AT_MIDNIGHT`, `EVERY_WEEKDAY`, `MONDAY_TO_FRIDAY_AT_9AM`, entre otras.

### Opciones de `#[cron]`

```rust
#[handlers]
impl ReportsService {
    #[cron("0 30 9,17 * * *", name = "fx", time_zone = "America/Caracas")]
    async fn refresh(&self) -> corvis::Result<()> {
        Ok(())
    }
}
```

| Opción | Efecto |
|---|---|
| `name = "..."` | Nombre en el registro (sin nombre recibe un UUID v4) |
| `time_zone = "..."` | Zona IANA (`"Europe/Madrid"`, sin distinguir mayúsculas) o `"UTC±H[:MM]"` |
| `utc_offset = -240` | Desfase fijo en minutos (excluyente con `time_zone`) |
| `disabled = true` | Registra la tarea sin arrancarla |
| `wait_for_completion = true` | Si la ejecución anterior sigue en curso, se salta el tick |
| `threshold = ms` | Avisa con `[Cron] Missed execution deadline` si un tick llega tarde más de `ms` |
| `initial_delay = ms` | Retrasa el arranque de la tarea |
| `unref_timeout = bool` | Se acepta por compatibilidad |

También puedes pasar una expresión `CronOptions` como segundo argumento: `#[cron(EXPR, CronOptions::new().name("x").wait_for_completion(true))]`.

Sin `time_zone` ni `utc_offset` se usa la zona del proceso: la variable `TZ` si existe, o la del sistema. Los saltos y repeticiones del horario de verano se resuelven como en el paquete original.

### Solapamiento

Las ejecuciones pueden solaparse: un `#[interval]` lanza cada ejecución aunque la anterior no haya terminado, igual que `setInterval`, y un `#[cron]` hace lo mismo salvo con `wait_for_completion = true`.

## Ciclo de vida

- Los handlers se recogen al inicializar la aplicación. Dos tareas del mismo tipo con el mismo nombre hacen fallar el arranque con `Interval with the given name (x) already exists. Ignored.` (o `Cron Job`, `Timeout`).
- Se ponen en marcha justo antes de los hooks `on_application_bootstrap` (primero los timeouts, luego los intervalos y al final los cron).
- Se detienen al cerrar la aplicación, antes de `before_application_shutdown`.

Ver [Eventos del ciclo de vida](/es/fundamentals/lifecycle-events).

## `SchedulerRegistry`: tareas en tiempo de ejecución

Inyecta `Arc<SchedulerRegistry>` para consultar, detener o crear tareas por nombre:

```rust
use corvis::schedule::{set_interval, set_timeout};

#[injectable]
pub struct ReportsService {
    registry: Arc<SchedulerRegistry>,
}

impl ReportsService {
    pub fn pause(&self) -> corvis::Result<()> {
        self.registry.get_cron_job("fx")?.stop();
        Ok(())
    }

    pub fn schedule_cleanup(&self) -> corvis::Result<()> {
        let job = CronJob::new("0 0 3 * * *", || async {
            Logger::new("Cleanup").log("running");
        })?;
        self.registry.add_cron_job("cleanup", job.clone())?;
        job.start();

        self.registry.add_interval("sync", set_interval(120_000, || async { /* ... */ }))?;
        self.registry.add_timeout("once", set_timeout(5_000, || async { /* ... */ }))?;
        Ok(())
    }
}
```

| Método | Descripción |
|---|---|
| `get_cron_job(n)`, `get_interval(n)`, `get_timeout(n)` | Error `No Cron Job was found with the given name (n)...` si no existe |
| `add_cron_job(n, job)`, `add_interval(n, timer)`, `add_timeout(n, timer)` | Error si el nombre ya existe |
| `delete_cron_job(n)`, `delete_interval(n)`, `delete_timeout(n)` | Detiene y elimina |
| `get_cron_jobs()` | Mapa nombre → `CronJob` en orden de inserción |
| `get_intervals()`, `get_timeouts()` | Nombres registrados |
| `does_exist("cron" \| "interval" \| "timeout", n)` | Comprueba si existe |

`add_cron_job` no arranca el job: llama a `start()` cuando quieras que empiece. `set_interval` y `set_timeout` devuelven un `TimerRef` con `clear()` e `is_pending()`.

### `CronJob`

`CronJob` es un port del objeto homónimo del paquete `cron`. Se crea con `CronJob::new(expr, closure)` o, con opciones, con `CronJob::from(CronJobParams::new(expr, closure).time_zone("UTC").start(true))`. Métodos principales: `start`, `stop`, `stop_and_wait`, `is_active`, `is_callback_running`, `next_date`, `next_dates(n)`, `last_date`, `fire_on_tick`, `add_callback` y `set_time(CronTime)`. `CronJobParams` admite `on_complete`, `run_on_init`, `wait_for_completion`, `error_handler` y `threshold`, y acepta una fecha en lugar de una expresión para un job que se ejecuta una sola vez.

Las funciones `corvis::schedule::send_at`, `timeout` y `validate_cron_expression` calculan la próxima fecha, los milisegundos que faltan o validan una expresión sin crear un job.

## Límites

- Las tareas viven en el proceso: si ejecutas varias instancias de la aplicación, cada una dispara sus propias tareas. Para que una tarea corra una sola vez en un clúster necesitas coordinación externa (ver [Bloqueos distribuidos](/es/reliability/locks)).
- No hay persistencia: un tick que cae mientras la aplicación está detenida no se recupera.
